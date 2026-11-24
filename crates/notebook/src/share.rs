//! Sharing a notebook with its copies over Git's smart HTTP protocol, for
//! whoever serves it. What is pushed goes straight into the working tree.
//!
//! A push is taken apart here rather than by `git receive-pack`, whose checks
//! would have to run as hooks in processes of their own.

mod pack;

use std::collections::BTreeSet;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context as _;

use crate::check::{Severity, check_files};
use crate::{Notebook, STATE, git, slash};

/// The most bytes one push may carry.
pub const PUSH_LIMIT: usize = 64 * 1024 * 1024;
/// The most bytes one file may have.
pub const FILE_LIMIT: u64 = 8 * 1024 * 1024;

/// The most bytes the files one push changes may have together, however small
/// they are packed.
pub const CHANGES_LIMIT: u64 = 256 * 1024 * 1024;
/// The most bytes the list of the files one push changes may have.
const LIST_LIMIT: u64 = 8 * 1024 * 1024;

/// What became of a push, and the answer to send back.
#[derive(Debug)]
pub struct Push {
    pub answer: Vec<u8>,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It was an opening with nothing in it.
    Nothing,
    Refused(String),
    Taken(Taken),
}

/// A push that is now in the notebook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    pub commit: String,
    /// The files it changed, relative to the notebook.
    pub files: Vec<String>,
}

/// The refs a copy may fetch: the notebook's branch alone.
pub fn refs_to_fetch(notebook: &Notebook) -> anyhow::Result<Vec<u8>> {
    let mut answer = opening("git-upload-pack");
    let mut command = upload_pack(notebook)?;
    command.args(["--http-backend-info-refs", "."]);
    answer.extend(run(command, &[])?);
    Ok(answer)
}

/// Answer a copy's request for objects.
pub fn fetch(notebook: &Notebook, request: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut command = upload_pack(notebook)?;
    command.arg(".");
    run(command, request)
}

/// The ref a copy may push to: the notebook's branch, and where it is.
pub fn refs_to_push(notebook: &Notebook) -> anyhow::Result<Vec<u8>> {
    let branch = branch(notebook)?;
    let head = head(notebook)?;
    let format = text(plain(notebook, &["rev-parse", "--show-object-format"]))?;
    let mut answer = opening("git-receive-pack");
    // Neither `delete-refs` nor `side-band-64k` is offered, so a copy asks
    // for no deletion and reads the answer as it is.
    answer.extend(line(&format!(
        "{head} {branch}\0report-status ofs-delta object-format={format} agent=famulen-notebook\n"
    )));
    answer.extend(b"0000");
    Ok(answer)
}

/// Take what a copy pushes into the notebook: the commits into its branch and
/// the files into its working tree, or nothing at all. `shared` is asked
/// whether it still may be, while the notebook is held to take it.
pub fn push(
    notebook: &Notebook,
    request: &[u8],
    shared: impl Fn() -> anyhow::Result<bool>,
) -> anyhow::Result<Push> {
    let (asked, pack) = asked(request)?;
    if asked.is_empty() {
        return Ok(Push {
            answer: Vec::new(),
            outcome: Outcome::Nothing,
        });
    }
    let outcome = match take(notebook, &asked, pack, &shared) {
        Ok(taken) => Outcome::Taken(taken),
        Err(error) => match error.downcast::<Refusal>() {
            Ok(Refusal(why)) => Outcome::Refused(why),
            Err(error) => return Err(error),
        },
    };
    let mut answer = line("unpack ok\n");
    for update in &asked {
        answer.extend(line(&match &outcome {
            Outcome::Refused(why) => {
                let why = why.split_whitespace().collect::<Vec<_>>().join(" ");
                let shown: String = why.chars().take(WHY_LIMIT).collect();
                format!("ng {} {shown}\n", update.to)
            }
            _ => format!("ok {}\n", update.to),
        }));
    }
    answer.extend(b"0000");
    Ok(Push { answer, outcome })
}

/// The most characters of a refusal sent back: one protocol line holds a
/// limited number of bytes.
const WHY_LIMIT: usize = 4000;

/// Why a push is not taken, as the one who pushed reads it.
#[derive(Debug)]
struct Refusal(String);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refusal {}

fn refuse<T>(why: impl Into<String>) -> anyhow::Result<T> {
    Err(Refusal(why.into()).into())
}

/// A ref a push asks to move.
#[derive(Debug)]
struct Update {
    from: String,
    to: String,
    commit: String,
}

/// What a push asks for, and the pack that follows.
fn asked(request: &[u8]) -> anyhow::Result<(Vec<Update>, &[u8])> {
    let mut rest = request;
    let mut updates = Vec::new();
    loop {
        let length = rest
            .get(..4)
            .and_then(|n| std::str::from_utf8(n).ok())
            .and_then(|n| usize::from_str_radix(n, 16).ok())
            .context("not a push")?;
        if length == 0 {
            return Ok((updates, &rest[4..]));
        }
        let body = rest.get(4..length).context("a push cut short")?;
        rest = &rest[length..];
        let body = body.split(|b| *b == 0).next().unwrap_or_default();
        let body = std::str::from_utf8(body).context("a push that is not UTF-8")?;
        let words: Vec<&str> = body.trim_end().split(' ').collect();
        match words[..] {
            [from, commit, to] if is_id(from) && is_id(commit) => updates.push(Update {
                from: from.to_owned(),
                to: to.to_owned(),
                commit: commit.to_owned(),
            }),
            ["shallow", _] => anyhow::bail!("a push from a shallow copy is not taken"),
            _ => anyhow::bail!("not a push"),
        }
    }
}

fn is_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn take(
    notebook: &Notebook,
    asked: &[Update],
    pack: &[u8],
    shared: &dyn Fn() -> anyhow::Result<bool>,
) -> anyhow::Result<Taken> {
    let branch = branch(notebook)?;
    let [update] = asked else {
        return refuse(format!("only {branch} can be pushed, alone"));
    };
    if update.to != branch {
        return refuse(format!("only {branch} can be pushed"));
    }
    if update.commit.bytes().all(|b| b == b'0') {
        return refuse(format!("{branch} cannot be deleted"));
    }
    if pack.len() > PUSH_LIMIT {
        return refuse("the push is too large");
    }
    if update.from != head(notebook)? {
        return refuse("fetch first");
    }
    let arrival = Arrival::unpack(notebook, pack, &update.commit)?;
    arrival.check_history(update)?;
    let changes = arrival.changes(update)?;
    arrival.check_changes(&changes)?;
    arrival.check_rules(&update.commit, &changes)?;
    let _hold = notebook.hold()?;
    if !shared()? {
        return refuse("the notebook is no longer shared");
    }
    arrival.settle(&branch, update)?;
    Ok(Taken {
        commit: update.commit.clone(),
        files: changes.into_iter().map(|c| c.path).collect(),
    })
}

/// A file a push changes.
struct Change {
    path: String,
    mode: String,
    object: String,
}

impl Change {
    fn is_removal(&self) -> bool {
        self.mode == "000000"
    }
}

/// What was pushed, unpacked apart from the notebook until it is taken.
struct Arrival<'a> {
    notebook: &'a Notebook,
    dir: PathBuf,
    /// Where the notebook keeps its objects.
    objects: PathBuf,
}

impl Drop for Arrival<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl<'a> Arrival<'a> {
    fn unpack(notebook: &'a Notebook, pack: &[u8], commit: &str) -> anyhow::Result<Self> {
        // Also what later commits take out again stays in the history.
        if let Err(error) = pack::bounded(pack, commit.len() / 2, FILE_LIMIT, CHANGES_LIMIT) {
            return refuse(format!("what was pushed is not taken: {error:#}"));
        }
        let objects = text(plain(notebook, &["rev-parse", "--git-path", "objects"]))?;
        let objects = std::path::absolute(notebook.root().join(objects))?;
        let dir = std::path::absolute(notebook.state_dir())?.join(format!(
            "incoming-{}",
            ulid::Ulid::generate().to_string().to_lowercase()
        ));
        let arrival = Self {
            notebook,
            dir,
            objects,
        };
        std::fs::create_dir_all(arrival.unpacked().join("pack"))?;
        std::fs::create_dir_all(arrival.unpacked().join("info"))?;
        std::fs::write(
            arrival.unpacked().join("info/alternates"),
            format!("{}\n", slash(&arrival.objects)),
        )?;
        let limit = format!("--max-input-size={PUSH_LIMIT}");
        let args = ["index-pack", "--stdin", "--fix-thin", "--strict", &limit];
        if let Err(error) = run(arrival.command(&args), pack) {
            return refuse(format!("what was pushed cannot be read: {error:#}"));
        }
        let kind = run(arrival.command(&["cat-file", "-t", commit]), &[]);
        if kind.ok().as_deref() != Some(b"commit\n") {
            return refuse("what was pushed is not a commit");
        }
        Ok(arrival)
    }

    fn unpacked(&self) -> PathBuf {
        self.dir.join("objects")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = plain(self.notebook, args);
        command.env("GIT_OBJECT_DIRECTORY", self.unpacked());
        command
    }

    fn check_history(&self, update: &Update) -> anyhow::Result<()> {
        let goes_on = self
            .command(&["merge-base", "--is-ancestor", &update.from, &update.commit])
            .output()
            .context("running git merge-base")?;
        if !goes_on.status.success() {
            return refuse("history cannot be rewritten; fetch first");
        }
        let whole = self.command(&[
            "rev-list",
            "--objects",
            "--quiet",
            &update.commit,
            "--not",
            &update.from,
        ]);
        if run(whole, &[]).is_err() {
            return refuse("what was pushed is not whole");
        }
        Ok(())
    }

    fn changes(&self, update: &Update) -> anyhow::Result<Vec<Change>> {
        // Directories that hold one another many times over make a long list
        // of a small push.
        let args = [
            "diff-tree",
            "-r",
            "-z",
            "--no-renames",
            &update.from,
            &update.commit,
        ];
        let mut child = self
            .command(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("running git diff-tree")?;
        let mut out = Vec::new();
        child
            .stdout
            .take()
            .expect("stdout is piped")
            .take(LIST_LIMIT + 1)
            .read_to_end(&mut out)?;
        if out.len() as u64 > LIST_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            return refuse("the push changes too many files");
        }
        anyhow::ensure!(child.wait()?.success(), "git diff-tree failed");
        let mut parts = out.split(|b| *b == 0);
        let mut changes = Vec::new();
        while let Some(about) = parts.next().filter(|a| !a.is_empty()) {
            let path = parts.next().context("a change without its file")?;
            let Ok(path) = std::str::from_utf8(path) else {
                return refuse(format!(
                    "a file's name is not UTF-8: {}",
                    String::from_utf8_lossy(path)
                ));
            };
            let about = String::from_utf8_lossy(about);
            let words: Vec<&str> = about.split(' ').collect();
            let [_, mode, _, object, _] = words[..] else {
                anyhow::bail!("unexpected git diff-tree line: {about:?}");
            };
            changes.push(Change {
                path: path.to_owned(),
                mode: mode.to_owned(),
                object: object.to_owned(),
            });
        }
        Ok(changes)
    }

    fn check_changes(&self, changes: &[Change]) -> anyhow::Result<()> {
        for change in changes {
            let path = &change.path;
            let lower = path.to_lowercase();
            let mut names = lower.split('/');
            if lower.contains(['\\', ':'])
                || names.next() == Some(STATE)
                || names.next_back().is_some_and(is_git_file)
                || lower.split('/').any(is_git_file)
            {
                return refuse(format!("{path} cannot be in a shared notebook"));
            }
            if !change.is_removal() && !matches!(&*change.mode, "100644" | "100755") {
                return refuse(format!("{path} is not an ordinary file"));
            }
        }
        let kept: Vec<&Change> = changes.iter().filter(|c| !c.is_removal()).collect();
        let asked: String = kept.iter().map(|c| format!("{}\n", c.object)).collect();
        let mut all = 0;
        for (change, size) in kept.iter().zip(self.sizes(asked.as_bytes())?) {
            if size > FILE_LIMIT {
                return refuse(format!("{} is too large", change.path));
            }
            all += size;
            if all > CHANGES_LIMIT {
                return refuse("the files pushed are too large together");
            }
        }
        Ok(())
    }

    /// The sizes of these objects, given one a line.
    fn sizes(&self, objects: &[u8]) -> anyhow::Result<Vec<u64>> {
        let sizes = run(
            self.command(&["cat-file", "--batch-check=%(objectsize)"]),
            objects,
        )?;
        String::from_utf8_lossy(&sizes)
            .lines()
            .map(|size| size.parse().context("reading an object's size"))
            .collect()
    }

    /// Refuse what would leave a changed file wrong where it was not.
    fn check_rules(&self, commit: &str, changes: &[Change]) -> anyhow::Result<()> {
        let tree = self.dir.join("tree");
        std::fs::create_dir_all(&tree)?;
        let index = self.dir.join("index");
        let with_index = |args: &[&str]| {
            let mut command = self.command(args);
            command.env("GIT_INDEX_FILE", &index);
            command
        };
        run(with_index(&["read-tree", commit]), &[])?;
        let prefix = format!("--prefix={}/", slash(&tree));
        let checkout = with_index(&["checkout-index", "--all", "-q", &prefix]);
        run(checkout, &[])?;

        let wrong = |notebook: &Notebook| -> anyhow::Result<BTreeSet<(String, &str, String)>> {
            let files: Vec<String> = changes
                .iter()
                .map(|c| c.path.clone())
                .filter(|p| notebook.root().join(p).is_file())
                .collect();
            Ok(check_files(notebook, &files)?
                .into_iter()
                .filter(|f| f.severity == Severity::Error)
                .map(|f| (f.path, f.rule, f.why))
                .collect())
        };
        let before = wrong(self.notebook)?;
        let new: Vec<String> = wrong(&Notebook::new(&tree))?
            .difference(&before)
            .map(|(path, rule, why)| format!("{path} would be wrong: {rule}: {why}"))
            .collect();
        if new.is_empty() {
            return Ok(());
        }
        refuse(new.join("; "))
    }

    /// Move the branch and the working tree to what was pushed. The notebook
    /// is held.
    fn settle(&self, branch: &str, update: &Update) -> anyhow::Result<()> {
        if head(self.notebook)? != update.from {
            return refuse("fetch first");
        }
        let kept = self.keep_objects()?;
        let moved = advance(self.notebook, branch, &update.from, &update.commit);
        // After any other failure the branch may have moved already.
        if matches!(moved, Err(NotMoved::Written | NotMoved::InTheWay(_))) {
            for file in kept.iter().rev() {
                let _ = std::fs::remove_file(file);
            }
        }
        match moved {
            Ok(()) => Ok(()),
            Err(NotMoved::Written) => refuse("the notebook is being written; fetch first"),
            Err(NotMoved::InTheWay(why)) => refuse(format!(
                "it touches files changed here and not yet committed; try again later: {why}"
            )),
            Err(NotMoved::Failed(error)) => Err(error),
        }
    }

    /// Give the notebook what was unpacked. The files it did not have yet.
    fn keep_objects(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut kept = Vec::new();
        let to = self.objects.join("pack");
        std::fs::create_dir_all(&to)?;
        let mut files: Vec<PathBuf> = std::fs::read_dir(self.unpacked().join("pack"))?
            .map(|f| Ok(f?.path()))
            .collect::<std::io::Result<_>>()?;
        // A pack is there for Git once its index is.
        files.sort_by_key(|f| f.extension().is_some_and(|e| e == "idx"));
        for file in files {
            let to = to.join(file.file_name().unwrap_or_default());
            if !to.exists() {
                std::fs::rename(&file, &to).with_context(|| format!("keeping {}", to.display()))?;
                kept.push(to);
            }
        }
        Ok(kept)
    }
}

/// Files Git itself reads, through which a push could change how the notebook
/// is checked out.
fn is_git_file(name: &str) -> bool {
    matches!(name, ".git" | ".gitattributes" | ".gitmodules")
}

/// Why a branch and the working tree were not moved.
#[derive(Debug)]
pub(crate) enum NotMoved {
    /// Someone else moves the branch.
    Written,
    /// Changes not yet committed would be lost.
    InTheWay(String),
    Failed(anyhow::Error),
}

impl std::fmt::Display for NotMoved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotMoved::Written => f.write_str("the notebook is being written"),
            NotMoved::InTheWay(why) => f.write_str(why),
            NotMoved::Failed(error) => write!(f, "{error:#}"),
        }
    }
}

/// Move the branch and the working tree from one commit to another, keeping
/// what is not committed; both or neither.
pub(crate) fn advance(
    notebook: &Notebook,
    branch: &str,
    from: &str,
    to: &str,
) -> Result<(), NotMoved> {
    let _turn = git::committing();
    // Files touched but left as they were would look changed.
    plain(notebook, &["update-index", "-q", "--refresh"])
        .output()
        .context("running git update-index")
        .map_err(NotMoved::Failed)?;
    let mut moving = MovingRef::prepare(notebook, branch, from, to).map_err(|error| {
        tracing::debug!(%error, "could not lock the branch");
        NotMoved::Written
    })?;
    if let Err(error) = run(plain(notebook, &["read-tree", "-m", "-u", from, to]), &[]) {
        moving.say("abort").map_err(NotMoved::Failed)?;
        return Err(NotMoved::InTheWay(format!("{error:#}")));
    }
    moving.say("commit").map_err(NotMoved::Failed)
}

/// A branch locked to be moved, so that the working tree is changed only when
/// the branch can be.
struct MovingRef {
    child: std::process::Child,
    answers: BufReader<std::process::ChildStdout>,
}

impl MovingRef {
    fn prepare(notebook: &Notebook, branch: &str, from: &str, to: &str) -> anyhow::Result<Self> {
        let mut child = plain(notebook, &["update-ref", "-m", "sync", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("running git update-ref")?;
        let answers = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut moving = Self { child, answers };
        moving.say("start")?;
        writeln!(moving.orders(), "update {branch} {to} {from}")?;
        moving.say("prepare")?;
        Ok(moving)
    }

    fn orders(&mut self) -> &mut std::process::ChildStdin {
        self.child.stdin.as_mut().expect("stdin is piped")
    }

    fn say(&mut self, word: &str) -> anyhow::Result<()> {
        writeln!(self.orders(), "{word}")?;
        let mut answer = String::new();
        self.answers.read_line(&mut answer)?;
        anyhow::ensure!(
            answer.trim() == format!("{word}: ok"),
            "git update-ref answered {:?} to {word}",
            answer.trim()
        );
        Ok(())
    }
}

impl Drop for MovingRef {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

fn opening(service: &str) -> Vec<u8> {
    let mut out = line(&format!("# service={service}\n"));
    out.extend(b"0000");
    out
}

/// One line of Git's protocol: its length, then itself.
fn line(text: &str) -> Vec<u8> {
    format!("{:04x}{text}", text.len() + 4).into_bytes()
}

fn upload_pack(notebook: &Notebook) -> anyhow::Result<Command> {
    let branch = branch(notebook)?;
    // Git's protocol 2, which lets a copy ask for objects no ref leads to, is
    // not spoken: GIT_PROTOCOL stays unset.
    let mut command = plain(notebook, &["-c", "transfer.hideRefs=refs", "-c"]);
    command.arg(format!("transfer.hideRefs=!{branch}"));
    command
        .args(["upload-pack", "--stateless-rpc"])
        .env_remove("GIT_PROTOCOL");
    Ok(command)
}

/// A Git command in the notebook that no configuration, attribute or hook
/// outside it, nor any hook of it, has a say in.
pub(crate) fn plain(notebook: &Notebook, args: &[&str]) -> Command {
    let no_hooks = notebook.state_dir().join("no-hooks");
    let mut command = git::command(notebook.root(), &[]);
    command
        .args(["-c", "core.attributesFile=", "-c", "core.fsmonitor=false"])
        .args(["-c", "core.protectHFS=true", "-c", "core.protectNTFS=true"])
        .args([
            "-c",
            &format!("core.hooksPath={}", slash(&absolute(&no_hooks))),
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("LC_ALL", "C");
    command
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

pub(crate) fn branch(notebook: &Notebook) -> anyhow::Result<String> {
    text(plain(notebook, &["symbolic-ref", "HEAD"])).context("the notebook is not on a branch")
}

pub(crate) fn head(notebook: &Notebook) -> anyhow::Result<String> {
    text(plain(notebook, &["rev-parse", "--verify", "HEAD"]))
}

pub(crate) fn text(command: Command) -> anyhow::Result<String> {
    Ok(String::from_utf8_lossy(&run(command, &[])?)
        .trim()
        .to_owned())
}

/// Run a command to its end, and give what it wrote.
pub(crate) fn run(mut command: Command, input: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running git")?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let output = std::thread::scope(|scope| {
        // Written beside, so that neither side waits for the other to read.
        scope.spawn(move || {
            // Git may end before it has read everything; its status tells.
            let _ = stdin.write_all(input);
        });
        child.wait_with_output()
    })
    .context("running git")?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}

#[cfg(test)]
mod tests;
