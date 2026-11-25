//! Keeping what was read in confidence out of wider places. What a turn read
//! is marked on it; before the secretary says something where more people
//! would see it than the mark allows, the owner is asked.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::Value;

use crate::app::App;
use crate::approvals::Approvals;
use crate::input::{Input, Reach, Sender, Source};
use crate::journal::Journal;
use crate::permissions::Permissions;
use crate::slack::clients::SlackClients;
use crate::slack::web::{Room, Slack};
use crate::store::Store;

mod reader;
mod sources;
mod taint;

pub use reader::ModelReader;
pub use sources::PLAIN;
pub(crate) use sources::{of_builtin, of_connection, of_files, of_own};
use taint::{Disclosure, Holder};
pub(crate) use taint::{Kind, Taint};

/// How the reason of a request that asks about a disclosure begins. It is
/// what tells such a request from others, as the gateway keeps it.
const MARK: &str = "機密を含みうる発言：";

/// Kept in the journal under this kind.
pub(crate) const JOURNAL: &str = "disclosure";

/// Whether a request asks about a disclosure, told by why it was held.
pub(crate) fn reason_of(reason: &str) -> Option<&str> {
    reason.strip_prefix(MARK)?.lines().next()
}

/// What is about to be said, as the reader reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saying {
    pub text: String,
    /// Where, in words.
    pub room: String,
    /// What the turn read in confidence, in words.
    pub read: Vec<String>,
    /// The conversation there is what it answers, or was read for it.
    pub here: bool,
    /// What the owner themselves said in the turn; empty when nothing.
    pub owner: String,
}

/// What the reader made of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reading {
    /// What in it should not be said there; none when nothing.
    pub discloses: Option<String>,
    /// The owner's words that allow saying it there, word for word.
    pub allowed_by: Option<String>,
}

/// Reads what is about to be said for what should not be said there.
pub trait Reader: Send + Sync {
    fn read<'a>(&'a self, saying: &'a Saying) -> BoxFuture<'a, anyhow::Result<Reading>>;
}

/// The shortest quotation that can allow anything.
const QUOTED_CHARS: usize = 4;

impl Reading {
    /// The owner's words that allow it, when they are really the owner's.
    fn allowed(&self, saying: &Saying) -> Option<&str> {
        self.allowed_by
            .as_deref()
            .map(str::trim)
            .filter(|q| q.chars().count() >= QUOTED_CHARS && saying.owner.contains(q))
    }
}

/// What decides whether a saying is asked about.
pub(crate) struct Gate<'a, S> {
    pub store: &'a Store,
    pub memory_dir: &'a Path,
    pub permissions: &'a Permissions,
    pub approvals: &'a Approvals<S>,
    pub reader: Option<&'a Arc<dyn Reader>>,
}

/// Why the owner is asked, and what is kept for when they answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Asking {
    pub(crate) why: String,
    workspace: String,
    channel: String,
    taints: BTreeSet<Taint>,
}

impl Asking {
    /// Keep what an approval of `ticket` would cover.
    pub(crate) fn held(&self, store: &Store, ticket: i64) {
        Journal::new(store.clone()).record(
            JOURNAL,
            format!(
                "確認 #{ticket} を出した：{}",
                reason_of(&self.why).unwrap_or(&self.why)
            ),
        );
        let kept = store.running().and_then(|turn| match turn {
            Some(turn) => taint::keep_disclosure(
                store,
                &Disclosure {
                    ticket,
                    input: turn.input,
                    workspace: self.workspace.clone(),
                    channel: self.channel.clone(),
                    taints: self.taints.clone(),
                },
            ),
            None => Ok(()),
        });
        if let Err(error) = kept {
            tracing::warn!(%error, ticket, "could not keep what a request would disclose");
        }
    }
}

impl<S: Slack> Gate<'_, S> {
    /// Why the owner is asked before `text` is said in `channel` by a call
    /// given `input`; `None` when it may be said.
    pub(crate) async fn asks(
        &self,
        workspace: &str,
        channel: &str,
        text: &str,
        input: &Value,
    ) -> Option<Asking> {
        let workspace = self.approvals.workspace(workspace).to_owned();
        if self.approvals.is_owners_dm(&workspace, channel).await {
            return None;
        }
        let room = self.approvals.room(&workspace, channel).await;
        match self.weigh(&workspace, channel, &room, text, input).await {
            Ok(asking) => asking,
            Err(error) => {
                tracing::warn!(%error, "could not tell what the turn read; asking");
                let earlier = BTreeSet::from([Taint::earlier()]);
                Some(asking(
                    &workspace,
                    channel,
                    &room,
                    earlier,
                    "このターンで読んだものを確かめられなかった",
                ))
            }
        }
    }

    /// Why the owner is asked before a connection's tool does what reaches
    /// other people. What was read through that connection may go back by it.
    pub(crate) async fn asks_outside(&self, server: &str, input: &Value) -> Option<Asking> {
        let asking = |taints: BTreeSet<Taint>, read: String| Asking {
            why: format!("{MARK}ほかの人に届く。{read}。"),
            workspace: server.to_owned(),
            channel: String::new(),
            taints,
        };
        match self.read_before(server, input).await {
            Ok((taints, _)) if taints.is_empty() => None,
            // It is asked of, so that the pass the owner's answer gave is used up.
            Ok((taints, true)) => Some(asking(taints, "承認された呼び出し".into())),
            Ok((taints, false)) => {
                let read = labels(&taints).join("、");
                Some(asking(taints, format!("{read}を読んだ")))
            }
            Err(error) => {
                tracing::warn!(%error, "could not tell what the turn read; asking");
                Some(asking(
                    BTreeSet::new(),
                    "このターンで読んだものを確かめられなかった".into(),
                ))
            }
        }
    }

    async fn read_before(
        &self,
        server: &str,
        input: &Value,
    ) -> anyhow::Result<(BTreeSet<Taint>, bool)> {
        let Some(row) = self.store.running_input()? else {
            anyhow::bail!("no turn is running");
        };
        let mut taints = self.taints(row.id, &row.input).await?;
        taints.retain(|t| !t.came_by(server));
        let covered = self.covered(row.id, &row.input, server, "", input)?;
        Ok((taints, covered.exact))
    }

    /// What the worker of a run was handed and read.
    fn of_run(&self, run: i64) -> anyhow::Result<BTreeSet<Taint>> {
        let mut taints = match taint::task_of(self.store, run)? {
            Some(task) => taint::held_by(self.store, &Holder::Task(task))?,
            None => BTreeSet::new(),
        };
        let files = taint::files_of(self.store, run)?;
        taints.extend(of_files(self.memory_dir, files.iter().map(String::as_str)));
        Ok(taints)
    }

    /// What the runs a list of delegated work tells of were handed and read.
    pub(crate) fn of_runs(&self, answer: &str) -> anyhow::Result<Vec<Taint>> {
        let answer: Value = serde_json::from_str(answer).unwrap_or_default();
        let list = |key: &str| answer[key].as_array().into_iter().flatten();
        let runs = list("ended").chain(list("queue").map(|task| &task["run"]));
        let mut taints = BTreeSet::new();
        for run in runs {
            let id = &run["id"];
            let id = id.as_i64().or_else(|| id.as_str()?.parse().ok());
            if let Some(id) = id {
                taints.extend(self.of_run(id)?);
            }
        }
        Ok(taints.into_iter().collect())
    }

    async fn weigh(
        &self,
        workspace: &str,
        channel: &str,
        room: &Room,
        text: &str,
        input: &Value,
    ) -> anyhow::Result<Option<Asking>> {
        // A call outside a turn says what nobody knows the source of.
        let Some(row) = self.store.running_input()? else {
            anyhow::bail!("no turn is running");
        };
        let (here, taints): (BTreeSet<Taint>, BTreeSet<Taint>) = self
            .taints(row.id, &row.input)
            .await?
            .into_iter()
            .partition(|t| t.stays_in(workspace, channel));
        let answers = row
            .input
            .place()
            .is_some_and(|p| p.workspace == workspace && p.channel == channel);
        let covered = self.covered(row.id, &row.input, workspace, channel, input)?;
        if covered.exact {
            // It is asked of, so that the pass the owner's answer gave is used up.
            return Ok(Some(asking(
                workspace,
                channel,
                room,
                taints,
                "承認された発言",
            )));
        }
        if !taints.is_empty() && taints.is_subset(&covered.taints) {
            return Ok(None);
        }
        let inward = row.input.reach() != Reach::Full;
        let saying = Saying {
            text: text.to_owned(),
            room: room.described(),
            read: labels(&taints),
            here: answers || !here.is_empty(),
            owner: owners_words(&row.input),
        };
        // Nothing of such a turn goes out unasked, whatever is in it.
        let reading = if inward || (!taints.is_empty() && saying.owner.is_empty()) {
            Ok(Reading::default())
        } else {
            self.read(&saying).await
        };
        let journal = Journal::new(self.store.clone());
        if let Ok(reading) = &reading
            && let Some(words) = reading.allowed(&saying)
            && (!taints.is_empty() || reading.discloses.is_some())
        {
            journal.record(
                JOURNAL,
                format!(
                    "{} への発言を確認なしで通した：主人の言葉「{words}」",
                    saying.room
                ),
            );
            return Ok(None);
        }
        if !taints.is_empty() {
            let why = saying.read.join("、");
            return Ok(Some(asking(
                workspace,
                channel,
                room,
                taints,
                &format!("{why}を読んだ"),
            )));
        }
        let why = match reading {
            Ok(Reading {
                discloses: None, ..
            }) => return Ok(None),
            Ok(Reading {
                discloses: Some(why),
                ..
            }) => why,
            Err(error) => format!("見極めが発言を確かめられなかった（{error:#}）"),
        };
        let earlier = BTreeSet::from([Taint::earlier()]);
        if earlier.is_subset(&covered.taints) {
            return Ok(None);
        }
        Ok(Some(asking(workspace, channel, room, earlier, &why)))
    }

    async fn read(&self, saying: &Saying) -> anyhow::Result<Reading> {
        match self.reader {
            Some(reader) => reader.read(saying).await,
            None => anyhow::bail!("見極めがない"),
        }
    }

    /// Everything a turn read in confidence, and the turns it goes on from.
    pub(crate) async fn taints(&self, id: i64, input: &Input) -> anyhow::Result<BTreeSet<Taint>> {
        let mut taints = BTreeSet::new();
        for (id, input) in self.lineage(id, input)? {
            taints.extend(taint::held_by(self.store, &Holder::Input(id))?);
            match &input.source {
                Source::Wake { id, .. } => {
                    taints.extend(taint::held_by(self.store, &Holder::Wake(*id))?);
                }
                Source::Worker { run, .. } => {
                    taints.extend(self.of_run(*run)?);
                }
                Source::Slack {
                    workspace, channel, ..
                } => {
                    if !self.approvals.is_owners_dm(workspace, channel).await {
                        let room = self.approvals.room(workspace, channel).await;
                        taints.extend(
                            room.private
                                .ne(&Some(false))
                                .then(|| Taint::room(workspace, channel, &room)),
                        );
                    }
                }
                Source::Arrival { connection, .. } => {
                    taints.insert(Taint::of_connection(connection, "から届いたもの"));
                }
                Source::Lookout { .. } => {
                    taints.insert(Taint::new(Kind::Private, "見張りが渡したもの"));
                }
                _ => {}
            }
        }
        Ok(taints)
    }

    /// A turn, and those it goes on from by the owner's answer to a request.
    fn lineage(&self, id: i64, input: &Input) -> anyhow::Result<Vec<(i64, Input)>> {
        let mut turns = vec![(id, input.clone())];
        while let Some((
            _,
            Input {
                source: Source::Approval { id, .. },
                ..
            },
        )) = turns.last()
        {
            let origin = self
                .permissions
                .gateway()
                .ticket(*id)?
                .and_then(|t| t.origin);
            let before = match origin.and_then(|o| o.parse().ok()) {
                Some(id) => self.store.get(id)?,
                None => None,
            };
            match before {
                Some(row) if turns.iter().all(|(id, _)| *id != row.id) => {
                    turns.push((row.id, row.input));
                }
                _ => break,
            }
        }
        Ok(turns)
    }

    /// What the owner already let be said there in these turns.
    fn covered(
        &self,
        id: i64,
        input: &Input,
        workspace: &str,
        channel: &str,
        call: &Value,
    ) -> anyhow::Result<Covered> {
        let mut covered = Covered::default();
        for (id, _) in self.lineage(id, input)? {
            for disclosure in taint::disclosures_of(self.store, id)? {
                if (disclosure.workspace.as_str(), disclosure.channel.as_str())
                    != (workspace, channel)
                {
                    continue;
                }
                let ticket = self.permissions.gateway().ticket(disclosure.ticket)?;
                let Some(ticket) =
                    ticket.filter(|t| matches!(t.status.as_str(), "approved" | "always_allowed"))
                else {
                    continue;
                };
                covered.exact |=
                    ticket.shown == *call && self.permissions.gateway().has_pass(ticket.id)?;
                covered.taints.extend(disclosure.taints);
            }
        }
        Ok(covered)
    }
}

/// What taints are called, each once.
fn labels(taints: &BTreeSet<Taint>) -> Vec<String> {
    let labels: BTreeSet<&str> = taints.iter().map(|t| t.label.as_str()).collect();
    labels.into_iter().map(str::to_owned).collect()
}

#[derive(Default)]
struct Covered {
    taints: BTreeSet<Taint>,
    /// The call is one the owner approved, and it was not made yet.
    exact: bool,
}

fn asking(
    workspace: &str,
    channel: &str,
    room: &Room,
    taints: BTreeSet<Taint>,
    why: &str,
) -> Asking {
    Asking {
        why: format!("{MARK}送り先は{}。{why}。", room.described()),
        workspace: workspace.to_owned(),
        channel: channel.to_owned(),
        taints,
    }
}

/// The words of a turn that are the owner's own.
fn owners_words(input: &Input) -> String {
    match input.source {
        Source::Slack {
            unmentioned: false, ..
        }
        | Source::Console { .. }
            if input.sender == Sender::Owner =>
        {
            input.text.clone()
        }
        _ => String::new(),
    }
}

/// Mark the running turn with what it just read.
pub(crate) fn note(store: &Store, taints: Vec<Taint>) -> anyhow::Result<()> {
    if taints.is_empty() {
        return Ok(());
    }
    match store.running()? {
        Some(turn) => taint::mark(store, &Holder::Input(turn.input), &taints),
        None => anyhow::bail!("no turn is running"),
    }
}

/// Mark the task of a worker's run with what the worker read.
pub(crate) fn note_run(store: &Store, run: i64, taints: &[Taint]) -> anyhow::Result<()> {
    if taints.is_empty() {
        return Ok(());
    }
    match taint::task_of(store, run)? {
        Some(task) => taint::mark(store, &Holder::Task(task), taints),
        None => anyhow::bail!("no run {run}"),
    }
}

impl<S: Slack> Gate<'_, S> {
    /// Hand on what the turn read to what it leaves for later: a wake-up or a
    /// delegated task, as the call that left it answered.
    pub(crate) async fn hand_on(&self, tool: &str, answer: &str) -> anyhow::Result<()> {
        let answer: Value = serde_json::from_str(answer).unwrap_or_default();
        let id = &answer["id"];
        let holder = match tool {
            "wake_at" => id
                .as_i64()
                .or_else(|| id.as_str().and_then(|id| id.parse().ok()))
                .map(Holder::Wake),
            "delegate_task" | "retry_delegated" => {
                id.as_str().map(|id| Holder::Task(id.to_owned()))
            }
            "save_record" if answer["kind"] == "tasks" => {
                id.as_str().map(|id| Holder::Task(id.to_owned()))
            }
            _ => None,
        };
        let (Some(holder), Some(row)) = (holder, self.store.running_input()?) else {
            return Ok(());
        };
        let taints: Vec<Taint> = self.taints(row.id, &row.input).await?.into_iter().collect();
        // A wake-up may be given the number of one cancelled before.
        if matches!(holder, Holder::Wake(_)) {
            taint::clear(self.store, &holder)?;
        }
        taint::mark(self.store, &holder, &taints)
    }
}

/// What a connection's answer holds in confidence.
pub(crate) async fn of_answer<S: Slack>(
    clients: Option<&SlackClients<S>>,
    permissions: &Permissions,
    server: &str,
    message: &Value,
    response: &Value,
) -> Vec<Taint> {
    match permissions.connection(server) {
        Some(connection) => of_connection(clients, &connection, message, response).await,
        None => Vec::new(),
    }
}

/// Tools of Slack that say nothing.
const SILENT: &[&str] = &[
    "add_reaction",
    "channel_info",
    "download_file",
    "get_permalink",
    "list_channels",
    "open_dm",
    "read_channel",
    "search_messages",
    "set_app_icon",
    "user_info",
];

/// Whether a tool of a connection says something on Slack. One that does more
/// than read is taken to, so that a new one does not slip through.
pub(crate) fn says_on_slack(permissions: &Permissions, server: &str, tool: &str) -> bool {
    !SILENT.contains(&tool)
        && permissions
            .connection(server)
            .is_some_and(|c| sources::is_slack(&c))
        && !permissions.only_reads(server, tool).unwrap_or(false)
}

/// Words in the name of a tool that puts something before others.
const TELLING: &[&str] = &[
    "comment", "issue", "post", "publish", "pull", "put", "patch", "reply", "share", "tweet",
];

/// Whether a connection's tool reaches other people, for what was read in
/// confidence.
pub(crate) fn reaches_others(permissions: &Permissions, server: &str, tool: &str) -> bool {
    permissions
        .always_asked(&format!("mcp__{server}__{tool}"))
        .unwrap_or(true)
        || (sources::words(tool).any(|word| TELLING.contains(&word.as_str()))
            && !permissions.only_reads(server, tool).unwrap_or(false))
}

/// Why a worker's call is refused: what it says would be read by others, and
/// nobody sees what it read.
pub(crate) fn refused_to_workers(app: &App, server: &str, message: &Value) -> Option<String> {
    let tool = message.pointer("/params/name").and_then(Value::as_str)?;
    (message["method"] == "tools/call" && says_on_slack(&app.permissions, server, tool)).then(
        || {
            "Workers say nothing on Slack: put it in your result, and the secretary says it."
                .to_owned()
        },
    )
}

#[cfg(test)]
mod tests;
