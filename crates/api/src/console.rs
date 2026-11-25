//! The conversation on the console and what waits for the owner.

use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use famulen_core::approvals::{self, Answer};
use famulen_core::attachments::{Attached, MAX_FILES, TOO_MANY};
use famulen_core::connections::Connection;
use famulen_core::console_log::{self, Event, Who};
use famulen_core::conversations::{self, message_key};
use famulen_core::input::{Input, ReplyTo, Sender, Source};
use famulen_core::store::StoredInput;
use famulen_core::wording;
use famulen_core::working;
use futures::StreamExt;
use serde_json::json;

use crate::paging::{page, page_limit};
use crate::pb::{self, *};
use crate::{Api, OrFail, fail, given};

fn meta_text<'a>(m: &'a console_log::Message, key: &str) -> Option<&'a str> {
    m.meta.get(key).and_then(serde_json::Value::as_str)
}

pub(crate) fn attachment(attached: Attached) -> Attachment {
    match attached {
        Attached::Kept(a) => Attachment {
            id: a.id,
            name: a.name,
            mime: a.mime,
            size: a.size as i64,
            ..Default::default()
        },
        Attached::OnSlack { name, .. } => Attachment {
            name,
            ..Default::default()
        },
        Attached::Missing { name, why } => Attachment {
            name,
            problem: why,
            ..Default::default()
        },
    }
}

fn speaker(who: Who) -> pb::Speaker {
    match who {
        Who::Owner => pb::Speaker::SPEAKER_OWNER,
        Who::Aide => pb::Speaker::SPEAKER_AIDE,
    }
}

/// How much of a message is quoted over a reply to it.
const QUOTED: usize = 200;

/// A message as it is shown while `workgroup` is chosen: nothing of those not
/// shown there is told with it.
fn message(
    log: &console_log::ConsoleLog,
    conversations: &conversations::Conversations,
    m: &console_log::Message,
    workgroup: &str,
) -> anyhow::Result<ConsoleMessage> {
    let meta = |key| meta_text(m, key).unwrap_or_default().to_owned();
    let reply_to = match m.reply_to() {
        Some(id) => log.get(id)?.filter(|to| to.shown_in(workgroup)),
        None => None,
    };
    Ok(ConsoleMessage {
        id: m.id,
        at: m.at,
        who: speaker(m.who).into(),
        reply_to: reply_to
            .map(|to| pb::RepliedTo {
                id: to.id,
                who: speaker(to.who).into(),
                text: to.excerpt(QUOTED),
                ..Default::default()
            })
            .into(),
        thread: m.thread().unwrap_or_default(),
        conversation: conversations.of_message(m)?.unwrap_or_default(),
        text: m.text.clone(),
        workgroup: meta("workgroup"),
        origin: meta("origin"),
        about: meta("about"),
        open: meta("open"),
        attachments: m.attached().into_iter().map(attachment).collect(),
        ..Default::default()
    })
}

/// Where a held call is made, with the account, which only the console tells.
fn place(tool: &str, connection: Option<&Connection>) -> String {
    let service = wording::place(tool, connection);
    match connection.map(|c| c.account.as_str()) {
        Some(account) if !account.is_empty() && account != service => {
            format!("{service} · {account}")
        }
        _ => service,
    }
}

fn approval_given(given: wording::Given) -> ApprovalGiven {
    ApprovalGiven {
        name: given.name,
        text: given.text,
        code: given.code,
        rows: given.rows.into_iter().map(approval_given).collect(),
        ..Default::default()
    }
}

/// The owner's answer, as the console names it.
pub(crate) fn answer_of(
    answer: &buffa::EnumValue<pb::ApprovalAnswer>,
) -> Result<Answer, connectrpc::ConnectError> {
    match answer.as_known() {
        Some(pb::ApprovalAnswer::APPROVAL_ANSWER_ALLOW) => Answer::from_name("approve"),
        Some(pb::ApprovalAnswer::APPROVAL_ANSWER_DENY) => Answer::from_name("reject"),
        Some(pb::ApprovalAnswer::APPROVAL_ANSWER_ALWAYS) => Answer::from_name("always"),
        _ => None,
    }
    .ok_or_else(|| fail("say allow, deny or always"))
}

pub(crate) fn spent(of: conversations::Spent) -> pb::ConversationSpent {
    pb::ConversationSpent {
        turns: of.turns,
        input_tokens: of.input_tokens,
        cached_tokens: of.cached_tokens,
        cache_write_tokens: of.cache_write_tokens,
        output_tokens: of.output_tokens,
        ..Default::default()
    }
}

/// How much of what a conversation began with names it.
const TITLE: usize = 60;

/// The first line of `text`, as far as `limit` characters of it.
fn excerpt(text: &str, limit: usize) -> String {
    let line = text.trim().lines().next().unwrap_or_default();
    match line.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_owned(),
    }
}

/// Where an input came from, as a kind and the one thing that tells it apart.
fn source_of(source: &Source) -> (&'static str, String) {
    match source {
        Source::Slack { channel, .. } => ("slack", channel.clone()),
        Source::Approval { id, .. } => ("approval", format!("#{id}")),
        Source::Schedule { name, .. } => ("schedule", name.clone()),
        Source::Wake { .. } => ("wake", String::new()),
        Source::Console { view, .. } => ("console", view.clone()),
        Source::Arrival { connection, .. } => ("arrival", connection.clone()),
        Source::Lookout { source, .. } => ("lookout", source.clone()),
        Source::Asked { request, .. } => ("approval", format!("#{request}")),
        Source::Worker { task, .. } => ("worker", task.clone()),
    }
}

fn input_item(stored: StoredInput, at: i64) -> InputItem {
    let (source, detail) = source_of(&stored.input.source);
    InputItem {
        id: stored.id,
        at,
        source: source.into(),
        detail,
        sender: stored.input.sender.as_str().into(),
        text: stored.input.gist(),
        status: stored.status.as_str().into(),
        ..Default::default()
    }
}

/// The most messages one answer holds, whatever is asked.
const MOST_MESSAGES: usize = 1000;

impl From<working::Shown> for pb::Working {
    fn from(shown: working::Shown) -> Self {
        Self {
            text: shown.text,
            seconds: shown.seconds as i32,
            ..Default::default()
        }
    }
}

impl Api {
    /// When an input arrived, in Unix seconds.
    fn arrived_at(&self, input: i64) -> anyhow::Result<i64> {
        let newest = self.app.store.recent_inputs(input + 1, 1)?;
        Ok(newest.first().map(|(_, at)| *at).unwrap_or_default())
    }

    fn messages(
        &self,
        found: &[console_log::Message],
        workgroup: &str,
    ) -> anyhow::Result<Vec<ConsoleMessage>> {
        found.iter().map(|m| self.message(m, workgroup)).collect()
    }

    fn message(&self, m: &console_log::Message, workgroup: &str) -> anyhow::Result<ConsoleMessage> {
        message(&self.app.console, &self.app.conversations, m, workgroup)
    }

    /// The thread the message `id` is in, as far as `workgroup` shows it.
    fn thread(&self, id: i64, workgroup: &str) -> anyhow::Result<Vec<console_log::Message>> {
        let log = &self.app.console;
        let root = log.get(id)?.and_then(|m| m.thread()).unwrap_or(id);
        let mut found: Vec<_> = log.get(root)?.into_iter().collect();
        found.extend(log.replies(root)?);
        found.retain(|m| m.shown_in(workgroup));
        Ok(found)
    }

    /// The message a caller says it replies to; `0` is none. One not shown in
    /// `workgroup` is not replied to from there, as its words go along.
    fn replied_to(
        &self,
        id: i64,
        workgroup: &str,
    ) -> Result<Option<console_log::Message>, connectrpc::ConnectError> {
        if id == 0 {
            return Ok(None);
        }
        match self.app.console.get(id).or_fail()? {
            None => Err(fail("返信先の発言が見つからない")),
            Some(m) if !m.shown_in(workgroup) => {
                Err(fail("返信先の発言は、ほかのワークグループのもの"))
            }
            found => Ok(found),
        }
    }

    /// The message what is said now belongs in the thread of: the owner's that
    /// is being answered, when it is itself a reply, and the one the owner
    /// asked with, when something comes back to it later.
    fn being_answered(&self) -> anyhow::Result<Option<console_log::Message>> {
        let Some(running) = self.app.store.running_input()? else {
            return Ok(None);
        };
        let message = match &running.input.source {
            Source::Console {
                message,
                reply_to: Some(_),
                ..
            } => Some(*message),
            Source::Approval { .. } | Source::Wake { .. } | Source::Worker { .. } => {
                let asked = self
                    .app
                    .conversations
                    .asked_with(&running, |request| self.app.request_made_in(request))?;
                match asked.map(|input| input.source) {
                    Some(Source::Console { message, .. }) => Some(message),
                    _ => None,
                }
            }
            _ => None,
        };
        message.map_or(Ok(None), |id| self.app.console.get(id))
    }

    pub(crate) fn conversation(
        &self,
        c: conversations::Conversation,
        workgroup: &str,
        used: conversations::Spent,
    ) -> anyhow::Result<pb::Conversation> {
        let all = &self.app.conversations;
        let first = all.first_input(c.id)?;
        let (source, detail) = first
            .as_ref()
            .map(|row| source_of(&row.input.source))
            .unwrap_or_default();
        let title = first
            .map(|row| excerpt(&row.input.gist(), TITLE))
            .unwrap_or_default();
        Ok(pb::Conversation {
            running: all.running()? == Some(c.id),
            id: c.id,
            title,
            source: source.into(),
            detail,
            workgroup: c.workgroup,
            opened_at: c.opened_at,
            used_at: c.used_at,
            context_tokens: c.context,
            spent: spent(used).into(),
            waiting: all.waiting(c.id)?,
            last: match all
                .messages(&self.app.console, c.id)?
                .iter()
                .rfind(|m| m.shown_in(workgroup))
            {
                Some(m) => Some(self.message(m, workgroup)?),
                None => None,
            }
            .into(),
            ..Default::default()
        })
    }

    fn say(
        &self,
        who: Who,
        text: &str,
        meta: serde_json::Value,
        to: Option<&console_log::Message>,
    ) -> anyhow::Result<console_log::Message> {
        match to {
            Some(to) => self.app.console.reply(who, text, meta, to),
            None => self.app.console.add(who, text, meta),
        }
    }

    /// A request in the owner's words.
    pub(crate) fn approval(&self, r: approvals::Request) -> Approval {
        let shown: serde_json::Value = serde_json::from_str(&r.input).unwrap_or_default();
        let connection = self.app.permissions.connection_of(&r.tool);
        let decided = shown.get("decides").unwrap_or(&shown);
        let ticket = self.app.gateway.ticket(r.id).ok().flatten();
        Approval {
            deed: wording::deed(&r.tool, connection.as_ref()),
            place: place(&r.tool, connection.as_ref()),
            given: wording::given(decided)
                .unwrap_or_default()
                .into_iter()
                .map(approval_given)
                .collect(),
            once: approvals::once(&self.app.permissions, &r.tool)
                || ticket.as_ref().is_some_and(approvals::discloses),
            questions: approvals::questions(&self.app.permissions, &r.tool, &shown)
                .into_iter()
                .map(|q| pb::ApprovalQuestion {
                    question: q.question,
                    several: q.several,
                    choices: q
                        .options
                        .into_iter()
                        .map(|c| pb::ApprovalChoice {
                            label: c.label,
                            description: c.description,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                })
                .collect(),
            summary: ticket
                .as_ref()
                .and_then(approvals::summary)
                .unwrap_or_default()
                .to_owned(),
            // What is decided stands apart from the secretary's words.
            input: shown
                .get("decides")
                .map_or(r.input, serde_json::Value::to_string),
            id: r.id,
            tool: r.tool,
            status: r.status,
            created_at: r.created_at,
            ..Default::default()
        }
    }

    fn inbox(&self) -> anyhow::Result<Inbox> {
        let held = self
            .app
            .store
            .held()?
            .into_iter()
            .map(|h| {
                Ok(HeldInput {
                    id: h.id,
                    text: h.input.gist(),
                    at: self.arrived_at(h.id)?,
                    ..Default::default()
                })
            })
            .collect::<anyhow::Result<_>>()?;
        let approvals = approvals::recent(&self.app.store, 30)?
            .into_iter()
            .filter(|r| r.status == "pending")
            .rev()
            .map(|r| self.approval(r))
            .collect();
        let accounts = self
            .app
            .records
            .accounts()?
            .into_iter()
            .flat_map(|(p, accounts)| {
                let (party, name) = (p.id.clone(), p.title());
                accounts
                    .into_iter()
                    .filter(|a| !a.confirmed)
                    .map(move |a| AccountQuestion {
                        account: a.id,
                        party: party.clone(),
                        party_name: name.clone(),
                        service: a.service,
                        handle: a.handle,
                        how: a.how,
                        ..Default::default()
                    })
            })
            .collect();
        Ok(Inbox {
            approvals,
            held,
            accounts,
            ..Default::default()
        })
    }

    fn status(&self) -> anyhow::Result<Status> {
        let running = self.app.store.running()?;
        let pending = self.app.store.count_pending()?;
        let doing = match &running {
            Some(turn) => self
                .app
                .store
                .get(turn.input)?
                .map(|i| i.input.gist().chars().take(60).collect())
                .unwrap_or_default(),
            None => String::new(),
        };
        Ok(Status {
            state: if running.is_some() {
                "running"
            } else {
                "waiting"
            }
            .into(),
            pending_inputs: pending as i32,
            doing,
            ..Default::default()
        })
    }
}

impl ConsoleService for Api {
    async fn list_messages(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListMessagesRequest>,
    ) -> ServiceResult<ListMessagesResponse> {
        let r = request.to_owned_message();
        if r.thread != 0 {
            let mut found = self.thread(r.thread, &r.workgroup).or_fail()?;
            // What it begins with stays, whatever is left out after it.
            let older = found.len().saturating_sub(MOST_MESSAGES);
            if older > 0 {
                found.drain(1..1 + older);
            }
            return Response::ok(ListMessagesResponse {
                messages: self.messages(&found, &r.workgroup).or_fail()?,
                more: older > 0,
                ..Default::default()
            });
        }
        if r.conversation != 0 {
            let mut found = self
                .app
                .conversations
                .messages(&self.app.console, r.conversation)
                .or_fail()?;
            found.retain(|m| m.shown_in(&r.workgroup));
            let older = found.len().saturating_sub(MOST_MESSAGES);
            found.drain(..older);
            return Response::ok(ListMessagesResponse {
                messages: self.messages(&found, &r.workgroup).or_fail()?,
                more: older > 0,
                ..Default::default()
            });
        }
        let limit = match r.limit {
            n if n > 0 => (n as usize).min(MOST_MESSAGES),
            _ => 120,
        };
        // One past the limit tells whether more remain.
        let mut found = self
            .app
            .console
            .before(r.before, limit + 1, |m| m.shown_in(&r.workgroup))
            .or_fail()?;
        let more = found.len() > limit;
        if more {
            found.remove(0);
        }
        Response::ok(ListMessagesResponse {
            messages: self.messages(&found, &r.workgroup).or_fail()?,
            more,
            ..Default::default()
        })
    }

    async fn send(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SendRequest>,
    ) -> ServiceResult<ConsoleMessage> {
        let r = request.to_owned_message();
        let text = r.text.trim();
        if text.is_empty() && r.attachments.is_empty() {
            return Err(fail("nothing to send"));
        }
        if r.attachments.len() > MAX_FILES {
            return Err(fail(TOO_MANY));
        }
        let attached = r
            .attachments
            .iter()
            .map(|id| {
                let found = self.app.attachments.get(id)?;
                found
                    .map(Attached::Kept)
                    .ok_or_else(|| anyhow::anyhow!("添えたファイルが見つからない。添え直す"))
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .or_fail()?;
        let workgroup = self.app.workgroups.resolve(Some(&r.workgroup)).or_fail()?;
        let to = self.replied_to(r.reply_to, &workgroup.id)?;
        if r.conversation != 0
            && self
                .app
                .conversations
                .get(r.conversation)
                .or_fail()?
                .is_none_or(|c| !c.shown_in(&workgroup.id))
        {
            return Err(fail("続ける会話が見つからない"));
        }
        let selection = given(&r.selection).map(str::to_owned);
        let about = [
            (!r.view.is_empty()).then(|| format!("{} · {}", workgroup.name, r.view)),
            selection.clone().map(|s| format!("「{s}」について")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let meta = json!({ "origin": "コンソール", "about": about, "workgroup": workgroup.id, "attached": attached });
        let m = self.say(Who::Owner, text, meta, to.as_ref()).or_fail()?;
        let input = Input {
            sender: Sender::Owner,
            source: Source::Console {
                message: m.id,
                workgroup: workgroup.id.clone(),
                view: r.view.clone(),
                selection,
                attached,
                reply_to: to.as_ref().map(ReplyTo::of),
            },
            text: text.to_owned(),
        };
        // Before it is queued, as its turn may begin at once.
        let conversations = &self.app.conversations;
        if to.is_none() {
            let chosen = match r.conversation {
                0 if r.new_conversation => Some(conversations.begin_anew(&input).or_fail()?),
                0 => None,
                id => Some(id),
            };
            if let Some(id) = chosen {
                conversations.choose(id, &input, m.id).or_fail()?;
            }
        }
        if let Some(id) = self.app.store.enqueue(&input).or_fail()?
            && let Some(row) = self.app.store.get(id).or_fail()?
            && let Err(error) = conversations.settle(&row, |_| None)
        {
            // It is queued; failing here would have it said twice.
            tracing::warn!(%error, "could not settle which conversation it is of; its turn will");
        }
        self.app.inputs.notify_one();
        self.app.working.tell();
        Response::ok(self.message(&m, &workgroup.id).or_fail()?)
    }

    async fn list_conversations(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListConversationsRequest>,
    ) -> ServiceResult<ListConversationsResponse> {
        let r = request.to_owned_message();
        let limit = page_limit(r.limit, 30);
        let all = &self.app.conversations;
        let before = match r.before {
            0 => None,
            id => Some(
                all.get(id)
                    .or_fail()?
                    .ok_or_else(|| fail(format!("no conversation {id}")))?,
            ),
        };
        let found = all
            .recent(&r.workgroup, before.as_ref(), limit + 1)
            .or_fail()?;
        let (found, more) = page(found, limit);
        let conversations = found
            .into_iter()
            .map(|c| {
                let spent = all.spent_by(c.id)?;
                self.conversation(c, &r.workgroup, spent)
            })
            .collect::<anyhow::Result<_>>()
            .or_fail()?;
        Response::ok(ListConversationsResponse {
            conversations,
            more,
            ..Default::default()
        })
    }

    async fn list_link_previews(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListLinkPreviewsRequest>,
    ) -> ServiceResult<ListLinkPreviewsResponse> {
        let said = self.app.console.get(request.message).or_fail()?;
        let previews = match said {
            Some(m) => self.app.link_previews.of(&m).await.or_fail()?,
            None => Vec::new(),
        };
        Response::ok(ListLinkPreviewsResponse {
            previews: previews
                .into_iter()
                .map(crate::link_previews::link_preview)
                .collect(),
            ..Default::default()
        })
    }

    async fn post_message(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, PostMessageRequest>,
    ) -> ServiceResult<ConsoleMessage> {
        let r = request.to_owned_message();
        let workgroup = self.app.workgroups.working().or_fail()?;
        let to = match r.reply_to {
            0 => self.being_answered().or_fail()?,
            id => self.replied_to(id, &workgroup)?,
        };
        let mut meta = json!({ "workgroup": workgroup });
        if !r.open.is_empty() {
            meta["open"] = json!(r.open);
        }
        let m = self.say(Who::Aide, &r.text, meta, to.as_ref()).or_fail()?;
        let mut keys = vec![message_key(m.id)];
        keys.extend(m.thread().map(message_key));
        // It is said; failing here would have it said twice.
        if let Err(error) = self.app.conversations.link_to_running(keys) {
            tracing::warn!(%error, "could not keep which conversation it was said in");
        }
        self.app.working.answered();
        Response::ok(self.message(&m, &workgroup).or_fail()?)
    }

    async fn get_inbox(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Inbox> {
        Response::ok(self.inbox().or_fail()?)
    }

    async fn answer_approval(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, AnswerApprovalRequest>,
    ) -> ServiceResult<Empty> {
        let r = request.to_owned_message();
        let answer = answer_of(&r.answer)?;
        let answerer = self
            .app
            .answerer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| fail("approvals are not running yet"))?;
        answerer(r.id, answer, r.replies).await.or_fail()?;
        self.app.console.notify(Event::Approvals);
        Response::ok(Empty::default())
    }

    async fn answer_held(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, AnswerHeldRequest>,
    ) -> ServiceResult<Empty> {
        let (id, go_on) = (request.id, request.go_on);
        if self.app.store.answer_held(id, go_on).or_fail()? {
            self.app.journal.record(
                "input",
                format!(
                    "held input #{id}: {}",
                    if go_on { "go on" } else { "dropped" }
                ),
            );
            self.app.inputs.notify_one();
            self.app.console.notify(Event::Approvals);
        }
        Response::ok(Empty::default())
    }

    async fn get_status(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Status> {
        Response::ok(self.status().or_fail()?)
    }

    async fn list_inputs(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListInputsRequest>,
    ) -> ServiceResult<ListInputsResponse> {
        let limit = page_limit(request.limit, 15);
        let rows = self
            .app
            .store
            .recent_inputs(request.before, limit + 1)
            .or_fail()?;
        let (rows, more) = page(rows, limit);
        let inputs = rows
            .into_iter()
            .map(|(stored, at)| input_item(stored, at))
            .collect();
        Response::ok(ListInputsResponse {
            inputs,
            more,
            ..Default::default()
        })
    }

    async fn subscribe_events(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SubscribeEventsRequest>,
    ) -> ServiceResult<ServiceStream<ConsoleEvent>> {
        use pb::__buffa::oneof::console_event::Event as E;
        let workgroup = request.workgroup.to_owned();
        let receiver = self.app.console.subscribe();
        let (working, log) = (self.app.working.clone(), self.app.console.clone());
        let conversations = self.app.conversations.clone();
        let stream = as_stream(receiver).filter_map(move |event| {
            let (workgroup, working, log) = (workgroup.clone(), working.clone(), log.clone());
            let conversations = conversations.clone();
            async move {
                let event = match event {
                    Event::Message(m) if m.shown_in(&workgroup) => {
                        match message(&log, &conversations, &m, &workgroup) {
                            Ok(m) => E::Message(Box::new(m)),
                            Err(error) => {
                                tracing::warn!(%error, "could not tell of a message");
                                return None;
                            }
                        }
                    }
                    Event::Message(_) => return None,
                    Event::Approvals => E::Inbox(Box::default()),
                    Event::Records => E::Records(Box::default()),
                    Event::Working => E::Working(Box::new(match working.shown(&workgroup) {
                        Ok(shown) => shown.map(pb::Working::from).unwrap_or_default(),
                        Err(error) => {
                            tracing::warn!(%error, "could not tell what the secretary is doing");
                            return None;
                        }
                    })),
                };
                Some(Ok(ConsoleEvent {
                    event: Some(event),
                    ..Default::default()
                }))
            }
        });
        Response::stream_ok(stream)
    }
}

/// What tells a screen to read everything again.
const AGAIN: [Event; 2] = [Event::Records, Event::Working];

/// The broadcast as a stream. A receiver is told to read everything again when
/// it begins, as what it read before may have changed since, and when it fell
/// behind.
fn as_stream(
    receiver: tokio::sync::broadcast::Receiver<Event>,
) -> impl futures::Stream<Item = Event> + Send + 'static {
    let told = futures::stream::unfold(receiver, |mut receiver| async move {
        match receiver.recv().await {
            Ok(event) => Some((vec![event], receiver)),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                Some((AGAIN.to_vec(), receiver))
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    });
    futures::stream::iter(AGAIN).chain(told.flat_map(futures::stream::iter))
}

#[cfg(test)]
mod tests {
    use famulen_core::console_log::Event;
    use futures::{FutureExt, StreamExt};
    use tokio::sync::broadcast;

    use super::as_stream;

    #[tokio::test]
    async fn who_begins_to_listen_is_told_to_read_again_and_then_what_changes() {
        let (tx, rx) = broadcast::channel(4);
        let mut events = Box::pin(as_stream(rx));
        assert_eq!(events.next().now_or_never(), Some(Some(Event::Records)));
        assert_eq!(events.next().now_or_never(), Some(Some(Event::Working)));
        assert_eq!(events.next().now_or_never(), None);
        tx.send(Event::Approvals).unwrap();
        assert_eq!(events.next().now_or_never(), Some(Some(Event::Approvals)));
    }

    #[tokio::test]
    async fn who_fell_behind_is_told_to_read_again() {
        let (tx, rx) = broadcast::channel(1);
        let mut events = Box::pin(as_stream(rx));
        assert_eq!(events.next().now_or_never(), Some(Some(Event::Records)));
        assert_eq!(events.next().now_or_never(), Some(Some(Event::Working)));
        for _ in 0..3 {
            tx.send(Event::Approvals).unwrap();
        }
        assert_eq!(events.next().now_or_never(), Some(Some(Event::Records)));
        assert_eq!(events.next().now_or_never(), Some(Some(Event::Working)));
        assert_eq!(events.next().now_or_never(), Some(Some(Event::Approvals)));
    }
}
