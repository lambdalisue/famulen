//! What a run did and does, as the console reads it and follows it.

use std::time::Duration;

use famulen_core::app::App;
use famulen_core::workers::{Change, Entry, Item, Live, PartOf, Step};
use futures::Stream;
use tokio::sync::broadcast::{self, error::RecvError};

use crate::Api;
use crate::pb::__buffa::oneof::run_entry::Item as I;
use crate::pb::__buffa::oneof::run_event::Event as E;
use crate::pb::*;

/// How many entries are read at once.
const PAGE: usize = 500;
/// How often how a run stands is read anew while it is watched.
const STATE_EVERY: Duration = Duration::from_secs(4);
/// The same once it ended, when little more is to come.
const ENDED_EVERY: Duration = Duration::from_secs(30);

pub(crate) fn step(s: &Step) -> WorkerStep {
    WorkerStep {
        id: s.id.clone(),
        parent: s.parent.clone(),
        title: s.title.clone(),
        status: s.status.clone(),
        note: s.note.clone(),
        weight: s.weight,
        description: s.description.clone(),
        ..Default::default()
    }
}

fn text(message: &str, text: &str) -> Box<RunText> {
    Box::new(RunText {
        message: message.into(),
        text: text.into(),
        ..Default::default()
    })
}

pub(crate) fn entry(e: Entry) -> RunEntry {
    let item = match e.item {
        Item::Asked { text: t } => I::Asked(text("", &t)),
        Item::Said { message, text: t } => I::Said(text(&message, &t)),
        Item::Thought { message, text: t } => I::Thought(text(&message, &t)),
        Item::Called { call, tool, input } => I::Called(Box::new(RunCall {
            call,
            tool,
            input: input.to_string(),
            ..Default::default()
        })),
        Item::Answered { call, ok, output } => I::Answered(Box::new(RunAnswer {
            call,
            ok,
            output,
            ..Default::default()
        })),
        Item::Changed { call, path, diff } => I::Changed(Box::new(RunFileChange {
            call,
            path,
            diff,
            ..Default::default()
        })),
        Item::Question { id, tool, what } => I::Question(Box::new(RunAsked {
            id,
            tool,
            what,
            ..Default::default()
        })),
        Item::Decided { id, allowed, why } => I::Decided(Box::new(RunDecided {
            id,
            allowed,
            why,
            ..Default::default()
        })),
        Item::Withdrawn { id } => I::Withdrawn(id),
        Item::Plan { from, steps } => I::Plan(Box::new(RunPlan {
            from: from.as_str().into(),
            steps: steps.iter().map(step).collect(),
            ..Default::default()
        })),
        Item::Used {
            message,
            model,
            input,
            cached,
            output,
        } => I::Used(Box::new(RunUsed {
            message,
            model,
            input,
            cached,
            output,
            ..Default::default()
        })),
        Item::Warned { what } => I::Warned(what),
        Item::TurnEnded { ok, text: t } => I::TurnEnded(Box::new(RunTurnEnded {
            ok,
            text: t,
            ..Default::default()
        })),
        Item::Ended { reason } => I::Ended(reason),
        Item::Full => I::Full(Box::default()),
        Item::Forgotten => I::Forgotten(Box::default()),
    };
    RunEntry {
        seq: e.seq,
        at: e.at,
        cut: e.cut,
        item: Some(item),
        ..Default::default()
    }
}

/// All that a run did after `after`.
pub(crate) fn entries(app: &App, run: i64, after: u64) -> anyhow::Result<Vec<RunEntry>> {
    let ledger = app.workers.sights.ledger();
    let mut all = Vec::new();
    let mut last = after;
    loop {
        let page = ledger.since(run, last, PAGE)?;
        let more = page.len() == PAGE;
        last = page.last().map_or(last, |e| e.seq);
        all.extend(page.into_iter().map(entry));
        if !more {
            return Ok(all);
        }
    }
}

pub(crate) fn changes(app: &App, run: i64) -> anyhow::Result<Vec<StepChange>> {
    Ok(app
        .workers
        .sights
        .ledger()
        .plan(run)?
        .changes
        .into_iter()
        .map(|c| {
            let (change, from, to) = match c.change {
                Change::Added => ("added", String::new(), String::new()),
                Change::Status { from, to } => ("status", from, to),
                Change::Title => ("title", String::new(), String::new()),
                Change::Note => ("note", String::new(), String::new()),
                Change::Removed => ("removed", String::new(), String::new()),
            };
            StepChange {
                seq: c.seq,
                at: c.at,
                step: c.step,
                title: c.title,
                change: change.into(),
                from,
                to,
                ..Default::default()
            }
        })
        .collect())
}

fn part(at: i64, of: PartOf, text: String) -> RunPart {
    let (of, id) = match of {
        PartOf::Said(id) => ("said", id),
        PartOf::Thought(id) => ("thought", id),
        PartOf::Input(id) => ("input", id),
        PartOf::Output(id) => ("output", id),
    };
    RunPart {
        at,
        of: of.into(),
        id,
        text,
        ..Default::default()
    }
}

fn event(event: E) -> RunEvent {
    RunEvent {
        event: Some(event),
        ..Default::default()
    }
}

struct Watching {
    api: Api,
    run: i64,
    /// The newest entry told.
    told: u64,
    live: broadcast::Receiver<Live>,
    tick: tokio::time::Interval,
    /// How it stood when last told, encoded.
    stood: RunOutput,
    /// What is to be told next, oldest first.
    queue: std::collections::VecDeque<RunEvent>,
    /// What the run was writing when the watch began, told once what was
    /// kept is.
    opening: Option<Vec<RunEvent>>,
}

/// What a run is writing, as it is told.
fn writing(now: Vec<Live>) -> Vec<RunEvent> {
    now.into_iter()
        .filter_map(|w| match w {
            Live::Part { at, of, text, .. } => Some(event(E::Part(Box::new(part(at, of, text))))),
            Live::Entry(_) => None,
        })
        .collect()
}

impl Watching {
    /// Tell how it stands when that changed.
    async fn state(&mut self) {
        match self.api.run_state(self.run).await {
            Ok(state) if state != self.stood => {
                let ended = |s: &RunOutput| s.run.as_option().is_some_and(|r| r.state == "ended");
                if ended(&state) != ended(&self.stood) {
                    let every = if ended(&state) {
                        ENDED_EVERY
                    } else {
                        STATE_EVERY
                    };
                    self.tick = tokio::time::interval(every);
                    self.tick.reset();
                }
                self.stood = state.clone();
                self.queue.push_back(event(E::State(Box::new(state))));
            }
            Ok(_) => {}
            Err(error) => tracing::debug!(run = self.run, %error, "could not read a run"),
        }
    }

    /// What was kept and is not told yet.
    /// Read apart from the runtime: a run may have kept megabytes.
    async fn kept(&mut self) {
        let (app, run, told) = (self.api.app.clone(), self.run, self.told);
        let read = tokio::task::spawn_blocking(move || entries(&app, run, told))
            .await
            .unwrap_or_else(|error| Err(error.into()));
        match read {
            Ok(all) => {
                for e in all {
                    self.told = self.told.max(e.seq);
                    self.queue.push_back(event(E::Entry(Box::new(e))));
                }
            }
            Err(error) => tracing::debug!(run = self.run, %error, "could not read a run's entries"),
        }
    }

    async fn next(&mut self) -> Option<RunEvent> {
        loop {
            if let Some(writing) = self.opening.take() {
                self.kept().await;
                self.queue.extend(writing);
            }
            if let Some(next) = self.queue.pop_front() {
                return Some(next);
            }
            tokio::select! {
                live = self.live.recv() => match live {
                    Ok(live) if live.run() == self.run => match live {
                        Live::Entry(e) => {
                            // What a session asks the owner is read with how it
                            // stands at the next tick; a worker asks of each call.
                            let settles = matches!(
                                e.item,
                                Item::Plan { .. } | Item::TurnEnded { .. } | Item::Ended { .. }
                            );
                            self.told = self.told.max(e.seq);
                            self.queue.push_back(event(E::Entry(Box::new(entry(e)))));
                            if settles {
                                self.state().await;
                            }
                        }
                        Live::Part { at, of, text, .. } => {
                            self.queue.push_back(event(E::Part(Box::new(part(at, of, text)))));
                        }
                    },
                    Ok(_) => {}
                    // Fallen behind, it begins anew: all that was kept, for an
                    // entry may have grown under a number told, and what is
                    // being written now, whole.
                    Err(RecvError::Lagged(_)) => {
                        let (live, now) = self.api.app.workers.sights.ledger().watch(self.run);
                        self.live = live;
                        self.told = 0;
                        self.opening = Some(writing(now));
                        self.queue.push_back(event(E::Reset(Box::default())));
                    }
                    Err(RecvError::Closed) => return None,
                },
                _ = self.tick.tick() => self.state().await,
            }
        }
    }
}

/// How a run stands, what it did after `after`, and then what it does.
pub(crate) fn watch(
    api: Api,
    run: i64,
    after: u64,
    first: RunOutput,
    (live, now): (broadcast::Receiver<Live>, Vec<Live>),
) -> impl Stream<Item = Result<RunEvent, connectrpc::ConnectError>> + Send + 'static {
    let ended = first.run.as_option().is_some_and(|r| r.state == "ended");
    let mut tick = tokio::time::interval(if ended { ENDED_EVERY } else { STATE_EVERY });
    tick.reset();
    let watching = Watching {
        api,
        run,
        told: after,
        live,
        tick,
        stood: first.clone(),
        queue: [event(E::State(Box::new(first)))].into(),
        opening: Some(writing(now)),
    };
    futures::stream::unfold(watching, |mut w| async move {
        w.next().await.map(|e| (Ok(e), w))
    })
}
