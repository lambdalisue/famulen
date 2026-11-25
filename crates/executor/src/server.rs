//! The executor's own process.

use std::io::Read as _;
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use anyhow::Context as _;
use serde_json::json;
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader,
};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};

use crate::journal::{self, Journal, NotKept};
use crate::process;
use crate::proof::{self, Side};
use crate::socket::Listener;
use crate::wire::{NONE, PROTOCOLS, Reply, Request, Spec, What, Why};

/// The first argument that has a program be an executor.
pub const COMMAND: &str = "--be-an-executor";
/// What the process that started it is told on stdout, with nothing after it
/// or why it could not start.
pub(crate) const STARTED: &str = "famulen-executor:";
/// How long whoever connects may take to say by which key.
const HELLO_WITHIN: Duration = Duration::from_secs(10);
/// How long the last of what it wrote is waited for once it is gone, which
/// another process it left may keep open, and how long readers may take it.
const SETTLE: Duration = Duration::from_secs(5);
/// How long taking in whoever connects rests after it failed.
const ACCEPT_AGAIN: Duration = Duration::from_millis(100);
/// How often it looks whether what it keeps is still there.
const LOOK_EVERY: Duration = Duration::from_secs(1);
/// The longest line taken from whoever connects.
const LONGEST: u64 = 64 * 1024 * 1024;

/// Be an executor: read what to do from stdin, do it, and end with it. The
/// caller's process does nothing else.
pub fn execute() -> anyhow::Result<()> {
    // The terminal and the session of whoever started it are not its own.
    #[cfg(unix)]
    rustix::process::setsid().ok();
    let mut spec = String::new();
    std::io::stdin().read_to_string(&mut spec)?;
    let spec: Spec = serde_json::from_str(&spec).context("reading what to do")?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let result = runtime.block_on(run(spec));
    if let Err(error) = &result {
        tell(Some(&format!("{error:#}")));
    }
    runtime.shutdown_background();
    result
}

fn tell(error: Option<&str>) {
    use std::io::Write as _;
    let mut out = std::io::stdout();
    // Nobody reads it once the start is over.
    writeln!(out, "{STARTED}{}", json!(error)).ok();
    out.flush().ok();
}

struct Shared {
    key: String,
    dir: std::path::PathBuf,
    journal: Mutex<Journal>,
    stdin: mpsc::UnboundedSender<String>,
    ending: watch::Sender<Option<(Why, Duration)>>,
}

impl Shared {
    fn journal(&self) -> MutexGuard<'_, Journal> {
        self.journal.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// End the program; the first reason stands.
    fn end(&self, why: Why, grace: Duration) {
        self.ending.send_if_modified(|ending| {
            let first = ending.is_none();
            if first {
                *ending = Some((why, grace));
            }
            first
        });
    }

    fn keep(&self, what: What) -> Result<bool, NotKept> {
        let kept = self.journal().keep(what);
        match &kept {
            Err(NotKept::Full) => self.end(Why::WroteTooMuch, Duration::ZERO),
            Err(NotKept::Failed) => self.end(Why::NotKept, Duration::ZERO),
            Ok(_) | Err(NotKept::Closed) => {}
        }
        kept
    }
}

/// What it made, taken away again unless the program started.
struct Made(Option<std::path::PathBuf>);

impl Drop for Made {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            std::fs::remove_dir_all(dir).ok();
        }
    }
}

async fn run(spec: Spec) -> anyhow::Result<()> {
    // Told to end from here on, it takes along what it started.
    let mut signalled = Signalled::listen()?;
    // Of two started on the same work, one fails here.
    crate::socket::private_dir(false)
        .create(&spec.dir)
        .with_context(|| format!("could not make {}", spec.dir.display()))?;
    let mut made = Made(Some(spec.dir.clone()));
    std::env::set_current_dir(&spec.dir)?;
    let mut listener =
        Listener::bind().with_context(|| format!("could not listen in {}", spec.dir.display()))?;
    let mut journal = Journal::create(&spec.dir, spec.limit_bytes)?;
    let mut child = start(&spec)?;
    let pid = child.id().context("the program has no number")?;
    let begun = journal.keep(What::Start {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        pid,
        born: process::born(pid),
        boot: process::boot(),
        meta: spec.meta.clone(),
    });
    if begun.is_err() {
        kill(&mut child, pid, true);
        anyhow::bail!("could not write to {}", spec.dir.display());
    }
    made.0 = None;

    let (stdin_tx, stdin_rx) = mpsc::unbounded_channel();
    let shared = Arc::new(Shared {
        key: spec.key,
        dir: spec.dir,
        journal: Mutex::new(journal),
        stdin: stdin_tx,
        ending: watch::Sender::new(None),
    });
    if let Some(stdin) = child.stdin.take() {
        tokio::spawn(write(stdin, stdin_rx));
    }
    let mut outputs = tokio::task::JoinSet::new();
    if let Some(stdout) = child.stdout.take() {
        outputs.spawn(read(stdout, shared.clone(), |line| What::Out { line }));
    }
    if let Some(stderr) = child.stderr.take() {
        outputs.spawn(read(stderr, shared.clone(), |line| What::Err { line }));
    }
    for (at, line) in spec.input.into_iter().enumerate() {
        put(&shared, format!("input:{at}"), line).ok();
    }
    tell(None);

    let connections = Arc::new(());
    let serving = tokio::spawn({
        let (shared, connections) = (shared.clone(), connections.clone());
        async move {
            loop {
                // Nobody could ask ever again if it gave up.
                let Ok(stream) = listener.accept().await else {
                    tokio::time::sleep(ACCEPT_AGAIN).await;
                    continue;
                };
                let (shared, connection) = (shared.clone(), connections.clone());
                tokio::spawn(async move {
                    serve(stream, &shared).await.ok();
                    drop(connection);
                });
            }
        }
    });

    let mut ending = shared.ending.subscribe();
    let (why, code) = tokio::select! {
        status = child.wait() => (Why::Ended, status.ok().and_then(|s| s.code())),
        why = async {
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(spec.limit_secs)) => {
                    (Why::TookTooLong, Duration::ZERO)
                }
                () = lost(&shared.dir) => (Why::Stopped, Duration::ZERO),
                () = signalled.wait() => (Why::Signalled, Duration::ZERO),
                found = ending.wait_for(Option::is_some) => {
                    found.ok().and_then(|f| *f).unwrap_or((Why::Stopped, Duration::ZERO))
                }
            }
        } => {
            let (why, grace) = why;
            if !grace.is_zero() {
                kill(&mut child, pid, false);
                tokio::time::timeout(grace, child.wait()).await.ok();
            }
            (why, None)
        }
    };
    // Whatever it started ends with it, even when it ended by itself.
    kill(&mut child, pid, true);
    let code = match code {
        Some(code) => Some(code),
        None => child.wait().await.ok().and_then(|s| s.code()),
    };
    tokio::time::timeout(SETTLE, async {
        while outputs.join_next().await.is_some() {}
    })
    .await
    .ok();
    shared.journal().keep(What::Exit { why, code }).ok();
    // Those reading on get the rest, and nobody new is taken in.
    serving.abort();
    let _ = serving.await;
    tokio::time::timeout(SETTLE, async {
        while Arc::strong_count(&connections) > 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .ok();
    Ok(())
}

fn start(spec: &Spec) -> anyhow::Result<Child> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .envs(&spec.env)
        .current_dir(&spec.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    command
        .spawn()
        .with_context(|| format!("could not start {}", spec.program))
}

/// Wait until what it keeps is taken away, and with it every way to ask it.
async fn lost(dir: &std::path::Path) {
    while journal::path(dir).exists() {
        tokio::time::sleep(LOOK_EVERY).await;
    }
}

/// End the program and every process it started, at once or as it likes.
fn kill(child: &mut Child, pid: u32, at_once: bool) {
    process::end_group(pid, at_once);
    if at_once {
        child.start_kill().ok();
    }
}

/// Told when the system has it end.
struct Signalled {
    #[cfg(unix)]
    told: tokio::signal::unix::Signal,
}

impl Signalled {
    /// From here on it is not ended at once.
    fn listen() -> std::io::Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            told: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
        })
    }

    async fn wait(&mut self) {
        #[cfg(unix)]
        self.told.recv().await;
        #[cfg(not(unix))]
        std::future::pending::<()>().await;
    }
}

/// Keep a line and pass it on to the program, unless one was by this key.
fn put(shared: &Shared, key: String, line: String) -> Result<bool, NotKept> {
    let fresh = shared.keep(What::In {
        key,
        line: line.clone(),
    })?;
    if fresh {
        shared.stdin.send(line).ok();
    }
    Ok(fresh)
}

async fn write(mut stdin: impl AsyncWrite + Unpin, mut lines: mpsc::UnboundedReceiver<String>) {
    while let Some(line) = lines.recv().await {
        let written = async {
            stdin.write_all(format!("{line}\n").as_bytes()).await?;
            stdin.flush().await
        };
        // A program that reads no more ends by itself, or is stopped.
        if written.await.is_err() {
            break;
        }
    }
}

async fn read(from: impl AsyncRead + Unpin, shared: Arc<Shared>, what: fn(String) -> What) {
    let mut lines = BufReader::new(from);
    let mut line = Vec::new();
    loop {
        line.clear();
        match lines.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\n', '\r']).to_owned();
        // Read on even when nothing is kept any more, as a program that
        // cannot write does not end.
        shared.keep(what(text)).ok();
    }
}

async fn next(
    lines: &mut (impl tokio::io::AsyncBufRead + Unpin),
) -> anyhow::Result<Option<Request>> {
    let mut line = Vec::new();
    let read = lines.take(LONGEST).read_until(b'\n', &mut line).await?;
    if read == 0 {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&line)?))
}

async fn say(to: &mut (impl AsyncWrite + Unpin), reply: &Reply) -> anyhow::Result<()> {
    let line = format!("{}\n", serde_json::to_string(reply)?);
    to.write_all(line.as_bytes()).await?;
    to.flush().await?;
    Ok(())
}

async fn serve(stream: crate::socket::Stream, shared: &Shared) -> anyhow::Result<()> {
    let (from, mut to) = tokio::io::split(stream);
    let mut from = BufReader::new(from);
    let Some(protocol) =
        tokio::time::timeout(HELLO_WITHIN, greet(&mut from, &mut to, shared)).await??
    else {
        return say(&mut to, &Reply::error("who asks is not known")).await;
    };
    say(&mut to, &Reply::default()).await?;
    while let Some(request) = next(&mut from).await? {
        let fresh = |kept: Result<bool, NotKept>| match kept {
            Ok(fresh) => Reply {
                fresh: Some(fresh),
                ..Reply::default()
            },
            Err(_) => Reply::error("the work is over"),
        };
        let reply = match request {
            Request::Stop { grace_ms } => {
                shared.end(Why::Stopped, Duration::from_millis(grace_ms));
                Reply::default()
            }
            _ if protocol == NONE => Reply::error("no protocol is shared"),
            Request::Write { key, line } => fresh(put(shared, key, line)),
            Request::Mark { key } => fresh(shared.keep(What::Mark { key })),
            Request::Attach { after } => {
                let reply = Reply {
                    kept: Some(*shared.journal().watch().borrow()),
                    ..Reply::default()
                };
                say(&mut to, &reply).await?;
                return tokio::select! {
                    told = tell_all(&mut to, shared, after) => told,
                    // Whoever went away is told no more.
                    _ = from.read_u8() => Ok(()),
                };
            }
            Request::Hello { .. } | Request::Prove { .. } | Request::Unknown => {
                Reply::error("not known here")
            }
        };
        say(&mut to, &reply).await?;
    }
    Ok(())
}

/// The protocol to speak with whoever shows to have the key.
async fn greet(
    from: &mut (impl tokio::io::AsyncBufRead + Unpin),
    to: &mut (impl AsyncWrite + Unpin),
    shared: &Shared,
) -> anyhow::Result<Option<u32>> {
    let Some(Request::Hello {
        from: oldest,
        to: newest,
        nonce: asks,
    }) = next(from).await?
    else {
        return Ok(None);
    };
    let answers = proof::nonce();
    let protocol = crate::wire::shared(PROTOCOLS, oldest, newest);
    let reply = Reply {
        protocol: Some(protocol),
        proof: Some(proof::of(&shared.key, Side::Answers, &asks, &answers)),
        nonce: Some(answers.clone()),
        ..Reply::default()
    };
    say(to, &reply).await?;
    let proved = match next(from).await? {
        Some(Request::Prove { proof }) => {
            proof::holds(&shared.key, Side::Asks, &asks, &answers, &proof)
        }
        _ => false,
    };
    Ok(proved.then_some(protocol))
}

async fn tell_all(
    to: &mut (impl AsyncWrite + Unpin),
    shared: &Shared,
    after: u64,
) -> anyhow::Result<()> {
    let mut kept = shared.journal().watch();
    let file = tokio::fs::File::open(journal::path(&shared.dir)).await?;
    let mut lines = BufReader::new(file);
    let mut line = Vec::new();
    let mut seq = 0;
    loop {
        // Only lines that are there whole are read.
        while seq < *kept.borrow_and_update() {
            line.clear();
            lines.read_until(b'\n', &mut line).await?;
            seq += 1;
            if seq > after {
                to.write_all(&line).await?;
            }
        }
        to.flush().await?;
        if shared.journal().is_closed() && seq == *kept.borrow() {
            return Ok(());
        }
        kept.changed().await?;
    }
}
