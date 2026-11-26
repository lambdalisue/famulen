//! How the owner wants delegated work on code finished: steps of their own,
//! added to what a worker is asked, so that they do not depend on who wrote
//! the request.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

use super::order::{Order, Runner};

/// The permissions of a session that changes nothing.
const READ_ONLY: &str = "read_only";
/// How long Git may take to tell what a directory was cloned from.
const GIT_TAKES: Duration = Duration::from_secs(5);

/// Steps that finish work on code.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finish {
    /// Work on this repository, as `host/path`, wherever it is cloned; any
    /// when empty.
    pub repository: String,
    pub steps: String,
}

impl Finish {
    /// `repository` as the owner writes it: its address, or `host/path`.
    pub fn new(repository: &str, steps: &str) -> anyhow::Result<Self> {
        let named = repository.trim();
        let repository = if named.is_empty() {
            String::new()
        } else {
            repository_name(named).ok_or_else(|| {
                anyhow::anyhow!("a repository is its URL or host/owner/name, not {named}")
            })?
        };
        let steps = steps.trim().to_owned();
        anyhow::ensure!(!steps.is_empty(), "a finish needs its steps written");
        Ok(Self { repository, steps })
    }
}

/// What Famulen brings along. The owner's own for the same repository takes
/// its place.
pub fn bundled_finishes() -> Vec<Finish> {
    vec![Finish {
        repository: "github.com/lambdalisue/famulen".into(),
        steps: include_str!("finish/famulen.md").trim().to_owned(),
    }]
}

pub(super) fn check_finishes(all: &[Finish]) -> anyhow::Result<()> {
    for (at, finish) in all.iter().enumerate() {
        anyhow::ensure!(
            !all[..at]
                .iter()
                .any(|f| same(&f.repository, &finish.repository)),
            "{:?} has two finishes",
            finish.repository
        );
    }
    Ok(())
}

/// Hosts tell no case apart, and the common ones tell none apart in paths.
fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The steps that finish `order`: those of the repository it works on, else
/// those for any work. None for work that changes no code of the owner's.
pub(super) async fn steps_for(own: &[Finish], order: &Order) -> String {
    if order.runner != Runner::Adroit || order.permissions == READ_ONLY {
        return String::new();
    }
    let repository = repository_of(Path::new(&order.directory)).await;
    steps_of(own, repository.as_deref())
}

fn steps_of(own: &[Finish], repository: Option<&str>) -> String {
    let bundled = bundled_finishes();
    let of = |name: &str| own.iter().find(|f| same(&f.repository, name));
    repository
        .and_then(|r| of(r).or_else(|| bundled.iter().find(|f| same(&f.repository, r))))
        .or_else(|| of(""))
        .map(|f| f.steps.clone())
        .unwrap_or_default()
}

/// The repository `directory` was cloned from, by its `origin` or its only
/// remote.
async fn repository_of(directory: &Path) -> Option<String> {
    let remotes = git(directory, &["remote"]).await?;
    let names: Vec<&str> = remotes.lines().collect();
    let remote = match names.as_slice() {
        [only] => *only,
        names => *names.iter().find(|n| **n == "origin")?,
    };
    let url = git(directory, &["remote", "get-url", remote]).await?;
    repository_name(&url)
}

async fn git(directory: &Path, args: &[&str]) -> Option<String> {
    let mut command = tokio::process::Command::new("git");
    command
        .args(args)
        .current_dir(directory)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(GIT_TAKES, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            tracing::debug!(directory = %directory.display(), %error, "git did not run");
            return None;
        }
        Err(_) => {
            tracing::debug!(directory = %directory.display(), "git took too long");
            return None;
        }
    };
    let text = String::from_utf8(output.stdout).ok()?;
    output.status.success().then(|| text.trim().to_owned())
}

/// A repository as `host/path`, from any address it is cloned by.
fn repository_name(address: &str) -> Option<String> {
    let address = address.trim();
    let (host, path) = if address.contains("://") {
        let url = Url::parse(address).ok()?;
        (url.host_str()?.to_owned(), url.path().to_owned())
    } else {
        let (host, path) = match address.split_once(':') {
            Some((host, path)) if !host.contains('/') => {
                (host.rsplit('@').next().unwrap_or(host), path)
            }
            _ => address.split_once('/').filter(|(h, _)| h.contains('.'))?,
        };
        (host.to_owned(), path.to_owned())
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let word = |s: &str| {
        !matches!(s, "" | "." | "..")
            && !s.contains(|c: char| c.is_whitespace() || c.is_control() || "\\?#:".contains(c))
    };
    // One letter before a colon is a drive.
    let named = host.chars().count() > 1 && word(&host) && path.split('/').all(word);
    named.then(|| format!("{}/{path}", host.to_lowercase()))
}

/// What a session is told of how its end is read. What it answers when a turn
/// ends is no sign: a pause, a step on the way and the last word look alike.
const DONE_BY_PLAN: &str = "Keep the plan of this work on your timeline with `timeline_tasks`, and mark each task completed once it is. Famulen takes the work for done only when your turn has ended and every task on it is completed; until then it is taken for unfinished, whatever your answer says. When you stop short of that, to pause, to wait or because the work was called off, leave what is not done as it stands, blocked with a note where you cannot go on.";

/// What a session is asked: the request, how its end is read, then how it is
/// finished.
pub(super) fn prompt(order: &Order) -> String {
    let request = format!("{}\n\n---\n\n{DONE_BY_PLAN}", with_vms(order));
    if order.finish.is_empty() {
        return request;
    }
    format!(
        "{request}\n\n---\n\n\
         # How this work is finished\n\n\
         The owner's standing steps, for every piece of work that changes code here. Take them in order once the work above is done, all of them, by yourself: the owner has decided on them, so do not stop in between to ask whether to go on. They stand over what the request says about finishing (a commit, a pull request, whether to merge), unless it says the owner asked otherwise for this work. Put right what fails on the way. Leave out what a change of nothing has no use for. Your turn ends when they are all done, or when one cannot be: then say which, and why. Say in your last answer how each of them went.\n\n{}",
        order.finish
    )
}

/// The request, and how VMs are named so that they are known to be this
/// work's when it is cleared away.
fn with_vms(order: &Order) -> String {
    if !order.worktree {
        return order.request.clone();
    }
    format!(
        "{}\n\n---\n\n\
         Name every disposable vitro VM you start for this work `{code}`, or `{code}-` and a word, and give no other VM such a name: once the work is over and this session is closed, Famulen destroys the VMs so named and removes this worktree.",
        order.request,
        code = order.code
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::testing::TempDir;
    use crate::paths::at;

    fn finish(repository: &str, steps: &str) -> Finish {
        Finish {
            repository: repository.into(),
            steps: steps.into(),
        }
    }

    fn order(directory: &str) -> Order {
        Order {
            directory: directory.into(),
            request: "Do it.".into(),
            ..Order::default()
        }
    }

    #[test]
    fn a_repository_has_one_name_whatever_address_it_is_cloned_by() {
        for address in [
            "git@github.com:Owner/name.git",
            "ssh://git@github.com:22/Owner/name.git",
            "https://user@GitHub.com/Owner/name",
            "https://github.com/Owner/name/",
            "github.com/Owner/name",
        ] {
            assert_eq!(
                repository_name(address).as_deref(),
                Some("github.com/Owner/name"),
                "{address}"
            );
        }
        for address in [
            "",
            "name",
            "owner/name",
            "/src/name",
            "./name",
            "../name",
            r"C:\src\name",
            "C:/src/name",
            "file:///src/name",
            "https://github.com",
        ] {
            assert_eq!(repository_name(address), None, "{address}");
        }
    }

    #[test]
    fn work_is_finished_by_the_steps_of_its_repository_else_by_those_for_any() {
        let own = [finish("", "any"), finish("github.com/o/api", "api")];
        for (repository, steps) in [
            (Some("github.com/O/api"), "api"),
            (Some("github.com/o/api-old"), "any"),
            (None, "any"),
        ] {
            assert_eq!(steps_of(&own, repository), steps);
        }
        assert_eq!(steps_of(&own[1..], None), "");
    }

    #[test]
    fn famulen_is_finished_by_what_it_brings_along_until_the_owner_writes_their_own() {
        let famulen = "github.com/lambdalisue/famulen";
        let any = finish("", "any");
        let bundled = steps_of(std::slice::from_ref(&any), Some(famulen));
        assert_eq!(bundled, bundled_finishes()[0].steps);
        let own = [any, finish(famulen, "own")];
        assert_eq!(steps_of(&own, Some(famulen)), "own");
    }

    #[tokio::test]
    async fn the_repository_is_told_from_anywhere_inside_a_clone() {
        let tmp = TempDir::new("finish-clone");
        let run = |args: &[&str]| {
            let done = std::process::Command::new("git")
                .args(args)
                .current_dir(tmp.path())
                .status();
            assert!(done.unwrap().success(), "{args:?}");
        };
        assert_eq!(repository_of(&tmp.path().join("gone")).await, None);
        run(&["init", "-q"]);
        assert_eq!(repository_of(tmp.path()).await, None);
        run(&["remote", "add", "up", "https://example.com/up/name.git"]);
        let inside = tmp.path().join("inside");
        std::fs::create_dir(&inside).unwrap();
        assert_eq!(
            repository_of(&inside).await.as_deref(),
            Some("example.com/up/name")
        );
        run(&["remote", "add", "origin", "git@example.com:me/name.git"]);
        assert_eq!(
            repository_of(&inside).await.as_deref(),
            Some("example.com/me/name")
        );
    }

    #[tokio::test]
    async fn what_changes_no_code_has_no_steps_to_finish_it() {
        let all = [finish("", "any")];
        let agent = Order {
            runner: Runner::Agent,
            ..order(&at("/src"))
        };
        let reads = Order {
            permissions: READ_ONLY.into(),
            ..order(&at("/src"))
        };
        for order in [agent, reads] {
            assert_eq!(steps_for(&all, &order).await, "");
        }
    }

    #[test]
    fn the_steps_stand_below_the_request() {
        let mut order = order(&at("/src"));
        order.finish = "1. Merge it.".into();
        let prompt = prompt(&order);
        assert!(prompt.starts_with("Do it.\n"), "{prompt}");
        assert!(prompt.ends_with("\n1. Merge it."), "{prompt}");
    }

    #[test]
    fn a_session_is_told_that_its_work_is_taken_for_done_by_its_plan() {
        let mut order = order(&at("/src"));
        for finish in ["", "1. Merge it."] {
            order.finish = finish.into();
            let prompt = prompt(&order);
            assert!(prompt.starts_with("Do it.\n"), "{prompt}");
            assert!(prompt.contains("timeline_tasks"), "{prompt}");
        }
    }

    #[test]
    fn a_session_in_a_worktree_is_told_how_to_name_its_vms() {
        let mut order = order(&at("/src"));
        order.code = "task-abc123".into();
        order.worktree = true;
        assert!(prompt(&order).contains("`task-abc123-`"));
        order.finish = "1. Merge it.".into();
        let prompt = prompt(&order);
        assert!(prompt.contains("`task-abc123-`"), "{prompt}");
        assert!(prompt.ends_with("\n1. Merge it."), "{prompt}");
    }

    #[test]
    fn a_finish_without_steps_or_a_repository_is_refused() {
        assert_eq!(
            Finish::new(" https://github.com/o/n.git ", " x ").unwrap(),
            finish("github.com/o/n", "x")
        );
        assert!(Finish::new("", "x").is_ok());
        for (repository, steps) in [("", " "), ("/src", "x"), ("o/n", "x")] {
            assert!(Finish::new(repository, steps).is_err(), "{repository}");
        }
        let two = [finish("github.com/o/n", "a"), finish("github.com/O/n", "b")];
        assert!(check_finishes(&two).is_err());
        assert!(check_finishes(&two[..1]).is_ok());
    }
}
