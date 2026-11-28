//! Asking the owner before a confirmed tool runs, and acting on the answer.
//!
//! The call that needs approval is refused at once, so the turn ends instead
//! of waiting. When the owner approves, exactly that call is granted once and
//! the session is told it may make it again.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use famulen_gateway::{Question, Ticket};

use crate::asking;
use crate::claude::session::OUTWARD;
use crate::disclosure;
use crate::input::{Input, Reach, Sender, Source};
use crate::permissions::{OUTSIDE_WORKGROUP, Permissions};
use crate::slack::clients::SlackClients;
use crate::slack::events::Action;
use crate::slack::mrkdwn::escape;
use crate::slack::web::{Outgoing, Room, Slack};
use crate::store::{Store, now};
use crate::wording;
use crate::workers::agent::NOTEBOOK_THEN_OTHERS;

pub(crate) mod asked;
mod exposure;
pub(crate) mod stalled;

const ACTION_PREFIX: &str = "famulen.approval.";
/// Slack rejects a section longer than 3000 characters.
const INPUT_PREVIEW_CHARS: usize = 2500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Approve,
    Reject,
    AlwaysAllow,
}

impl Answer {
    fn action_id(self) -> String {
        let name = match self {
            Answer::Approve => "approve",
            Answer::Reject => "reject",
            Answer::AlwaysAllow => "always",
        };
        format!("{ACTION_PREFIX}{name}")
    }

    fn from_action_id(action_id: &str) -> Option<Self> {
        match action_id.strip_prefix(ACTION_PREFIX)? {
            "approve" => Some(Answer::Approve),
            "reject" => Some(Answer::Reject),
            "always" => Some(Answer::AlwaysAllow),
            _ => None,
        }
    }

    /// The answer a console button names: `approve`, `reject` or `always`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::from_action_id(&format!("{ACTION_PREFIX}{name}"))
    }

    /// What the owner reads of it once given.
    fn note(self) -> &'static str {
        match self {
            Answer::Approve => "承認しました",
            Answer::Reject => "却下しました",
            Answer::AlwaysAllow => "承認し、今後は常に許可にしました",
        }
    }

    fn status(self) -> &'static str {
        match self {
            Answer::Approve => "approved",
            Answer::Reject => "rejected",
            Answer::AlwaysAllow => "always_allowed",
        }
    }
}

/// An approval request as the console lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: i64,
    pub tool: String,
    pub input: String,
    /// `pending`, `approved`, `rejected`, `always_allowed` or `withdrawn`.
    pub status: String,
    pub created_at: i64,
}

/// The newest requests first.
pub fn recent(store: &Store, limit: usize) -> anyhow::Result<Vec<Request>> {
    let conn = store.conn();
    let mut statement = conn.prepare(
        "SELECT id, tool, input, status, created_at FROM approvals ORDER BY id DESC LIMIT ?1",
    )?;
    let requests = statement
        .query_map(params![limit as i64], |row| {
            Ok(Request {
                id: row.get(0)?,
                tool: row.get(1)?,
                input: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(requests)
}

/// Whether a request put to the owner waits no more.
pub fn answered(store: &Store, id: i64) -> anyhow::Result<bool> {
    let status: Option<String> = store
        .conn()
        .query_row("SELECT status FROM approvals WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(status.is_some_and(|s| s != "pending"))
}

/// Whether a Slack action belongs to an approval.
pub fn is_approval_action(action: &Action) -> bool {
    action.action_id.starts_with(ACTION_PREFIX) || action.action_id.starts_with(asking::PREFIX)
}

impl From<Answer> for famulen_gateway::Answer {
    fn from(answer: Answer) -> Self {
        match answer {
            Answer::Approve => Self::Approve,
            Answer::Reject => Self::Reject,
            Answer::AlwaysAllow => Self::AlwaysAllow,
        }
    }
}

fn ticket_tool(ticket: &Ticket) -> String {
    wording::tool_of(&ticket.connection, &ticket.tool)
}

pub struct Approvals<S> {
    slack: Option<SlackClients<S>>,
    console: Option<crate::console_log::ConsoleLog>,
    notices: Option<crate::notices::Notices>,
    store: Store,
    permissions: Permissions,
    /// The owner's ID in each workspace.
    owners: BTreeMap<String, String>,
}

impl<S: Slack> Approvals<S> {
    pub fn new(
        slack: Option<SlackClients<S>>,
        store: Store,
        permissions: Permissions,
        owners: BTreeMap<String, String>,
    ) -> Self {
        Self {
            slack,
            console: None,
            notices: None,
            store,
            permissions,
            owners,
        }
    }

    /// Tell open consoles when a request is asked or answered.
    pub fn with_console(mut self, console: crate::console_log::ConsoleLog) -> Self {
        self.console = Some(console);
        self
    }

    /// Tell the owner's devices when a request is asked.
    pub fn with_notices(mut self, notices: crate::notices::Notices) -> Self {
        self.notices = Some(notices);
        self
    }

    /// The devices are told aside: a push service that is slow keeps neither
    /// Slack nor the next request waiting.
    fn tell_devices(&self, ticket: &Ticket) {
        let Some(notices) = self.notices.clone() else {
            return;
        };
        let ticket = ticket.clone();
        tokio::spawn(async move {
            if let Err(error) = notices.tell(&ticket).await {
                tracing::warn!(%error, id = ticket.id, "could not tell the devices of a request");
            }
        });
    }

    fn tell_console(&self) {
        if let Some(console) = &self.console {
            console.notify(crate::console_log::Event::Approvals);
        }
    }

    /// Put a ticket the gateway held to the owner: on the console, and on
    /// Slack when it is connected. A ticket already put is left alone.
    pub async fn present(&self, ticket: &Ticket) -> anyhow::Result<()> {
        if self.lapsed(ticket)? {
            return Ok(());
        }
        self.put(ticket).await?;
        // It may have been taken back while it was being put.
        if self.lapsed(ticket)? {
            withdraw(&self.store, self.slack.as_ref(), ticket.id).await?;
            self.tell_console();
        }
        Ok(())
    }

    fn lapsed(&self, ticket: &Ticket) -> anyhow::Result<bool> {
        let now = self.permissions.gateway().ticket(ticket.id)?;
        Ok(now.is_some_and(|t| t.status == "lapsed"))
    }

    async fn put(&self, ticket: &Ticket) -> anyhow::Result<()> {
        let (id, tool, input) = (ticket.id, ticket_tool(ticket), &ticket.shown);
        let added = self.store.conn().execute(
            "INSERT OR IGNORE INTO approvals (id, tool, input, status, created_at)
             VALUES (?1, ?2, ?3, 'pending', ?4)",
            params![id, tool, input.to_string(), ticket.created_at],
        )?;
        if added == 0 {
            return Ok(());
        }
        self.tell_console();
        self.tell_devices(ticket);
        let Some(slack) = &self.slack else {
            return Ok(());
        };
        let message = self.ask_message(ticket, &tool);
        match self.ask_in_conversation(ticket, &message).await {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, id, "could not ask in the conversation; asking in the DM");
                self.store
                    .conn()
                    .execute("DELETE FROM approval_places WHERE approval = ?1", [id])?;
            }
        }
        let channel = slack.main().open_dm(self.owner(slack.main_id())).await?;
        let ts = slack.main().post(&channel, message).await?;
        self.posted(id, &channel, &ts)
    }

    fn owner(&self, workspace: &str) -> &str {
        self.owners.get(workspace).map_or("", String::as_str)
    }

    pub(crate) fn slack(&self) -> Option<&SlackClients<S>> {
        self.slack.as_ref()
    }

    /// The workspace a call names; the main one when it names none.
    pub(crate) fn workspace<'a>(&'a self, named: &'a str) -> &'a str {
        match &self.slack {
            Some(slack) if named.is_empty() => slack.main_id(),
            _ => named,
        }
    }

    /// What Slack tells of a conversation; nothing when it cannot be asked.
    pub(crate) async fn room(&self, workspace: &str, channel: &str) -> Room {
        let slack = self.slack.as_ref();
        match slack.and_then(|s| s.get(Some(workspace)).ok()) {
            Some(slack) => slack.room_or_unknown(channel).await,
            None => Room::default(),
        }
    }

    /// Whether `channel` is the bot's DM with the owner, in `workspace` or
    /// the main one when it is empty.
    pub async fn is_owners_dm(&self, workspace: &str, channel: &str) -> bool {
        let Some(slack) = &self.slack else {
            return false;
        };
        let workspace = if workspace.is_empty() {
            slack.main_id()
        } else {
            workspace
        };
        let owner = self.owner(workspace);
        if owner.is_empty() {
            return false;
        }
        match slack.get(Some(workspace)) {
            Ok(slack) => slack.open_dm(owner).await.is_ok_and(|dm| dm == channel),
            Err(_) => false,
        }
    }

    fn posted(&self, id: i64, channel: &str, ts: &str) -> anyhow::Result<()> {
        self.store.conn().execute(
            "UPDATE approvals SET channel = ?1, ts = ?2 WHERE id = ?3",
            params![channel, ts, id],
        )?;
        Ok(())
    }

    /// Ask in the thread of the Slack conversation the call was made in.
    /// False when it was not made in one, or may not be asked there.
    async fn ask_in_conversation(
        &self,
        ticket: &Ticket,
        message: &Outgoing,
    ) -> anyhow::Result<bool> {
        let Some(place) = self.conversation_of(ticket)? else {
            return Ok(false);
        };
        let Some(slack) = self
            .slack
            .as_ref()
            .and_then(|s| s.get(Some(&place.workspace)).ok())
        else {
            return Ok(false);
        };
        let owner = self.owner(&place.workspace);
        if owner.is_empty() {
            return Ok(false);
        }
        let in_dm = slack.open_dm(owner).await? == place.channel;
        if !in_dm && exposure::kept_to_dm(ticket) {
            return Ok(false);
        }
        // Before the post, so that no press can come before it is known.
        self.store.conn().execute(
            "INSERT INTO approval_places (approval, workspace, channel, thread)
             VALUES (?1, ?2, ?3, ?4)",
            params![ticket.id, place.workspace, place.channel, place.thread],
        )?;
        let message = Outgoing {
            thread_ts: Some(place.thread),
            ..message.clone()
        };
        if in_dm {
            let ts = slack.post(&place.channel, message).await?;
            self.posted(ticket.id, &place.channel, &ts)?;
        } else {
            slack.post_ephemeral(&place.channel, owner, message).await?;
        }
        Ok(true)
    }

    /// The Slack thread the owner was talking in when the call was made. One
    /// made after the owner's answer to a request belongs where that did.
    /// What others said does not count: the owner may not be looking there.
    fn conversation_of(&self, ticket: &Ticket) -> anyhow::Result<Option<Place>> {
        let mut origin = match ticket.actor.as_str() {
            crate::permissions::SECRETARY => ticket.origin.clone(),
            _ => None,
        };
        while let Some(input) = match origin.take() {
            Some(origin) => self.store.input_of_origin(&origin)?,
            None => None,
        }
        .filter(|input| input.reach() != Reach::Guest)
        {
            match input.source {
                Source::Slack {
                    workspace,
                    channel,
                    ts,
                    thread_ts,
                    ..
                } if input.sender == Sender::Owner => {
                    return Ok(Some(Place {
                        workspace,
                        channel,
                        thread: thread_ts.unwrap_or(ts),
                    }));
                }
                Source::Approval { id, .. } => {
                    origin = self
                        .permissions
                        .gateway()
                        .ticket(id)?
                        .and_then(|before| before.origin);
                }
                _ => {}
            }
        }
        Ok(None)
    }

    /// The owner's ID where a request was asked; empty without Slack.
    fn owner_asked(&self, id: i64) -> anyhow::Result<&str> {
        let place = place_of(&self.store, id)?;
        let workspace = match (&place, &self.slack) {
            (Some(place), _) => place.workspace.as_str(),
            (None, Some(slack)) => slack.main_id(),
            (None, None) => return Ok(""),
        };
        Ok(self.owner(workspace))
    }

    /// Whether a press is the owner's, in a workspace the request it is about
    /// was asked in.
    fn pressed_by_owner(&self, action: &Action, id: i64) -> anyhow::Result<bool> {
        let Some(slack) = &self.slack else {
            return Ok(false);
        };
        let asked_in = match place_of(&self.store, id)? {
            Some(place) => place.workspace,
            None => slack.main_id().to_owned(),
        };
        Ok(action.workspace == asked_in
            && !action.user.is_empty()
            && action.user == self.owner(&asked_in))
    }

    /// The message that asks for an answer to a request.
    fn ask_message(&self, ticket: &Ticket, tool: &str) -> Outgoing {
        let (id, input) = (ticket.id, &ticket.shown);
        let questions = questions(&self.permissions, tool, input);
        if questions.is_empty() {
            let asked = Asked::of(&self.permissions, tool);
            let once = once(&self.permissions, tool) || answered_once(ticket);
            let summary = summary(ticket);
            let still_asked = still_asked(ticket).filter(|_| !once);
            return request_message(id, &asked, input, summary, None, once, still_asked);
        }
        question_message(id, tool, summary(ticket), &questions)
    }

    /// Put every ticket still held that the owner has not been shown.
    pub async fn present_held(&self) -> anyhow::Result<()> {
        for ticket in self.permissions.gateway().held_tickets()? {
            self.present(&ticket).await?;
        }
        Ok(())
    }

    /// Act on a button press. Returns the input that tells the session the
    /// outcome, or `None` when the press is not the owner's or the request
    /// was already answered.
    pub async fn answer(&self, action: &Action) -> anyhow::Result<Option<Input>> {
        let Ok(id) = action.value.parse::<i64>() else {
            return Ok(None);
        };
        if !self.pressed_by_owner(action, id)? {
            return Ok(None);
        }
        if action.action_id.starts_with(asking::PREFIX) {
            let Some(ticket) = self.permissions.gateway().ticket(id)? else {
                return Ok(None);
            };
            let asked = questions(&self.permissions, &ticket_tool(&ticket), &ticket.shown);
            return match asking::replies(&action.action_id, &action.fields, &asked) {
                Some(replies) => self.fill_by_id(id, replies).await,
                None => Ok(None),
            };
        }
        let Some(answer) = Answer::from_action_id(&action.action_id) else {
            return Ok(None);
        };
        self.answer_by_id(id, answer).await
    }

    /// Answer the questions a request asks, as given on the console or by a
    /// press that passed its checks.
    pub async fn fill_by_id(&self, id: i64, replies: Vec<String>) -> anyhow::Result<Option<Input>> {
        let said = replies.join(" / ");
        if let Some(ticket) = self.asked(id)? {
            return self.settle_asked(ticket, Answer::Approve, Some(said)).await;
        }
        if let Some(ticket) = self.stall(id)? {
            return self.settle_stall(ticket, Answer::Approve, Some(said)).await;
        }
        let Some(answered) = self.permissions.gateway().fill_ticket(id, replies).await? else {
            return Ok(None);
        };
        self.settle(id, Answer::Approve, answered, Some(said)).await
    }

    /// Act on an answer given on the console, or a button press that passed
    /// its checks. `None` when there is nothing to tell the session: it was
    /// answered before, or the call is not the session's.
    pub async fn answer_by_id(&self, id: i64, answer: Answer) -> anyhow::Result<Option<Input>> {
        if let Some(ticket) = self.asked(id)? {
            return self.settle_asked(ticket, answer, None).await;
        }
        if let Some(ticket) = self.stall(id)? {
            return self.settle_stall(ticket, answer, None).await;
        }
        let before = self.permissions.gateway().ticket(id)?;
        let answer = match answer {
            Answer::AlwaysAllow
                if before
                    .as_ref()
                    .is_some_and(|t| answered_once(t) || still_asked(t).is_some()) =>
            {
                Answer::Approve
            }
            answer => answer,
        };
        let Some(answered) = self
            .permissions
            .gateway()
            .answer_ticket(id, answer.into())
            .await?
        else {
            tracing::debug!(id, "the gateway's ticket was answered or lapsed already");
            return Ok(None);
        };
        self.settle(id, answer, answered, None).await
    }

    /// The request `id`, when it asks whether someone else may be told.
    fn asked(&self, id: i64) -> anyhow::Result<Option<Ticket>> {
        Ok(self.permissions.gateway().ticket(id)?.filter(asked::is_ask))
    }

    /// Act on the owner's word on what someone else asks.
    async fn settle_asked(
        &self,
        ticket: Ticket,
        answer: Answer,
        said: Option<String>,
    ) -> anyhow::Result<Option<Input>> {
        let word = asked::Word::of(answer, said.as_deref());
        let answer = word.answer();
        let Some(answered) = self
            .permissions
            .gateway()
            .answer_ticket(ticket.id, answer.into())
            .await?
        else {
            return Ok(None);
        };
        // A message that cannot be changed does not take the word back.
        if let Err(error) = self.settle(ticket.id, answer, answered, said).await {
            tracing::warn!(%error, id = ticket.id, "could not show the answer");
        }
        asked::outcome(&self.store, &ticket, word)
    }

    /// The request `id`, when it asks what to do with stopped work.
    fn stall(&self, id: i64) -> anyhow::Result<Option<Ticket>> {
        let ticket = self.permissions.gateway().ticket(id)?;
        Ok(ticket.filter(stalled::is_stall))
    }

    /// Keep the owner's word on stopped work, for the pool to act on.
    pub(crate) async fn settle_stall(
        &self,
        ticket: Ticket,
        answer: Answer,
        said: Option<String>,
    ) -> anyhow::Result<Option<Input>> {
        // What is kept is what the ticket takes: the console and Slack answer
        // one at a time.
        static ANSWERING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let answering = ANSWERING.lock().await;
        let held = self.permissions.gateway().ticket(ticket.id)?;
        if held.is_none_or(|t| t.status != "held") {
            return Ok(None);
        }
        let approved = answer != Answer::Reject;
        let word = stalled::Word::of(&ticket.shown, approved, said.as_deref());
        // Kept before the ticket is answered, so that an answered one is never
        // found without it.
        crate::workers::Runs::answer_stall(&self.store, &ticket, &word.kept())?;
        let answer = if approved {
            Answer::Approve
        } else {
            Answer::Reject
        };
        let gateway = self.permissions.gateway();
        let Some(answered) = gateway.answer_ticket(ticket.id, answer.into()).await? else {
            // Taken back in between.
            crate::workers::Runs::withdrawn_stall(&self.store, ticket.id)?;
            return Ok(None);
        };
        drop(answering);
        // It lets no call run.
        gateway.drop_pass(ticket.id)?;
        // A message that cannot be changed does not take the word back.
        if let Err(error) = self.settle(ticket.id, answer, answered, said).await {
            tracing::warn!(%error, id = ticket.id, "could not show the answer");
        }
        Ok(None)
    }

    /// Show a request as answered, and tell the session.
    async fn settle(
        &self,
        id: i64,
        answer: Answer,
        answered: famulen_gateway::Answered,
        said: Option<String>,
    ) -> anyhow::Result<Option<Input>> {
        let Some(Claimed {
            tool,
            input,
            channel,
            ts,
        }) = self.claim(id, answer)?
        else {
            return Ok(None);
        };
        self.tell_console();
        if discloses(&answered.ticket) {
            crate::journal::Journal::new(self.store.clone()).record(
                disclosure::JOURNAL,
                format!("確認 #{id}：{}", answer.note()),
            );
        }
        if let Some(clients) = &self.slack {
            let place = place_of(&self.store, id)?;
            let workspace = place.as_ref().map(|p| p.workspace.as_str());
            match (clients.get(workspace), channel, ts, &place) {
                (Ok(slack), Some(channel), Some(ts), _) => {
                    let asked = Asked::of(&self.permissions, &tool);
                    let summary = summary(&answered.ticket);
                    let failed = match &answered.ran {
                        Some(Err(error)) => Some(format!("届けられませんでした: {error:#}")),
                        _ => None,
                    };
                    let note = failed.or(said.map(|said| format!("「{said}」と答えました")));
                    let message = match note {
                        Some(note) => noted_message(id, &asked, summary, &note),
                        None => {
                            request_message(id, &asked, &input, summary, Some(answer), false, None)
                        }
                    };
                    slack.update(&channel, &ts, message).await?;
                }
                // What only the owner saw cannot be changed, so they are told.
                (Ok(slack), _, _, Some(place)) => {
                    let message = place.said(format!("確認 #{id}: {}", answer.note()));
                    let owner = self.owner(&place.workspace);
                    if let Err(error) = slack.post_ephemeral(&place.channel, owner, message).await {
                        tracing::warn!(%error, id, "could not show the answer in the conversation");
                    }
                }
                _ => {}
            }
        }
        // A worker's call is made by the pool, which sees the answer itself.
        if crate::permissions::is_worker(&answered.ticket.actor) {
            return Ok(None);
        }
        if let Some(words) = asked::held_answer(&self.store, &answered.ticket)? {
            return match answer {
                Answer::Reject => Ok(None),
                _ => asked::approved(&self.store, &answered.ticket, words),
            };
        }
        let text = match answered.ran {
            Some(Ok(())) if answer == Answer::Reject => {
                format!("Request #{id} ({tool}) was rejected by the owner. Do not do it.")
            }
            Some(Ok(())) => format!(
                "Request #{id} ({tool}) was approved and done, and the owner sees so. Say nothing of it."
            ),
            Some(Err(error)) if answer == Answer::Reject => {
                format!(
                    "Request #{id} ({tool}) was rejected, but that could not be told: {error:#}"
                )
            }
            Some(Err(error)) => {
                format!("Request #{id} ({tool}) was approved but failed: {error:#}")
            }
            None => outcome_text(id, &tool, &input, answer),
        };
        Ok(Some(Input {
            sender: Sender::Owner,
            source: Source::Approval {
                id,
                user: self.owner_asked(id)?.to_owned(),
            },
            text,
        }))
    }

    /// Mark a pending request answered, once. Returns what it asked for.
    fn claim(&self, id: i64, answer: Answer) -> anyhow::Result<Option<Claimed>> {
        let conn = self.store.conn();
        let changed = conn.execute(
            "UPDATE approvals SET status = ?1, answered_at = ?2 WHERE id = ?3 AND status = 'pending'",
            params![answer.status(), now(), id],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        let row: Option<(String, String, Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT tool, input, channel, ts FROM approvals WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        row.map(|(tool, input, channel, ts)| {
            Ok(Claimed {
                tool,
                input: serde_json::from_str(&input)?,
                channel,
                ts,
            })
        })
        .transpose()
    }
}

/// [`withdraw`] requests the gateway took back, apart from what goes on, and
/// tell open consoles.
pub(crate) fn withdraw_apart(app: &crate::app::App, ids: Vec<i64>) {
    let (store, console) = (app.store.clone(), app.console.clone());
    let slack = app.slack.read().unwrap_or_else(|p| p.into_inner()).clone();
    tokio::spawn(async move {
        for id in ids {
            if let Err(error) = withdraw(&store, slack.as_deref(), id).await {
                tracing::warn!(%error, id, "could not take back a request");
            }
        }
        console.notify(crate::console_log::Event::Approvals);
    });
}

/// Take back a request that is asked no more: it leaves the console, and its
/// message on Slack loses its buttons.
pub async fn withdraw<S: Slack>(
    store: &Store,
    slack: Option<&SlackClients<S>>,
    id: i64,
) -> anyhow::Result<()> {
    let row: Option<(String, Option<String>, Option<String>)> = {
        let conn = store.conn();
        let changed = conn.execute(
            "UPDATE approvals SET status = 'withdrawn', answered_at = ?1 WHERE id = ?2 AND status = 'pending'",
            params![now(), id],
        )?;
        if changed == 0 {
            return Ok(());
        }
        conn.query_row(
            "SELECT tool, channel, ts FROM approvals WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
    };
    let place = place_of(store, id)?;
    let workspace = place.as_ref().map(|p| p.workspace.as_str());
    let slack = slack.and_then(|s| s.get(workspace).ok());
    if let (Some(slack), Some((tool, Some(channel), Some(ts)))) = (slack, row) {
        let text = format!(
            "*確認 #{id}* 「{}」は、もう要らなくなったので取り下げました",
            escape(&wording::deed(&tool, None))
        );
        let blocks = json!([section(&text)]);
        slack
            .update(
                &channel,
                &ts,
                Outgoing {
                    text,
                    blocks: Some(blocks),
                    thread_ts: None,
                },
            )
            .await?;
    }
    Ok(())
}

/// A thread on Slack a request was asked in.
struct Place {
    workspace: String,
    channel: String,
    thread: String,
}

impl Place {
    fn said(&self, text: String) -> Outgoing {
        Outgoing {
            text,
            thread_ts: Some(self.thread.clone()),
            ..Default::default()
        }
    }
}

/// Where a request was asked, when not in the main workspace's DM.
fn place_of(store: &Store, id: i64) -> anyhow::Result<Option<Place>> {
    Ok(store
        .conn()
        .query_row(
            "SELECT workspace, channel, thread FROM approval_places WHERE approval = ?1",
            params![id],
            |row| {
                Ok(Place {
                    workspace: row.get(0)?,
                    channel: row.get(1)?,
                    thread: row.get(2)?,
                })
            },
        )
        .optional()?)
}

/// A request that was just answered, and where its message is.
struct Claimed {
    tool: String,
    input: Value,
    channel: Option<String>,
    ts: Option<String>,
}

/// Whether the owner is asked of every call of `tool`, so it cannot be let
/// run from now on.
pub fn once(permissions: &Permissions, tool: &str) -> bool {
    tool == asked::TOOL
        || tool == stalled::TOOL
        || famulen_gateway::permissions::split_tool(tool).is_some_and(|(connection, tool)| {
            permissions
                .gateway()
                .asks_each_time(connection, tool)
                .unwrap_or(true)
        })
}

/// Whether what a request shows may hold a secret, or words from elsewhere.
pub fn may_show_secret(ticket: &Ticket) -> bool {
    exposure::kept_to_dm(ticket)
}

/// Whether a request asks to let be said what may be confidential. It is asked
/// each time.
pub fn discloses(ticket: &Ticket) -> bool {
    disclosure::reason_of(&ticket.reason).is_some()
}

/// Whether a request is answered for its one call alone, however the owner
/// answers: it tells something more widely, or a run of work run directly
/// asks it, whose approval would let every caller use the tool.
pub fn answered_once(ticket: &Ticket) -> bool {
    discloses(ticket) || crate::permissions::is_job_actor(&ticket.actor)
}

/// Why allowing the tool would not stop calls like the one `ticket` holds,
/// so the owner is not offered to.
pub fn still_asked(ticket: &Ticket) -> Option<&'static str> {
    // A ticket held without the ask kept apart still begins its reason with it.
    let ask = ticket.ask.as_deref().or_else(|| {
        ticket.reason.lines().next().filter(|line| {
            *line == OUTWARD || *line == NOTEBOOK_THEN_OTHERS || line.contains(OUTSIDE_WORKGROUP)
        })
    })?;
    if answered_once(ticket) {
        return None;
    }
    Some(if ask == OUTWARD {
        "見張りが拾ったものを扱っているので、外へ出すものは許可にしていても聞きます"
    } else if ask == NOTEBOOK_THEN_OTHERS {
        "手帳を渡した手代が他人の書いたものを読んだので、許可にしていても聞きます"
    } else if ask.contains(OUTSIDE_WORKGROUP) {
        "このつなぎ先はワークグループの外なので、許可にしていても聞きます"
    } else {
        "この場では、道具を許可にしていても聞きます"
    })
}

/// Whether the owner answers questions in a request instead of approving it.
pub fn asks_questions(permissions: &Permissions, ticket: &Ticket) -> bool {
    !questions(permissions, &ticket_tool(ticket), &ticket.shown).is_empty()
}

/// The questions the owner answers in a request instead of approving it.
pub fn questions(permissions: &Permissions, tool: &str, shown: &Value) -> Vec<Question> {
    if tool == asked::TOOL {
        return vec![asked::question(shown)];
    }
    if tool == stalled::TOOL {
        return vec![stalled::question(shown)];
    }
    famulen_gateway::permissions::split_tool(tool)
        .and_then(|(connection, tool)| {
            permissions
                .gateway()
                .questions_of(connection, tool, shown)
                .ok()
        })
        .unwrap_or_default()
}

/// What is said of a request beside what is asked: by the secretary, or why
/// it would disclose something.
pub fn summary(ticket: &Ticket) -> Option<&str> {
    disclosure::reason_of(&ticket.reason)
        .or_else(|| ticket.shown["call"]["summary"].as_str())
        .filter(|s| !s.trim().is_empty())
}

fn section(text: &str) -> Value {
    json!({ "type": "section", "text": { "type": "mrkdwn", "text": text } })
}

fn context(text: &str) -> Value {
    json!({ "type": "context", "elements": [{ "type": "mrkdwn", "text": text }] })
}

/// What a held call does, and where.
struct Asked {
    deed: String,
    place: String,
}

impl Asked {
    fn of(permissions: &Permissions, tool: &str) -> Self {
        let connection = permissions.connection_of(tool);
        Self {
            deed: wording::deed(tool, connection.as_ref()),
            place: wording::place(tool, connection.as_ref()),
        }
    }

    fn mrkdwn(&self) -> String {
        format!("「{}」（{}）", escape(&self.deed), escape(&self.place))
    }
}

/// The part of a request's message that says what it is about.
fn heading(id: i64, what: &str, summary: Option<&str>) -> Vec<Value> {
    let mut blocks = vec![section(&format!("*確認 #{id}* {what}"))];
    if let Some(summary) = summary {
        let text: String = crate::slack::mrkdwn::to_mrkdwn(summary)
            .chars()
            .take(INPUT_PREVIEW_CHARS)
            .collect();
        blocks.push(section(&text));
    }
    blocks
}

fn question_message(id: i64, tool: &str, summary: Option<&str>, asked: &[Question]) -> Outgoing {
    let what = if tool == asked::TOOL {
        "ほかの人からの質問"
    } else if tool == stalled::TOOL {
        "止まった委託"
    } else {
        "セッションからの質問"
    };
    let mut blocks = heading(id, what, summary);
    match asking::blocks(id, asked) {
        Some(asking) => blocks.extend(asking),
        None => blocks.push(context("長くて全部は出せないので、コンソールで答える")),
    }
    Outgoing {
        text: format!(
            "確認 #{id}: {}",
            asked
                .first()
                .map(|q| q.question.as_str())
                .unwrap_or_default()
        ),
        blocks: Some(Value::Array(blocks)),
        thread_ts: None,
    }
}

/// A request's message once it is answered, with how.
fn noted_message(id: i64, asked: &Asked, summary: Option<&str>, note: &str) -> Outgoing {
    let mut blocks = heading(id, &asked.mrkdwn(), summary);
    blocks.push(json!({ "type": "section", "text": { "type": "plain_text", "text": note.chars().take(INPUT_PREVIEW_CHARS).collect::<String>() } }));
    Outgoing {
        text: format!("確認 #{id}: {note}"),
        blocks: Some(Value::Array(blocks)),
        thread_ts: None,
    }
}

fn request_message(
    id: i64,
    asked: &Asked,
    input: &Value,
    summary: Option<&str>,
    answered: Option<Answer>,
    once: bool,
    still_asked: Option<&str>,
) -> Outgoing {
    let mut blocks = heading(id, &format!("{}を許しますか？", asked.mrkdwn()), summary);
    // What is decided is shown as it is, without the secretary's own words.
    let input = input.get("decides").unwrap_or(input);
    let said = said(input);
    blocks.push(json!({ "type": "section", "text": { "type": "plain_text", "text": preview(&said), "emoji": false } }));
    match answered {
        None => {
            // What is decided only once is not approved unread.
            let unread = once && said.chars().count() > INPUT_PREVIEW_CHARS;
            let mut buttons = Vec::new();
            if unread {
                blocks.push(context("長くて全部は出せないので、承認はコンソールでする"));
            } else {
                buttons.push(button("承認", Answer::Approve, id, Some("primary")));
            }
            buttons.push(button("却下", Answer::Reject, id, Some("danger")));
            if let Some(why) = still_asked {
                blocks.push(context(why));
            } else if !once {
                buttons.push(button("今後は常に許可", Answer::AlwaysAllow, id, None));
            }
            blocks.push(json!({ "type": "actions", "elements": buttons }));
        }
        Some(answer) => {
            blocks.push(context(answer.note()));
        }
    }
    Outgoing {
        text: format!("確認 #{id}: {}", asked.deed),
        blocks: Some(Value::Array(blocks)),
        thread_ts: None,
    }
}

fn button(label: &str, answer: Answer, id: i64, style: Option<&str>) -> Value {
    let mut button = json!({
        "type": "button",
        "text": { "type": "plain_text", "text": label },
        "action_id": answer.action_id(),
        "value": id.to_string(),
    });
    if let Some(style) = style {
        button["style"] = json!(style);
    }
    button
}

fn pretty(input: &Value) -> String {
    serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string())
}

/// What a call is given, in the owner's words.
fn said(input: &Value) -> String {
    match wording::given(input) {
        Some(rows) if rows.is_empty() => "（何も渡さない）".into(),
        Some(rows) => wording::plain(&rows),
        None => pretty(input),
    }
}

fn preview(text: &str) -> String {
    if text.chars().count() <= INPUT_PREVIEW_CHARS {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(INPUT_PREVIEW_CHARS).collect();
    cut.push_str("\n…");
    cut
}

fn outcome_text(id: i64, tool: &str, input: &Value, answer: Answer) -> String {
    match answer {
        Answer::Approve => format!(
            "Request #{id} was approved. You may now call {tool} once, with exactly this input:\n{input}"
        ),
        Answer::AlwaysAllow => format!(
            "Request #{id} was approved, and the owner now allows {tool} without asking. Call it with this input:\n{input}"
        ),
        Answer::Reject => {
            format!("Request #{id} ({tool}) was rejected by the owner. Do not do it.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{Bound, Caller, Decision, Permission};
    use crate::slack::fake::FakeSlack;

    const OWNER: &str = "U1";

    fn press(answer: Answer, id: i64, user: &str) -> Action {
        Action {
            workspace: "w1".into(),
            user: user.into(),
            action_id: answer.action_id(),
            value: id.to_string(),
            fields: Value::Null,
        }
    }

    /// One of Famulen's own tools the owner is asked about first.
    const TOOL: &str = "mcp__famulen__send_mail";

    fn setup(slack: &FakeSlack) -> (Approvals<&FakeSlack>, Permissions) {
        let store = Store::open_in_memory().unwrap();
        let permissions = Permissions::new(
            store.clone(),
            crate::permissions::test_gateway(
                famulen_gateway::store::Store::open_in_memory().unwrap(),
            ),
        );
        permissions
            .record_own_tools(&[crate::hub::ToolDef {
                name: "send_mail".into(),
                description: String::new(),
                input_schema: json!({}),
                read_only: false,
                confirm: true,
            }])
            .unwrap();
        let clients = ["w1", "w2"].map(|w| (w.to_owned(), slack));
        let clients = SlackClients::new(clients, "w1").unwrap();
        let owners = [("w1", OWNER), ("w2", "U9")]
            .map(|(w, o)| (w.to_owned(), o.to_owned()))
            .into();
        (
            Approvals::new(Some(clients), store, permissions.clone(), owners),
            permissions,
        )
    }

    fn input() -> Value {
        json!({ "to": "a@example.com", "body": "hi" })
    }

    #[tokio::test]
    async fn the_owners_devices_are_told_once_of_a_request() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        let (notices, kept) =
            crate::notices::testing::notices(approvals.store.clone(), permissions.clone());
        notices
            .add(&crate::notices::testing::device("phone"), "iPhone")
            .unwrap();
        let approvals = approvals.with_notices(notices);
        request(&approvals, &permissions).await;
        approvals.present_held().await.unwrap();
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert_eq!(kept.0.lock().unwrap().len(), 1);
    }

    /// Have the call held and put to the owner, as a turn would.
    async fn request(approvals: &Approvals<&FakeSlack>, permissions: &Permissions) -> i64 {
        let Decision::Held(id) = permissions.decide(TOOL, &input()).await.unwrap() else {
            panic!("held");
        };
        approvals.present_held().await.unwrap();
        id
    }

    #[tokio::test]
    async fn a_request_reaches_the_owners_dm_with_three_buttons() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        let id = request(&approvals, &permissions).await;
        let posts = slack.posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0, "DM-U1");
        let blocks = posts[0].1.blocks.clone().unwrap();
        let buttons = blocks[2]["elements"].as_array().unwrap();
        let ids: Vec<&str> = buttons
            .iter()
            .map(|b| b["action_id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "famulen.approval.approve",
                "famulen.approval.reject",
                "famulen.approval.always"
            ]
        );
        assert!(
            buttons
                .iter()
                .all(|b| b["value"].as_str() == Some(id.to_string().as_str()))
        );
        assert!(
            blocks[1]["text"]["text"]
                .as_str()
                .unwrap()
                .contains("宛先：a@example.com")
        );
        assert_eq!(
            blocks[0]["text"]["text"],
            format!("*確認 #{id}* 「メールを送る」（Famulen）を許しますか？")
        );
        assert_eq!(blocks[1]["text"]["type"], "plain_text");
        assert!(!posts[0].1.text.contains("mcp__"));
    }

    /// Start the turn of an input, as the session would.
    fn turn(approvals: &Approvals<&FakeSlack>, sender: Sender, source: Source) {
        let store = &approvals.store;
        store.end_turn().unwrap();
        let input = Input {
            sender,
            source,
            text: "x".into(),
        };
        let id = store.enqueue(&input).unwrap().unwrap();
        assert!(store.start_turn(id, "all").unwrap());
    }

    fn said_in(workspace: &str, channel: &str, thread_ts: Option<&str>) -> Source {
        Source::Slack {
            workspace: workspace.into(),
            channel: channel.into(),
            ts: "5.5".into(),
            thread_ts: thread_ts.map(Into::into),
            user: OWNER.into(),
            unmentioned: false,
            others_before: false,
            attached: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_call_made_in_a_channels_thread_is_asked_there_for_the_owner_alone() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        turn(&approvals, Sender::Owner, said_in("w1", "C1", Some("5.0")));
        let id = request(&approvals, &permissions).await;
        assert!(slack.posts().is_empty());
        let asked = slack.ephemerals();
        assert_eq!(asked.len(), 1);
        assert_eq!((asked[0].0.as_str(), asked[0].1.as_str()), ("C1", OWNER));
        assert_eq!(asked[0].2.thread_ts.as_deref(), Some("5.0"));
        approvals
            .answer(&press(Answer::Approve, id, OWNER))
            .await
            .unwrap()
            .unwrap();
        assert!(slack.updates().is_empty());
        let told = slack.ephemerals().pop().unwrap();
        assert!(told.2.text.contains("承認しました"));
        assert_eq!(told.2.thread_ts.as_deref(), Some("5.0"));
    }

    #[tokio::test]
    async fn a_call_made_in_the_dm_is_asked_in_the_thread_of_the_message_and_stays() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        turn(&approvals, Sender::Owner, said_in("w1", "DM-U1", None));
        request(&approvals, &permissions).await;
        let posts = slack.posts();
        assert_eq!(posts[0].0, "DM-U1");
        assert_eq!(posts[0].1.thread_ts.as_deref(), Some("5.5"));
        assert!(slack.ephemerals().is_empty());
    }

    #[tokio::test]
    async fn a_call_made_in_another_workspace_is_asked_and_answered_there_by_its_owner() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        turn(&approvals, Sender::Owner, said_in("w2", "C1", None));
        let id = request(&approvals, &permissions).await;
        assert_eq!(slack.ephemerals()[0].1, "U9");
        let press_in = |workspace: &str, user: &str| Action {
            workspace: workspace.into(),
            ..press(Answer::Approve, id, user)
        };
        for (workspace, user) in [("w1", "U9"), ("w1", OWNER), ("w2", OWNER), ("w2", "U2")] {
            let answered = approvals.answer(&press_in(workspace, user)).await.unwrap();
            assert_eq!(answered, None, "{workspace} {user}");
        }
        let told = approvals.answer(&press_in("w2", "U9")).await.unwrap();
        assert!(matches!(told.unwrap().source, Source::Approval { user, .. } if user == "U9"));
    }

    #[tokio::test]
    async fn a_call_not_made_in_a_conversation_with_the_owner_is_asked_in_the_dm() {
        let others = [
            (
                Sender::System,
                Source::Schedule {
                    name: "n".into(),
                    at: "1".into(),
                    routine: String::new(),
                },
            ),
            (
                Sender::System,
                Source::Lookout {
                    item: 1,
                    source: "s".into(),
                },
            ),
            (
                Sender::Owner,
                Source::Console {
                    message: 1,
                    workgroup: "all".into(),
                    view: "v".into(),
                    selection: None,
                    attached: Vec::new(),
                    reply_to: None,
                },
            ),
            (Sender::Other, said_in("w1", "C1", Some("5.0"))),
        ];
        for (sender, source) in others {
            let slack = FakeSlack::default();
            let (approvals, permissions) = setup(&slack);
            turn(&approvals, sender, source);
            request(&approvals, &permissions).await;
            assert_eq!(slack.posts()[0].0, "DM-U1");
            assert_eq!(slack.posts()[0].1.thread_ts, None);
            assert!(slack.ephemerals().is_empty());
        }
    }

    #[tokio::test]
    async fn a_call_made_after_an_answer_is_asked_where_the_first_was() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        turn(&approvals, Sender::Owner, said_in("w1", "C1", Some("5.0")));
        let id = request(&approvals, &permissions).await;
        turn(
            &approvals,
            Sender::Owner,
            Source::Approval {
                id,
                user: OWNER.into(),
            },
        );
        request(&approvals, &permissions).await;
        assert_eq!(slack.ephemerals().len(), 2);
        assert!(slack.posts().is_empty());
    }

    #[tokio::test]
    async fn a_request_is_asked_where_its_call_was_made_though_another_turn_runs_by_then() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        turn(&approvals, Sender::Owner, said_in("w1", "C1", Some("5.0")));
        permissions.decide(TOOL, &input()).await.unwrap();
        turn(
            &approvals,
            Sender::System,
            Source::Wake {
                id: 1,
                at: "1".into(),
            },
        );
        approvals.present_held().await.unwrap();
        assert_eq!(slack.ephemerals()[0].0, "C1");
    }

    #[tokio::test]
    async fn a_call_that_may_show_a_secret_is_asked_in_the_dm() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        turn(&approvals, Sender::Owner, said_in("w1", "C1", Some("5.0")));
        let input = json!({ "to": "a", "body": "password: hunter2" });
        permissions.decide(TOOL, &input).await.unwrap();
        approvals.present_held().await.unwrap();
        assert!(slack.ephemerals().is_empty());
        assert_eq!(slack.posts()[0].0, "DM-U1");
    }

    #[tokio::test]
    async fn what_slack_refuses_to_show_in_the_thread_is_asked_in_the_dm() {
        let slack = FakeSlack {
            no_ephemeral: true,
            ..Default::default()
        };
        let (approvals, permissions) = setup(&slack);
        turn(&approvals, Sender::Owner, said_in("w2", "C1", None));
        let id = request(&approvals, &permissions).await;
        assert_eq!(slack.posts()[0].0, "DM-U1");
        approvals
            .answer(&press(Answer::Reject, id, OWNER))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(slack.updates().len(), 1);
    }

    #[tokio::test]
    async fn approving_lets_exactly_that_call_through_once_and_tells_the_session() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        let id = request(&approvals, &permissions).await;
        let outcome = approvals
            .answer(&press(Answer::Approve, id, OWNER))
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(outcome.source, Source::Approval { id: got, .. } if got == id));
        assert!(outcome.text.contains("approved"));
        assert_eq!(
            permissions.decide(TOOL, &input()).await.unwrap(),
            Decision::Allow
        );
        assert!(matches!(
            permissions.decide(TOOL, &input()).await.unwrap(),
            Decision::Held(_)
        ));
    }

    #[tokio::test]
    async fn the_owners_word_on_what_someone_else_asks_is_carried_out_once() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        let asked = |ts: &str| {
            let mut source = said_in("w1", "C1", Some(ts));
            if let Source::Slack {
                user, ts: said_at, ..
            } = &mut source
            {
                *user = "U9".into();
                *said_at = format!("{ts}1");
            }
            turn(&approvals, Sender::Other, source);
            let input = approvals.store.running().unwrap().unwrap().input;
            let asker = asked::Asker {
                name: "Bさん".into(),
                place: "#general".into(),
                said: "給与は？".into(),
            };
            asked::Ask::new(input, &asker, "Aさんの給与")
                .put(&permissions)
                .unwrap();
            permissions.gateway().held_tickets().unwrap()[0].id
        };
        let id = asked("5.0");
        approvals.present_held().await.unwrap();
        assert_eq!(slack.posts()[0].0, "DM-U1", "never where others are");
        let shown = format!("{:?}", slack.posts()[0].1.blocks);
        assert!(shown.contains("教えてよい") && shown.contains("Aさんの給与"));

        let told = approvals
            .fill_by_id(id, vec!["教えてよい".into()])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(told.reach(), Reach::Answering);
        let again = approvals.fill_by_id(id, vec!["教えてよい".into()]).await;
        assert_eq!(again.unwrap(), None);
        assert_eq!(slack.updates().len(), 1);

        let id = asked("6.0");
        approvals.present_held().await.unwrap();
        let left = approvals.answer_by_id(id, Answer::Reject).await.unwrap();
        assert_eq!(left, None);
    }

    #[tokio::test]
    async fn an_approved_answer_to_someone_else_is_said_without_the_resident_session() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        turn(&approvals, Sender::Other, said_in("w1", "C1", Some("5.0")));
        let send = json!({ "workspace": "w1", "channel": "C1", "text": "こんにちは" });
        let held = permissions.decide("mcp__famulen__send_slack", &send).await;
        let Decision::Held(id) = held.unwrap() else {
            panic!("it is held");
        };
        approvals.present_held().await.unwrap();
        let told = approvals
            .answer_by_id(id, Answer::Approve)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(told.reach(), Reach::Guest);
        assert!(told.text.ends_with("こんにちは"));
        assert_eq!(told.place().unwrap().thread(), "5.0");
    }

    #[tokio::test]
    async fn rejecting_grants_nothing_and_tells_the_session() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        let id = request(&approvals, &permissions).await;
        let outcome = approvals
            .answer(&press(Answer::Reject, id, OWNER))
            .await
            .unwrap()
            .unwrap();
        assert!(outcome.text.contains("rejected"));
        assert!(matches!(
            permissions.decide(TOOL, &input()).await.unwrap(),
            Decision::Held(_)
        ));
    }

    #[tokio::test]
    async fn always_allow_raises_the_tool_to_allow() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        let id = request(&approvals, &permissions).await;
        approvals
            .answer(&press(Answer::AlwaysAllow, id, OWNER))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(permissions.permission(TOOL).unwrap(), Permission::Allow);
        assert_eq!(
            permissions
                .decide(TOOL, &json!({ "other": 1 }))
                .await
                .unwrap(),
            Decision::Allow
        );
    }

    #[tokio::test]
    async fn a_call_allowing_would_not_stop_says_why_and_offers_no_from_now_on() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        let first = request(&approvals, &permissions).await;
        approvals
            .answer(&press(Answer::AlwaysAllow, first, OWNER))
            .await
            .unwrap();

        let outside = Bound {
            ask: permissions.ask_in("mail", "work").unwrap(),
            waive: false,
        };
        let Decision::Held(id) = permissions
            .decide_bound(Caller::Secretary, TOOL, &json!({ "other": 1 }), outside)
            .await
            .unwrap()
        else {
            panic!("an allowed tool is still held outside the workgroup");
        };
        approvals.present_held().await.unwrap();

        let blocks = slack.posts().last().unwrap().1.blocks.clone().unwrap();
        let blocks = blocks.as_array().unwrap();
        let buttons: Vec<&str> = blocks
            .iter()
            .filter(|b| b["type"] == "actions")
            .flat_map(|b| b["elements"].as_array().unwrap())
            .map(|b| b["text"]["text"].as_str().unwrap())
            .collect();
        assert_eq!(buttons, ["承認", "却下"]);
        let ticket = permissions.gateway().ticket(id).unwrap().unwrap();
        let why = still_asked(&ticket).unwrap();
        assert!(why.contains("ワークグループの外"));
        assert!(
            blocks
                .iter()
                .any(|b| b["elements"][0]["text"].as_str() == Some(why))
        );
    }

    #[tokio::test]
    async fn an_old_from_now_on_press_leaves_the_permission_of_such_a_call() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        let before = permissions.permission(TOOL).unwrap();
        let lookout = Bound {
            ask: Some(crate::claude::session::OUTWARD.to_owned()),
            waive: false,
        };
        let Decision::Held(id) = permissions
            .decide_bound(Caller::Secretary, TOOL, &input(), lookout)
            .await
            .unwrap()
        else {
            panic!("held");
        };
        approvals.present_held().await.unwrap();

        approvals
            .answer(&press(Answer::AlwaysAllow, id, OWNER))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(permissions.permission(TOOL).unwrap(), before);
        let ticket = permissions.gateway().ticket(id).unwrap().unwrap();
        assert_eq!(ticket.status, "approved");
    }

    #[test]
    fn a_ticket_that_kept_its_ask_only_in_its_reason_is_still_known_to_be_asked() {
        let mut ticket = Ticket {
            id: 1,
            actor: "secretary".into(),
            connection: String::new(),
            tool: TOOL.into(),
            shown: json!({}),
            reason: format!("{OUTWARD}\nit sends"),
            ask: None,
            origin: None,
            status: "held".into(),
            created_at: 0,
        };
        assert!(still_asked(&ticket).is_some());

        ticket.reason = "it sends".into();
        assert!(still_asked(&ticket).is_none());
    }

    #[tokio::test]
    async fn the_answered_message_loses_its_buttons() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        let id = request(&approvals, &permissions).await;
        approvals
            .answer(&press(Answer::Approve, id, OWNER))
            .await
            .unwrap();
        let updates = slack.updates();
        assert_eq!(updates.len(), 1);
        assert_eq!(
            (updates[0].0.as_str(), updates[0].1.as_str()),
            ("DM-U1", "9.1")
        );
        let blocks = updates[0].2.blocks.clone().unwrap();
        assert!(
            blocks
                .as_array()
                .unwrap()
                .iter()
                .all(|b| b["type"] != "actions")
        );
    }

    #[tokio::test]
    async fn an_answered_request_cannot_be_answered_again() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        let id = request(&approvals, &permissions).await;
        approvals
            .answer(&press(Answer::Reject, id, OWNER))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            approvals
                .answer(&press(Answer::Approve, id, OWNER))
                .await
                .unwrap(),
            None
        );
        assert!(matches!(
            permissions.decide(TOOL, &input()).await.unwrap(),
            Decision::Held(_)
        ));
    }

    #[tokio::test]
    async fn a_press_in_another_workspace_does_nothing() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        let id = request(&approvals, &permissions).await;
        let mut action = press(Answer::Approve, id, OWNER);
        action.workspace = "w2".into();
        assert_eq!(approvals.answer(&action).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_press_by_someone_else_does_nothing() {
        let slack = FakeSlack::default();
        let (approvals, permissions) = setup(&slack);
        let id = request(&approvals, &permissions).await;
        assert_eq!(
            approvals
                .answer(&press(Answer::Approve, id, "U2"))
                .await
                .unwrap(),
            None
        );
        assert!(matches!(
            permissions.decide(TOOL, &input()).await.unwrap(),
            Decision::Held(_)
        ));
        assert!(slack.updates().is_empty());
    }

    #[tokio::test]
    async fn a_long_input_is_cut_on_a_character_boundary() {
        let long = json!({ "body": "あ".repeat(4000) });
        let shown = preview(&said(&long));
        assert!(shown.chars().count() <= INPUT_PREVIEW_CHARS + 2);
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn what_is_decided_once_has_no_button_for_from_now_on_and_is_not_approved_unread() {
        let asked = Asked {
            deed: "使う".into(),
            place: "どこか".into(),
        };
        let buttons = |input: &Value, once: bool| -> Vec<String> {
            let blocks = request_message(1, &asked, input, None, None, once, None)
                .blocks
                .unwrap();
            blocks
                .as_array()
                .unwrap()
                .iter()
                .filter(|b| b["type"] == "actions")
                .flat_map(|b| b["elements"].as_array().unwrap().clone())
                .map(|e| e["text"]["text"].as_str().unwrap().to_owned())
                .collect()
        };
        let short = json!({ "a": 1 });
        let long = json!({ "a": "x".repeat(INPUT_PREVIEW_CHARS) });
        assert_eq!(buttons(&short, false), ["承認", "却下", "今後は常に許可"]);
        assert_eq!(buttons(&short, true), ["承認", "却下"]);
        assert_eq!(buttons(&long, true), ["却下"]);
        assert_eq!(buttons(&long, false).len(), 3);
    }
}
