//! Famulen's side: starting executors, finding them again and asking them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context as _, bail};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, ReadHalf, WriteHalf};
use tokio::process::Command;

use crate::proof::{self, Side};
use crate::server::STARTED;
use crate::socket::{self, Stream};
use crate::wire::{NONE, PROTOCOLS, Record, Reply, Request, Spec, What, shared};
use crate::{journal, process};

/// How long an executor may take to answer, or to start.
const ANSWERS_WITHIN: Duration = Duration::from_secs(20);

/// How long one that did not start in time may take to end.
const ENDS_WITHIN: Duration = Duration::from_secs(10);
/// How often whoever hangs up before it is known is asked again, and after
/// how long: it may be ending, or have waited too long.
const HELLO_TRIES: u32 = 3;
const HELLO_AGAIN: Duration = Duration::from_millis(100);

/// How an executor is started: a program that calls [`crate::execute`].
#[derive(Debug, Clone)]
pub struct Program {
    pub path: PathBuf,
    pub args: Vec<String>,
}

/// Work to start an executor on.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Start {
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// Variables of this process the program must not have.
    pub hidden_env: Vec<String>,
    pub cwd: PathBuf,
    pub input: Vec<String>,
    pub limit: Duration,
    pub limit_bytes: u64,
    pub meta: Value,
}

/// How the executor of some work stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// There is none, and nothing is left of one.
    None,
    Working,
    /// It works, and speaks no protocol that is spoken here.
    Foreign,
    /// Its work is over, and what was kept of it is there.
    Over,
    /// It went away with its work under way.
    Gone,
}

/// Every executor of one Famulen.
#[derive(Debug, Clone)]
pub struct Executors {
    dir: PathBuf,
    key: String,
    program: Program,
}

impl Executors {
    /// Those kept in `dir`, asked by `key`.
    pub fn new(dir: impl Into<PathBuf>, key: impl Into<String>, program: Program) -> Self {
        Self {
            dir: dir.into(),
            key: key.into(),
            program,
        }
    }

    /// The names of work something is kept of.
    pub fn names(&self) -> std::io::Result<Vec<String>> {
        let entries = match std::fs::read_dir(&self.dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            entries => entries?,
        };
        let mut names = Vec::new();
        for entry in entries {
            names.extend(entry?.file_name().into_string().ok());
        }
        names.sort();
        Ok(names)
    }

    pub fn of(&self, name: impl AsRef<str>) -> Executor {
        Executor {
            dir: self.dir.join(name.as_ref()),
            key: self.key.clone(),
        }
    }

    /// Start an executor on work by a name nothing is kept under, and return
    /// once its program runs.
    pub async fn start(&self, name: impl AsRef<str>, start: Start) -> anyhow::Result<Executor> {
        let executor = self.of(name);
        socket::private_dir(true)
            .create(&self.dir)
            .with_context(|| format!("could not make {}", self.dir.display()))?;
        let spec = Spec {
            key: self.key.clone(),
            dir: std::path::absolute(&executor.dir)?,
            program: start.program,
            args: start.args,
            env: start.env,
            cwd: start.cwd,
            input: start.input,
            limit_secs: start.limit.as_secs(),
            limit_bytes: start.limit_bytes,
            meta: start.meta,
        };
        let mut command = Command::new(&self.program.path);
        command
            .args(&self.program.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // It holds no directory in use but its own.
        #[cfg(unix)]
        command.current_dir("/");
        #[cfg(windows)]
        command.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        for name in &start.hidden_env {
            command.env_remove(name);
        }
        let mut child = command.spawn().with_context(|| {
            format!(
                "could not start the executor {}",
                self.program.path.display()
            )
        })?;
        let mut stdin = child.stdin.take().context("no stdin")?;
        let mut lines = BufReader::new(child.stdout.take().context("no stdout")?).lines();
        let told = tokio::time::timeout(ANSWERS_WITHIN, async {
            stdin.write_all(&serde_json::to_vec(&spec)?).await?;
            drop(stdin);
            while let Some(line) = lines.next_line().await? {
                if let Some(told) = line.strip_prefix(STARTED) {
                    return Ok(serde_json::from_str::<Option<String>>(told)?);
                }
            }
            bail!("the executor ended before it started anything")
        })
        .await
        .context("the executor did not start in time")
        .flatten();
        if told.is_err() {
            // What it may have started is not to go on unknown.
            if let Some(pid) = child.id() {
                process::ask_to_end(pid);
            }
            tokio::time::timeout(ENDS_WITHIN, child.wait()).await.ok();
            child.kill().await.ok();
            executor.forget().await.ok();
        }
        // It is its own from here; only its end is waited for.
        tokio::spawn(async move { child.wait().await });
        match told? {
            Some(why) => bail!(why),
            None => Ok(executor),
        }
    }
}

/// The executor of one piece of work.
#[derive(Debug, Clone)]
pub struct Executor {
    dir: PathBuf,
    key: String,
}

impl Executor {
    /// `None` when nobody listens any more.
    async fn connect(&self) -> anyhow::Result<Option<Connection>> {
        let connecting = async {
            for _ in 0..HELLO_TRIES {
                let stream = match socket::connect(&self.dir).await {
                    Err(error) if socket::is_gone(&error) => return Ok(None),
                    stream => stream.context("could not reach the executor")?,
                };
                if let Some(stream) = stream
                    && let Some(connection) = self.greet(stream).await?
                {
                    return Ok(Some(connection));
                }
                tokio::time::sleep(HELLO_AGAIN).await;
            }
            bail!("the executor hangs up")
        };
        tokio::time::timeout(ANSWERS_WITHIN, connecting)
            .await
            .context("the executor does not answer")?
    }

    /// `None` when it hangs up.
    async fn greet(&self, stream: Stream) -> anyhow::Result<Option<Connection>> {
        let (from, to) = tokio::io::split(stream);
        let mut connection = Connection {
            from: BufReader::new(from),
            to,
            protocol: NONE,
        };
        let asks = proof::nonce();
        let hello = Request::Hello {
            from: *PROTOCOLS.start(),
            to: *PROTOCOLS.end(),
            nonce: asks.clone(),
        };
        let Some(reply) = connection.ask(&hello).await? else {
            return Ok(None);
        };
        let answers = reply.nonce.unwrap_or_default();
        let theirs = reply.proof.unwrap_or_default();
        anyhow::ensure!(
            proof::holds(&self.key, Side::Answers, &asks, &answers, &theirs),
            "who answers is not known to be the executor"
        );
        let prove = Request::Prove {
            proof: proof::of(&self.key, Side::Asks, &asks, &answers),
        };
        if connection.ask(&prove).await?.is_none() {
            return Ok(None);
        }
        let told = reply.protocol.unwrap_or(NONE);
        connection.protocol = shared(PROTOCOLS, told, told);
        Ok(Some(connection))
    }

    pub async fn standing(&self) -> anyhow::Result<Standing> {
        if let Some(connection) = self.connect().await? {
            return Ok(match connection.protocol {
                NONE => Standing::Foreign,
                _ => Standing::Working,
            });
        }
        // One that went away before its program ran left nothing to tell.
        Ok(match self.last()?.map(|r| r.what) {
            None => Standing::None,
            Some(What::Exit { .. }) => Standing::Over,
            Some(_) => Standing::Gone,
        })
    }

    /// The last of what was kept, read without asking.
    fn last(&self) -> anyhow::Result<Option<Record>> {
        Ok(present(journal::last(&self.dir))?.flatten())
    }

    fn is_over(&self) -> anyhow::Result<bool> {
        Ok(matches!(
            self.last()?.map(|r| r.what),
            Some(What::Exit { .. })
        ))
    }

    async fn ask(&self, request: Request) -> anyhow::Result<Reply> {
        let mut connection = self.connect().await?.context("the executor is gone")?;
        let asked = tokio::time::timeout(ANSWERS_WITHIN, connection.ask(&request));
        asked
            .await
            .context("the executor does not answer")??
            .context("the executor is gone")
    }

    /// Write a line to the program. Whether it was: one written by the same
    /// key before is not written again.
    pub async fn write(
        &self,
        key: impl Into<String>,
        line: impl Into<String>,
    ) -> anyhow::Result<bool> {
        let request = Request::Write {
            key: key.into(),
            line: line.into(),
        };
        Ok(self.ask(request).await?.fresh.unwrap_or(false))
    }

    /// Keep a mark. Whether it was not there before.
    pub async fn mark(&self, key: impl Into<String>) -> anyhow::Result<bool> {
        let request = Request::Mark { key: key.into() };
        Ok(self.ask(request).await?.fresh.unwrap_or(false))
    }

    /// Every record after the one counted `after`, those to come included.
    pub fn attach(&self, after: u64) -> Records {
        Records {
            executor: self.clone(),
            seq: after,
            kept: None,
            from: From::Nowhere,
            line: Vec::new(),
        }
    }

    /// End the program and all it started, and return once it is over. Until
    /// `grace` is over it may end as it likes.
    pub async fn stop(&self, grace: Duration) -> anyhow::Result<()> {
        if let Some(mut connection) = self.connect().await? {
            let request = Request::Stop {
                grace_ms: grace.as_millis() as u64,
            };
            let stopping = async {
                connection.ask(&request).await?;
                while self.connect().await?.is_some() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Ok(())
            };
            return tokio::time::timeout(grace + ANSWERS_WITHIN, stopping)
                .await
                .context("the executor does not stop")?;
        }
        self.end_left()
    }

    /// End a program its executor went away from, and what it started.
    fn end_left(&self) -> anyhow::Result<()> {
        if self.is_over()? {
            return Ok(());
        }
        let first = present(journal::first(&self.dir))?.flatten();
        if let Some(What::Start {
            pid,
            born: Some(born),
            boot: Some(boot),
            ..
        }) = first.map(|r| r.what)
            && process::boot() == Some(boot)
            // A number is not given out again while a group goes by it.
            && process::born(pid).is_none_or(|now| now == born)
        {
            process::end_group(pid, true);
        }
        Ok(())
    }

    /// Stop it, and keep nothing of it. One that cannot be asked to stop does
    /// so when what it keeps is gone.
    pub async fn forget(&self) -> anyhow::Result<()> {
        self.stop(Duration::ZERO).await.ok();
        match std::fs::remove_dir_all(&self.dir) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// `None` for what is not there.
fn present<T>(read: std::io::Result<T>) -> std::io::Result<Option<T>> {
    match read {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        read => read.map(Some),
    }
}

struct Connection {
    from: BufReader<ReadHalf<Stream>>,
    to: WriteHalf<Stream>,
    protocol: u32,
}

impl Connection {
    /// `None` when it is gone before it answers.
    async fn ask(&mut self, request: &Request) -> anyhow::Result<Option<Reply>> {
        let line = format!("{}\n", serde_json::to_string(request)?);
        let mut answer = String::new();
        let asked = async {
            self.to.write_all(line.as_bytes()).await?;
            self.to.flush().await?;
            self.from.read_line(&mut answer).await
        };
        match asked.await {
            Ok(_) if answer.ends_with('\n') => {}
            Ok(_) => return Ok(None),
            Err(error) if broke(&error) => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        let reply: Reply = serde_json::from_str(&answer)?;
        match reply.error {
            Some(error) => bail!(error),
            None => Ok(Some(reply)),
        }
    }
}

fn broke(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        error.kind(),
        ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::UnexpectedEof
    )
}

/// The records of one executor, in order and each once, whether it is there
/// or what it kept is all there is.
pub struct Records {
    executor: Executor,
    seq: u64,
    /// How many there were when they were first asked for.
    kept: Option<u64>,
    from: From,
    /// The part of the next record that came so far: whoever gave up waiting
    /// for it may ask again.
    line: Vec<u8>,
}

enum From {
    Nowhere,
    Executor(Connection),
    Kept(Option<journal::Kept>),
}

/// What comes next.
#[derive(Debug, Clone, PartialEq)]
pub enum Next {
    Record(Record),
    /// Nothing comes after the last record.
    Over,
    /// Its executor went away, and the last record is missing.
    Gone,
}

impl Records {
    pub async fn next(&mut self) -> anyhow::Result<Next> {
        loop {
            match &mut self.from {
                From::Nowhere => self.begin().await?,
                From::Kept(kept) => {
                    let next = match kept {
                        Some(kept) => kept.next(),
                        None => Ok(None),
                    };
                    let next = match next {
                        Ok(next) => next,
                        Err(error) => {
                            // Read anew, not over: nothing is left out.
                            self.from = From::Nowhere;
                            return Err(error.into());
                        }
                    };
                    match next {
                        Some(record) if record.seq <= self.seq => {}
                        Some(record) => return Ok(self.take(record)),
                        None if self.executor.is_over()? => return Ok(Next::Over),
                        None => return Ok(Next::Gone),
                    }
                }
                From::Executor(connection) => {
                    match connection.from.read_until(b'\n', &mut self.line).await {
                        Ok(_) if self.line.ends_with(b"\n") => {
                            match serde_json::from_slice(&self.line) {
                                Ok(record) => {
                                    self.line.clear();
                                    return Ok(self.take(record));
                                }
                                Err(error) => {
                                    // Read anew, not over: nothing is left out.
                                    self.from = From::Nowhere;
                                    return Err(error.into());
                                }
                            }
                        }
                        Err(error) if !broke(&error) => {
                            self.from = From::Nowhere;
                            return Err(error.into());
                        }
                        Ok(_) | Err(_) => self.from = From::Nowhere,
                    }
                }
            }
        }
    }

    /// Whether all that had happened before these were first asked for was
    /// read. What was asked and answered by then is known from there on.
    pub fn caught_up(&self) -> bool {
        self.kept.is_some_and(|kept| self.seq >= kept)
    }

    fn take(&mut self, record: Record) -> Next {
        self.seq = record.seq;
        Next::Record(record)
    }

    async fn begin(&mut self) -> anyhow::Result<()> {
        self.line.clear();
        if let Some(mut connection) = self.executor.connect().await? {
            anyhow::ensure!(
                connection.protocol != NONE,
                "the executor is of a version that is not spoken with here"
            );
            let request = Request::Attach { after: self.seq };
            if let Some(reply) = connection.ask(&request).await? {
                self.kept.get_or_insert(reply.kept.unwrap_or(0));
                self.from = From::Executor(connection);
                return Ok(());
            }
        }
        // Its socket is closed after the last record, so the rest is kept.
        if self.executor.connect().await?.is_none() {
            let last = self.executor.last()?.map_or(0, |record| record.seq);
            self.kept.get_or_insert(last);
            let kept = present(journal::Kept::open(&self.executor.dir))?;
            self.from = From::Kept(kept);
        }
        Ok(())
    }
}
