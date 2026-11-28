//! What Famulen does with jj and Git for the work of a run. The agent can
//! write most of the repository, so nothing here takes its word from a ref,
//! a hook or a replacement it could have left: the remote is the one pinned
//! at the start, and commits are named by their ids.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use tokio::process::Command;

use super::{Content, Snapshot};

/// How long one command of jj, Git or the forge is given.
const LIMIT: Duration = Duration::from_secs(120);

/// The change that names the work: the latest that holds anything or joins
/// lines of work. What is pushed is the workspace's whole tree, not only
/// what this one holds.
const NAMED_REVSET: &str = "latest(heads(::@ ~ (empty() ~ merges())))";
const HEAD_TEMPLATE: &str =
    r#"change_id ++ "\n" ++ commit_id ++ "\n" ++ conflict ++ "\n" ++ description"#;

/// Where a run's work is.
#[derive(Debug, Clone)]
pub(crate) struct Repo {
    pub jj: String,
    pub checkout: PathBuf,
    pub workspace: PathBuf,
}

impl Repo {
    /// The address of the checkout's `origin`, and the branch it is built on.
    pub(crate) async fn remote(jj: &str, checkout: &Path) -> anyhow::Result<(String, String)> {
        let url = git(jj, checkout, &["remote", "get-url", "--push", "origin"])
            .await
            .context("the checkout has no origin to push to")?;
        let head = git(jj, checkout, &["ls-remote", "--symref", &url, "HEAD"]).await?;
        let target = head
            .lines()
            .find_map(|l| l.strip_prefix("ref: refs/heads/")?.split_once('\t'))
            .map(|(branch, _)| branch.to_owned())
            .context("origin names no default branch")?;
        Ok((url, target))
    }

    /// The work as it stands; `title` names it when its change has no
    /// description. What jj knows as the target of origin is brought up to
    /// the remote first, for the agent has no key to ask for it with.
    pub(crate) async fn snapshot(
        &self,
        remote: &str,
        target: &str,
        title: &str,
    ) -> anyhow::Result<Snapshot> {
        let head = self.log("@").await?;
        let mut lines = head.splitn(4, '\n').skip(1);
        let (Some(commit), Some(conflict)) = (lines.next(), lines.next()) else {
            anyhow::bail!("jj told nothing of the work");
        };
        let named = self.log(NAMED_REVSET).await?;
        let mut named = named.splitn(4, '\n');
        let change = named.next().unwrap_or_default();
        let description = named.nth(2).unwrap_or_default().trim();
        anyhow::ensure!(is_id(commit), "jj named no commit: {commit}");
        let tip = self.take_up(remote, target).await?;
        if conflict == "true" {
            return Ok(Snapshot::Conflicted);
        }
        let base = self.git(&["merge-base", commit, &tip]).await?;
        let tree = self.tree(commit).await?;
        if base == commit || tree == self.tree(&base).await? {
            return Ok(Snapshot::Unchanged);
        }
        let ci = [
            "diff",
            "--no-renames",
            "--name-only",
            &base,
            commit,
            "--",
            ".github",
        ];
        let changes_ci = !self.git(&ci).await?.is_empty();
        let (first, body) = description.split_once('\n').unwrap_or((description, ""));
        let first = if first.is_empty() { title } else { first };
        Ok(Snapshot::Changed(Content {
            change: change.to_owned(),
            commit: commit.to_owned(),
            tree,
            base,
            title: first.trim().to_owned(),
            body: body.trim().to_owned(),
            changes_ci,
        }))
    }

    async fn log(&self, revset: &str) -> anyhow::Result<String> {
        let mut command = Command::new(&self.jj);
        command
            .arg("-R")
            .arg(&self.workspace)
            .args(["log", "--no-graph", "-r", revset, "-T", HEAD_TEMPLATE])
            .current_dir(&self.workspace);
        output(command).await.context("jj could not tell the work")
    }

    /// The setting of Git that holds a key, which the agent could read.
    pub(crate) async fn credential_setting(
        jj: &str,
        checkout: &Path,
    ) -> anyhow::Result<Option<String>> {
        let listing = git(jj, checkout, &["config", "--list", "--includes", "-z"]).await?;
        Ok(super::pin::credential_key(&listing))
    }

    /// Fetch the target, refuse a `merged` it does not hold, and say whether
    /// `at`, a commit of the target, has it already.
    pub(super) async fn holds(
        &self,
        remote: &str,
        target: &str,
        merged: &str,
        at: &str,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(is_id(merged), "the forge named no commit: {merged}");
        let tip = self.take_up(remote, target).await?;
        anyhow::ensure!(
            self.is_ancestor(merged, &tip).await,
            "{merged} is not on {target} of the remote"
        );
        Ok(is_id(at) && self.is_ancestor(merged, at).await && self.is_ancestor(at, &tip).await)
    }

    async fn is_ancestor(&self, ancestor: &str, of: &str) -> bool {
        self.git(&["merge-base", "--is-ancestor", ancestor, of])
            .await
            .is_ok()
    }

    /// Fetch the target as the remote has it, make jj see it as that of
    /// origin, and say which commit it is.
    async fn take_up(&self, remote: &str, target: &str) -> anyhow::Result<String> {
        let tip = self
            .remote_head(remote, target)
            .await?
            .with_context(|| format!("origin has no branch {target}"))?;
        self.git(&["fetch", "--no-tags", "--no-write-fetch-head", remote, &tip])
            .await?;
        let tracking = format!("refs/remotes/origin/{target}");
        // The agent can write refs: one it made point at another is replaced,
        // not written through.
        self.git(&["update-ref", "--no-deref", &tracking, &tip])
            .await?;
        let mut import = jj_on(&self.jj, &self.checkout);
        import.args(["git", "import"]);
        output(import)
            .await
            .context("jj could not take up the target of origin")?;
        Ok(tip)
    }

    /// Push the content as one commit on its base, and say which. `before`
    /// is what was pushed last: a branch that holds anything else is left as
    /// it is.
    pub(crate) async fn push(
        &self,
        remote: &str,
        branch: &str,
        content: &Content,
        before: Option<&str>,
    ) -> anyhow::Result<String> {
        // Objects from elsewhere would be pushed as the work's.
        let common = self.git(&["rev-parse", "--git-common-dir"]).await?;
        anyhow::ensure!(
            !self
                .checkout
                .join(common)
                .join("objects/info/alternates")
                .exists(),
            "the repository takes objects from elsewhere: nothing is pushed"
        );
        let there = self.remote_head(remote, branch).await?;
        if let Some(there) = &there {
            // Pushed already, by a step that was cut off before it told.
            if self.is_commit_of(there, content).await {
                return Ok(there.clone());
            }
            anyhow::ensure!(
                before == Some(there.as_str()),
                "{branch} holds what Famulen did not push"
            );
        }
        let commit = {
            let mut args = vec![
                "commit-tree",
                &content.tree,
                "-p",
                &content.base,
                "-m",
                &content.title,
            ];
            if !content.body.is_empty() {
                args.extend(["-m", &content.body]);
            }
            self.git(&args).await?
        };
        let lease = format!(
            "--force-with-lease=refs/heads/{branch}:{}",
            there.as_deref().unwrap_or_default()
        );
        let refspec = format!("{commit}:refs/heads/{branch}");
        self.git(&["push", "--no-verify", &lease, remote, &refspec])
            .await?;
        Ok(commit)
    }

    /// What the content changes of its base.
    pub(crate) async fn diff(&self, content: &Content) -> anyhow::Result<String> {
        self.git(&[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            &content.base,
            &content.commit,
        ])
        .await
    }

    /// Delete the branch, while it holds `commit` still.
    pub(crate) async fn delete_branch(
        &self,
        remote: &str,
        branch: &str,
        commit: &str,
    ) -> anyhow::Result<()> {
        if self.remote_head(remote, branch).await?.is_none() {
            return Ok(());
        }
        let lease = format!("--force-with-lease=refs/heads/{branch}:{commit}");
        let refspec = format!(":refs/heads/{branch}");
        self.git(&["push", "--no-verify", &lease, remote, &refspec])
            .await?;
        Ok(())
    }

    async fn is_commit_of(&self, commit: &str, content: &Content) -> bool {
        let parents = self
            .git(&["rev-list", "--parents", "-n", "1", commit])
            .await;
        let one_on_base = parents.is_ok_and(|p| p == format!("{commit} {}", content.base));
        one_on_base && self.tree(commit).await.is_ok_and(|t| t == content.tree)
    }

    async fn tree(&self, commit: &str) -> anyhow::Result<String> {
        self.git(&["rev-parse", "--verify", &format!("{commit}^{{tree}}")])
            .await
    }

    async fn remote_head(&self, remote: &str, branch: &str) -> anyhow::Result<Option<String>> {
        let name = format!("refs/heads/{branch}");
        let listed = self.git(&["ls-remote", remote, &name]).await?;
        Ok(listed
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .find(|(_, at)| *at == name)
            .map(|(commit, _)| commit.to_owned()))
    }

    async fn git(&self, args: &[&str]) -> anyhow::Result<String> {
        git(&self.jj, &self.checkout, args).await
    }
}

fn is_id(text: &str) -> bool {
    text.len() >= 40 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// jj on the checkout, leaving the owner's working copy as it is.
fn jj_on(jj: &str, checkout: &Path) -> Command {
    let mut command = Command::new(jj);
    command
        .arg("-R")
        .arg(checkout)
        .arg("--ignore-working-copy")
        .current_dir(checkout);
    command
}

/// Git on the repository jj names: one that is not colocated is not where Git
/// would look from the checkout.
async fn git(jj: &str, checkout: &Path, args: &[&str]) -> anyhow::Result<String> {
    let mut root = jj_on(jj, checkout);
    root.args(["git", "root"]);
    let root = output(root)
        .await
        .context("jj could not tell where the Git repository is")?;
    let mut command = Command::new("git");
    command
        .arg(format!("--git-dir={}", root.trim_end_matches('\n')))
        .args(["--no-replace-objects"])
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(["-c", "core.fsmonitor=false"])
        .args(["-c", "core.commitGraph=false"])
        .args(args)
        .current_dir(checkout);
    Ok(output(command).await?.trim().to_owned())
}

/// What `command` writes, run as Famulen's own: with the owner's keys and
/// none of Famulen's secrets.
pub(super) async fn output(command: Command) -> anyhow::Result<String> {
    let ran = ran(command).await?;
    anyhow::ensure!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&ran.stdout).into_owned())
}

pub(super) async fn ran(mut command: Command) -> anyhow::Result<std::process::Output> {
    crate::config::hide_secrets(&mut command);
    command
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let program = command.as_std().get_program().to_owned();
    tokio::time::timeout(LIMIT, command.output())
        .await
        .with_context(|| format!("{program:?} took longer than {}s", LIMIT.as_secs()))?
        .with_context(|| format!("could not run {program:?}"))
}

#[cfg(test)]
mod tests;
