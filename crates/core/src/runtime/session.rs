//! Keeping the secretary's session running, and feeding it every input.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::Notify;

use super::RESTART_AFTER;
use crate::app::App;
use crate::claude::guest::Guest;
use crate::claude::session::{self, Audience, Handle, InFlight, Options, Services};
use crate::console_log::Event;
use crate::conversations::Conversation;
use crate::input::{Input, Source};
use crate::slack::web::GatewaySlack;
use crate::store::{Store, StoredInput};
use crate::workgroups::{ALL, MemberKind, Workgroups};

/// How long a new session waits for the last one's tool calls to finish.
pub(super) const SETTLE_LIMIT: Duration = Duration::from_secs(120);
/// How long someone other than the owner is being answered. Their turn may
/// not keep the owner's waiting.
const GUEST_LIMIT: Duration = Duration::from_secs(2 * 60);
/// Set when the engine should be rebuilt between turns.
pub(super) const RELOAD_KEY: &str = "engine.reload";

/// What each kind of session is started with.
pub(super) struct OfSessions {
    pub(super) owners: Options,
    /// The system prompt of the owner's conversations, with memory as it
    /// stands when one is started.
    pub(super) prompt: Box<dyn Fn() -> String + Send + Sync>,
    pub(super) guests: Options,
}

/// What outlives one engine and ties its sessions together: the tool calls
/// still running, the session loop's word that the engine may be rebuilt,
/// and the Famulen they run in.
#[derive(Clone)]
pub(super) struct SessionLinks {
    pub(super) in_flight: InFlight,
    pub(super) reload_now: Arc<Notify>,
    pub(super) app: App,
}

/// The workgroup an input belongs to.
pub(super) fn workgroup_of(workgroups: &Workgroups, input: &Input) -> String {
    match &input.source {
        Source::Console { workgroup, .. } | Source::Worker { workgroup, .. } => workgroup.clone(),
        Source::Slack { workspace, .. } | Source::Asked { workspace, .. } => workgroups
            .workgroup_for_channel(format!("slack:{workspace}"))
            .unwrap_or_else(|_| ALL.to_owned()),
        Source::Lookout { source, .. } => source
            .parse::<i64>()
            .ok()
            .and_then(|_| workgroups.workgroups_of(MemberKind::Lookout, source).ok())
            .and_then(|c| (c.len() == 1).then(|| c[0].clone()))
            .unwrap_or_else(|| ALL.to_owned()),
        _ => ALL.to_owned(),
    }
}

/// The text the session receives: the header with the time, then the
/// workgroup line and what was attached, then what the session is told
/// besides, then the rest.
fn deliverable(
    workgroups: &Workgroups,
    input: &Input,
    besides: &str,
    now: impl std::fmt::Display,
) -> String {
    let workgroup = workgroups
        .resolve(Some(&workgroup_of(workgroups, input)))
        .ok();
    let line = workgroup
        .as_ref()
        .and_then(|c| workgroups.describe_for_prompt(c).ok())
        .unwrap_or_default();
    let rendered = input.render_at(now);
    let (header, body) = rendered.split_once('\n').unwrap_or((&rendered, ""));
    // Each attached file has its line right below the workgroup's.
    let mut attached = 0;
    for line in body.split_inclusive('\n').take(input.attached().len()) {
        attached += line.len();
    }
    let (attached, said) = body.split_at(attached);
    format!("{header}\n{line}\n{attached}{besides}{said}")
}

/// Where what comes back to a conversation is answered, when the owner asked
/// for it somewhere else than the console, where the answer finds its thread
/// by itself.
fn asked_in(app: &App, row: &StoredInput) -> anyhow::Result<String> {
    if row.input.place().is_some() {
        return Ok(String::new());
    }
    let place = app
        .conversations
        .asked_with(row, |request| app.request_made_in(request))?
        .and_then(|input| input.place());
    Ok(place
        .map(|place| {
            format!(
                "The owner asked for it on Slack, so answer there: workspace: {} channel: {} thread_ts: {}\n",
                place.workspace,
                place.channel,
                place.thread()
            )
        })
        .unwrap_or_default())
}

/// The text of an input answered without the notebook. What the workgroup
/// lets the secretary use, and where files lie on this machine, is no guest's
/// to know.
fn guests_text(input: &Input, now: impl std::fmt::Display) -> String {
    let mut input = input.clone();
    if let Source::Slack { attached, .. } = &mut input.source
        && !attached.is_empty()
    {
        attached.clear();
        input
            .text
            .push_str("\n\n(Files came along, which you cannot open here. Say so.)");
    }
    input.render_at(now)
}

/// The process of the conversation the last turn was in.
struct Resident {
    conversation: i64,
    handle: Handle,
    task: AbortOnDrop<anyhow::Result<()>>,
}

impl Resident {
    async fn end(mut self) {
        self.handle.end(&mut self.task.0).await;
    }
}

/// Why the owner's process is done. Waits for ever when there is none.
async fn resident_gone(resident: &mut Option<Resident>) -> String {
    let Some(Resident { task, .. }) = resident else {
        return std::future::pending().await;
    };
    match (&mut task.0).await {
        Ok(Ok(())) => "it ended".into(),
        Ok(Err(error)) => format!("{error:#}"),
        Err(error) => error.to_string(),
    }
}

/// Wait for the tool calls still running: before a process is started, so
/// nothing is left unowned if the loop is stopped while waiting. A call that
/// never answers is given up on.
async fn settled(services: &Services<GatewaySlack>) -> bool {
    let settled = tokio::time::timeout(SETTLE_LIMIT, services.in_flight.settled())
        .await
        .is_ok();
    if !settled {
        tracing::warn!("tool calls of the last session did not finish; going on anyway");
    }
    settled
}

/// Feed every pending input to the conversation it is of, one turn at a time,
/// in a Claude Code process that goes on with it. The process stays for the
/// next turn of the same conversation and ends before one of another. Inputs
/// that hold others' words are each answered by a process of its own, as
/// `guests` says.
pub(super) async fn run_session(
    of: OfSessions,
    services: Arc<Services<GatewaySlack>>,
    store: Store,
    arrived: Arc<Notify>,
    links: SessionLinks,
    tz: chrono_tz::Tz,
) -> anyhow::Result<()> {
    let changed = &links.reload_now;
    let app = &links.app;
    let mut resident: Option<Resident> = None;
    let mut guest: Option<AnsweringGuest> = None;
    let finished = settled(&services).await;
    if recover_interrupted(&store, finished)?.is_some() && !finished {
        app.console.notify(Event::Approvals);
    }
    loop {
        if store.running()?.is_none() {
            guest = None;
            services.compare_written().await;
        }
        // Inputs came or a turn ended.
        app.working.tell();
        // The reload is started here, where turns are started, so no turn
        // can start between seeing none running and the engine stopping.
        if store.running()?.is_none() && store.state(RELOAD_KEY)?.is_some() {
            store.clear_state(RELOAD_KEY)?;
            if let Some(resident) = resident.take() {
                resident.end().await;
            }
            changed.notify_one();
            std::future::pending::<()>().await;
        }
        // One input per turn. A turn runs under one workgroup, and the tools
        // it calls are judged by it; the next input waits for the turn to
        // end. The store lets only one input run, so this holds however
        // the turn ends.
        if store.running()?.is_none()
            && let Some(row) = next_input(app).await?
        {
            if let Some(who) = Guest::of(&row) {
                let who = who.named(app).await;
                guest = answer_guest(&of.guests, &services, app, &row, who, tz)?;
                continue;
            }
            let conversation = app
                .conversations
                .settle(&row, |request| app.request_made_in(request))?;
            if resident
                .as_ref()
                .is_none_or(|r| r.conversation != conversation.id)
            {
                if let Some(before) = resident.take() {
                    before.end().await;
                    settled(&services).await;
                }
                match start(&of, &services, &conversation) {
                    Ok(started) => resident = Some(started),
                    Err(error) => {
                        tracing::warn!(%error, bin = %of.owners.bin, "could not start the secretary; is the agent CLI installed?");
                        services.journal.record(
                            "session",
                            format!("could not start {}: {error}", of.owners.bin),
                        );
                        tokio::time::sleep(RESTART_AFTER * 6).await;
                    }
                }
                // Ending one and starting another took a while: whatever came
                // meanwhile is looked at again.
                continue;
            }
            if let Some(resident) = &resident {
                if let Err(error) = services.mark_begun(&row).await {
                    tracing::warn!(%error, id = row.id, "could not record the notebook; the input waits");
                    services
                        .journal
                        .record("session", format!("an input waits: {error}"));
                    tokio::time::sleep(RESTART_AFTER * 6).await;
                    continue;
                }
                give(app, &services, resident, &conversation, &row, tz)?;
            }
        }
        tokio::select! {
            () = arrived.notified() => {}
            why = guest_gone(&mut guest) => {
                guest = None;
                tracing::warn!(%why, "a guest's session ended before its turn");
                services
                    .journal
                    .record("session", format!("a guest's session ended before its turn: {why}"));
                // Not given again: it may have answered, and what others
                // write must not hold up the owner's inputs.
                store.end_turn()?;
            }
            why = resident_gone(&mut resident) => {
                resident = None;
                tracing::warn!(%why, "claude session ended");
                // A guest's turn goes on without it.
                if guest.is_none() {
                    let finished = settled(&services).await;
                    if recover_interrupted(&store, finished)?.is_some() && !finished {
                        app.console.notify(Event::Approvals);
                    }
                }
                tokio::time::sleep(RESTART_AFTER).await;
            }
        }
    }
}

/// Start the process that goes on with a conversation.
fn start(
    of: &OfSessions,
    services: &Arc<Services<GatewaySlack>>,
    conversation: &Conversation,
) -> anyhow::Result<Resident> {
    let options = Options {
        resume: conversation.session.clone(),
        system_prompt: (of.prompt)(),
        ..of.owners.clone()
    };
    tracing::info!(conversation = conversation.id, resume = ?options.resume, model = ?options.model, effort = ?options.effort, "starting claude");
    let (handle, task) =
        session::spawn(&options, services.clone(), Audience::Owner(conversation.id))?;
    Ok(Resident {
        conversation: conversation.id,
        handle,
        task: AbortOnDrop(task),
    })
}

/// Start the turn of an input in its conversation.
fn give(
    app: &App,
    services: &Services<GatewaySlack>,
    resident: &Resident,
    conversation: &Conversation,
    row: &StoredInput,
    tz: chrono_tz::Tz,
) -> anyhow::Result<()> {
    let workgroup = workgroup_of(&app.workgroups, &row.input);
    let said = app
        .conversations
        .said_before(&app.console, conversation, row, &workgroup)?;
    // What it is handed is in its model from now on.
    crate::disclosure::carry(
        &app.store,
        &said.conversations,
        &said.inputs,
        conversation.id,
    )?;
    let besides = format!("{}{}", asked_in(app, row)?, said.text);
    let now = Utc::now().with_timezone(&tz).to_rfc3339();
    let text = deliverable(&app.workgroups, &row.input, &besides, now);
    let text = if row.interrupted { resent(&text) } else { text };
    let header = text.lines().next().unwrap_or_default().to_owned();
    if !app.store.start_turn(row.id, &workgroup)? {
        return Ok(());
    }
    match resident.handle.send(text) {
        Ok(()) => {
            app.conversations
                .began(conversation.id, &row.input, &workgroup)?;
            app.working.began(row.id);
            services.journal.record(
                "input",
                format!("{header} conversation: #{}", conversation.id),
            );
        }
        Err(error) => {
            tracing::warn!(%error, id = row.id, "could not deliver; will retry after restart");
            app.store.undo_start(row.id)?;
        }
    }
    Ok(())
}

/// The process answering one guest, and when it is given up on.
struct AnsweringGuest {
    task: AbortOnDrop<anyhow::Result<()>>,
    until: tokio::time::Instant,
}

/// Why the guest's process is done without its turn having ended. Waits for
/// ever when there is none.
async fn guest_gone(guest: &mut Option<AnsweringGuest>) -> String {
    let Some(AnsweringGuest { task, until }) = guest else {
        return std::future::pending().await;
    };
    match tokio::time::timeout_at(*until, &mut task.0).await {
        Ok(Ok(Ok(()))) => "it ended".into(),
        Ok(Ok(Err(error))) => format!("{error:#}"),
        Ok(Err(error)) => error.to_string(),
        Err(_) => "it took too long".into(),
    }
}

/// Start the turn of an input that holds others' words, in a process of its
/// own that knows nothing of the owner's.
fn answer_guest(
    options: &Options,
    services: &Arc<Services<GatewaySlack>>,
    app: &App,
    row: &StoredInput,
    who: Guest,
    tz: chrono_tz::Tz,
) -> anyhow::Result<Option<AnsweringGuest>> {
    let text = guests_text(&row.input, Utc::now().with_timezone(&tz).to_rfc3339());
    let text = if row.interrupted { resent(&text) } else { text };
    let header = text.lines().next().unwrap_or_default().to_owned();
    let workgroup = workgroup_of(&app.workgroups, &row.input);
    if !app.store.start_turn(row.id, &workgroup)? {
        return Ok(None);
    }
    app.working.began(row.id);
    services.journal.record("input", header);
    services.journal.record(
        "session",
        format!(
            "answering input #{} without the notebook: others' words are in it",
            row.id
        ),
    );
    tracing::info!(id = row.id, "answering a guest without the notebook");
    std::fs::remove_dir_all(&options.workdir).ok();
    let started = session::spawn(options, services.clone(), Audience::Guest(who)).and_then(
        |(handle, task)| {
            let task = AbortOnDrop(task);
            handle.send(text)?;
            Ok(task)
        },
    );
    match started {
        Ok(task) => Ok(Some(AnsweringGuest {
            task,
            until: tokio::time::Instant::now() + GUEST_LIMIT,
        })),
        Err(error) => {
            tracing::warn!(%error, id = row.id, "could not answer a guest");
            services
                .journal
                .record("session", format!("could not answer a guest: {error}"));
            app.store.end_turn()?;
            Ok(None)
        }
    }
}

/// The next input, with what was attached to it here.
async fn next_input(app: &App) -> anyhow::Result<Option<StoredInput>> {
    let Some(row) = app.store.next_pending()? else {
        return Ok(None);
    };
    if row.input.attached().is_empty() {
        return Ok(Some(row));
    }
    if let Err(error) = super::files::settle(app, row.id).await {
        tracing::warn!(%error, id = row.id, "could not fetch what was attached");
    }
    app.store.get(row.id)
}

/// An input given again after its turn was cut off, told so: the turn may
/// have done part of the work, such as sending a mail.
fn resent(text: &str) -> String {
    let (header, body) = text.split_once('\n').unwrap_or((text, ""));
    format!(
        "{header}\n(Given before, but its turn was cut off before it ended. Check what was already done, such as mails sent or events made, before doing anything again.)\n{body}"
    )
}

/// A turn that was running when the last process stopped (a crash, a
/// restart) never ended. Once the calls it made have `settled`, its input
/// waits again, marked as cut off. While one may still land, the input is
/// held for the owner instead, so the next turn cannot repeat it. Returns
/// the input.
fn recover_interrupted(store: &Store, settled: bool) -> anyhow::Result<Option<i64>> {
    let id = if settled {
        store.interrupt_turn()?
    } else {
        store.hold_turn()?
    };
    if let Some(id) = id {
        tracing::info!(
            id,
            settled,
            "the input of an interrupted turn goes in again or waits for the owner"
        );
    }
    Ok(id)
}

/// Stops the Claude Code task, and with it the process, when the session
/// loop is stopped: dropping a `JoinHandle` alone leaves the task running.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Sender;

    #[test]
    fn a_console_input_carries_its_workgroup_and_a_slack_one_the_workspaces() {
        let store = Store::open_in_memory().unwrap();
        let (_tmp, records) = crate::records::testing::records("runtime-console-input");
        let workgroups = Workgroups::new(store, records.notebook().clone()).unwrap();
        let console = Input {
            sender: Sender::Owner,
            source: Source::Console {
                message: 1,
                workgroup: "work".into(),
                view: "タスク".into(),
                selection: None,
                attached: Vec::new(),
                reply_to: None,
            },
            text: "これは".into(),
        };
        let text = deliverable(&workgroups, &console, "", "now");
        assert_eq!(workgroup_of(&workgroups, &console), "work");
        assert!(text.lines().nth(1).unwrap().contains("仕事"), "{text}");
        let slack = Input {
            sender: Sender::Owner,
            source: Source::Slack {
                workspace: "w1".into(),
                channel: "D1".into(),
                ts: "1".into(),
                thread_ts: None,
                user: "U".into(),
                unmentioned: false,
                others_before: false,
                attached: Vec::new(),
            },
            text: "x".into(),
        };
        assert_eq!(workgroup_of(&workgroups, &slack), ALL);
        workgroups
            .set_member("work", MemberKind::Channel, "slack:w1", true)
            .unwrap();
        assert_eq!(workgroup_of(&workgroups, &slack), "work");
    }

    #[test]
    fn the_inputs_of_an_interrupted_turn_go_in_again() {
        let store = Store::open_in_memory().unwrap();
        let input = Input {
            sender: Sender::Owner,
            source: Source::Console {
                message: 1,
                workgroup: "work".into(),
                view: "タスク".into(),
                selection: None,
                attached: Vec::new(),
                reply_to: None,
            },
            text: "x".into(),
        };
        let id = store.enqueue(&input).unwrap().unwrap();
        assert!(store.start_turn(id, "work").unwrap());
        recover_interrupted(&store, true).unwrap();
        let again = store.next_pending().unwrap().unwrap();
        assert_eq!(again.id, id);
        assert!(again.interrupted);
        assert_eq!(store.running().unwrap(), None);
        let (_tmp, records) = crate::records::testing::records("runtime-interrupted");
        let text = deliverable(
            &Workgroups::new(store.clone(), records.notebook().clone()).unwrap(),
            &again.input,
            "",
            "now",
        );
        assert!(
            resent(&text).lines().nth(1).unwrap().contains("cut off"),
            "the next turn is told the last one was cut off"
        );
    }

    #[test]
    fn a_guest_is_told_nothing_of_what_the_workgroup_lets_the_secretary_use() {
        let input = Input {
            sender: Sender::Owner,
            source: Source::Slack {
                workspace: "w1".into(),
                channel: "C1".into(),
                ts: "1".into(),
                thread_ts: None,
                user: "U".into(),
                unmentioned: true,
                others_before: true,
                attached: vec![crate::attachments::Attached::missing(
                    "/home/me/secret.png",
                    "x",
                )],
            },
            text: "body".into(),
        };
        let text = guests_text(&input, "now");
        assert_eq!(text.lines().nth(1), Some("body"));
        assert!(!text.contains("/home/me") && text.contains("Files came along"));
    }

    #[test]
    fn an_interrupted_input_is_held_while_a_call_may_still_land() {
        let store = Store::open_in_memory().unwrap();
        let input = Input {
            sender: Sender::Owner,
            source: Source::Console {
                message: 2,
                workgroup: "work".into(),
                view: String::new(),
                selection: None,
                attached: Vec::new(),
                reply_to: None,
            },
            text: "x".into(),
        };
        let id = store.enqueue(&input).unwrap().unwrap();
        store.start_turn(id, "work").unwrap();
        assert_eq!(recover_interrupted(&store, false).unwrap(), Some(id));
        assert_eq!(
            store.next_pending().unwrap(),
            None,
            "not given again by itself"
        );
        assert_eq!(store.held().unwrap()[0].id, id);
    }
}
