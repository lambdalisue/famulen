//! Keeping the secretary's session running, and feeding it every input.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::Notify;

use super::RESTART_AFTER;
use crate::app::App;
use crate::claude::guest::Guest;
use crate::claude::session::{self, Audience, InFlight, Options, Services};
use crate::console_log::Event;
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
    pub(super) resident: Options,
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
/// workgroup line, then the body.
fn deliverable(
    workgroups: &Workgroups,
    input: &Input,
    now: impl std::fmt::Display,
) -> (String, String) {
    let workgroup_id = workgroup_of(workgroups, input);
    let workgroup = workgroups.resolve(Some(&workgroup_id)).ok();
    let line = workgroup
        .as_ref()
        .and_then(|c| workgroups.describe_for_prompt(c).ok())
        .unwrap_or_default();
    let rendered = input.render_at(now);
    let (header, body) = rendered.split_once('\n').unwrap_or((&rendered, ""));
    (workgroup_id, format!("{header}\n{line}\n{body}"))
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

/// Keep one Claude Code process running on the saved conversation, and feed
/// it every pending input but those that hold others' words, each of which
/// a process of its own answers as `guests` says.
pub(super) async fn run_session(
    OfSessions {
        resident: options,
        guests,
    }: OfSessions,
    services: Arc<Services<GatewaySlack>>,
    store: Store,
    workgroups: Workgroups,
    arrived: Arc<Notify>,
    links: SessionLinks,
    tz: chrono_tz::Tz,
) -> anyhow::Result<()> {
    let changed = &links.reload_now;
    loop {
        let options = Options {
            resume: session::saved_session(&store)?,
            ..options.clone()
        };
        // A reload asked for while no session runs (it could not start, or
        // the last one ended) is carried out at once: no turn can be cut.
        if store.state(RELOAD_KEY)?.is_some() {
            store.clear_state(RELOAD_KEY)?;
            changed.notify_one();
            std::future::pending::<()>().await;
        }
        // Tool calls of the last session still running finish first: before a
        // process is started, so nothing is left unowned if this loop is
        // stopped while waiting. A call that never answers is given up on.
        let settled = tokio::time::timeout(SETTLE_LIMIT, services.in_flight.settled())
            .await
            .is_ok();
        if !settled {
            tracing::warn!("tool calls of the last session did not finish; starting anyway");
        }
        tracing::info!(resume = ?options.resume, model = ?options.model, effort = ?options.effort, "starting claude");
        let (handle, task) = match session::spawn(&options, services.clone(), Audience::Owner) {
            Ok(started) => {
                if recover_interrupted(&store, settled)?.is_some() && !settled {
                    links.app.console.notify(Event::Approvals);
                }
                started
            }
            Err(error) => {
                tracing::warn!(%error, bin = %options.bin, "could not start the secretary; is the agent CLI installed?");
                services.journal.record(
                    "session",
                    format!("could not start {}: {error}", options.bin),
                );
                tokio::time::sleep(RESTART_AFTER * 6).await;
                continue;
            }
        };
        let mut task = AbortOnDrop(task);
        let mut guest: Option<AnsweringGuest> = None;
        loop {
            if store.running()?.is_none() {
                guest = None;
            }
            // Inputs came or a turn ended.
            links.app.working.tell();
            // The reload is started here, where turns are started, so no turn
            // can start between seeing none running and the engine stopping.
            if store.running()?.is_none() && store.state(RELOAD_KEY)?.is_some() {
                store.clear_state(RELOAD_KEY)?;
                changed.notify_one();
                std::future::pending::<()>().await;
            }
            // One input per turn. A turn runs under one workgroup, and the tools
            // it calls are judged by it; the next input waits for the turn to
            // end. The store lets only one input run, so this holds however
            // the turn ends.
            if store.running()?.is_none()
                && let Some(row) = next_input(&links.app).await?
            {
                if let Some(who) = Guest::of(&row) {
                    let who = who.named(&links.app).await;
                    guest = answer_guest(&guests, &services, &links.app, &row, who, tz)?;
                    continue;
                }
                let (workgroup, text) = deliverable(
                    &workgroups,
                    &row.input,
                    Utc::now().with_timezone(&tz).to_rfc3339(),
                );
                let text = if row.interrupted { resent(&text) } else { text };
                let header = text.lines().next().unwrap_or_default().to_owned();
                if store.start_turn(row.id, &workgroup)? {
                    match handle.send(text) {
                        Ok(()) => {
                            links.app.working.began(row.id);
                            services.journal.record("input", header);
                        }
                        Err(error) => {
                            tracing::warn!(%error, id = row.id, "could not deliver; will retry after restart");
                            store.undo_start(row.id)?;
                        }
                    }
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
                ended = &mut task.0 => {
                    match ended {
                        Ok(Ok(())) => tracing::warn!("claude session ended"),
                        Ok(Err(error)) => tracing::warn!(%error, "claude session ended"),
                        Err(error) => tracing::warn!(%error, "claude session task failed"),
                    }
                    break;
                }
            }
        }
        tokio::time::sleep(RESTART_AFTER).await;
    }
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
            },
            text: "これは".into(),
        };
        let (workgroup, text) = deliverable(&workgroups, &console, "now");
        assert_eq!(workgroup, "work");
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
        let (_, text) = deliverable(
            &Workgroups::new(store.clone(), records.notebook().clone()).unwrap(),
            &again.input,
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
