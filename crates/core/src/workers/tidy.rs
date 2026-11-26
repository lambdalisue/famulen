//! What delegated work leaves on this machine is cleared away once it is of
//! no more use: the worktree of a session that was closed, and the VMs named
//! after its task. Only what is known to hold no work is removed; whatever
//! cannot be told stays, and the owner is told of it.

mod ground;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use chrono::Timelike as _;

use self::ground::{Ground, Machine, Ran, Session};
use super::adroit::token;
use super::order::Order;
use super::runs::{Run, State};
use crate::app::App;
use crate::input::{Input, Sender, Source};
use crate::records::{TASKS, is_delegated, is_over};
use crate::store::Store;

/// How often it is looked for what can be cleared away.
const EVERY: Duration = Duration::from_secs(60 * 60);
/// How long to wait once a session was told of, for more and for its run to
/// come to its end.
const SETTLE: Duration = Duration::from_secs(30);
/// From this hour of the day the owner is told of what stays.
const TELL_AT: u32 = 9;
/// The day the owner was last told of what stays, and what they were told.
const TOLD_KEY: &str = "workers.tidy_told";
/// Where Adroit makes its worktrees, below the checkout.
const WORKTREES: [&str; 2] = [".claude", "worktrees"];
/// jj's own name for the workspace of the checkout itself.
const DEFAULT: &str = "default";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Worktree,
    Vm,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Worktree => "worktree",
            Kind::Vm => "vm",
        }
    }
}

/// What becomes of something delegated work left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fate {
    Removed,
    /// It would be removed.
    Removable,
    /// It is in use, or may be again. Why, for the owner.
    InUse(String),
    /// Its work is over, and yet it was not removed. Why, for the owner.
    Kept(String),
}

impl Fate {
    pub fn as_str(&self) -> &'static str {
        match self {
            Fate::Removed => "removed",
            Fate::Removable => "removable",
            Fate::InUse(_) => "in_use",
            Fate::Kept(_) => "kept",
        }
    }

    pub fn why(&self) -> &str {
        match self {
            Fate::InUse(why) | Fate::Kept(why) => why,
            _ => "",
        }
    }
}

/// Something delegated work left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Left {
    pub kind: Kind,
    /// The worktree's path, or the VM's name.
    pub name: String,
    pub task: String,
    pub code: String,
    pub title: String,
    pub fate: Fate,
    /// What a worktree takes on the disk, when it could be told.
    pub bytes: Option<u64>,
}

/// Whether what can be removed is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    Preview,
    Remove,
}

/// Clear away what can be. It goes on to its end even when the answer is no
/// longer waited for.
pub async fn tidy(app: &App, how: How) -> anyhow::Result<Vec<Left>> {
    let app = app.clone();
    tokio::spawn(async move {
        let _alone = app.workers.tidying.clone().lock_owned().await;
        Tidy::new(&app, Arc::new(Machine::new(app.clone())))
            .all(how)
            .await
    })
    .await?
}

/// Clear away what can be, as sessions close and once in a while.
pub async fn keep_tidy(app: App) {
    let mut tick = tokio::time::interval(EVERY);
    let mut before = BTreeSet::new();
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            () = app.workers.closed.notified() => {}
        }
        tokio::time::sleep(SETTLE).await;
        match tidy(&app, How::Remove).await {
            Ok(left) => {
                let local = chrono::Utc::now().with_timezone(&app.tz());
                if let Err(error) = tell(&app, &left, &before, local) {
                    tracing::warn!(%error, "could not tell what delegated work left");
                }
                before = left.iter().filter_map(matter).collect();
            }
            Err(error) => tracing::warn!(%error, "could not clear away after delegated work"),
        }
    }
}

/// What is kept and why, without what differs each time it is looked at: the
/// processes there are and what a program said.
fn matter(left: &Left) -> Option<(String, String)> {
    let Fate::Kept(why) = &left.fate else {
        return None;
    };
    let kind = why.split(['（', ':']).next().unwrap_or_default();
    Some((left.name.clone(), kind.trim().to_owned()))
}

/// Have the owner told of what stays although its work is over: once a day at
/// most, and only when some of it was not told before. What was not so when it
/// was last looked at may pass by itself, and is not told yet.
fn tell(
    app: &App,
    left: &[Left],
    before: &BTreeSet<(String, String)>,
    now: chrono::DateTime<chrono_tz::Tz>,
) -> anyhow::Result<()> {
    let lasting: Vec<(&Left, (String, String))> = left
        .iter()
        .filter_map(|l| Some((l, matter(l)?)))
        .filter(|(_, m)| before.contains(m))
        .collect();
    if now.hour() < TELL_AT {
        return Ok(());
    }
    let today = now.date_naive().to_string();
    let (day, told) = told_last(&app.store)?;
    if day == today || lasting.iter().all(|(_, m)| told.contains(m)) {
        return Ok(());
    }
    let list: Vec<String> = lasting
        .iter()
        .map(|(l, _)| {
            format!(
                "- {}（{}「{}」）: {}",
                l.name,
                l.code,
                l.title,
                l.fate.why()
            )
        })
        .collect();
    // What was told before is not forgotten while it is there: a reason may
    // give way to another for a while.
    let kept: BTreeSet<(String, String)> = told
        .into_iter()
        .filter(|(name, _)| left.iter().any(|l| l.name == *name))
        .chain(lasting.into_iter().map(|(_, m)| m))
        .collect();
    let input = Input {
        sender: Sender::System,
        source: Source::Schedule {
            name: "worktrees".into(),
            at: today.clone(),
            routine: String::new(),
        },
        text: format!(
            "These are left of delegated work that is over, and Famulen did not remove them because it could not tell that nothing would be lost. Tell the owner in one message what is left, where and why, and change nothing yourself.\n{}",
            list.join("\n")
        ),
    };
    if app.store.enqueue(&input)?.is_some() {
        app.inputs.notify_one();
    }
    app.store
        .set_state(TOLD_KEY, serde_json::to_string(&(today, kept))?)
}

fn told_last(store: &Store) -> anyhow::Result<(String, BTreeSet<(String, String)>)> {
    Ok(match store.state(TOLD_KEY)? {
        Some(json) => serde_json::from_str(&json)?,
        None => Default::default(),
    })
}

/// A checkout, and what its trunk is called.
struct Checkout {
    path: PathBuf,
    /// The trunk as jj writes it, once it was fetched; or why it was not.
    trunk: Option<Result<String, String>>,
}

struct Tidy<'a> {
    app: &'a App,
    ground: Arc<dyn Ground>,
    runs: Vec<Run>,
    /// The sessions read for a run, by its id.
    sessions: HashMap<i64, Result<Vec<Session>, String>>,
}

impl<'a> Tidy<'a> {
    fn new(app: &'a App, ground: Arc<dyn Ground>) -> Self {
        Self {
            app,
            ground,
            runs: Vec::new(),
            sessions: HashMap::new(),
        }
    }

    async fn all(mut self, how: How) -> anyhow::Result<Vec<Left>> {
        self.runs = self.app.workers.runs.on_adroit()?;
        let mut left = Vec::new();
        for checkout in self.checkouts() {
            self.worktrees(checkout, how, &mut left).await;
        }
        self.vms(how, &mut left).await;
        Ok(left)
    }

    /// The checkouts worktrees were asked for in.
    fn checkouts(&self) -> Vec<Checkout> {
        let paths: BTreeSet<PathBuf> = self
            .runs
            .iter()
            .filter(|r| r.order.worktree)
            .filter_map(|r| checkout_of(Path::new(&r.order.directory)))
            .collect();
        paths
            .into_iter()
            .map(|path| Checkout { path, trunk: None })
            .collect()
    }

    async fn worktrees(&mut self, mut checkout: Checkout, how: How, left: &mut Vec<Left>) {
        for name in below(&checkout.path) {
            let Some(run) = self.run_of(&checkout.path, &name) else {
                continue;
            };
            let path = place(&checkout.path, &name);
            let mut bytes = None;
            let fate = match self.judge(&mut checkout, &path, &run).await {
                Err(fate) => fate,
                Ok((workspace, size)) => {
                    bytes = Some(size);
                    match how {
                        How::Preview => Fate::Removable,
                        How::Remove => self.remove(&checkout, &path, &workspace, &run, size).await,
                    }
                }
            };
            if let Fate::Kept(why) = &fate {
                tracing::info!(path = %path.display(), why, "kept a worktree of delegated work");
            }
            left.push(Left {
                kind: Kind::Worktree,
                name: path.display().to_string(),
                task: run.task,
                code: run.order.code,
                title: run.order.title,
                fate,
                bytes,
            });
        }
    }

    /// The run whose worktree in `checkout` is called `name`.
    fn run_of(&self, checkout: &Path, name: &str) -> Option<Run> {
        self.runs
            .iter()
            .find(|r| {
                r.order.worktree
                    && !r.order.code.is_empty()
                    && named_after(name, &token(&r.order.code, r.id))
                    && checkout_of(Path::new(&r.order.directory)).as_deref() == Some(checkout)
            })
            .cloned()
    }

    /// Whether a run is over for good: its task is done or cancelled and the
    /// session it ran in was closed.
    async fn over(&mut self, run: &Run) -> Result<(), Fate> {
        match self.app.workers.runs.get(run.id) {
            Ok(Some(now)) if now.state == State::Ended => {}
            Ok(Some(_)) => return Err(in_use("委託がまだ終わっていない")),
            Ok(None) => return Err(kept("委託の控えがない")),
            Err(error) => return Err(unsure("委託の様子", error)),
        }
        // Read anew each time: a task put back in the queue is not over.
        match self.app.records.get(TASKS, &run.task) {
            Ok(Some(task)) if is_over(&task) => {}
            Ok(Some(_)) => return Err(in_use("タスクが済んでもやめられてもいない")),
            Ok(None) => return Err(kept("タスクが見つからない")),
            Err(error) => return Err(unsure("タスク", error)),
        }
        if run.session.is_empty() {
            return Err(kept("どのセッションで動いたか分からない"));
        }
        let sessions = self.sessions_of(run).await?;
        let mut own = sessions.iter().filter(|s| s.id == run.session).peekable();
        if own.peek().is_none() {
            return Err(kept("Adroit がそのセッションを知らない"));
        }
        if own.any(|s| !s.ended) {
            return Err(in_use("セッションが閉じられていない"));
        }
        Ok(())
    }

    async fn sessions_of(&mut self, run: &Run) -> Result<Vec<Session>, Fate> {
        if !self.sessions.contains_key(&run.id) {
            let read = self.ground.sessions(run).await;
            self.sessions
                .insert(run.id, read.map_err(|error| format!("{error:#}")));
        }
        self.sessions[&run.id]
            .clone()
            .map_err(|error| unsure("Adroit のセッション", error))
    }

    /// Nothing works in `path`.
    async fn unused(&mut self, path: &Path, run: &Run) -> Result<(), Fate> {
        let within = |dir: &Path| {
            dir.starts_with(path) || dir.canonicalize().is_ok_and(|d| d.starts_with(path))
        };
        let sessions = self.sessions_of(run).await?;
        if sessions.iter().any(|s| !s.ended && within(&s.dir)) {
            return Err(kept("ほかのセッションがここで動いている"));
        }
        if self
            .awaited(within)
            .map_err(|error| unsure("これから始まる委託", error))?
        {
            return Err(in_use("ここで始まる委託がある"));
        }
        let pids = processes_in(&*self.ground, path)
            .await
            .map_err(|error| unsure("動いているプロセス", error))?;
        if pids.is_empty() {
            return Ok(());
        }
        Err(kept(format!(
            "プロセスがここを使っている（{}）",
            pids.into_iter().take(5).collect::<Vec<_>>().join("、")
        )))
    }

    /// Whether delegated work that is not over is to work `within` here.
    fn awaited(&self, within: impl Fn(&Path) -> bool) -> anyhow::Result<bool> {
        let runs = self.app.workers.runs.open()?;
        if runs.iter().any(|r| within(Path::new(&r.order.directory))) {
            return Ok(true);
        }
        Ok(self
            .app
            .records
            .list(TASKS)?
            .iter()
            .filter(|t| is_delegated(t) && !is_over(t))
            .filter_map(|t| Order::of(t, "").ok())
            .any(|o| within(Path::new(&o.directory))))
    }

    /// The jj workspace to remove and what it takes on the disk, when nothing
    /// in it would be lost.
    async fn judge(
        &mut self,
        checkout: &mut Checkout,
        path: &Path,
        run: &Run,
    ) -> Result<(String, u64), Fate> {
        self.over(run).await?;
        real_place(&checkout.path, path).map_err(|error| unsure("作業場所", error))?;
        let workspace = self
            .workspace(path)
            .await
            .map_err(|error| unsure("jj の作業場所", error))?;
        // What takes long comes first, what is quick and may change last.
        self.unused(path, run).await?;
        let bytes = {
            let path = path.to_owned();
            tokio::task::spawn_blocking(move || measure(&path))
                .await
                .map_err(|error| unsure("中身", error))?
                .map_err(|error| unsure("中身", error))?
        };
        self.trunk(checkout, path).await?;
        self.holds_nothing(checkout, path).await?;
        self.sessions.clear();
        self.over(run).await?;
        self.unused(path, run).await?;
        real_place(&checkout.path, path).map_err(|error| unsure("作業場所", error))?;
        Ok((workspace, bytes))
    }

    async fn jj(&self, path: &Path, args: &[&str]) -> anyhow::Result<Ran> {
        let of = path.to_str().context("its path is no text")?;
        let mut all = vec!["-R", of, "--color=never", "--no-pager"];
        all.extend(args);
        let ran = self.ground.run(path, "jj", &all).await?;
        anyhow::ensure!(ran.ok, "jj {}: {}", args.join(" "), ran.why());
        Ok(ran)
    }

    /// The name of the one jj workspace kept in `path`.
    async fn workspace(&self, path: &Path) -> anyhow::Result<String> {
        let listed = self
            .jj(
                path,
                &[
                    "--ignore-working-copy",
                    "workspace",
                    "list",
                    "-T",
                    r#"name ++ "\t" ++ root ++ "\n""#,
                ],
            )
            .await?;
        let all = || listed.out.lines().filter_map(|l| l.split_once('\t'));
        anyhow::ensure!(
            !all().any(|(_, root)| root != path.as_os_str() && Path::new(root).starts_with(path)),
            "another workspace is kept inside it"
        );
        let mut here = all()
            .filter(|(_, root)| {
                Path::new(root)
                    .canonicalize()
                    .is_ok_and(|root| root == path)
            })
            .map(|(name, _)| name);
        let name = here.next().context("no workspace is kept there")?;
        anyhow::ensure!(here.next().is_none(), "several workspaces are kept there");
        anyhow::ensure!(
            name != DEFAULT && !name.is_empty() && !name.starts_with('-'),
            "the workspace {name:?} is not one to remove"
        );
        Ok(name.to_owned())
    }

    /// Know the trunk of `checkout` as it is now on the remote.
    async fn trunk(&self, checkout: &mut Checkout, path: &Path) -> Result<(), Fate> {
        if checkout.trunk.is_none() {
            let fetched = self.fetch(path).await;
            checkout.trunk = Some(fetched.map_err(|error| format!("{error:#}")));
        }
        Ok(())
    }

    /// Everything `path` stands on and holds is in the trunk.
    async fn holds_nothing(&self, checkout: &Checkout, path: &Path) -> Result<(), Fate> {
        // What is in the files is taken into `@` first, so that it is among
        // the changes asked about below.
        let status = self
            .jj(path, &["status"])
            .await
            .map_err(|error| unsure("未保存の変更", error))?;
        let Some(held) = all_taken_in(&status) else {
            return Err(kept("未保存の変更がある"));
        };
        let trunk = match &checkout.trunk {
            Some(Ok(trunk)) => trunk,
            Some(Err(error)) => return Err(unsure("リモートの本流", error)),
            None => return Err(kept("リモートの本流が分からない")),
        };
        let apart = self
            .jj(
                path,
                &[
                    "--ignore-working-copy",
                    "log",
                    "--no-graph",
                    "-r",
                    &format!("{trunk}..@ ~ empty()"),
                    "-T",
                    r#"change_id.short() ++ "\n""#,
                ],
            )
            .await
            .map_err(|error| unsure("本流に入っていない変更", error))?;
        let apart: Vec<&str> = apart.out.split_whitespace().collect();
        if !apart.is_empty() && held {
            return Err(kept("未保存の変更がある"));
        }
        if !apart.is_empty() {
            return Err(kept(format!(
                "本流（{}）に入っていない変更がある: {}",
                trunk.replace('"', ""),
                apart.into_iter().take(5).collect::<Vec<_>>().join("、")
            )));
        }
        self.agrees_with_git(path)
            .await
            .map_err(|error| unsure("未保存の変更", error))?
            .then_some(())
            .ok_or_else(|| kept("未保存の変更がある"))
    }

    /// Whether Git, which reads the files in a way of its own, finds nothing
    /// in `path` that `@` does not hold. Its own head is left out: it may stand
    /// before `@`, or have been left behind.
    async fn agrees_with_git(&self, path: &Path) -> anyhow::Result<bool> {
        if path.join(".git").symlink_metadata().is_err() {
            return Ok(true);
        }
        let args = [
            "--ignore-working-copy",
            "log",
            "--no-graph",
            "-r",
            "@",
            "-T",
            "commit_id",
        ];
        let commit = self.jj(path, &args).await?.out;
        let commit = commit.trim();
        anyhow::ensure!(
            commit.len() >= 40 && commit.chars().all(|c| c.is_ascii_hexdigit()),
            "no commit is known by {commit:?}"
        );
        let changed = [
            "--no-optional-locks",
            "diff",
            "--name-only",
            "--no-ext-diff",
            "--ignore-submodules=none",
            commit,
        ];
        let new = [
            "--no-optional-locks",
            "ls-files",
            "--others",
            "--exclude-standard",
        ];
        for args in [&changed[..], &new[..]] {
            let git = self.ground.run(path, "git", args).await?;
            anyhow::ensure!(git.ok, "git {}: {}", args.join(" "), git.why());
            if !git.out.trim().is_empty() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Fetch the trunk, and nothing else: a branch deleted on the remote would
    /// take the changes it named out of sight, and they would not be missed.
    /// For the same reason a trunk that was rewound takes nothing with it.
    async fn fetch(&self, path: &Path) -> anyhow::Result<String> {
        let named = self
            .jj(
                path,
                &[
                    "--ignore-working-copy",
                    "log",
                    "--no-graph",
                    "-r",
                    "trunk() & remote_bookmarks()",
                    "-T",
                    r#"remote_bookmarks.map(|b| b.name() ++ "\t" ++ b.remote()).join("\n") ++ "\n""#,
                ],
            )
            .await?;
        let (branch, remote) = trunk_of(&named.out)?;
        self.jj(
            path,
            &[
                "--ignore-working-copy",
                "--config=git.abandon-unreachable-commits=false",
                "git",
                "fetch",
                "--remote",
                &remote,
                "--branch",
                &branch,
            ],
        )
        .await?;
        Ok(format!("\"{branch}\"@\"{remote}\""))
    }

    async fn remove(
        &self,
        checkout: &Checkout,
        path: &Path,
        workspace: &str,
        run: &Run,
        bytes: u64,
    ) -> Fate {
        let size = size_text(bytes);
        tracing::info!(
            path = %path.display(), workspace, bytes, task = %run.order.code, session = %run.session,
            "removing the worktree of delegated work: its task is over, its session closed, nothing works in it, and nothing it stands on is missing from the trunk"
        );
        self.app.journal.record(
            "worker",
            format!(
                "{}: 作業場所 {}（{size}）を外す。セッションは閉じ、使うプロセスもセッションもなく、未保存の変更も、作業場所が立つ変更のうち本流に入っていないものもない",
                run.order.code,
                path.display()
            ),
        );
        // Asked of the workspace itself, so that the checkout's own working
        // copy is left alone, and from outside it, as it is about to go.
        let removed = async {
            let of = path.to_str().context("its path is no text")?;
            let args = [
                "-R",
                of,
                "--color=never",
                "--no-pager",
                // What is written at the last moment is kept as a change,
                // however large; jj would delete it otherwise.
                "--config=snapshot.max-new-file-size=1000000000000",
                "workspace",
                "remove",
                workspace,
            ];
            let ran = self.ground.run_whole(&checkout.path, "jj", &args).await?;
            anyhow::ensure!(ran.ok, "{}", ran.why());
            anyhow::ensure!(path.symlink_metadata().is_err(), "it is still there");
            Ok(())
        };
        match removed.await {
            Ok(()) => Fate::Removed,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "could not remove a worktree");
                kept(format!("外せなかった（{error:#}）"))
            }
        }
    }

    /// The VMs named after a task, once nothing else of the task is left.
    async fn vms(&mut self, how: How, left: &mut Vec<Left>) {
        if self.runs.is_empty() {
            return;
        }
        // A machine without vitro has none.
        let home = Path::new("/");
        let names = match self.ground.run(home, "vitro", &["ls", "--json"]).await {
            Ok(ran) if ran.ok => vm_names(&ran.out),
            Ok(ran) => Err(anyhow::anyhow!("{}", ran.why())),
            Err(error) => {
                tracing::debug!(%error, "vitro did not run");
                return;
            }
        };
        let names = match names {
            Ok(names) => names,
            Err(error) => {
                tracing::warn!(%error, "could not list the VMs");
                return;
            }
        };
        for name in names {
            // Read anew: clearing away worktrees takes long, and another run
            // of the task gives its VMs the same names.
            let runs = match self.app.workers.runs.on_adroit() {
                Ok(all) => all
                    .into_iter()
                    .filter(|r| !r.order.code.is_empty() && named_after(&name, &r.order.code))
                    .collect::<Vec<_>>(),
                Err(error) => {
                    tracing::warn!(%error, "could not tell whose the VMs are");
                    return;
                }
            };
            let Some(of) = runs.last().cloned() else {
                continue;
            };
            let fate = match self.vm(&runs, &of, left).await {
                Err(fate) => fate,
                Ok(()) if how == How::Preview => Fate::Removable,
                Ok(()) => {
                    self.app
                        .journal
                        .record("worker", format!("{}: VM {name} を消す", of.order.code));
                    tracing::info!(vm = name, task = %of.order.code, "destroying the VM of delegated work");
                    match self.ground.run(home, "vitro", &["destroy", &name]).await {
                        Ok(ran) if ran.ok => Fate::Removed,
                        Ok(ran) => kept(format!("消せなかった（{}）", ran.why())),
                        Err(error) => kept(format!("消せなかった（{error:#}）")),
                    }
                }
            };
            left.push(Left {
                kind: Kind::Vm,
                name,
                task: of.task,
                code: of.order.code,
                title: of.order.title,
                fate,
                bytes: None,
            });
        }
    }

    async fn vm(&mut self, runs: &[Run], of: &Run, left: &[Left]) -> Result<(), Fate> {
        if runs.iter().any(|r| r.task != of.task) {
            return Err(kept("どのタスクのものか分からない"));
        }
        self.sessions.clear();
        for run in runs {
            // A run that never had a session made no VM.
            if run.session.is_empty() && run.state == State::Ended && run.started_at.is_none() {
                continue;
            }
            self.over(run).await?;
        }
        match left.iter().find(|l| {
            l.kind == Kind::Worktree
                && l.task == of.task
                && matches!(l.fate, Fate::InUse(_) | Fate::Kept(_))
        }) {
            Some(Left {
                fate: Fate::InUse(_),
                ..
            }) => Err(in_use("作業場所が使われている")),
            Some(_) => Err(kept("作業場所が残っている")),
            None => Ok(()),
        }
    }
}

fn kept(why: impl Into<String>) -> Fate {
    Fate::Kept(why.into())
}

fn in_use(why: &str) -> Fate {
    Fate::InUse(why.to_owned())
}

fn unsure(what: &str, error: impl std::fmt::Display) -> Fate {
    Fate::Kept(format!("{what}を確かめられない（{error:#}）"))
}

fn size_text(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Whether `name` is `owner`, or `owner` and more after a hyphen.
fn named_after(name: &str, owner: &str) -> bool {
    name.strip_prefix(owner)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
}

fn is_real_dir(path: &Path) -> bool {
    path.symlink_metadata().is_ok_and(|m| m.is_dir())
}

/// The checkout that holds `directory`: where jj keeps the repository itself,
/// not one of its other workspaces.
fn checkout_of(directory: &Path) -> Option<PathBuf> {
    let directory = directory.canonicalize().ok()?;
    let root = directory
        .ancestors()
        .find(|d| d.join(".jj").symlink_metadata().is_ok())?;
    (is_real_dir(&root.join(".jj")) && is_real_dir(&root.join(".jj/repo"))).then(|| root.to_owned())
}

fn place(checkout: &Path, name: &str) -> PathBuf {
    let mut path = checkout.to_owned();
    path.extend(WORKTREES);
    path.join(name)
}

/// What lies where Adroit makes its worktrees.
fn below(checkout: &Path) -> Vec<String> {
    let mut at = checkout.to_owned();
    for part in WORKTREES {
        at.push(part);
        if !is_real_dir(&at) {
            return Vec::new();
        }
    }
    let mut names: Vec<String> = std::fs::read_dir(&at)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// `path` is a worktree of `checkout` and nothing else: a real directory right
/// below the place of worktrees, of the checkout's own repository.
fn real_place(checkout: &Path, path: &Path) -> anyhow::Result<()> {
    let mut at = checkout.to_owned();
    for part in WORKTREES {
        at.push(part);
        anyhow::ensure!(is_real_dir(&at), "{} is no directory", at.display());
    }
    anyhow::ensure!(
        path.parent() == Some(&at) && path.file_name().is_some(),
        "it is not right below {}",
        at.display()
    );
    anyhow::ensure!(is_real_dir(path), "it is no directory");
    anyhow::ensure!(
        path.canonicalize()? == path,
        "it is reached by a symbolic link"
    );
    anyhow::ensure!(is_real_dir(&path.join(".jj")), "it is no jj workspace");
    let repo = path.join(".jj/repo");
    anyhow::ensure!(
        repo.symlink_metadata().is_ok_and(|m| m.is_file()),
        "it is no workspace of another checkout"
    );
    let of = std::fs::read_to_string(&repo)?;
    let of = path.join(".jj").join(of.trim_end_matches('\n'));
    anyhow::ensure!(
        of.canonicalize()? == checkout.join(".jj/repo"),
        "it is a workspace of another repository"
    );
    Ok(())
}

/// What `path` takes on the disk. An error when a repository lies inside it:
/// where it is ignored neither jj nor Git tells of what it holds.
fn measure(path: &Path) -> anyhow::Result<u64> {
    let mut bytes = 0;
    let mut counted = HashSet::new();
    let mut open = vec![path.to_owned()];
    while let Some(dir) = open.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let at = entry.path();
            if dir != path && (entry.file_name() == ".git" || entry.file_name() == ".jj") {
                anyhow::bail!("{} holds a repository of its own", dir.display());
            }
            let meta = at.symlink_metadata()?;
            if meta.is_dir() {
                open.push(at);
            } else if counts(&meta, &mut counted) {
                bytes += meta.len();
            }
        }
    }
    Ok(bytes)
}

/// Whether a file is to be counted: one with several names only once.
#[cfg(unix)]
fn counts(meta: &std::fs::Metadata, counted: &mut HashSet<(u64, u64)>) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    meta.nlink() < 2 || counted.insert((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn counts(_meta: &std::fs::Metadata, _counted: &mut HashSet<(u64, u64)>) -> bool {
    true
}

/// Whether `@` holds a change of its own, when `jj status` told that all there
/// is in the files was taken into it.
fn all_taken_in(status: &Ran) -> Option<bool> {
    let line = |text: &str| status.out.lines().any(|l| l == text);
    if line("Untracked paths:") || status.err.contains("Refused to snapshot") {
        return None;
    }
    match (
        line("The working copy has no changes."),
        line("Working copy changes:"),
    ) {
        (true, false) => Some(false),
        (false, true) => Some(true),
        _ => None,
    }
}

/// The branch and remote that are the trunk, of those that name it.
fn trunk_of(named: &str) -> anyhow::Result<(String, String)> {
    let plain = |text: &str| {
        !text.is_empty()
            && !text.starts_with('-')
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
    };
    let all: Vec<(&str, &str)> = named
        .lines()
        .filter_map(|l| l.split_once('\t'))
        // jj's own name for the Git repository it is kept in.
        .filter(|(branch, remote)| *remote != "git" && plain(branch) && plain(remote))
        .filter(|(branch, _)| matches!(*branch, "main" | "master" | "trunk"))
        .collect();
    all.iter()
        .find(|(_, remote)| *remote == "origin")
        .or(all.first())
        .map(|(branch, remote)| ((*branch).to_owned(), (*remote).to_owned()))
        .context("no branch of a remote is its trunk")
}

/// The processes that have anything in `path` open, their own directory and
/// program among it.
async fn processes_in(ground: &dyn Ground, path: &Path) -> anyhow::Result<Vec<String>> {
    let of = path.to_str().context("its path is no text")?;
    // lsof writes other letters in a way of its own.
    anyhow::ensure!(
        of.is_ascii() && !of.contains(|c: char| c.is_control() || c == '\\'),
        "its path cannot be looked for"
    );
    let before = running(ground).await?;
    let ran = ground
        .run(Path::new("/"), "lsof", &["-nPw", "-F", "pn"])
        .await?;
    anyhow::ensure!(ran.ok, "lsof: {}", ran.why());
    let after = running(ground).await?;
    let mut pid = "";
    let mut all = BTreeSet::new();
    let mut using = Vec::new();
    for line in ran.out.lines() {
        if let Some(p) = line.strip_prefix('p') {
            pid = p;
            all.insert(p);
        } else if let Some(file) = line.strip_prefix('n')
            && Path::new(file).starts_with(path)
            && using.last().map(String::as_str) != Some(pid)
        {
            using.push(pid.to_owned());
        }
    }
    // In a sandbox lsof passes over the processes of others without a word.
    // One that ran all the while is one it had to see.
    let unseen: Vec<&str> = before
        .intersection(&after)
        .map(String::as_str)
        .filter(|pid| !all.contains(pid))
        .collect();
    anyhow::ensure!(
        unseen.is_empty(),
        "lsof does not see the processes of this machine ({})",
        unseen.into_iter().take(5).collect::<Vec<_>>().join(", ")
    );
    Ok(using)
}

/// The processes of whoever runs Famulen.
async fn running(ground: &dyn Ground) -> anyhow::Result<BTreeSet<String>> {
    let listed = ground
        .run(Path::new("/"), "ps", &["-axo", "pid=,uid=,stat="])
        .await?;
    anyhow::ensure!(listed.ok, "ps: {}", listed.why());
    let all: Vec<(&str, &str, &str)> = listed
        .out
        .lines()
        .filter_map(|l| {
            let mut words = l.split_whitespace();
            Some((words.next()?, words.next()?, words.next()?))
        })
        .collect();
    let own = std::process::id().to_string();
    let uid = all
        .iter()
        .find(|(pid, ..)| *pid == own)
        .map(|(_, uid, _)| *uid);
    anyhow::ensure!(
        uid.is_some() && all.iter().any(|(pid, ..)| *pid == "1"),
        "ps does not see the processes of this machine"
    );
    Ok(all
        .into_iter()
        // One that is over, or stuck on its way out, holds no file.
        .filter(|(_, of, stat)| Some(*of) == uid && !stat.contains(['Z', 'E']))
        .map(|(pid, ..)| pid.to_owned())
        .collect())
}

fn vm_names(listed: &str) -> anyhow::Result<Vec<String>> {
    let listed: serde_json::Value = serde_json::from_str(listed)?;
    listed["vms"]
        .as_array()
        .context("the VMs could not be read")?
        .iter()
        .map(|vm| {
            vm["name"]
                .as_str()
                .map(str::to_owned)
                .context("a VM has no name")
        })
        .collect()
}

#[cfg(all(test, unix))]
mod tests;
