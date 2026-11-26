//! Work on code run directly: Claude Code started by Famulen itself, under an
//! executor, in a jj workspace Famulen makes for the run, in the one
//! repository the owner named. What needs the owner waits for their answer
//! instead of being refused.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Kept;
use super::adroit::token;
use super::order::Order;
use super::runs::Run;
use crate::agents::Fence;
use crate::app::App;
use crate::approvals::Answer;
use crate::store::Store;

/// Where work may run directly, as the owner set it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Direct {
    /// The one repository: the checkout jj keeps it in.
    pub repository: PathBuf,
    /// The jj that makes the workspaces.
    pub jj: String,
    /// Whether the sandbox here keeps an agent from writing what it must
    /// not, places not made yet among them. Seen to on macOS alone so far.
    pub fenced: bool,
}

impl Direct {
    pub fn new(repository: impl Into<PathBuf>) -> Self {
        Self {
            repository: repository.into(),
            jj: "jj".into(),
            fenced: cfg!(target_os = "macos"),
        }
    }
}

/// What a Claude Code started in a workspace later reads as its own settings
/// or starts as its servers, wherever below the workspace it is.
pub(super) const UNWRITABLE_NAMES: &[&str] = &[".claude", ".mcp.json"];

/// Where the workspaces are made, below the checkout, named as Adroit's
/// worktrees of delegated work are.
const WORKTREES: [&str; 2] = [".claude", "worktrees"];

/// How long the checkout's build is given to be lent to a workspace.
const CLONE_LIMIT: Duration = Duration::from_secs(120);

/// Where a run works.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Place {
    pub checkout: PathBuf,
    pub workspace: PathBuf,
    /// The workspace's name in jj.
    pub name: String,
    /// The directory of the task, in the workspace.
    pub directory: PathBuf,
}

/// The checkout an order runs in, and its directory inside it; or why it does
/// not run directly there.
pub(super) fn checkout(
    direct: Option<&Direct>,
    order: &Order,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    let direct = direct.context(
        "no repository is set for work run directly ([workers] direct, FAMULEN_WORKERS_DIRECT)",
    )?;
    anyhow::ensure!(
        direct.fenced,
        "work runs directly only on macOS for now, where its sandbox was seen to keep it from what it must not write"
    );
    let repository = direct
        .repository
        .canonicalize()
        .with_context(|| format!("no repository at {}", direct.repository.display()))?;
    anyhow::ensure!(
        repository.join(".jj/repo").is_dir(),
        "{} is no checkout of jj",
        repository.display()
    );
    // It need not be in the owner's own working copy: it is looked for in
    // the workspace, made from the revision the work begins from.
    let given = Path::new(&order.directory);
    let directory = real_as_far_as_there(given);
    anyhow::ensure!(
        !directory
            .components()
            .any(|c| c == std::path::Component::ParentDir),
        "{} is not a plain path",
        order.directory
    );
    let inside = directory.strip_prefix(&repository).with_context(|| {
        format!(
            "work runs directly only in {}, not in {}",
            repository.display(),
            directory.display()
        )
    })?;
    let below_worktrees = inside
        .components()
        .take(WORKTREES.len())
        .map(|c| c.as_os_str().to_string_lossy())
        .eq(WORKTREES);
    anyhow::ensure!(
        !below_worktrees,
        "work runs directly in the checkout {}, not in another workspace",
        repository.display()
    );
    Ok((repository, inside.to_owned()))
}

/// `path` with what is there of it read as it really is, and the rest as
/// written.
fn real_as_far_as_there(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = path;
    loop {
        if let Ok(real) = at.canonicalize() {
            return rest.iter().rev().fold(real, |p, part| p.join(part));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                at = parent;
            }
            _ => return crate::paths::plain(path.to_owned()),
        }
    }
}

/// Where a run works: where it began working, kept as it began, whatever
/// the owner set since; or, for a run yet to begin, where it is to work. A
/// run that begins again where the run before it, `cut`, was cut off goes on
/// in that one's workspace, with what it left there.
pub(super) fn place(
    direct: Option<&Direct>,
    run: &Run,
    store: &Store,
    cut: Option<i64>,
) -> anyhow::Result<Place> {
    if let Some(kept) = kept_place(store, run.id) {
        return Ok(kept);
    }
    let (checkout, inside) = checkout(direct, &run.order)?;
    let before = cut
        .filter(|_| run.attempt > 1)
        .and_then(|cut| kept_place(store, cut))
        .filter(|before| before.checkout == checkout && before.workspace.join(".jj").is_dir());
    let (name, workspace) = match before {
        Some(before) => (before.name, before.workspace),
        None => {
            let name = token(&run.order.code, run.id);
            (name.clone(), worktrees(&checkout).join(name))
        }
    };
    Ok(Place {
        directory: workspace.join(inside),
        checkout,
        workspace,
        name,
    })
}

pub(super) fn kept_place(store: &Store, run: i64) -> Option<Place> {
    let kept = store.state(place_key(run)).ok()??;
    serde_json::from_str(&kept).ok()
}

/// Where the workspaces of a checkout are.
pub(super) fn worktrees(checkout: &Path) -> PathBuf {
    let mut below = checkout.to_owned();
    below.extend(WORKTREES);
    below
}

/// Make the workspace of a run, unless it is there from a start before, and
/// make ready what a build reuses. A workspace just made that cannot be
/// worked in is taken away again, so that the next start makes it anew.
pub(super) async fn prepare(direct: &Direct, place: &Place, base: &str) -> anyhow::Result<()> {
    let base = if base.is_empty() { "trunk()" } else { base };
    let made = !place.workspace.join(".jj").is_dir();
    if made {
        // A workspace jj knows by the name already is not this run's, wherever
        // it is: it is neither made over nor forgotten.
        let mut list = tokio::process::Command::new(&direct.jj);
        list.args(["--ignore-working-copy", "-R"])
            .arg(&place.checkout)
            .args(["workspace", "list"]);
        let listed = jj_output(direct, list)
            .await
            .context("the workspaces could not be listed")?;
        anyhow::ensure!(
            !listed.lines().any(|l| l
                .split_once(':')
                .is_some_and(|(name, _)| name.trim() == place.name)),
            "jj has a workspace named {} already, which was not made for this work",
            place.name
        );
        // jj makes the workspace but not the directories above it.
        if let Some(above) = place.workspace.parent() {
            std::fs::create_dir_all(above)?;
        }
        let mut add = tokio::process::Command::new(&direct.jj);
        // jj looks at the owner's own working copy first; what is new there
        // is left as it is, however large. It must update the new one.
        add.args(["--config", "snapshot.auto-track=\"none()\"", "-R"])
            .arg(&place.checkout)
            .args(["workspace", "add", "--name", &place.name, "-r", base])
            .arg(&place.workspace);
        let added = jj(direct, add)
            .await
            .context("the workspace could not be made");
        if added.is_err() {
            forget(direct, place).await;
        }
        added?;
    }
    let usable = usable(place, base);
    if usable.is_err() && made {
        forget(direct, place).await;
    }
    usable?;
    if let Err(error) = clone_builds(place).await {
        tracing::warn!(%error, workspace = %place.workspace.display(), "made a workspace without the checkout's build");
    }
    Ok(())
}

/// Take away a workspace that cannot be worked in: jj may have made some of
/// it before it failed.
async fn forget(direct: &Direct, place: &Place) {
    let mut forget = tokio::process::Command::new(&direct.jj);
    forget
        .args(["--ignore-working-copy", "-R"])
        .arg(&place.checkout)
        .args(["workspace", "forget", &place.name]);
    if let Err(error) = jj(direct, forget).await {
        tracing::warn!(%error, workspace = %place.name, "could not forget a workspace that could not be worked in");
    }
    std::fs::remove_dir_all(&place.workspace).ok();
}

/// How long jj is given to make or forget a workspace: the pool waits on it.
const JJ_LIMIT: Duration = Duration::from_secs(120);

async fn jj(direct: &Direct, command: tokio::process::Command) -> anyhow::Result<()> {
    jj_output(direct, command).await.map(|_| ())
}

/// What jj writes when it does what `command` asks.
async fn jj_output(
    direct: &Direct,
    mut command: tokio::process::Command,
) -> anyhow::Result<String> {
    // What it runs, a signer or a watcher, sees none of Famulen's secrets.
    crate::config::hide_secrets(&mut command);
    command
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let ran = tokio::time::timeout(JJ_LIMIT, command.output())
        .await
        .with_context(|| format!("{} took longer than {}s", direct.jj, JJ_LIMIT.as_secs()))?
        .with_context(|| format!("could not run {}", direct.jj))?;
    anyhow::ensure!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&ran.stdout).into_owned())
}

/// Why a workspace cannot be worked in, if it cannot.
fn usable(place: &Place, base: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        place.directory.is_dir(),
        "{} is not in the workspace: {base} has no such directory",
        place.directory.display()
    );
    let holds = std::fs::read_dir(&place.workspace)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| !matches!(e.file_name().to_str(), Some(".jj" | ".git")));
    anyhow::ensure!(
        holds,
        "the workspace holds nothing: {base} has no files there; name the revision to work from as Base"
    );
    Ok(())
}

/// Lend the workspace what the checkout built, so that its first build does
/// not begin from nothing. A copy that shares the blocks where the system can.
async fn clone_builds(place: &Place) -> anyhow::Result<()> {
    let (from, to) = (
        place.checkout.join("target"),
        place.workspace.join("target"),
    );
    if !from.is_dir() || to.exists() {
        return Ok(());
    }
    // The system's own: a GNU cp on the PATH knows no clone on macOS.
    let mut copy = if cfg!(target_os = "macos") {
        let mut copy = tokio::process::Command::new("/bin/cp");
        copy.args(["-c", "-R"]);
        copy
    } else {
        // Shared blocks or nothing: a whole copy of a build is not waited for.
        let mut copy = tokio::process::Command::new("cp");
        copy.args(["-R", "--reflink=always"]);
        copy
    };
    crate::config::hide_secrets(&mut copy);
    copy.arg(&from).arg(&to).kill_on_drop(true);
    // A build that takes long to copy is not waited for: the pool waits on
    // the start. What was copied of it is not left half made.
    let copied = match tokio::time::timeout(CLONE_LIMIT, copy.output()).await {
        Ok(Ok(copied)) if copied.status.success() => return Ok(()),
        Ok(Ok(copied)) => String::from_utf8_lossy(&copied.stderr).trim().to_owned(),
        Ok(Err(error)) => error.to_string(),
        Err(_) => format!("it took longer than {}s", CLONE_LIMIT.as_secs()),
    };
    // Not on the pool's own threads: a build is many files.
    tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&to).ok())
        .await
        .ok();
    anyhow::bail!("{copied}")
}

/// What the shell of a run is kept from. It writes its workspace, and of the
/// repository what jj and Git write for it; not what tells jj and Git of the
/// repository what to run or where to find it, which would run as the owner
/// the next time they use the checkout.
pub(super) fn fence(kept: &Kept, runs: &Path, place: &Place) -> Fence {
    let (jj, git) = (place.checkout.join(".jj"), place.checkout.join(".git"));
    Fence {
        sealed: kept.places.clone(),
        hosts: kept.hosts.clone(),
        unread: vec![runs.to_owned()],
        readable: Vec::new(),
        unwritable: vec![
            jj.join("repo/config.toml"),
            jj.join("workspace-config.toml"),
            place.workspace.join(".jj/workspace-config.toml"),
            jj.join("repo/store/git_target"),
            // The owner's own working copy, as jj and Git keep it.
            jj.join("working_copy"),
            git.join("HEAD"),
            git.join("index"),
            // The Git store of a repository jj keeps apart from Git.
            jj.join("repo/store/git/config"),
            jj.join("repo/store/git/hooks"),
            jj.join("repo/store/git/info"),
            jj.join("repo/store/git/modules"),
            git.join("config"),
            git.join("config.worktree"),
            git.join("hooks"),
            git.join("info"),
            git.join("modules"),
            git.join("commondir"),
            // The workspace's own ways to the repository.
            place.workspace.join(".jj/repo"),
            place.workspace.join(".git"),
        ]
        .into_iter()
        // What a Claude Code started there later reads as its own settings,
        // and the servers it starts.
        .chain(
            [&place.workspace, &place.directory]
                .into_iter()
                .flat_map(|at| {
                    // The whole of `.claude`, so that it is not made elsewhere
                    // and linked in either.
                    UNWRITABLE_NAMES.iter().map(move |name| at.join(name))
                }),
        )
        // What tells each worktree of Git, its own and the others there are
        // when it starts, where its repository and settings are.
        .chain(
            worktrees_of_git(&git, &place.name)
                .into_iter()
                .flat_map(|w| ["commondir", "gitdir", "config.worktree"].map(|name| w.join(name))),
        )
        .collect(),
        // What is not there is not made writable: a repository jj keeps
        // apart from Git has no Git directory beside it to write.
        writable: [place.workspace.clone(), jj, git]
            .into_iter()
            .filter(|w| *w == place.workspace || w.exists())
            .collect(),
    }
}

/// The worktrees Git keeps of a repository, `own` among them whether or not
/// it is made yet.
fn worktrees_of_git(git: &Path, own: &str) -> Vec<PathBuf> {
    let below = git.join("worktrees");
    let mut all: Vec<PathBuf> = std::fs::read_dir(&below)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    let own = below.join(own);
    if !all.contains(&own) {
        all.push(own);
    }
    all.sort();
    all
}

/// How a turn that went wrong is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fault {
    /// It passes by itself: the same conversation is told to go on.
    Passing,
    /// What the account may use is used up: it goes on once it may again.
    Limit,
    /// The agent is logged out.
    Login,
    Other,
}

pub(super) fn fault(why: &str) -> Fault {
    if super::adroit::passes(why) {
        return Fault::Passing;
    }
    let why = why.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| why.contains(w));
    if has(&[
        "usage limit",
        "session limit",
        "weekly limit",
        "hit your limit",
        "reached your limit",
        "spend limit",
        "out of extra usage",
    ]) {
        Fault::Limit
    } else if has(&[
        "/login",
        "please log in",
        "logged out",
        "not logged in",
        "invalid api key",
        "oauth token has expired",
        "oauth token revoked",
        "authentication_error",
        "error: 401",
        "401 unauthorized",
    ]) {
        Fault::Login
    } else {
        Fault::Other
    }
}

/// How long a run waits before it is told to go on after each fault, and
/// how often at most.
pub(super) fn waits(fault: Fault) -> Option<(Duration, u32)> {
    match fault {
        Fault::Passing => Some((Duration::from_secs(60), 3)),
        // Allowances come back within hours; a run that waits longer than
        // it may run ends on its own.
        Fault::Limit => Some((Duration::from_secs(30 * 60), 16)),
        Fault::Login | Fault::Other => None,
    }
}

/// What the owner is asked by a run.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Owed {
    /// Whether a tool may be used, as this request of the gateway.
    Permission { ticket: i64 },
    /// What it asks its user.
    Question { choices: Vec<crate::agents::Choice> },
}

/// What the owner is asked, as the run keeps it to be shown.
pub(super) fn shown(id: &str, tool: &str, input: &Value, owed: &Owed) -> Value {
    match owed {
        // What the call is given is read from the request, which keeps its
        // secrets sealed.
        Owed::Permission { ticket } => json!({
            "id": id, "kind": "permission", "tool": tool, "ticket": ticket,
        }),
        Owed::Question { choices } => json!({
            "id": id, "kind": "question", "tool": tool, "input": input,
            "choices": choices.iter().map(|c| json!({
                "text": c.text, "options": c.options, "several": c.several,
            })).collect::<Vec<_>>(),
        }),
    }
}

/// What the owner answered to what a run asked its user, by the question.
pub(super) fn replies_key(run: i64) -> String {
    format!("worker.replies.{run}")
}

/// The requests of the gateway a run waits on, by where its process asked:
/// each with the question it puts, and whether its approval was used.
pub(super) fn tickets_key(run: i64) -> String {
    format!("worker.tickets.{run}")
}

/// The requests of the gateway a run waited on between turns, by the record
/// of the turn that ended to wait.
pub(super) fn waits_key(run: i64) -> String {
    format!("worker.waits.{run}")
}

/// What is kept of a run under `key`, one value by each name.
pub(super) fn kept(store: &Store, key: &str) -> serde_json::Map<String, Value> {
    store
        .state(key)
        .ok()
        .flatten()
        .and_then(|kept| serde_json::from_str(&kept).ok())
        .unwrap_or_default()
}

/// Held while what is kept by name is read and written again, so that what
/// runs at once does not write over another's.
static KEEPING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Keep `value` by `name` under `key`, or take it away when it is `None`.
pub(super) fn keep(
    store: &Store,
    key: &str,
    name: &str,
    value: Option<Value>,
) -> anyhow::Result<()> {
    let _held = KEEPING.lock().unwrap_or_else(|p| p.into_inner());
    let mut all = kept(store, key);
    match value {
        Some(value) => all.insert(name.to_owned(), value),
        None => all.remove(name),
    };
    if all.is_empty() {
        store.clear_state(key)
    } else {
        store.set_state(key, Value::Object(all).to_string())
    }
}

/// Take back every request a run's process left waiting, and forget what
/// the owner answered it: a process started after it asks anew.
pub(super) fn forget_asked(app: &App, run: i64) -> anyhow::Result<()> {
    let tickets = kept(&app.store, &tickets_key(run));
    let mut over = Vec::new();
    for ticket in tickets.values().filter_map(|e| e["ticket"].as_i64()) {
        if app.gateway.withdraw_ticket(ticket)? {
            over.push(ticket);
        } else {
            // Answered, and not used by what asked: used by nothing later.
            app.gateway.drop_pass(ticket)?;
        }
    }
    // Its connections' requests too: no turn of a next process waits on them.
    let mut before = Vec::new();
    for ticket in app.gateway.tickets_of(crate::permissions::job_actor(run))? {
        if ticket.connection.is_empty() {
            continue;
        }
        before.push(ticket.id);
        if app.gateway.withdraw_ticket(ticket.id)? {
            over.push(ticket.id);
        } else {
            app.gateway.drop_pass(ticket.id)?;
        }
    }
    if !over.is_empty() {
        crate::approvals::withdraw_apart(app, over);
    }
    if before.is_empty() {
        app.store.clear_state(waits_key(run))?;
    } else {
        // Under no record's number, so that no turn takes them for its own.
        keep(&app.store, &waits_key(run), "before", Some(json!(before)))?;
    }
    app.store.clear_state(tickets_key(run))?;
    app.store.clear_state(replies_key(run))?;
    app.store.clear_state(owed_key(run))?;
    app.workers.sights.owe(run, None);
    Ok(())
}

/// Where a run works, kept when it begins; kept after it ends too, so that
/// a run that begins again finds the workspace an earlier one left.
pub(super) fn place_key(run: i64) -> String {
    format!("worker.place.{run}")
}

/// What a run waits on the owner for, as it is shown: kept as long as it
/// waits, wherever else it is shown from.
pub(super) fn owed_key(run: i64) -> String {
    format!("worker.owed.{run}")
}

pub(super) fn conversation_key(run: i64) -> String {
    format!("worker.conversation.{run}")
}

pub(super) fn resumed_key(run: i64) -> String {
    format!("worker.resumed.{run}")
}

/// The turns of a run that went wrong and were gone on after, by the kind of
/// fault.
pub(super) fn went_on_key(run: i64) -> String {
    format!("worker.went_on.{run}")
}

/// A new conversation's id, chosen before it is started so that it can be
/// found again whatever becomes of what started it.
pub(super) fn new_conversation() -> String {
    let mut bytes: [u8; 16] = rand::random();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// What a run run directly asks the owner, as the console shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectQuestion {
    pub id: String,
    pub tool: String,
    pub input: Value,
    /// The request of the gateway it is, or 0 for what it asks its user.
    pub ticket: i64,
    /// What it asks its user, each with what may be chosen.
    pub choices: Vec<(String, Vec<String>, bool)>,
}

/// What a run asks the owner now, if it waits for them.
pub fn direct_question(app: &App, run: &Run) -> Option<DirectQuestion> {
    if run.order.runner != super::Runner::Direct {
        return None;
    }
    let asked: Value = serde_json::from_str(&app.store.state(owed_key(run.id)).ok()??).ok()?;
    let text = |key: &str| asked[key].as_str().unwrap_or_default().to_owned();
    Some(DirectQuestion {
        id: text("id"),
        tool: text("tool"),
        input: asked["input"].clone(),
        ticket: asked["ticket"].as_i64().unwrap_or_default(),
        choices: asked["choices"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                let options = c["options"].as_array().into_iter().flatten();
                (
                    c["text"].as_str().unwrap_or_default().to_owned(),
                    options
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect(),
                    c["several"] == true,
                )
            })
            .collect(),
    })
}

/// Give a run the owner's answer to what it asks: a request of the gateway is
/// answered as any is, so that it is answered once wherever it is shown; what
/// it asks its user is kept for it to read.
pub async fn answer_direct(
    app: &App,
    run: i64,
    id: impl AsRef<str>,
    answer: Answer,
    replies: Vec<String>,
) -> anyhow::Result<()> {
    let run = app
        .workers
        .runs
        .get(run)?
        .with_context(|| format!("no run #{run}"))?;
    let id = id.as_ref();
    let question = direct_question(app, &run)
        .filter(|q| q.id == id)
        .context("this question is waited for no more")?;
    if question.ticket > 0 {
        anyhow::ensure!(replies.is_empty(), "a permission is answered yes or no");
        let answerer = app
            .answerer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .context("approvals are not running yet")?;
        answerer(question.ticket, answer, Vec::new()).await?;
    } else {
        // Declined: it decides by itself.
        if answer == Answer::Reject {
            keep(&app.store, &replies_key(run.id), id, Some(Value::Null))?;
            app.workers.poke();
            return Ok(());
        }
        anyhow::ensure!(
            answer == Answer::Approve
                && replies.len() == question.choices.len()
                && replies.iter().all(|r| !r.trim().is_empty()),
            "a question is answered in words, one for each"
        );
        keep(&app.store, &replies_key(run.id), id, Some(json!(replies)))?;
    }
    app.workers.poke();
    Ok(())
}

/// Where work may run directly, as tests have it whatever the system.
#[cfg(test)]
pub(super) fn fenced(repository: impl Into<PathBuf>) -> Direct {
    Direct {
        fenced: true,
        ..Direct::new(repository)
    }
}

#[cfg(test)]
mod tests;
