//! Pull requests, made with `gh` or a command that takes its arguments.

use std::path::Path;

use anyhow::Context as _;
use serde::Deserialize;
use tokio::process::Command;

use super::repo::{output, ran};
use super::{Ci, PullRequest};

pub(crate) struct Forge<'a> {
    command: &'a [String],
    /// Where the command runs. Never the workspace, which the agent writes.
    checkout: &'a Path,
    repository: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    number: u64,
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Viewed {
    state: String,
    head_ref_oid: String,
}

#[derive(Deserialize)]
struct Check {
    name: String,
    bucket: String,
    #[serde(default)]
    link: String,
}

impl<'a> Forge<'a> {
    pub(crate) fn new(command: &'a [String], checkout: &'a Path, remote: &str) -> Self {
        Self {
            command,
            checkout,
            repository: crate::workers::finish::repository_name(remote)
                .unwrap_or_else(|| remote.to_owned()),
        }
    }

    /// The open pull request of `branch`, made if there is none and told
    /// anew if there is one. `head` is the commit pushed to it.
    pub(crate) async fn open(
        &self,
        branch: &str,
        target: &str,
        head: &str,
        title: &str,
        body: &str,
    ) -> anyhow::Result<PullRequest> {
        let listed = self.open_of(branch, target).await?;
        let listed = match listed {
            Some(listed) => {
                let number = listed.number.to_string();
                let mut command = self.command(&["pr", "edit", &number])?;
                command.args(["--title", title, "--body", body]);
                output(command)
                    .await
                    .context("the pull request could not be told anew")?;
                listed
            }
            None => {
                let mut command = self.command(&["pr", "create", "--head", branch])?;
                command.args(["--base", target, "--title", title, "--body", body]);
                output(command)
                    .await
                    .context("the pull request could not be made")?;
                self.open_of(branch, target)
                    .await?
                    .context("the pull request made is not listed")?
            }
        };
        Ok(PullRequest {
            number: listed.number,
            url: listed.url,
            head: head.to_owned(),
        })
    }

    /// How CI stands on the commit pushed, and never on another.
    pub(crate) async fn ci(&self, pull: &PullRequest) -> anyhow::Result<Ci> {
        if self.view(pull.number).await?.head_ref_oid != pull.head {
            return Ok(Ci::Pending);
        }
        let number = pull.number.to_string();
        let mut command = self.command(&["pr", "checks", &number])?;
        command.args(["--json", "name,bucket,link"]);
        // It ends with an error while checks fail or run, having told them.
        let told = ran(command).await?;
        let checks: Vec<Check> = match serde_json::from_slice(&told.stdout) {
            Ok(checks) => checks,
            Err(error) => {
                let said = String::from_utf8_lossy(&told.stderr);
                if said.contains("no checks reported") {
                    return Ok(Ci::Pending);
                }
                return Err(error).with_context(|| format!("CI could not be asked: {said}"));
            }
        };
        let failed = |c: &&Check| matches!(c.bucket.as_str(), "fail" | "cancel");
        if let Some(check) = checks.iter().find(failed) {
            return Ok(Ci::Failed {
                name: check.name.clone(),
                log: self.log(check).await,
            });
        }
        let passed = |c: &Check| matches!(c.bucket.as_str(), "pass" | "skipping");
        Ok(
            if checks.iter().any(|c| c.bucket == "pass") && checks.iter().all(passed) {
                Ci::Passed
            } else {
                Ci::Pending
            },
        )
    }

    /// Whether the commit pushed is merged.
    pub(crate) async fn merged(&self, pull: &PullRequest) -> anyhow::Result<bool> {
        let viewed = self.view(pull.number).await?;
        if viewed.state != "MERGED" {
            return Ok(false);
        }
        anyhow::ensure!(
            viewed.head_ref_oid == pull.head,
            "the pull request was merged holding another commit than the one pushed"
        );
        Ok(true)
    }

    pub(crate) async fn merge(&self, pull: &PullRequest) -> anyhow::Result<()> {
        let number = pull.number.to_string();
        let mut command = self.command(&["pr", "merge", &number])?;
        command.args(["--merge", "--match-head-commit", &pull.head]);
        output(command).await?;
        Ok(())
    }

    async fn open_of(&self, branch: &str, target: &str) -> anyhow::Result<Option<Listed>> {
        let mut command = self.command(&["pr", "list", "--head", branch])?;
        command.args(["--base", target, "--state", "open", "--json", "number,url"]);
        let listed: Vec<Listed> = serde_json::from_str(&output(command).await?)
            .context("the pull requests could not be read")?;
        Ok(listed.into_iter().next())
    }

    async fn view(&self, number: u64) -> anyhow::Result<Viewed> {
        let number = number.to_string();
        let mut command = self.command(&["pr", "view", &number])?;
        command.args(["--json", "state,headRefOid"]);
        serde_json::from_str(&output(command).await?).context("the pull request could not be read")
    }

    /// What a failed check wrote, as far as the forge tells.
    async fn log(&self, check: &Check) -> String {
        let run = check
            .link
            .split_once("/actions/runs/")
            .map(|(_, rest)| rest.split('/').next().unwrap_or_default());
        let told = match run {
            Some(run) => match self.command(&["run", "view", run, "--log-failed"]) {
                Ok(command) => output(command).await.ok(),
                Err(_) => None,
            },
            None => None,
        };
        told.filter(|log| !log.trim().is_empty())
            .unwrap_or_else(|| format!("No log could be read: {}", check.link))
    }

    fn command(&self, args: &[&str]) -> anyhow::Result<Command> {
        let (program, leading) = self
            .command
            .split_first()
            .context("no command for pull requests is set")?;
        let mut command = Command::new(program);
        command
            .args(leading)
            .args(args)
            .args(["--repo", &self.repository])
            .current_dir(self.checkout);
        Ok(command)
    }
}
