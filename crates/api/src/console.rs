//! The conversation on the console and what waits for the owner.

use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream};
use famulen_core::approvals::{self, Answer};
use famulen_core::connections::Connection;
use famulen_core::console_log::{self, Event, Who};
use famulen_core::input::{Input, Sender, Source};
use famulen_core::store::StoredInput;
use famulen_core::wording;
use famulen_core::workgroups::ALL;
use futures::StreamExt;
use serde_json::json;

use crate::paging::{page, page_limit};
use crate::pb::{self, *};
use crate::{Api, OrFail, fail, given};

fn meta_text<'a>(m: &'a console_log::Message, key: &str) -> Option<&'a str> {
    m.meta.get(key).and_then(serde_json::Value::as_str)
}

fn message(m: &console_log::Message) -> ConsoleMessage {
    let meta = |key| meta_text(m, key).unwrap_or_default().to_owned();
    ConsoleMessage {
        id: m.id,
        at: m.at,
        who: match m.who {
            Who::Owner => pb::Speaker::SPEAKER_OWNER,
            Who::Aide => pb::Speaker::SPEAKER_AIDE,
        }
        .into(),
        text: m.text.clone(),
        workgroup: meta("workgroup"),
        origin: meta("origin"),
        about: meta("about"),
        open: meta("open"),
        ..Default::default()
    }
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
        text: stored.input.text,
        status: stored.status.as_str().into(),
        ..Default::default()
    }
}

/// Whether a message shows while `workgroup` is chosen: one said in another
/// workgroup is left out, one said in すべて is kept.
fn shown_in(m: &console_log::Message, workgroup: &str) -> bool {
    workgroup.is_empty()
        || workgroup == ALL
        || meta_text(m, "workgroup").is_none_or(|c| c == workgroup || c == ALL)
}

impl Api {
    /// When an input arrived, in Unix seconds.
    fn arrived_at(&self, input: i64) -> anyhow::Result<i64> {
        let newest = self.app.store.recent_inputs(input + 1, 1)?;
        Ok(newest.first().map(|(_, at)| *at).unwrap_or_default())
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
                    text: h.input.text,
                    at: self.arrived_at(h.id)?,
                    ..Default::default()
                })
            })
            .collect::<anyhow::Result<_>>()?;
        let approvals = approvals::recent(&self.app.store, 30)?
            .into_iter()
            .filter(|r| r.status == "pending")
            .rev()
            .map(|r| {
                let shown: serde_json::Value = serde_json::from_str(&r.input).unwrap_or_default();
                let connection = self.app.permissions.connection_of(&r.tool);
                let decided = shown.get("decides").unwrap_or(&shown);
                Approval {
                    deed: wording::deed(&r.tool, connection.as_ref()),
                    place: place(&r.tool, connection.as_ref()),
                    given: wording::given(decided)
                        .unwrap_or_default()
                        .into_iter()
                        .map(approval_given)
                        .collect(),
                    once: approvals::once(&self.app.permissions, &r.tool),
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
                    summary: approvals::summary(&shown).unwrap_or_default().to_owned(),
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
            })
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
                .map(|i| i.input.text.chars().take(60).collect())
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
        let limit = if r.limit > 0 { r.limit as usize } else { 120 };
        let mut all: Vec<console_log::Message> = self
            .app
            .console
            .recent(1000)
            .or_fail()?
            .into_iter()
            .filter(|m| shown_in(m, &r.workgroup))
            .filter(|m| r.before == 0 || m.id < r.before)
            .collect();
        all.sort_by_key(|m| m.id);
        let start = all.len().saturating_sub(limit);
        Response::ok(ListMessagesResponse {
            messages: all[start..].iter().map(message).collect(),
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
        if text.is_empty() {
            return Err(fail("nothing to send"));
        }
        let workgroup = self.app.workgroups.resolve(Some(&r.workgroup)).or_fail()?;
        let selection = given(&r.selection).map(str::to_owned);
        let about = [
            (!r.view.is_empty()).then(|| format!("{} · {}", workgroup.name, r.view)),
            selection.clone().map(|s| format!("「{s}」について")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let m = self
            .app
            .console
            .add(
                Who::Owner,
                text,
                json!({ "origin": "コンソール", "about": about, "workgroup": workgroup.id }),
            )
            .or_fail()?;
        let input = Input {
            sender: Sender::Owner,
            source: Source::Console {
                message: m.id,
                workgroup: workgroup.id.clone(),
                view: r.view.clone(),
                selection,
            },
            text: text.to_owned(),
        };
        self.app.store.enqueue(&input).or_fail()?;
        self.app.inputs.notify_one();
        Response::ok(message(&m))
    }

    async fn post_message(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, PostMessageRequest>,
    ) -> ServiceResult<ConsoleMessage> {
        let r = request.to_owned_message();
        let mut meta = json!({ "workgroup": self.app.workgroups.working().or_fail()? });
        if !r.open.is_empty() {
            meta["open"] = json!(r.open);
        }
        let m = self.app.console.add(Who::Aide, &r.text, meta).or_fail()?;
        Response::ok(message(&m))
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
        let answer = match r.answer.as_known() {
            Some(pb::ApprovalAnswer::APPROVAL_ANSWER_ALLOW) => Answer::from_name("approve"),
            Some(pb::ApprovalAnswer::APPROVAL_ANSWER_DENY) => Answer::from_name("reject"),
            Some(pb::ApprovalAnswer::APPROVAL_ANSWER_ALWAYS) => Answer::from_name("always"),
            _ => None,
        }
        .ok_or_else(|| fail("say allow, deny or always"))?;
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
        let stream = as_stream(receiver).filter_map(move |event| {
            let workgroup = workgroup.clone();
            async move {
                let event = match event {
                    Event::Message(m) if shown_in(&m, &workgroup) => {
                        E::Message(Box::new(message(&m)))
                    }
                    Event::Message(_) => return None,
                    Event::Approvals => E::Inbox(Box::default()),
                    Event::Records => E::Records(Box::default()),
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

/// The broadcast as a stream. A receiver is told to read everything again when
/// it begins, as what it read before may have changed since, and when it fell
/// behind.
fn as_stream(
    receiver: tokio::sync::broadcast::Receiver<Event>,
) -> impl futures::Stream<Item = Event> + Send + 'static {
    let told = futures::stream::unfold(receiver, |mut receiver| async move {
        match receiver.recv().await {
            Ok(event) => Some((event, receiver)),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                Some((Event::Records, receiver))
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    });
    futures::stream::iter([Event::Records]).chain(told)
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
        assert_eq!(events.next().now_or_never(), None);
        tx.send(Event::Approvals).unwrap();
        assert_eq!(events.next().now_or_never(), Some(Some(Event::Approvals)));
    }

    #[tokio::test]
    async fn who_fell_behind_is_told_to_read_again() {
        let (tx, rx) = broadcast::channel(1);
        let mut events = Box::pin(as_stream(rx));
        assert_eq!(events.next().now_or_never(), Some(Some(Event::Records)));
        for _ in 0..3 {
            tx.send(Event::Approvals).unwrap();
        }
        assert_eq!(events.next().now_or_never(), Some(Some(Event::Records)));
        assert_eq!(events.next().now_or_never(), Some(Some(Event::Approvals)));
    }
}
