//! What each run did, as settled events in the database, in the order they
//! came. What is being written is told to whoever watches and not kept: what
//! settles holds it all. It is the worker's words, and nothing is decided by
//! it.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use rusqlite::{OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

use super::sight::Step;
use crate::store::Store;

/// How much of one text an entry keeps.
pub const ENTRY_CHARS: usize = 64 * 1024;
/// How much of one text is kept once its run kept all it may.
pub const FULL_CHARS: usize = 1000;
/// How much one run keeps in all, as JSON, before what comes is cut short.
pub const RUN_BYTES: i64 = 8 * 1024 * 1024;
/// How long after its run ended what it did is kept, but for its plan.
pub const KEEP_DAYS: i64 = 30;

/// Where a plan came from, by how far it is to be trusted to be one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanFrom {
    /// Read from the notes an agent keeps for itself.
    Notes,
    /// The timeline of an Adroit session.
    Adroit,
    /// Written with Famulen's own timeline tool.
    Timeline,
}

impl PlanFrom {
    pub fn as_str(self) -> &'static str {
        match self {
            PlanFrom::Notes => "notes",
            PlanFrom::Adroit => "adroit",
            PlanFrom::Timeline => "timeline",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Item {
    /// What the worker was asked.
    Asked {
        text: String,
    },
    Said {
        message: String,
        text: String,
    },
    Thought {
        message: String,
        text: String,
    },
    Called {
        call: String,
        tool: String,
        input: Value,
    },
    Answered {
        call: String,
        ok: bool,
        output: String,
    },
    Changed {
        call: String,
        path: String,
        diff: String,
    },
    /// What the worker asked to do or be told; `what` as the console shows it.
    Question {
        id: String,
        tool: String,
        what: String,
    },
    Decided {
        id: String,
        allowed: bool,
        why: String,
    },
    Withdrawn {
        id: String,
    },
    Plan {
        from: PlanFrom,
        steps: Vec<Step>,
    },
    Used {
        message: String,
        model: String,
        input: u64,
        cached: u64,
        output: u64,
    },
    Warned {
        what: String,
    },
    TurnEnded {
        ok: bool,
        text: String,
    },
    Ended {
        reason: String,
    },
    /// The run kept all it may; what comes after is cut short.
    Full,
    /// What it did was forgotten for its age, but for its plan.
    Forgotten,
}

impl Item {
    pub fn kind(&self) -> &'static str {
        match self {
            Item::Asked { .. } => "asked",
            Item::Said { .. } => "said",
            Item::Thought { .. } => "thought",
            Item::Called { .. } => "called",
            Item::Answered { .. } => "answered",
            Item::Changed { .. } => "changed",
            Item::Question { .. } => "question",
            Item::Decided { .. } => "decided",
            Item::Withdrawn { .. } => "withdrawn",
            Item::Plan { .. } => "plan",
            Item::Used { .. } => "used",
            Item::Warned { .. } => "warned",
            Item::TurnEnded { .. } => "turn_ended",
            Item::Ended { .. } => "ended",
            Item::Full => "full",
            Item::Forgotten => "forgotten",
        }
    }

    /// Cut its texts to `chars`; whether any was longer.
    fn cut(&mut self, chars: usize) -> bool {
        let mut cut = false;
        let mut each = |text: &mut String| cut |= cut_text(text, chars);
        match self {
            Item::Asked { text }
            | Item::Said { text, .. }
            | Item::Thought { text, .. }
            | Item::TurnEnded { text, .. } => each(text),
            Item::Answered { output, .. } => each(output),
            Item::Changed { diff, .. } => each(diff),
            Item::Question { what, .. } => each(what),
            Item::Called { input, .. } => cut |= cut_value(input, chars),
            Item::Decided { .. }
            | Item::Withdrawn { .. }
            | Item::Plan { .. }
            | Item::Used { .. }
            | Item::Warned { .. }
            | Item::Ended { .. }
            | Item::Full
            | Item::Forgotten => {}
        }
        cut
    }
}

fn cut_text(text: &mut String, chars: usize) -> bool {
    match text.char_indices().nth(chars) {
        Some((at, _)) => {
            text.truncate(at);
            true
        }
        None => false,
    }
}

fn cut_value(value: &mut Value, chars: usize) -> bool {
    match value {
        Value::String(text) => cut_text(text, chars),
        Value::Array(items) => items
            .iter_mut()
            .fold(false, |cut, v| cut_value(v, chars) | cut),
        Value::Object(fields) => fields
            .values_mut()
            .fold(false, |cut, v| cut_value(v, chars) | cut),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub run: i64,
    /// Counts the entries of its run from 1. An entry told again as it grows
    /// keeps its number.
    pub seq: u64,
    /// Unix milliseconds.
    pub at: i64,
    /// Some of its text was not kept.
    pub cut: bool,
    pub item: Item,
}

/// What is being written, which the next settled entry holds whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartOf {
    /// What a message says, by the message.
    Said(String),
    Thought(String),
    /// The input of a call, by the call, as a piece of its JSON.
    Input(String),
    /// What a call puts out while it works, by the call.
    Output(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Live {
    Entry(Entry),
    Part {
        run: i64,
        at: i64,
        of: PartOf,
        text: String,
    },
}

impl Live {
    pub fn run(&self) -> i64 {
        match self {
            Live::Entry(entry) => entry.run,
            Live::Part { run, .. } => *run,
        }
    }
}

/// How one step changed from one plan to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added,
    Status { from: String, to: String },
    Title,
    Note,
    Removed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepChange {
    /// The entry of the plan it changed in.
    pub seq: u64,
    pub at: i64,
    pub step: String,
    pub title: String,
    pub change: Change,
}

/// A run's plan as it stands, and how it came to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub from: Option<PlanFrom>,
    pub steps: Vec<Step>,
    pub changes: Vec<StepChange>,
}

/// A plan as it was kept.
struct KeptPlan {
    seq: u64,
    at: i64,
    from: PlanFrom,
    steps: Vec<Step>,
}

#[derive(Debug, Clone)]
struct Writing {
    at: i64,
    of: PartOf,
    text: String,
}

/// How many live parts of one run may wait for one who watches slowly.
const LIVE_ROOM: usize = 4096;

#[derive(Clone)]
pub struct Ledger {
    store: Store,
    /// What is told of each run that is watched, apart from the others: a
    /// busy run does not have one who watches another fall behind.
    live: Arc<Mutex<HashMap<i64, broadcast::Sender<Live>>>>,
    /// How many bytes each run keeps, once counted.
    bytes: Arc<Mutex<HashMap<i64, i64>>>,
    /// What each run is writing now, which one who begins to watch is told
    /// first. Never kept.
    writing: Arc<Mutex<HashMap<i64, Vec<Writing>>>>,
    /// Held while a plan is changed with the timeline tool.
    timeline: Arc<Mutex<()>>,
}

impl Ledger {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            live: Arc::default(),
            bytes: Arc::default(),
            writing: Arc::default(),
            timeline: Arc::default(),
        }
    }

    /// Tell those who watch a run; none is kept for a run nobody watches.
    fn tell(&self, live: Live) {
        let mut all = self.live.lock().unwrap_or_else(|p| p.into_inner());
        let run = live.run();
        if let Some(sender) = all.get(&run)
            && sender.send(live).is_err()
        {
            all.remove(&run);
        }
    }

    /// How many bytes a run keeps, counted once and then kept count of.
    fn bytes_of(&self, conn: &rusqlite::Connection, run: i64) -> anyhow::Result<i64> {
        let mut all = self.bytes.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(bytes) = all.get(&run) {
            return Ok(*bytes);
        }
        let bytes = conn.query_row(
            "SELECT total(length(CAST(body AS BLOB))) FROM run_events WHERE run = ?1",
            params![run],
            |r| r.get::<_, f64>(0),
        )? as i64;
        all.insert(run, bytes);
        Ok(bytes)
    }

    fn grew(&self, run: i64, by: i64) {
        let mut all = self.bytes.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(bytes) = all.get_mut(&run) {
            *bytes += by;
        }
    }

    /// Keep what `key` names of a run once. The same key again keeps what
    /// grew in its place, under the same number. The entry, when it changed.
    pub fn put(
        &self,
        run: i64,
        key: &str,
        at: i64,
        mut item: Item,
    ) -> anyhow::Result<Option<Entry>> {
        let mut cut = item.cut(ENTRY_CHARS);
        let conn = self.store.conn();
        let kept: Option<(i64, String)> = conn
            .query_row(
                "SELECT seq, body FROM run_events WHERE run = ?1 AND key = ?2",
                params![run, key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let bytes = self.bytes_of(&conn, run)?;
        // What was kept whole before the run filled up is not cut short when
        // it is told again; what was kept after is, however it grows.
        let full = bytes >= RUN_BYTES
            && match &kept {
                None => true,
                Some((seq, _)) => conn
                    .query_row(
                        "SELECT seq FROM run_events WHERE run = ?1 AND key = ?2",
                        params![run, FULL_KEY],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()?
                    .is_some_and(|filled| *seq > filled),
            };
        if full {
            cut |= item.cut(FULL_CHARS);
        }
        let body = serde_json::to_string(&item)?;
        let seq = match kept {
            Some((_, before)) if before == body => {
                // Told again when Famulen reads a run anew: what was being
                // written of it is over all the same.
                drop(conn);
                self.settled(run, &item);
                return Ok(None);
            }
            Some((seq, before)) => {
                let seq = seq as u64;
                conn.execute(
                    "UPDATE run_events SET body = ?3, at = ?4, cut = ?5 WHERE run = ?1 AND seq = ?2",
                    params![run, seq as i64, body, at, cut],
                )?;
                self.grew(run, body.len() as i64 - before.len() as i64);
                seq
            }
            None => {
                let mut next = next_seq(&conn, run)?;
                if full && !has_key(&conn, run, FULL_KEY)? {
                    self.grew(
                        run,
                        insert(&conn, run, next, FULL_KEY, at, &Item::Full, false)?,
                    );
                    self.tell(Live::Entry(Entry {
                        run,
                        seq: next,
                        at,
                        cut: false,
                        item: Item::Full,
                    }));
                    next += 1;
                }
                self.grew(run, insert(&conn, run, next, key, at, &item, cut)?);
                next
            }
        };
        drop(conn);
        self.settled(run, &item);
        let entry = Entry {
            run,
            seq,
            at,
            cut,
            item,
        };
        self.tell(Live::Entry(entry.clone()));
        Ok(Some(entry))
    }

    /// Tell of what is being written, without keeping it.
    pub fn part(&self, run: i64, at: i64, of: PartOf, text: String) {
        if text.is_empty() {
            return;
        }
        // Told while held, so that one who begins to watch meanwhile is told of
        // it once: in what is being written, or as it comes.
        let mut writing = self.writing.lock().unwrap_or_else(|p| p.into_inner());
        let all = writing.entry(run).or_default();
        match all.iter_mut().find(|w| w.of == of) {
            Some(w) => {
                w.text.push_str(&text);
                // No more than an entry keeps of it: the end of what a tool
                // puts out, the start of the rest.
                let over = w.text.chars().count().saturating_sub(ENTRY_CHARS);
                if over > 0 {
                    if matches!(of, PartOf::Output(_)) {
                        let at = w.text.char_indices().nth(over).map_or(0, |(i, _)| i);
                        w.text.drain(..at);
                    } else {
                        cut_text(&mut w.text, ENTRY_CHARS);
                    }
                }
            }
            None => all.push(Writing {
                at,
                of: of.clone(),
                text: text.clone(),
            }),
        }
        self.tell(Live::Part { run, at, of, text });
    }

    /// Begin to watch: what comes from now on, and what a run is writing now,
    /// each piece whole so far.
    pub fn watch(&self, run: i64) -> (broadcast::Receiver<Live>, Vec<Live>) {
        let writing = self.writing.lock().unwrap_or_else(|p| p.into_inner());
        let live = {
            let mut all = self.live.lock().unwrap_or_else(|p| p.into_inner());
            // Runs nobody watches any more are told nothing.
            all.retain(|_, sender| sender.receiver_count() > 0);
            all.entry(run)
                .or_insert_with(|| broadcast::Sender::new(LIVE_ROOM))
                .subscribe()
        };
        let now = writing
            .get(&run)
            .into_iter()
            .flatten()
            .map(|w| Live::Part {
                run,
                at: w.at,
                of: w.of.clone(),
                text: w.text.clone(),
            })
            .collect();
        (live, now)
    }

    /// Forget all a run was writing: its process went away without a word.
    pub fn forget_writing(&self, run: i64) {
        self.writing
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&run);
        let mut live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        if live.get(&run).is_some_and(|s| s.receiver_count() == 0) {
            live.remove(&run);
        }
    }

    /// Forget what was being written that `item` holds whole; all of it once
    /// the run's process is gone.
    fn settled(&self, run: i64, item: &Item) {
        let done = match item {
            Item::Said { message, .. } => Some(PartOf::Said(message.clone())),
            Item::Thought { message, .. } => Some(PartOf::Thought(message.clone())),
            Item::Called { call, .. } => Some(PartOf::Input(call.clone())),
            Item::Answered { call, .. } => Some(PartOf::Output(call.clone())),
            Item::Ended { .. } | Item::TurnEnded { .. } => None,
            _ => return,
        };
        let mut writing = self.writing.lock().unwrap_or_else(|p| p.into_inner());
        match done {
            Some(of) => {
                if let Some(all) = writing.get_mut(&run) {
                    all.retain(|w| w.of != of);
                }
            }
            None => {
                writing.remove(&run);
            }
        }
    }

    /// The entries of a run after `after`, oldest first, at most `limit`.
    pub fn since(&self, run: i64, after: u64, limit: usize) -> anyhow::Result<Vec<Entry>> {
        let conn = self.store.conn();
        let mut statement = conn.prepare(
            "SELECT seq, at, cut, body FROM run_events WHERE run = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
        )?;
        let rows = statement
            .query_map(params![run, after as i64, limit as i64], |r| {
                Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, i64>(1)?,
                    r.get::<_, bool>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            // One that cannot be read keeps its place, so that what comes after
            // is read too.
            .map(|(seq, at, cut, body)| {
                let item = serde_json::from_str(&body).unwrap_or_else(|error| Item::Warned {
                    what: format!("an entry that cannot be read: {error}"),
                });
                Entry {
                    run,
                    seq,
                    at,
                    cut,
                    item,
                }
            })
            .collect())
    }

    /// What a run wrote last under `key`, or a key made from it with
    /// [`fresh_key`], with that key.
    pub fn last_said_under(&self, run: i64, key: &str) -> anyhow::Result<Option<(String, String)>> {
        let kept: Option<(String, String)> = self
            .store
            .conn()
            .query_row(
                "SELECT key, body FROM run_events
                 WHERE run = ?1 AND kind = 'said' AND (key = ?2 OR substr(key, 1, length(?2) + 1) = ?2 || ':')
                 ORDER BY seq DESC LIMIT 1",
                params![run, key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(
            kept.and_then(|(key, body)| match serde_json::from_str(&body).ok()? {
                Item::Said { text, .. } => Some((key, text)),
                _ => None,
            }),
        )
    }

    /// The text the run wrote last, if any.
    pub fn last_said(&self, run: i64) -> anyhow::Result<Option<String>> {
        let body: Option<String> = self
            .store
            .conn()
            .query_row(
                "SELECT body FROM run_events WHERE run = ?1 AND kind = 'said' ORDER BY seq DESC LIMIT 1",
                params![run],
                |r| r.get(0),
            )
            .optional()?;
        Ok(body
            .and_then(|b| serde_json::from_str::<Item>(&b).ok())
            .and_then(|item| match item {
                Item::Said { text, .. } => Some(text),
                _ => None,
            }))
    }

    /// The plan of a run as it stands, from where it is best trusted, and
    /// how its steps changed.
    pub fn plan(&self, run: i64) -> anyhow::Result<Plan> {
        let plans = self.plans(run)?;
        let Some(from) = plans.iter().map(|p| p.from).max() else {
            return Ok(Plan::default());
        };
        let mut plan = Plan {
            from: Some(from),
            ..Plan::default()
        };
        for KeptPlan { seq, at, steps, .. } in plans.into_iter().filter(|p| p.from == from) {
            plan.changes
                .extend(
                    changes(&plan.steps, &steps)
                        .into_iter()
                        .map(|(step, title, change)| StepChange {
                            seq,
                            at,
                            step,
                            title,
                            change,
                        }),
                );
            plan.steps = steps;
        }
        Ok(plan)
    }

    /// The plan a run follows now, without how it came to: read for every run
    /// in a list.
    pub fn current(&self, run: i64) -> anyhow::Result<(Option<PlanFrom>, Vec<Step>)> {
        let body: Option<String> = self
            .store
            .conn()
            .query_row(
                "SELECT body FROM run_events WHERE run = ?1 AND kind = 'plan'
                 ORDER BY CASE json_extract(body, '$.from')
                     WHEN 'timeline' THEN 3 WHEN 'adroit' THEN 2 ELSE 1 END DESC, seq DESC
                 LIMIT 1",
                params![run],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match body.and_then(|b| serde_json::from_str(&b).ok()) {
            Some(Item::Plan { from, steps }) => (Some(from), steps),
            _ => (None, Vec::new()),
        })
    }

    /// The newest plan from `from`, as it was kept.
    pub fn plan_from(&self, run: i64, from: PlanFrom) -> anyhow::Result<Vec<Step>> {
        let body: Option<String> = self
            .store
            .conn()
            .query_row(
                "SELECT body FROM run_events WHERE run = ?1 AND kind = 'plan' AND json_extract(body, '$.from') = ?2
                 ORDER BY seq DESC LIMIT 1",
                params![run, from.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match body.and_then(|b| serde_json::from_str(&b).ok()) {
            Some(Item::Plan { steps, .. }) => steps,
            _ => Vec::new(),
        })
    }

    /// Every plan of a run, oldest first.
    fn plans(&self, run: i64) -> anyhow::Result<Vec<KeptPlan>> {
        let conn = self.store.conn();
        let mut statement = conn.prepare(
            "SELECT seq, at, body FROM run_events WHERE run = ?1 AND kind = 'plan' ORDER BY seq",
        )?;
        let rows = statement
            .query_map(params![run], |r| {
                Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(seq, at, body)| match serde_json::from_str(&body).ok()? {
                Item::Plan { from, steps } => Some(KeptPlan {
                    seq,
                    at,
                    from,
                    steps,
                }),
                _ => None,
            })
            .collect())
    }

    /// Change the plan a run keeps with the timeline tool, one change at a
    /// time whoever asks: two at once would each start from what the other
    /// leaves out. The plan as it is after, and what `change` tells besides.
    pub fn change_timeline<T>(
        &self,
        run: i64,
        change: impl FnOnce(Vec<Step>) -> (Vec<Step>, T),
    ) -> anyhow::Result<(Vec<Step>, T)> {
        let _one = self.timeline.lock().unwrap_or_else(|p| p.into_inner());
        let before = self.plan_from(run, PlanFrom::Timeline)?;
        let (steps, told) = change(before.clone());
        if steps != before {
            let item = Item::Plan {
                from: PlanFrom::Timeline,
                steps: steps.clone(),
            };
            self.put(
                run,
                &fresh_key("timeline"),
                chrono::Utc::now().timestamp_millis(),
                item,
            )?;
        }
        Ok((steps, told))
    }

    /// Forget what runs that ended more than [`KEEP_DAYS`] before `now` did,
    /// but for their plans. How many entries went.
    pub fn forget_old(&self, now: i64) -> anyhow::Result<usize> {
        let before = now - KEEP_DAYS * 24 * 60 * 60;
        let conn = self.store.conn();
        // Marked first, after all it kept, so that no number is given twice;
        // what is read of the run again, as from an Adroit session that is
        // still there, is not kept again.
        conn.execute(
            "INSERT OR IGNORE INTO run_events (run, seq, key, at, kind, body)
             SELECT id, (SELECT coalesce(max(seq), 0) + 1 FROM run_events e WHERE e.run = worker_runs.id),
                    ?2, ?3, 'forgotten', ?4
             FROM worker_runs WHERE state = 'ended' AND ended_at IS NOT NULL AND ended_at < ?1",
            params![before, FORGOTTEN_KEY, now * 1000, serde_json::to_string(&Item::Forgotten)?],
        )?;
        let gone = conn.execute(
            "DELETE FROM run_events WHERE kind NOT IN ('plan', 'forgotten') AND run IN (
                 SELECT id FROM worker_runs WHERE state = 'ended' AND ended_at IS NOT NULL AND ended_at < ?1
             )",
            params![before],
        )?;
        // Counted anew, under the same hold as what went.
        self.bytes.lock().unwrap_or_else(|p| p.into_inner()).clear();
        Ok(gone)
    }

    /// Whether what a run did was forgotten for its age.
    pub fn forgotten(&self, run: i64) -> anyhow::Result<bool> {
        has_key(&self.store.conn(), run, FORGOTTEN_KEY)
    }
}

const FULL_KEY: &str = "full";
const FORGOTTEN_KEY: &str = "forgotten";

/// A key no other entry takes, for what is kept anew each time it changes.
pub fn fresh_key(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let at = chrono::Utc::now().timestamp_millis();
    format!("{prefix}:{at}-{}", COUNT.fetch_add(1, Ordering::Relaxed))
}

fn next_seq(conn: &rusqlite::Connection, run: i64) -> anyhow::Result<u64> {
    Ok(conn.query_row(
        "SELECT coalesce(max(seq), 0) + 1 FROM run_events WHERE run = ?1",
        params![run],
        |r| r.get::<_, i64>(0),
    )? as u64)
}

fn has_key(conn: &rusqlite::Connection, run: i64, key: &str) -> anyhow::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM run_events WHERE run = ?1 AND key = ?2",
            params![run, key],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn insert(
    conn: &rusqlite::Connection,
    run: i64,
    seq: u64,
    key: &str,
    at: i64,
    item: &Item,
    cut: bool,
) -> anyhow::Result<i64> {
    let body = serde_json::to_string(item)?;
    conn.execute(
        "INSERT INTO run_events (run, seq, key, at, kind, cut, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![run, seq as i64, key, at, item.kind(), cut, body],
    )?;
    Ok(body.len() as i64)
}

/// How each step changed from `before` to `after`, by its id.
fn changes(before: &[Step], after: &[Step]) -> Vec<(String, String, Change)> {
    let was: BTreeMap<&str, &Step> = before.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut all = Vec::new();
    for step in after {
        let change = |change| (step.id.clone(), step.title.clone(), change);
        match was.get(step.id.as_str()) {
            None => {
                all.push(change(Change::Added));
                if step.status != "pending" && !step.status.is_empty() {
                    all.push(change(Change::Status {
                        from: String::new(),
                        to: step.status.clone(),
                    }));
                }
            }
            Some(old) => {
                if old.status != step.status {
                    all.push(change(Change::Status {
                        from: old.status.clone(),
                        to: step.status.clone(),
                    }));
                }
                if old.title != step.title {
                    all.push(change(Change::Title));
                }
                if old.note != step.note && !step.note.is_empty() {
                    all.push(change(Change::Note));
                }
            }
        }
    }
    for old in before {
        if !after.iter().any(|s| s.id == old.id) {
            all.push((old.id.clone(), old.title.clone(), Change::Removed));
        }
    }
    all
}

#[cfg(test)]
mod tests;
