//! The notebook's history, kept with the `git` command. Every function takes
//! a directory inside the repository and works on that directory alone, so the
//! memory layers can be committed, logged and put back without touching the
//! records beside them.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use anyhow::Context as _;
use chrono::{DateTime, FixedOffset};

use crate::STATE;

/// Who made a change to the notebook.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Author {
    pub name: String,
    pub email: String,
}

impl Author {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Self {
        Author {
            name: name.into(),
            email: email.into(),
        }
    }
}

/// The committer of every commit; the author is whoever wrote the change.
const COMMITTER_NAME: &str = "famulen-notebook";
const COMMITTER_EMAIL: &str = "famulen-notebook@localhost";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub id: String,
    pub author: Author,
    pub time: DateTime<FixedOffset>,
    pub message: String,
}

/// Lines added and removed in one file; `None` for a binary file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub path: String,
    pub added: Option<usize>,
    pub removed: Option<usize>,
}

/// Keeps every file's bytes as written: Git for Windows otherwise checks text
/// out with CRLF.
const AS_WRITTEN: [(&str, &str); 2] = [("core.autocrlf", "false"), ("core.eol", "lf")];

/// Make `dir` a repository when it is not one, and one that keeps files as
/// they are written.
pub(crate) fn init(dir: &Path) -> anyhow::Result<()> {
    if !dir.join(".git").exists() {
        return create(dir);
    }
    let set = git(dir, &["config", "--local", "--list"])?;
    for (key, value) in AS_WRITTEN {
        let current = set
            .lines()
            .rev()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='));
        if current != Some(value) {
            git(dir, &["config", "--local", key, value])?;
        }
    }
    Ok(())
}

fn create(dir: &Path) -> anyhow::Result<()> {
    // An empty template keeps the user's global hook templates out of the
    // notebook.
    git(dir, &["init", "-q", "--template=", "-b", "main"])?;
    let mut config = format!(
        "[user]\n\tname = {COMMITTER_NAME}\n\temail = {COMMITTER_EMAIL}\n\
         [commit]\n\tgpgsign = false\n\
         [core]\n\thooksPath = .git/hooks\n\tquotePath = false\n"
    );
    for (key, value) in AS_WRITTEN {
        let name = key.strip_prefix("core.").expect("a key of core");
        config.push_str(&format!("\t{name} = {value}\n"));
    }
    // Not with `git config`, which sets one value per process: starting one
    // is slow on macOS.
    let path = dir.join(".git").join("config");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(config.as_bytes()))
        .with_context(|| format!("writing {}", path.display()))
}

/// Wait for the commits this process makes to the repository `dir` is in to
/// be done, and keep others from starting.
pub(crate) fn committing(dir: &Path) -> MutexGuard<'static, ()> {
    static COMMITTING: Mutex<BTreeMap<PathBuf, &'static Mutex<()>>> = Mutex::new(BTreeMap::new());
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let root = dir
        .ancestors()
        .find(|at| at.join(".git").exists())
        .unwrap_or(&dir);
    let turn = *COMMITTING
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(root.to_path_buf())
        .or_insert_with(|| Box::leak(Box::default()));
    turn.lock().unwrap_or_else(|p| p.into_inner())
}

/// Record every change under `dir` as one commit by `author`, or do nothing
/// when nothing there changed. Returns the new commit's id.
pub fn commit_all(dir: &Path, author: &Author, message: &str) -> anyhow::Result<Option<String>> {
    commit_paths(dir, author, message, &["."])
}

/// Record the changes to `paths`, given relative to `dir`, as one commit by
/// `author`, leaving every other change as it is. A path that is neither on
/// disk nor in the history is passed over. Returns the new commit's id, or
/// `None` when none of them changed.
pub fn commit_paths(
    dir: &Path,
    author: &Author,
    message: &str,
    paths: &[impl AsRef<str>],
) -> anyhow::Result<Option<String>> {
    // git keeps one index per repository and refuses a second writer while
    // the first holds `index.lock`, so commits from this process take turns.
    let _turn = committing(dir);
    let mut known = Vec::new();
    for path in paths {
        let path = path.as_ref();
        if dir.join(path).exists() || !git(dir, &["ls-files", "--", path])?.trim().is_empty() {
            known.push(path);
        }
    }
    if known.is_empty() {
        return Ok(None);
    }
    let with_paths = |args: &[&'static str]| {
        let mut all: Vec<&str> = args.to_vec();
        all.push("--");
        all.extend(known.iter().copied());
        all
    };
    git(dir, &with_paths(&["add", "-A"]))?;
    // Not left to `.gitignore`, which a copy of the notebook may change.
    let state = format!(":(top){STATE}");
    let args = ["rm", "-rq", "--cached", "--ignore-unmatch", "--", &state];
    git(dir, &args)?;
    let unchanged = command(dir, &with_paths(&["diff", "--cached", "--quiet"]))
        .status()
        .context("running git diff")?
        .success();
    if unchanged {
        return Ok(None);
    }
    let output = command(
        dir,
        &with_paths(&["commit", "-q", "--no-verify", "-F", "-"]),
    )
    .env("GIT_COMMITTER_NAME", COMMITTER_NAME)
    .env("GIT_COMMITTER_EMAIL", COMMITTER_EMAIL)
    .env("GIT_AUTHOR_NAME", &author.name)
    .env("GIT_AUTHOR_EMAIL", &author.email)
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .spawn()
    .and_then(|mut child| {
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(message.as_bytes())?;
        child.wait_with_output()
    })
    .context("running git commit")?;
    anyhow::ensure!(
        output.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(Some(git(dir, &["rev-parse", "HEAD"])?.trim().to_string()))
}

/// The latest commits that changed something under `dir`, newest first.
pub fn log(dir: &Path, limit: usize) -> anyhow::Result<Vec<CommitInfo>> {
    if limit == 0 || !has_head(dir)? {
        return Ok(Vec::new());
    }
    let out = git(
        dir,
        &[
            "log",
            &format!("-n{limit}"),
            "--format=%H%x1f%an%x1f%ae%x1f%aI%x1f%B%x1e",
            "--",
            ".",
        ],
    )?;
    out.split('\x1e')
        .map(|record| record.trim_start_matches('\n'))
        .filter(|record| !record.is_empty())
        .map(|record| {
            let fields: Vec<&str> = record.splitn(5, '\x1f').collect();
            let [id, name, email, time, message] = fields[..] else {
                anyhow::bail!("unexpected git log record: {record:?}");
            };
            Ok(CommitInfo {
                id: id.to_string(),
                author: Author::new(name, email),
                time: DateTime::parse_from_rfc3339(time)
                    .with_context(|| format!("parsing commit time {time:?}"))?,
                message: message.trim_end().to_string(),
            })
        })
        .collect()
}

/// The files under `dir` one commit changed, relative to `dir`.
pub fn diff_stat(dir: &Path, commit: &str) -> anyhow::Result<Vec<FileStat>> {
    check_rev(commit)?;
    let out = git(
        dir,
        &[
            "show",
            "--numstat",
            "--relative",
            "--format=",
            commit,
            "--",
            ".",
        ],
    )?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let added = fields.next()?;
            let removed = fields.next()?;
            let path = fields.next()?;
            Some(FileStat {
                path: path.to_string(),
                added: added.parse().ok(),
                removed: removed.parse().ok(),
            })
        })
        .collect())
}

/// Put every file under `dir` back as it was at `commit`, dropping files made
/// since. History is left alone: the next [`commit_all`] records the return, and
/// `revert_to(dir, "HEAD")` throws away changes not yet committed.
pub fn revert_to(dir: &Path, commit: &str) -> anyhow::Result<()> {
    check_rev(commit)?;
    git(
        dir,
        &[
            "restore",
            "--source",
            commit,
            "--staged",
            "--worktree",
            "--",
            ".",
        ],
    )?;
    git(dir, &["clean", "-fdq", "--", "."])?;
    Ok(())
}

fn check_rev(commit: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !commit.is_empty() && !commit.starts_with('-'),
        "invalid commit: {commit:?}"
    );
    Ok(())
}

/// Let a command commit as this tool.
pub(crate) fn as_tool(command: &mut Command) {
    for who in ["AUTHOR", "COMMITTER"] {
        command
            .env(format!("GIT_{who}_NAME"), COMMITTER_NAME)
            .env(format!("GIT_{who}_EMAIL"), COMMITTER_EMAIL);
    }
}

fn has_head(dir: &Path) -> anyhow::Result<bool> {
    Ok(command(dir, &["rev-parse", "-q", "--verify", "HEAD"])
        .output()
        .context("running git rev-parse")?
        .status
        .success())
}

pub(crate) fn command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = command(dir, args)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    anyhow::ensure!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TempDir, write};

    fn me() -> Author {
        Author::new("Alice", "alice@example.com")
    }

    fn repo(name: &str) -> TempDir {
        let dir = TempDir::new(name);
        init(dir.path()).unwrap();
        dir
    }

    #[test]
    fn a_new_repository_commits_as_its_author_without_signing() {
        let dir = repo("git-init");
        std::fs::write(dir.path().join("core.md"), "# x\n").unwrap();

        let id = commit_all(dir.path(), &me(), "set up").unwrap();

        let log = log(dir.path(), 10).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(Some(&log[0].id), id.as_ref());
        assert_eq!(log[0].author, me());
        assert_eq!(log[0].message, "set up");
    }

    #[test]
    fn files_come_back_with_the_line_endings_they_were_written_with() {
        let dir = repo("git-line-endings");
        git(dir.path(), &["config", "core.autocrlf", "true"]).unwrap();
        init(dir.path()).unwrap();
        for text in ["a\n", "a\r\n"] {
            let path = dir.path().join("core.md");
            std::fs::write(&path, text).unwrap();
            commit_all(dir.path(), &me(), "write").unwrap();
            std::fs::write(&path, "changed\n").unwrap();

            revert_to(dir.path(), "HEAD").unwrap();

            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
    }

    #[test]
    fn nothing_is_committed_when_nothing_changed() {
        let dir = repo("git-unchanged");
        std::fs::write(dir.path().join("core.md"), "# x\n").unwrap();
        commit_all(dir.path(), &me(), "first").unwrap();

        let second = commit_all(dir.path(), &me(), "second").unwrap();

        assert_eq!(second, None);
        assert_eq!(log(dir.path(), 10).unwrap().len(), 1);
    }

    #[test]
    fn an_empty_repository_has_nothing_to_commit_or_log() {
        let dir = repo("git-empty");

        assert_eq!(commit_all(dir.path(), &me(), "x").unwrap(), None);
        assert!(log(dir.path(), 10).unwrap().is_empty());
    }

    #[test]
    fn the_owners_edit_is_authored_by_the_owner_and_keeps_a_multiline_message() {
        let dir = repo("git-owner");
        std::fs::write(dir.path().join("core.md"), "# x\n").unwrap();
        commit_all(dir.path(), &me(), "first").unwrap();
        let owner = Author::new("Owner", "owner@example.com");
        std::fs::write(dir.path().join("core.md"), "# x\n- y\n").unwrap();

        commit_all(dir.path(), &owner, "edit core\n\nby hand").unwrap();

        let log = log(dir.path(), 1).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].author, owner);
        assert_eq!(log[0].message, "edit core\n\nby hand");
    }

    #[test]
    fn diff_stat_lists_the_lines_a_commit_changed_per_file() {
        let dir = repo("git-stat");
        std::fs::write(dir.path().join("core.md"), "a\n").unwrap();
        commit_all(dir.path(), &me(), "first").unwrap();
        std::fs::write(dir.path().join("core.md"), "b\nc\n").unwrap();
        std::fs::write(dir.path().join("pending.md"), "d\n").unwrap();
        let id = commit_all(dir.path(), &me(), "second").unwrap().unwrap();

        let stat = diff_stat(dir.path(), &id).unwrap();

        assert_eq!(
            stat,
            vec![
                FileStat {
                    path: "core.md".into(),
                    added: Some(2),
                    removed: Some(1)
                },
                FileStat {
                    path: "pending.md".into(),
                    added: Some(1),
                    removed: Some(0)
                },
            ]
        );
    }

    #[test]
    fn reverting_to_head_throws_away_uncommitted_edits_and_new_files() {
        let dir = repo("git-revert");
        std::fs::write(dir.path().join("core.md"), "a\n").unwrap();
        commit_all(dir.path(), &me(), "first").unwrap();
        std::fs::write(dir.path().join("core.md"), "changed\n").unwrap();
        std::fs::create_dir_all(dir.path().join("long")).unwrap();
        std::fs::write(dir.path().join("long/new.md"), "x\n").unwrap();

        revert_to(dir.path(), "HEAD").unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("core.md")).unwrap(),
            "a\n"
        );
        assert!(!dir.path().join("long/new.md").exists());
    }

    #[test]
    fn reverting_to_an_older_commit_restores_its_files_and_keeps_history() {
        let dir = repo("git-revert-old");
        std::fs::write(dir.path().join("core.md"), "a\n").unwrap();
        let first = commit_all(dir.path(), &me(), "first").unwrap().unwrap();
        std::fs::write(dir.path().join("core.md"), "b\n").unwrap();
        std::fs::write(dir.path().join("pending.md"), "p\n").unwrap();
        commit_all(dir.path(), &me(), "dream").unwrap();

        revert_to(dir.path(), &first).unwrap();
        commit_all(dir.path(), &me(), "undo dream").unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("core.md")).unwrap(),
            "a\n"
        );
        assert!(!dir.path().join("pending.md").exists());
        assert_eq!(log(dir.path(), 10).unwrap().len(), 3);
    }

    #[test]
    fn an_option_is_not_accepted_as_a_commit() {
        let dir = repo("git-option");

        assert!(revert_to(dir.path(), "--hard").is_err());
        assert!(diff_stat(dir.path(), "-p").is_err());
    }

    #[test]
    fn commits_made_at_once_each_land() {
        let dir = repo("git-at-once");
        write(dir.path(), "README.md", "notes\n");
        commit_all(dir.path(), &me(), "set up").unwrap();

        std::thread::scope(|s| {
            for n in 0..8 {
                let root = dir.path();
                s.spawn(move || {
                    let rel = format!("notes/{n}.md");
                    write(root, &rel, "x\n");
                    commit_paths(root, &me(), &format!("note {n}"), &[rel]).unwrap();
                });
            }
        });

        assert_eq!(log(dir.path(), 20).unwrap().len(), 9);
    }

    #[test]
    fn a_commit_waits_for_none_to_another_repository() {
        let (busy, free) = (repo("git-busy"), repo("git-free"));
        write(free.path(), "README.md", "notes\n");
        let _turn = committing(busy.path());
        let (done, finished) = std::sync::mpsc::channel();

        let root = free.path().to_owned();
        std::thread::spawn(move || done.send(commit_all(&root, &me(), "set up").unwrap()));

        let commit = finished.recv_timeout(std::time::Duration::from_secs(30));
        assert!(commit.is_ok_and(|id| id.is_some()));
    }

    #[test]
    fn a_directory_of_a_repository_takes_turns_with_the_whole() {
        let root = repo("git-turns");
        let memory = root.path().join("memory");
        write(root.path(), "memory/core.md", "a\n");
        let turn = committing(root.path());
        let (done, finished) = std::sync::mpsc::channel();

        std::thread::spawn(move || done.send(commit_all(&memory, &me(), "set up").unwrap()));

        let early = finished.recv_timeout(std::time::Duration::from_millis(300));
        assert!(early.is_err(), "it waits");
        drop(turn);
        assert!(finished.recv().unwrap().is_some());
    }

    #[test]
    fn a_new_repository_is_set_up_as_this_tool_wants_it() {
        let dir = repo("git-config");

        for (key, value) in [
            ("user.name", COMMITTER_NAME),
            ("user.email", COMMITTER_EMAIL),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", ".git/hooks"),
            ("core.quotePath", "false"),
            ("core.autocrlf", "false"),
            ("core.eol", "lf"),
        ] {
            let set = git(dir.path(), &["config", "--local", "--get-all", key]).unwrap();
            assert_eq!(set.trim(), value, "{key}");
        }
    }

    #[test]
    fn a_directory_is_committed_logged_and_put_back_without_its_neighbours() {
        let root = repo("git-scope");
        let memory = root.path().join("memory");
        write(root.path(), "memory/core.md", "a\n");
        write(root.path(), "records/tasks/t1.md", "x\n");
        commit_all(root.path(), &me(), "set up").unwrap();
        write(root.path(), "memory/core.md", "b\n");
        write(root.path(), "records/tasks/t1.md", "y\n");

        let id = commit_all(&memory, &me(), "memory only").unwrap().unwrap();

        assert_eq!(
            diff_stat(&memory, &id).unwrap(),
            vec![FileStat {
                path: "core.md".into(),
                added: Some(1),
                removed: Some(1)
            }]
        );
        assert!(
            !command(root.path(), &["diff", "--quiet", "--", "records"])
                .status()
                .unwrap()
                .success(),
            "the record's edit stays out of the commit"
        );
        commit_all(root.path(), &me(), "records").unwrap();
        assert_eq!(log(&memory, 10).unwrap().len(), 2);
        assert_eq!(log(root.path(), 10).unwrap().len(), 3);

        write(root.path(), "memory/new.md", "n\n");
        write(root.path(), "records/tasks/t2.md", "z\n");
        revert_to(&memory, &id).unwrap();
        assert!(!memory.join("new.md").exists());
        assert!(root.path().join("records/tasks/t2.md").exists());
    }
}
