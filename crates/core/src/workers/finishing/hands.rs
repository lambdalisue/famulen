//! What Famulen does for the finishing of one run: everything that reaches
//! past the workspace is done here, with credentials the agent never has.

use std::path::PathBuf;

use serde_json::json;

use super::{
    Answer, Ci, Content, Finishing, Forge, Hands, Pinned, PullRequest, Repo, Snapshot, Started,
    Verdict, check, deployed, review_message, start_deploy, verdict,
};
use crate::app::App;
use crate::permissions::Decision;
use crate::workers::direct::{self, Direct, Place};
use crate::workers::{Approval, approval};

/// How long the owner may take to approve a merge.
const APPROVAL_SECONDS: i64 = 7 * 24 * 60 * 60;

/// Where what Famulen keeps of the finishing of `run` is: nowhere the agent
/// can write.
pub(crate) fn finishing_dir(app: &App, run: i64) -> PathBuf {
    app.workers.dir.join(format!("finishing-{run}"))
}

pub(crate) struct OwnHands {
    pub app: App,
    pub run: i64,
    pub(in crate::workers) place: Place,
    pub(in crate::workers) direct: Direct,
    /// The code of the task, its title and what it asks.
    pub task: String,
    pub title: String,
    pub request: String,
}

impl OwnHands {
    fn repo(&self) -> Repo {
        Repo {
            jj: self.direct.jj.clone(),
            checkout: self.place.checkout.clone(),
            workspace: self.place.workspace.clone(),
        }
    }

    fn forge<'a>(&'a self, pinned: &'a Pinned) -> Forge<'a> {
        Forge::new(&pinned.rule.forge, &self.place.checkout, &pinned.remote)
    }
}

/// The body of the pull request: what the change says of itself, and what the
/// last review left.
fn body(content: &Content, remaining: Option<&str>) -> String {
    match remaining {
        None => content.body.clone(),
        Some(remaining) => format!(
            "{}\n\n## Left by the last review\n\n{remaining}",
            content.body
        )
        .trim_start()
        .to_owned(),
    }
}

impl Hands for OwnHands {
    fn keep(&self, finishing: &Finishing) -> anyhow::Result<()> {
        finishing.save(&self.app.store, self.run)
    }

    async fn snapshot(&self, pinned: &Pinned) -> anyhow::Result<Snapshot> {
        self.repo()
            .snapshot(&pinned.remote, &pinned.target, &self.title)
            .await
    }

    async fn check(&self, command: &str) -> anyhow::Result<Result<(), String>> {
        let fence = direct::fence(&self.app.workers.kept, &self.app.workers.dir, &self.place);
        let log = finishing_dir(&self.app, self.run).join("check.log");
        check(command, &self.place.workspace, &fence, &log).await
    }

    async fn review(&self, _: &Pinned, content: &Content) -> anyhow::Result<Verdict> {
        let diff = self.repo().diff(content).await?;
        let Some(message) = review_message(&self.request, content, &diff) else {
            return Ok(Verdict::Findings {
                text: "The change is too large to be reviewed whole. Make it smaller.".to_owned(),
                serious: true,
            });
        };
        let report =
            crate::workers::agent::review(&self.app, self.run, &self.place, message).await?;
        verdict(&report)
    }

    async fn publish(
        &self,
        pinned: &Pinned,
        content: &Content,
        before: Option<&str>,
        remaining: Option<&str>,
    ) -> anyhow::Result<PullRequest> {
        let head = self
            .repo()
            .push(&pinned.remote, &pinned.branch, content, before)
            .await?;
        self.forge(pinned)
            .open(
                &pinned.branch,
                &pinned.target,
                &head,
                &content.title,
                &body(content, remaining),
            )
            .await
    }

    async fn ci(&self, pinned: &Pinned, pull: &PullRequest) -> anyhow::Result<Ci> {
        self.forge(pinned).ci(pull).await
    }

    async fn merged(&self, pinned: &Pinned, pull: &PullRequest) -> anyhow::Result<bool> {
        self.forge(pinned).merged(pull).await
    }

    async fn merge(&self, pinned: &Pinned, pull: &PullRequest) -> anyhow::Result<()> {
        self.forge(pinned).merge(pull).await
    }

    async fn confirm_merge(
        &self,
        pull: &PullRequest,
        ticket: Option<i64>,
    ) -> anyhow::Result<Answer> {
        if let Some(held) = ticket {
            return Ok(match approval(&self.app, ticket)? {
                Approval::Waiting => Answer::Waiting(held),
                Approval::Approved => Answer::Yes,
                Approval::Refused => Answer::No,
            });
        }
        let input = json!({
            "task": self.task,
            "title": self.title,
            "pull_request": pull.url,
            "commit": pull.head,
        });
        let until = crate::store::now() + APPROVAL_SECONDS;
        let why = "this repository's merges are not allowed without asking";
        Ok(
            match self
                .app
                .permissions
                .decide_merge(self.run, &input, why, until)
                .await?
            {
                Decision::Allow => Answer::Yes,
                Decision::Held(ticket) => Answer::Waiting(ticket),
                Decision::Forbid(_) => Answer::No,
            },
        )
    }

    async fn deploy(&self, pinned: &Pinned, command: &str) -> anyhow::Result<Started> {
        start_deploy(
            &self.direct.jj,
            &self.place.checkout,
            &pinned.target,
            command,
            &finishing_dir(&self.app, self.run).join("deploy"),
            &self.app.workers.dir.join("deploy.lock"),
        )
        .await
    }

    async fn deployed(&self) -> anyhow::Result<Option<Result<(), String>>> {
        deployed(&finishing_dir(&self.app, self.run).join("deploy"))
    }

    async fn tidy(&self, pinned: &Pinned, merged: Option<&str>) -> anyhow::Result<()> {
        crate::workers::agent::forget_review(&self.app, self.run).await?;
        if let Some(head) = merged {
            self.repo()
                .delete_branch(&pinned.remote, &pinned.branch, head)
                .await?;
        }
        direct::forget(&self.direct, &self.place).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_last_review_left_goes_with_the_pull_request() {
        let mut content = Content {
            change: "c".into(),
            commit: "a".into(),
            tree: "t".into(),
            base: "b".into(),
            title: "Add b".into(),
            body: "why".into(),
        };
        assert_eq!(body(&content, None), "why");
        assert_eq!(
            body(&content, Some("a.rs:1")),
            "why\n\n## Left by the last review\n\na.rs:1"
        );
        content.body.clear();
        assert!(body(&content, Some("a.rs:1")).starts_with("## Left"));
    }
}
