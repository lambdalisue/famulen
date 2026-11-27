//! How work on code is finished by Famulen: the stage a run is at, what each
//! stage may do, and what is kept of it so that it goes on after a restart.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::store::Store;

/// How often a failure of one kind is handed back before the owner is asked.
const MOST_FAILURES: u32 = 3;
/// How long CI is waited for, and how often it is asked.
const CI_LIMIT_SECONDS: i64 = 3 * 60 * 60;
const CI_EVERY: Duration = Duration::from_secs(30);
const ASKED_EVERY: Duration = Duration::from_secs(15);
/// How long deploying is waited for.
const DEPLOY_LIMIT_SECONDS: i64 = 60 * 60;
/// How much of a log is handed back to the agent.
const LOG_CHARS: usize = 8000;

/// What the owner lets Famulen do with the work in one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// The checkout work runs directly in.
    pub repository: PathBuf,
    /// Shell commands that must pass before the work is reviewed. They run
    /// with no network and none of the owner's keys.
    #[serde(default)]
    pub checks: Vec<String>,
    /// How often the work is reviewed at most.
    #[serde(default = "two")]
    pub reviews: u32,
    /// Whether a pull request is merged without asking.
    #[serde(default)]
    pub merge: bool,
    /// A shell command that puts the merged work to use.
    #[serde(default)]
    pub deploy: Option<String>,
    /// The command pull requests are made with.
    #[serde(default = "gh")]
    pub forge: Vec<String>,
}

fn two() -> u32 {
    2
}

fn gh() -> Vec<String> {
    vec!["gh".into()]
}

/// A rule as it stood when the run started, with what was read of the
/// repository then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pinned {
    pub rule: Rule,
    /// Where it is pushed to, and the branch it is merged into.
    pub remote: String,
    pub target: String,
    /// The branch the work is pushed as.
    pub branch: String,
}

/// What was looked at: a change, as it was then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Content {
    pub change: String,
    pub commit: String,
    pub tree: String,
    /// The commit of the target it stands on.
    pub base: String,
    pub title: String,
    pub body: String,
}

/// How the work stands when looked at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Snapshot {
    /// Nothing differs from the target.
    Unchanged,
    /// Conflicts are left in it.
    Conflicted,
    Changed(Content),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// The agent is at work.
    #[default]
    Implementing,
    Checking,
    Reviewing,
    Publishing,
    Ci,
    Merging,
    Deploying,
    Tidying,
    Done,
    /// It is the owner's now.
    Stopped,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Implementing => "implementing",
            Self::Checking => "checking",
            Self::Reviewing => "reviewing",
            Self::Publishing => "publishing",
            Self::Ci => "ci",
            Self::Merging => "merging",
            Self::Deploying => "deploying",
            Self::Tidying => "tidying",
            Self::Done => "done",
            Self::Stopped => "stopped",
        }
    }

    pub fn over(self) -> bool {
        matches!(self, Self::Done | Self::Stopped)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    /// The commit pushed for it.
    pub head: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    pub text: String,
    /// A fault of correctness or safety is among it.
    pub serious: bool,
}

/// What is kept of the finishing of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finishing {
    pub pinned: Pinned,
    #[serde(default)]
    pub stage: Stage,
    /// What the checks passed on, and what goes out.
    #[serde(default)]
    pub content: Option<Content>,
    #[serde(default)]
    pub reviews: u32,
    /// The commit last reviewed.
    #[serde(default)]
    pub reviewed: Option<String>,
    /// What that review found.
    #[serde(default)]
    pub found: Option<Found>,
    /// What a last review found and was left.
    #[serde(default)]
    pub remaining: Option<String>,
    #[serde(default)]
    pub pull: Option<PullRequest>,
    #[serde(default)]
    pub failures: BTreeMap<String, u32>,
    /// What failed last, or why it stopped.
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub ci_since: Option<i64>,
    /// The request the owner was asked to merge with.
    #[serde(default)]
    pub ticket: Option<i64>,
    #[serde(default)]
    pub merge_approved: bool,
    #[serde(default)]
    pub merged: bool,
    /// When deploying was started, in seconds since the epoch.
    #[serde(default)]
    pub deploy_started: Option<i64>,
    /// When deploying was first tried, which it waits from while another
    /// deployment is under way.
    #[serde(default)]
    pub deploy_waiting_since: Option<i64>,
    /// What is yet to be told to the agent.
    #[serde(default)]
    pub tell: Option<String>,
    /// When the agent was given the turn it is in, in seconds since the epoch.
    #[serde(default)]
    pub turn_began: Option<i64>,
    /// What the agent said of its work when its last turn ended.
    #[serde(default)]
    pub report: String,
}

fn key(run: i64) -> String {
    format!("worker.finishing.{run}")
}

impl Finishing {
    pub fn new(pinned: Pinned) -> Self {
        Self {
            pinned,
            stage: Stage::default(),
            content: None,
            reviews: 0,
            reviewed: None,
            found: None,
            remaining: None,
            pull: None,
            failures: BTreeMap::new(),
            note: None,
            ci_since: None,
            ticket: None,
            merge_approved: false,
            merged: false,
            deploy_started: None,
            deploy_waiting_since: None,
            tell: None,
            turn_began: None,
            report: String::new(),
        }
    }

    pub fn load(store: &Store, run: i64) -> anyhow::Result<Option<Self>> {
        Ok(match store.state(key(run))? {
            Some(json) => Some(serde_json::from_str(&json)?),
            None => None,
        })
    }

    pub fn save(&self, store: &Store, run: i64) -> anyhow::Result<()> {
        store.set_state(key(run), serde_json::to_string(self)?)
    }

    fn stop(&mut self, why: impl Into<String>) -> Next {
        let why = why.into();
        self.note = Some(why.clone());
        self.stage = Stage::Stopped;
        Next::Stop(why)
    }

    /// Hand a failure back to the agent, or to the owner once it came back
    /// too often.
    fn failed(&mut self, what: &str, log: &str, ask: &str) -> Next {
        let times = {
            let times = self.failures.entry(what.to_owned()).or_default();
            *times += 1;
            *times
        };
        if times >= MOST_FAILURES {
            return self.stop(format!(
                "{what} failed {times} times. The last of it:\n{}",
                tail(log)
            ));
        }
        self.note = Some(format!("{what} failed ({times})"));
        self.turn(format!("{ask}\n\n{}", tail(log)))
    }

    fn turn(&mut self, message: String) -> Next {
        self.stage = Stage::Implementing;
        self.tell = Some(message.clone());
        // What the owner was asked was about what is to change now.
        self.ticket = None;
        self.merge_approved = false;
        Next::Turn(message)
    }

    /// The agent's turn is over: what it left is looked at.
    pub fn turned(&mut self) {
        if self.stage == Stage::Implementing {
            self.stage = Stage::Checking;
            self.tell = None;
        }
    }

    /// Whether the agent has the work, and nothing is yet to be done here.
    pub fn with_agent(&self) -> bool {
        self.stage == Stage::Implementing && self.tell.is_none()
    }

    /// The agent was told what there was to tell it.
    pub fn given(&mut self, now: i64) {
        self.tell = None;
        self.turn_began = Some(now);
    }

    /// How the work came out, in the agent's words and in what was done with it.
    pub fn result(&self) -> String {
        let done = match self.stage {
            Stage::Stopped => self.note.clone().unwrap_or_default(),
            _ => self.summary(),
        };
        format!("{}\n\n---\n\n{done}", self.report)
    }

    /// Whether the owner is asked and has not answered.
    pub fn asking(&self) -> bool {
        self.stage == Stage::Merging && self.ticket.is_some() && !self.merge_approved
    }

    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        match &self.pull {
            Some(pull) if self.merged => lines.push(format!("Merged: {}", pull.url)),
            Some(pull) => lines.push(format!("Left open, not merged: {}", pull.url)),
            None => lines.push("Nothing differs, so nothing was pushed.".to_owned()),
        }
        if self.pinned.rule.deploy.is_some() && self.merged {
            lines.push("Deployed.".to_owned());
        }
        if let Some(note) = &self.note {
            lines.push(note.clone());
        }
        lines.join("\n")
    }

    /// Take one step. What it changes is to be kept before the next one.
    pub async fn step<H: Hands>(&mut self, hands: &H, now: i64) -> anyhow::Result<Next> {
        match self.stage {
            Stage::Implementing => Ok(match &self.tell {
                Some(message) => Next::Turn(message.clone()),
                None => {
                    self.turned();
                    Next::Again
                }
            }),
            Stage::Checking => self.check(hands).await,
            Stage::Reviewing => self.review(hands).await,
            Stage::Publishing => self.publish(hands).await,
            Stage::Ci => self.ci(hands, now).await,
            Stage::Merging => self.merge(hands).await,
            Stage::Deploying => self.deploy(hands, now).await,
            Stage::Tidying => {
                let merged = self
                    .pull
                    .as_ref()
                    .filter(|_| self.merged)
                    .map(|p| p.head.as_str());
                if let Err(error) = hands.tidy(&self.pinned, merged).await {
                    self.note = Some(format!("Not tidied: {error:#}"));
                }
                self.stage = Stage::Done;
                Ok(Next::Done)
            }
            Stage::Done => Ok(Next::Done),
            Stage::Stopped => Ok(Next::Stop(self.note.clone().unwrap_or_default())),
        }
    }

    async fn check<H: Hands>(&mut self, hands: &H) -> anyhow::Result<Next> {
        let content = match hands.snapshot(&self.pinned).await? {
            Snapshot::Unchanged => {
                self.stage = Stage::Tidying;
                return Ok(Next::Again);
            }
            Snapshot::Conflicted => {
                let target = &self.pinned.target;
                let ask = format!(
                    "The work holds conflicts. Resolve them with jj, on `{target}@origin`."
                );
                return Ok(self.failed("conflicts", "", &ask));
            }
            Snapshot::Changed(content) => content,
        };
        for command in self.pinned.rule.checks.clone() {
            if let Err(log) = hands.check(&command).await? {
                return Ok(self.failed(
                    &format!("check `{command}`"),
                    &log,
                    &format!("`{command}` failed on your work. Fix it."),
                ));
            }
        }
        // What a check wrote into the work is not what it passed on.
        if hands.snapshot(&self.pinned).await? != Snapshot::Changed(content.clone()) {
            return Ok(self.failed(
                "a change under the checks",
                "",
                "The work changed while it was checked. Leave it as it is to be, with nothing the checks rewrite.",
            ));
        }
        self.content = Some(content);
        self.stage = Stage::Reviewing;
        Ok(Next::Again)
    }

    async fn review<H: Hands>(&mut self, hands: &H) -> anyhow::Result<Next> {
        let content = self.checked()?;
        // Handed back and left as it was, it still has what was found.
        if self.reviewed.as_deref() == Some(&content.commit) {
            match self.found.clone() {
                Some(Found {
                    text,
                    serious: true,
                }) => return Ok(self.stop_faulty(&text)),
                found => self.remaining = found.map(|f| f.text),
            }
            self.stage = Stage::Publishing;
            return Ok(Next::Again);
        }
        if self.reviews >= self.pinned.rule.reviews {
            self.stage = Stage::Publishing;
            return Ok(Next::Again);
        }
        let verdict = hands.review(&self.pinned, &content).await?;
        self.reviews += 1;
        self.reviewed = Some(content.commit.clone());
        self.found = match &verdict {
            Verdict::Clean => None,
            Verdict::Findings { text, serious } => Some(Found {
                text: text.clone(),
                serious: *serious,
            }),
        };
        let last = self.reviews >= self.pinned.rule.reviews;
        match verdict {
            Verdict::Clean => self.remaining = None,
            Verdict::Findings { text, .. } if !last => {
                self.remaining = None;
                return Ok(self.turn(format!(
                    "A review of your work found this. Fix what holds; say why for what does not.\n\n{}",
                    tail(&text)
                )));
            }
            Verdict::Findings {
                text,
                serious: true,
            } => return Ok(self.stop_faulty(&text)),
            Verdict::Findings { text, .. } => self.remaining = Some(text),
        }
        self.stage = Stage::Publishing;
        Ok(Next::Again)
    }

    fn stop_faulty(&mut self, found: &str) -> Next {
        self.stop(format!(
            "The last review still finds faults of correctness or safety, so nothing was pushed:\n{}",
            tail(found)
        ))
    }

    fn checked(&self) -> anyhow::Result<Content> {
        self.content
            .clone()
            .ok_or_else(|| anyhow::anyhow!("nothing was checked"))
    }

    /// Whether what is there is still what was checked. When not, it is
    /// checked again.
    async fn unmoved<H: Hands>(&mut self, hands: &H) -> anyhow::Result<bool> {
        let content = self.checked()?;
        let same = matches!(
            hands.snapshot(&self.pinned).await?,
            Snapshot::Changed(now) if now.change == content.change && now.commit == content.commit
        );
        if !same {
            self.content = None;
            self.stage = Stage::Checking;
        }
        Ok(same)
    }

    async fn publish<H: Hands>(&mut self, hands: &H) -> anyhow::Result<Next> {
        if !self.unmoved(hands).await? {
            return Ok(Next::Again);
        }
        let content = self.checked()?;
        let before = self.pull.as_ref().map(|p| p.head.as_str());
        let pull = hands
            .publish(&self.pinned, &content, before, self.remaining.as_deref())
            .await?;
        self.pull = Some(pull);
        self.ci_since = None;
        self.ticket = None;
        self.merge_approved = false;
        self.stage = Stage::Ci;
        Ok(Next::Again)
    }

    fn pulled(&self) -> anyhow::Result<PullRequest> {
        self.pull
            .clone()
            .ok_or_else(|| anyhow::anyhow!("nothing was pushed"))
    }

    async fn ci<H: Hands>(&mut self, hands: &H, now: i64) -> anyhow::Result<Next> {
        if !self.unmoved(hands).await? {
            return Ok(Next::Again);
        }
        let pull = self.pulled()?;
        let since = *self.ci_since.get_or_insert(now);
        match hands.ci(&self.pinned, &pull).await? {
            Ci::Pending if now - since > CI_LIMIT_SECONDS => Ok(self.stop(format!(
                "CI has not passed after {} hours: {}",
                CI_LIMIT_SECONDS / 3600,
                pull.url
            ))),
            Ci::Pending => Ok(Next::Wait(CI_EVERY)),
            Ci::Failed { name, log } => Ok(self.failed(
                &format!("CI `{name}`"),
                &log,
                &format!("CI `{name}` failed on your work. Fix it."),
            )),
            Ci::Passed => {
                self.stage = Stage::Merging;
                Ok(Next::Again)
            }
        }
    }

    async fn merge<H: Hands>(&mut self, hands: &H) -> anyhow::Result<Next> {
        let pull = self.pulled()?;
        // Merged by a Famulen that went away before it kept so.
        if !hands.merged(&self.pinned, &pull).await? {
            if !self.unmoved(hands).await? {
                return Ok(Next::Again);
            }
            if !self.pinned.rule.merge && !self.merge_approved {
                match hands.confirm_merge(&pull, self.ticket).await? {
                    Answer::Waiting(ticket) => {
                        self.ticket = Some(ticket);
                        return Ok(Next::Wait(ASKED_EVERY));
                    }
                    Answer::Yes => self.merge_approved = true,
                    Answer::No => {
                        self.note = Some("The owner did not approve the merge.".to_owned());
                        self.stage = Stage::Done;
                        return Ok(Next::Done);
                    }
                }
                // What was approved is merged only as it passed.
                return Ok(Next::Again);
            }
            // CI is asked again right before: it may have been run anew.
            if hands.ci(&self.pinned, &pull).await? != Ci::Passed {
                self.stage = Stage::Ci;
                return Ok(Next::Again);
            }
            if let Err(error) = hands.merge(&self.pinned, &pull).await {
                return Ok(self.failed(
                    "merge",
                    &format!("{error:#}"),
                    &format!(
                        "The pull request could not be merged. Rebase the work onto `{}@origin` with jj and resolve what conflicts.",
                        self.pinned.target
                    ),
                ));
            }
        }
        self.merged = true;
        self.note = None;
        self.stage = match self.pinned.rule.deploy {
            Some(_) => Stage::Deploying,
            None => Stage::Tidying,
        };
        Ok(Next::Again)
    }

    async fn deploy<H: Hands>(&mut self, hands: &H, now: i64) -> anyhow::Result<Next> {
        let Some(command) = self.pinned.rule.deploy.clone() else {
            self.stage = Stage::Tidying;
            return Ok(Next::Again);
        };
        let Some(since) = self.deploy_started else {
            let waiting_since = *self.deploy_waiting_since.get_or_insert(now);
            // Kept before it runs, so that it never runs twice by itself.
            self.deploy_started = Some(now);
            hands.keep(self)?;
            return Ok(match hands.deploy(&self.pinned, &command).await {
                Ok(Started::Yes) => Next::Again,
                Ok(Started::Busy) => {
                    self.deploy_started = None;
                    if now - waiting_since > DEPLOY_LIMIT_SECONDS {
                        self.stop("Merged, but not deployed: another deployment has not ended.")
                    } else {
                        Next::Wait(ASKED_EVERY)
                    }
                }
                Err(error) => self.stop(format!(
                    "Merged, but deploying could not start:\n{}",
                    tail(&format!("{error:#}"))
                )),
            });
        };
        Ok(match hands.deployed().await? {
            Some(Ok(())) => {
                self.stage = Stage::Tidying;
                Next::Again
            }
            Some(Err(log)) => self.stop(format!("Merged, but deploying failed:\n{}", tail(&log))),
            None if now - since > DEPLOY_LIMIT_SECONDS => self
                .stop("Merged, but whether deploying succeeded is not known: it has told nothing."),
            None => Next::Wait(ASKED_EVERY),
        })
    }
}

/// The end of a log, where what went wrong is told.
fn tail(log: &str) -> String {
    let skip = log.chars().count().saturating_sub(LOG_CHARS);
    log.chars().skip(skip).collect()
}

/// What is to be done after a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    Again,
    Wait(Duration),
    /// Tell the agent this, and wait for its turn to end.
    Turn(String),
    Done,
    Stop(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Clean,
    Findings { text: String, serious: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ci {
    /// Still running, or nothing has run.
    Pending,
    Passed,
    Failed {
        name: String,
        log: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Waiting(i64),
    Yes,
    No,
}

/// What a step does outside what it keeps.
pub trait Hands: Send + Sync {
    fn keep(&self, finishing: &Finishing) -> anyhow::Result<()>;
    fn snapshot(&self, pinned: &Pinned) -> impl Future<Output = anyhow::Result<Snapshot>> + Send;
    /// The log of a check that did not pass, as the inner error.
    fn check(
        &self,
        command: &str,
    ) -> impl Future<Output = anyhow::Result<Result<(), String>>> + Send;
    fn review(
        &self,
        pinned: &Pinned,
        content: &Content,
    ) -> impl Future<Output = anyhow::Result<Verdict>> + Send;
    /// Push `content` and open its pull request, or bring the one there is up
    /// to it; `before` is the commit pushed last.
    fn publish(
        &self,
        pinned: &Pinned,
        content: &Content,
        before: Option<&str>,
        remaining: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<PullRequest>> + Send;
    fn ci(
        &self,
        pinned: &Pinned,
        pull: &PullRequest,
    ) -> impl Future<Output = anyhow::Result<Ci>> + Send;
    fn merged(
        &self,
        pinned: &Pinned,
        pull: &PullRequest,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;
    /// Merge, only while the pull request holds the commit pushed.
    fn merge(
        &self,
        pinned: &Pinned,
        pull: &PullRequest,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn confirm_merge(
        &self,
        pull: &PullRequest,
        ticket: Option<i64>,
    ) -> impl Future<Output = anyhow::Result<Answer>> + Send;
    /// Start deploying, unless another deployment is under way. It goes on by
    /// itself, whatever becomes of Famulen.
    fn deploy(
        &self,
        pinned: &Pinned,
        command: &str,
    ) -> impl Future<Output = anyhow::Result<Started>> + Send;
    /// How deploying ended, once it has; its log when it failed.
    fn deployed(&self) -> impl Future<Output = anyhow::Result<Option<Result<(), String>>>> + Send;
    /// `merged` is the commit that was pushed, once it is merged.
    fn tidy(
        &self,
        pinned: &Pinned,
        merged: Option<&str>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

mod check;
mod deploy;
mod drive;
mod forge;
mod hands;
mod pin;
mod repo;
mod review;

pub(crate) use check::*;
pub(crate) use deploy::*;
pub(crate) use drive::*;
pub(crate) use forge::*;
pub(crate) use hands::*;
pub(crate) use pin::*;
pub(crate) use repo::*;
pub(crate) use review::*;

#[cfg(test)]
pub(super) mod tests;
