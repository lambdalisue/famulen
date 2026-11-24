//! Keeping a copy of a notebook one with the notebook it was cloned from.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context as _;

use crate::doc::merge;
use crate::memory::Layer;
use crate::share::{advance, branch, head, plain, run, text};
use crate::{Doc, MEMORY, Notebook, git};

const REMOTE: &str = "origin";
/// Set only in a copy made by [`clone`]: a remote the owner added for
/// something else is never talked to.
const ENABLED: &str = "famulen-notebook.sync";
const STAMP: &str = "synced";

/// How long to wait to hear whether there is anything new.
const ASK: Duration = Duration::from_secs(3);
/// How long what there is may take to come or go.
const CARRY: Duration = Duration::from_secs(120);
/// How often to try again when others push at the same time.
const TRIES: usize = 4;

/// Why a copy is not one with the notebook it was cloned from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trouble {
    /// It does not answer.
    Away(String),
    /// These files were changed in both, in ways that do not go together.
    Conflict(Vec<String>),
    /// Changes not yet committed are in the way of what came.
    InTheWay(String),
    /// It does not take what was written here.
    Refused(String),
}

impl std::fmt::Display for Trouble {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Trouble::Away(why) => write!(f, "the shared notebook does not answer: {why}"),
            Trouble::Conflict(files) => write!(
                f,
                "changed both here and in the shared notebook: {}; put them together with \
                 `git merge {REMOTE}`, then run `famulen-notebook sync`",
                files.join(", ")
            ),
            Trouble::InTheWay(why) => write!(
                f,
                "what is not yet committed here is in the way of what came: {why}"
            ),
            Trouble::Refused(why) => {
                write!(
                    f,
                    "the shared notebook did not take what was written: {why}"
                )
            }
        }
    }
}

impl std::error::Error for Trouble {}

impl Trouble {
    /// Whether it goes by itself once the shared notebook answers.
    pub fn passes(&self) -> bool {
        matches!(self, Trouble::Away(_))
    }
}

/// Make a copy of the notebook at `url` in `dir`, one that is kept in sync.
pub fn clone(url: impl AsRef<str>, dir: impl AsRef<Path>) -> anyhow::Result<Notebook> {
    let dir = dir.as_ref();
    let mut command = git::command(
        Path::new("."),
        &["clone", "-q", "--template=", "-o", REMOTE],
    );
    for (key, value) in [
        ("core.autocrlf", "false"),
        ("core.eol", "lf"),
        ("core.hooksPath", ".git/hooks"),
        ("core.quotePath", "false"),
        ("commit.gpgsign", "false"),
        (ENABLED, "true"),
    ] {
        command.arg("-c").arg(format!("{key}={value}"));
    }
    command.arg("--").arg(url.as_ref()).arg(dir);
    run(command, &[]).context("cloning the notebook")?;
    let notebook = Notebook::new(dir);
    anyhow::ensure!(
        notebook.memory_dir().is_dir() && notebook.records_dir().is_dir(),
        "what {} holds is not a notebook",
        url.as_ref()
    );
    Ok(notebook)
}

/// A copy of a notebook, made by [`clone`].
pub struct Sync {
    notebook: Notebook,
}

impl Sync {
    /// Nothing when the notebook is no such copy.
    pub fn of(notebook: &Notebook) -> Option<Self> {
        if !notebook.root().join(".git").exists() {
            return None;
        }
        let args = ["config", "--local", "--type=bool", "--get", ENABLED];
        let enabled = text(git::command(notebook.root(), &args)).ok()?;
        (enabled == "true").then(|| Self {
            notebook: notebook.clone(),
        })
    }

    /// Bring in what is new, unless that was tried within `age`.
    pub fn refresh(&self, age: Duration) -> anyhow::Result<()> {
        let tried = std::fs::metadata(self.notebook.state_dir().join(STAMP))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok());
        match tried {
            Some(since) if since < age => Ok(()),
            _ => self.pull(),
        }
    }

    /// Bring in what is new in the shared notebook.
    pub fn pull(&self) -> anyhow::Result<()> {
        let _hold = self.notebook.hold()?;
        let pulled = self.fetch().and_then(|()| self.take_in());
        // Also while the shared notebook is away, so that not every read
        // waits for it; but what does not go by itself is told every time.
        let settled = match &pulled {
            Ok(()) => true,
            Err(error) => error.downcast_ref().is_some_and(Trouble::passes),
        };
        let stamp = self.notebook.state_dir().join(STAMP);
        if settled {
            std::fs::write(stamp, "")?;
        } else {
            let _ = std::fs::remove_file(stamp);
        }
        pulled
    }

    /// Bring in what is new, and send what was written here.
    pub fn sync(&self) -> anyhow::Result<()> {
        let _hold = self.notebook.hold()?;
        self.pull()?;
        self.push()
    }

    /// Send what was written here, taking in what others sent meanwhile.
    pub fn push(&self) -> anyhow::Result<()> {
        let _hold = self.notebook.hold()?;
        self.take_in()?;
        for _ in 1..TRIES {
            match self.send()? {
                Sent::Yes => return Ok(()),
                Sent::Overtaken => self.take_in()?,
            }
        }
        Err(Trouble::Refused("others write it all the time; try again later".into()).into())
    }

    fn fetch(&self) -> anyhow::Result<()> {
        let branch = branch(&self.notebook)?;
        let seen = format!("refs/remotes/{REMOTE}/{}", short(&branch));
        let asked = self.remote(&["ls-remote", "-q", "--exit-code", REMOTE, &branch], ASK)?;
        if !asked.ok {
            return Err(Trouble::Away(asked.err).into());
        }
        let there = asked.out.split_whitespace().next().unwrap_or_default();
        if self.at(&seen).as_deref() == Some(there) {
            return Ok(());
        }
        let spec = format!("+{branch}:{seen}");
        let fetched = self.remote(&["fetch", "-q", "--no-tags", REMOTE, &spec], CARRY)?;
        if !fetched.ok {
            return Err(Trouble::Away(fetched.err).into());
        }
        Ok(())
    }

    fn at(&self, name: &str) -> Option<String> {
        text(plain(
            &self.notebook,
            &["rev-parse", "-q", "--verify", name],
        ))
        .ok()
    }

    /// Make what was fetched and what is here one.
    fn take_in(&self) -> anyhow::Result<()> {
        let branch = branch(&self.notebook)?;
        let here = head(&self.notebook)?;
        let there = self
            .at(&format!("refs/remotes/{REMOTE}/{}", short(&branch)))
            .context("nothing was fetched")?;
        if self.leads_to(&there, &here)? {
            return Ok(());
        }
        let to = if self.leads_to(&here, &there)? {
            there
        } else {
            self.merge(&here, &there)?
        };
        advance(&self.notebook, &branch, &here, &to)
            .map_err(|error| Trouble::InTheWay(format!("{error:#}")).into())
    }

    fn leads_to(&self, old: &str, new: &str) -> anyhow::Result<bool> {
        Ok(
            plain(&self.notebook, &["merge-base", "--is-ancestor", old, new])
                .output()
                .context("running git merge-base")?
                .status
                .success(),
        )
    }

    /// A commit that puts both together, made without touching the working
    /// tree.
    fn merge(&self, here: &str, there: &str) -> anyhow::Result<String> {
        let merged = plain(
            &self.notebook,
            &[
                "-c",
                "merge.renames=false",
                "merge-tree",
                "--write-tree",
                "-z",
                "--no-messages",
                here,
                there,
            ],
        )
        .output()
        .context("running git merge-tree")?;
        let out = String::from_utf8_lossy(&merged.stdout).into_owned();
        let mut parts = out.split('\0').filter(|p| !p.is_empty());
        let tree = match (merged.status.code(), parts.next()) {
            (Some(0 | 1), Some(tree)) => tree.trim().to_owned(),
            _ => anyhow::bail!(
                "git merge-tree failed: {}",
                String::from_utf8_lossy(&merged.stderr).trim()
            ),
        };
        let mut conflicts: BTreeMap<&str, [Option<&str>; 3]> = Default::default();
        for part in parts {
            let (about, path) = part.split_once('\t').context("a conflict without a file")?;
            let words: Vec<&str> = about.split(' ').collect();
            let [_, object, stage] = words[..] else {
                anyhow::bail!("unexpected git merge-tree line: {part:?}");
            };
            let stage: usize = stage.parse().context("reading a conflict's stage")?;
            let stages = conflicts.entry(path).or_default();
            *stages
                .get_mut(stage.wrapping_sub(1))
                .context("unknown stage")? = Some(object);
        }
        // Git may put lines together that name the same item.
        let both = self.items_changed_in_both(here, there)?;
        for (path, stages) in &both {
            let stages = stages.each_ref().map(|s| s.as_deref());
            conflicts.insert(path, stages);
        }
        let mut settled = Vec::new();
        let mut unsettled = Vec::new();
        for (path, stages) in conflicts {
            match self.settle(path, stages)? {
                Some(text) => settled.push((path, text)),
                None => unsettled.push(path.to_owned()),
            }
        }
        if !unsettled.is_empty() {
            return Err(Trouble::Conflict(unsettled).into());
        }
        let tree = self.tree_with(&tree, &settled)?;
        let message = format!(
            "別のコピーの変更とまとめた\n\nNotebook-Tool: {}",
            self.notebook.tool
        );
        let mut commit = plain(&self.notebook, &["commit-tree", &tree]);
        commit.args(["-p", here, "-p", there]);
        git::as_tool(&mut commit);
        Ok(String::from_utf8_lossy(&run(commit, message.as_bytes())?)
            .trim()
            .to_owned())
    }

    /// The files of items with ids that differ between both, and from what
    /// both come from: what each was, is here and is there.
    fn items_changed_in_both(
        &self,
        here: &str,
        there: &str,
    ) -> anyhow::Result<BTreeMap<String, [Option<String>; 3]>> {
        let base = text(plain(&self.notebook, &["merge-base", here, there]))?;
        let mut files: BTreeMap<String, [Option<String>; 3]> = BTreeMap::new();
        for (side, commit) in [&*base, here, there].into_iter().enumerate() {
            let args = ["ls-tree", "-r", "-z", commit, "--", MEMORY];
            let out = run(plain(&self.notebook, &args), &[])?;
            for line in String::from_utf8_lossy(&out).split('\0') {
                let Some((about, path)) = line.split_once('\t') else {
                    continue;
                };
                if holds_items(path) {
                    let object = about.rsplit(' ').next().map(str::to_owned);
                    files.entry(path.to_owned()).or_default()[side] = object;
                }
            }
        }
        files.retain(|_, [was, ours, theirs]| {
            ours.is_some() && theirs.is_some() && ours != theirs && was != ours && was != theirs
        });
        Ok(files)
    }

    /// A conflicted file made one, when it holds items with ids.
    fn settle(&self, path: &str, stages: [Option<&str>; 3]) -> anyhow::Result<Option<String>> {
        let [base, Some(ours), Some(theirs)] = stages else {
            return Ok(None);
        };
        if !holds_items(path) {
            return Ok(None);
        }
        let read = |object: &str| -> anyhow::Result<Option<Doc>> {
            let bytes = run(plain(&self.notebook, &["cat-file", "blob", object]), &[])?;
            Ok(String::from_utf8(bytes).ok().map(|t| Doc::parse(&t)))
        };
        let base = match base {
            Some(object) => read(object)?,
            None => Some(Doc::default()),
        };
        let (Some(base), Some(ours), Some(theirs)) = (base, read(ours)?, read(theirs)?) else {
            return Ok(None);
        };
        Ok(merge::merge(&base, &ours, &theirs).map(|d| d.render()))
    }

    fn tree_with(&self, tree: &str, files: &[(&str, String)]) -> anyhow::Result<String> {
        if files.is_empty() {
            return Ok(tree.to_owned());
        }
        let index = std::path::absolute(self.notebook.state_dir().join("merge-index"))?;
        let with_index = |args: &[&str]| {
            let mut command = plain(&self.notebook, args);
            command.env("GIT_INDEX_FILE", &index);
            command
        };
        run(with_index(&["read-tree", tree]), &[])?;
        for (path, content) in files {
            let write = plain(&self.notebook, &["hash-object", "-w", "--stdin"]);
            let object = String::from_utf8_lossy(&run(write, content.as_bytes())?)
                .trim()
                .to_owned();
            let info = format!("100644,{object},{path}");
            run(with_index(&["update-index", "--cacheinfo", &info]), &[])?;
        }
        let tree = text(with_index(&["write-tree"]));
        let _ = std::fs::remove_file(&index);
        tree
    }

    fn send(&self) -> anyhow::Result<Sent> {
        let branch = branch(&self.notebook)?;
        let seen = format!("refs/remotes/{REMOTE}/{}", short(&branch));
        let there = self.at(&seen);
        if there == Some(head(&self.notebook)?) {
            return Ok(Sent::Yes);
        }
        let spec = format!("HEAD:{branch}");
        let pushed = self.remote(&["push", "--porcelain", REMOTE, &spec], CARRY)?;
        if pushed.ok {
            return Ok(Sent::Yes);
        }
        self.fetch()?;
        if self.at(&seen) != there {
            return Ok(Sent::Overtaken);
        }
        let why = pushed
            .out
            .lines()
            .filter_map(|l| l.strip_prefix('!'))
            .map(|l| l.trim().rsplit('\t').next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("; ");
        Err(Trouble::Refused(if why.is_empty() { pushed.err } else { why }).into())
    }

    fn remote(&self, args: &[&str], within: Duration) -> anyhow::Result<Ran> {
        // The owner's own Git configuration applies here: their proxy, say,
        // is how the shared notebook is reached.
        let mut command = git::command(self.notebook.root(), args);
        command.env("LC_ALL", "C");
        run_within(command, within)
    }
}

enum Sent {
    Yes,
    /// Others sent something first.
    Overtaken,
}

/// What a command said.
struct Ran {
    ok: bool,
    out: String,
    err: String,
}

/// Run a command, ending it when it takes too long.
fn run_within(mut command: Command, within: Duration) -> anyhow::Result<Ran> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running git")?;
    // What Git started may outlive it and keep the pipes open, so what they
    // carry is waited for no longer than the command itself.
    let heard = |mut from: Box<dyn std::io::Read + Send>| {
        let (told, heard) = mpsc::channel();
        std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = from.read_to_end(&mut text);
            let _ = told.send(String::from_utf8_lossy(&text).trim().to_owned());
        });
        heard
    };
    let out = heard(Box::new(child.stdout.take().expect("stdout is piped")));
    let err = heard(Box::new(child.stderr.take().expect("stderr is piped")));
    let until = Instant::now() + within;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= until {
            child.kill()?;
            child.wait()?;
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let Some(status) = status else {
        return Ok(Ran {
            ok: false,
            out: String::new(),
            err: format!("no answer within {} seconds", within.as_secs()),
        });
    };
    let rest = Duration::from_secs(5);
    Ok(Ran {
        ok: status.success(),
        out: out.recv_timeout(rest).unwrap_or_default(),
        err: err.recv_timeout(rest).unwrap_or_default(),
    })
}

/// Whether a file holds items with ids, a line each.
fn holds_items(path: &str) -> bool {
    let layer = path
        .strip_prefix(MEMORY)
        .and_then(|p| p.strip_prefix('/'))
        .and_then(Layer::of);
    matches!(
        layer,
        Some(Layer::Core | Layer::Pending | Layer::Episodic | Layer::Forgotten)
    )
}

fn short(branch: &str) -> &str {
    branch.strip_prefix("refs/heads/").unwrap_or(branch)
}

#[cfg(test)]
mod tests;
