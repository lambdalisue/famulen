//! Keeping the secretary's session running, and feeding it every input.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::Notify;

use super::RESTART_AFTER;
use crate::app::App;
use crate::claude::session::{self, InFlight, Options, Services};
use crate::console_log::Event;
use crate::input::{Input, Source};
use crate::slack::web::GatewaySlack;
use crate::store::{Store, StoredInput};
use crate::workgroups::{ALL, MemberKind, Workgroups};

/// How long a new session waits for the last one's tool calls to finish.
pub(super) const SETTLE_LIMIT: Duration = Duration::from_secs(120);
/// Set when the engine should be rebuilt between turns.
pub(super) const RELOAD_KEY: &str = "engine.reload";

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
        Source::Slack { workspace, .. } => workgroups
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

/// Keep one Claude Code process running on the saved conversation, and feed
/// it every pending input.
pub(super) async fn run_session(
    options: Options,
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
        let (handle, task) = match session::spawn(&options, services.clone()) {
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
        loop {
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
                let (workgroup, text) = deliverable(
                    &workgroups,
                    &row.input,
                    Utc::now().with_timezone(&tz).to_rfc3339(),
                );
                let text = if row.interrupted { resent(&text) } else { text };
                let header = text.lines().next().unwrap_or_default().to_owned();
                if store.start_turn(row.id, &workgroup)? {
                    match handle.send(text) {
                        Ok(()) => services.journal.record("input", header),
                        Err(error) => {
                            tracing::warn!(%error, id = row.id, "could not deliver; will retry after restart");
                            store.undo_start(row.id)?;
                        }
                    }
                }
            }
            tokio::select! {
                () = arrived.notified() => {}
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
