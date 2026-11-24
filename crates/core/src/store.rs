//! The database: every input as a row written before it reaches the session,
//! and the tables the rest of Famulen keeps its state in.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::input::Input;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Waiting for the session.
    Pending,
    /// Given to the session; its turn has not ended. At most one input is.
    Running,
    /// Its turn ended.
    Done,
    /// Its turn was cut off while a call to the outside had not finished;
    /// it waits for the owner to say whether to go on.
    Held,
    /// The owner dropped it while it was held.
    Failed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Running => "running",
            Status::Done => "done",
            Status::Held => "held",
            Status::Failed => "failed",
        }
    }

    fn parse(s: &str) -> anyhow::Result<Self> {
        Ok(match s {
            "pending" => Status::Pending,
            "running" => Status::Running,
            "done" => Status::Done,
            "held" => Status::Held,
            "failed" => Status::Failed,
            other => anyhow::bail!("unknown input status: {other}"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredInput {
    pub id: i64,
    pub input: Input,
    pub status: Status,
    /// Its turn was cut off once before it ended.
    pub interrupted: bool,
}

/// The turn that is running: its input and the workgroup it runs under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub input: i64,
    pub workgroup: String,
}

/// A handle to the database. Clones share one connection.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> anyhow::Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Record an input. Returns `None` when the same delivery was already kept.
    pub fn enqueue(&self, input: &Input) -> anyhow::Result<Option<i64>> {
        let now = now();
        let conn = self.conn();
        let changed = conn.execute(
            "INSERT INTO inputs (dedup_key, sender, body, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT (dedup_key) DO NOTHING",
            params![
                input.dedup_key(),
                input.sender.as_str(),
                serde_json::to_string(input)?,
                Status::Pending.as_str(),
                now
            ],
        )?;
        Ok((changed == 1).then(|| conn.last_insert_rowid()))
    }

    /// The oldest input that has not reached the session yet.
    pub fn next_pending(&self) -> anyhow::Result<Option<StoredInput>> {
        self.query_input(
            "SELECT id, body, status, interrupted FROM inputs WHERE status = ?1 ORDER BY id LIMIT 1",
            params![Status::Pending.as_str()],
        )
    }

    /// Let an input no turn has taken wait no more.
    pub fn drop_pending(&self, id: i64) -> anyhow::Result<bool> {
        let changed = self.conn().execute(
            "UPDATE inputs SET status = ?1, updated_at = ?2 WHERE id = ?3 AND status = ?4",
            params![Status::Done.as_str(), now(), id, Status::Pending.as_str()],
        )?;
        Ok(changed == 1)
    }

    /// The input kept for a delivery.
    pub fn id_of(&self, dedup_key: &str) -> anyhow::Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id FROM inputs WHERE dedup_key = ?1",
                params![dedup_key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn get(&self, id: i64) -> anyhow::Result<Option<StoredInput>> {
        self.query_input(
            "SELECT id, body, status, interrupted FROM inputs WHERE id = ?1",
            params![id],
        )
    }

    fn query_input(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> anyhow::Result<Option<StoredInput>> {
        let row = self
            .conn()
            .query_row(sql, params, |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? != 0,
                ))
            })
            .optional()?;
        row.map(|(id, body, status, interrupted)| {
            Ok(StoredInput {
                id,
                input: serde_json::from_str(&body)?,
                status: Status::parse(&status)?,
                interrupted,
            })
        })
        .transpose()
    }

    /// The input of the running turn.
    pub fn running_input(&self) -> anyhow::Result<Option<StoredInput>> {
        match self.running()? {
            Some(turn) => self.get(turn.input),
            None => Ok(None),
        }
    }

    /// The newest inputs first, with when they arrived: only those older than
    /// `before` when it is not 0.
    pub fn recent_inputs(
        &self,
        before: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<(StoredInput, i64)>> {
        let rows: Vec<(i64, String, String, i64, bool)> = {
            let conn = self.conn();
            let mut statement = conn.prepare(
                "SELECT id, body, status, created_at, interrupted FROM inputs WHERE ?1 = 0 OR id < ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            statement
                .query_map(params![before, limit as i64], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get::<_, i64>(4)? != 0,
                    ))
                })?
                .collect::<Result<_, _>>()?
        };
        rows.into_iter()
            .map(|(id, body, status, created_at, interrupted)| {
                let input = serde_json::from_str(&body)?;
                Ok((
                    StoredInput {
                        id,
                        input,
                        status: Status::parse(&status)?,
                        interrupted,
                    },
                    created_at,
                ))
            })
            .collect()
    }

    pub fn count_pending(&self) -> anyhow::Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM inputs WHERE status = ?1",
            params![Status::Pending.as_str()],
            |row| row.get(0),
        )?)
    }

    /// Start the turn of a pending input under `workgroup`. Refused (false) while
    /// another turn runs: the database lets only one input run, so two
    /// callers cannot both start one.
    pub fn start_turn(&self, id: i64, workgroup: impl AsRef<str>) -> anyhow::Result<bool> {
        let result = self.conn().execute(
            "UPDATE inputs SET status = 'running', workgroup = ?2, updated_at = ?3 WHERE id = ?1 AND status = 'pending'",
            params![id, workgroup.as_ref(), now()],
        );
        match result {
            Ok(changed) => Ok(changed == 1),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(false)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The input could not be handed over after all: it waits again.
    pub fn undo_start(&self, id: i64) -> anyhow::Result<()> {
        self.conn().execute(
            "UPDATE inputs SET status = 'pending', updated_at = ?2 WHERE id = ?1 AND status = 'running'",
            params![id, now()],
        )?;
        Ok(())
    }

    /// The running turn ended. Returns its input.
    pub fn end_turn(&self) -> anyhow::Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "UPDATE inputs SET status = 'done', updated_at = ?1 WHERE status = 'running' RETURNING id",
                params![now()],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The running turn was cut off before it ended (the process stopped):
    /// its input waits again, marked so its next turn knows. Returns it.
    pub fn interrupt_turn(&self) -> anyhow::Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "UPDATE inputs SET status = 'pending', interrupted = 1, updated_at = ?1 WHERE status = 'running' RETURNING id",
                params![now()],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The running turn was cut off while a call it made to the outside may
    /// still land: its input is held for the owner instead of given again.
    pub fn hold_turn(&self) -> anyhow::Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "UPDATE inputs SET status = 'held', interrupted = 1, updated_at = ?1 WHERE status = 'running' RETURNING id",
                params![now()],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The inputs held for the owner, oldest first.
    pub fn held(&self) -> anyhow::Result<Vec<StoredInput>> {
        let ids: Vec<i64> = {
            let conn = self.conn();
            let mut stmt =
                conn.prepare("SELECT id FROM inputs WHERE status = 'held' ORDER BY id")?;
            stmt.query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        ids.into_iter()
            .filter_map(|id| self.get(id).transpose())
            .collect()
    }

    /// The owner's answer to a held input: go on (it waits for its turn
    /// again) or drop it. False when it was not held.
    pub fn answer_held(&self, id: i64, go_on: bool) -> anyhow::Result<bool> {
        let (status, error) = if go_on {
            ("pending", None)
        } else {
            ("failed", Some("主人がやめた"))
        };
        Ok(self.conn().execute(
            "UPDATE inputs SET status = ?2, error = ?3, updated_at = ?4 WHERE id = ?1 AND status = 'held'",
            params![id, status, error, now()],
        )? == 1)
    }

    pub fn running(&self) -> anyhow::Result<Option<Turn>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, workgroup FROM inputs WHERE status = 'running'",
                [],
                |r| {
                    Ok(Turn {
                        input: r.get(0)?,
                        workgroup: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    })
                },
            )
            .optional()?)
    }

    pub fn state(&self, key: impl AsRef<str>) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT value FROM state WHERE key = ?1",
                params![key.as_ref()],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn clear_state(&self, key: impl AsRef<str>) -> anyhow::Result<()> {
        self.conn()
            .execute("DELETE FROM state WHERE key = ?1", params![key.as_ref()])?;
        Ok(())
    }

    pub fn set_state(&self, key: impl AsRef<str>, value: impl AsRef<str>) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO state (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key.as_ref(), value.as_ref()],
        )?;
        Ok(())
    }
}

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
CREATE TABLE IF NOT EXISTS inputs (
    id          INTEGER PRIMARY KEY,
    dedup_key   TEXT NOT NULL UNIQUE,
    sender      TEXT NOT NULL,
    body        TEXT NOT NULL,
    status      TEXT NOT NULL,
    error       TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    workgroup   TEXT,
    interrupted INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS inputs_status ON inputs (status, id);
CREATE UNIQUE INDEX IF NOT EXISTS inputs_one_running ON inputs (status) WHERE status = 'running';
CREATE TABLE IF NOT EXISTS state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tools (
    name    TEXT PRIMARY KEY,
    seen_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS approvals (
    id          INTEGER PRIMARY KEY,
    tool        TEXT NOT NULL,
    input       TEXT NOT NULL,
    status      TEXT NOT NULL,
    channel     TEXT,
    ts          TEXT,
    created_at  INTEGER NOT NULL,
    answered_at INTEGER
);
CREATE TABLE IF NOT EXISTS wakeups (
    id    INTEGER PRIMARY KEY,
    at    INTEGER NOT NULL,
    note  TEXT NOT NULL,
    fired INTEGER NOT NULL DEFAULT 0,
    why   TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS journal (
    id         INTEGER PRIMARY KEY,
    kind       TEXT NOT NULL,
    detail     TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS workgroup_members (
    workgroup TEXT NOT NULL,
    kind      TEXT NOT NULL,
    ref       TEXT NOT NULL,
    PRIMARY KEY (workgroup, kind, ref)
);
CREATE TABLE IF NOT EXISTS workgroup_accounts (
    workgroup  TEXT NOT NULL,
    purpose    TEXT NOT NULL,
    connection TEXT NOT NULL,
    PRIMARY KEY (workgroup, purpose)
);
-- When each Google calendar was last read, and why it could not be.
CREATE TABLE IF NOT EXISTS calendar_reads (
    calendar   TEXT PRIMARY KEY,
    fetched_at INTEGER,
    error      TEXT
);
-- What was read from Google, so that showing it does not wait.
CREATE TABLE IF NOT EXISTS calendar_cache (
    calendar TEXT NOT NULL,
    uid      TEXT NOT NULL,
    start    INTEGER NOT NULL,
    end      INTEGER NOT NULL,
    event    TEXT NOT NULL,
    PRIMARY KEY (calendar, uid, start)
);
-- When each routine runs next, worked out for the rule it had.
CREATE TABLE IF NOT EXISTS routine_runs (
    routine  TEXT PRIMARY KEY,
    rule     TEXT NOT NULL,
    next_run INTEGER,
    last_run INTEGER,
    -- what the last run made: a task's id, or `input:` and the key of the
    -- secretary's input
    task     TEXT
);
CREATE TABLE IF NOT EXISTS models (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    spec        TEXT NOT NULL,
    monthly_yen REAL
);
-- Models found on this machine, and the ones the owner removed.
CREATE TABLE IF NOT EXISTS found_models (
    id      TEXT PRIMARY KEY,
    source  TEXT NOT NULL,
    spec    TEXT NOT NULL,
    present INTEGER NOT NULL DEFAULT 1,
    removed INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS roles (
    role     TEXT PRIMARY KEY,
    model    TEXT NOT NULL,
    cap      REAL,
    cap_unit TEXT NOT NULL DEFAULT 'calls',
    effort   TEXT
);
CREATE TABLE IF NOT EXISTS usage (
    id            INTEGER PRIMARY KEY,
    at            INTEGER NOT NULL,
    role          TEXT NOT NULL,
    model         TEXT NOT NULL,
    input_tokens  INTEGER,
    output_tokens INTEGER,
    calls         INTEGER NOT NULL,
    cost_yen      REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS console_messages (
    id    INTEGER PRIMARY KEY,
    at    INTEGER NOT NULL,
    who   TEXT NOT NULL,
    text  TEXT NOT NULL,
    meta  TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE IF NOT EXISTS lookout_sources (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL,
    connection TEXT NOT NULL,
    tool       TEXT NOT NULL,
    args       TEXT NOT NULL DEFAULT '{}',
    -- mail, chat or other: which default interval it follows
    feed       TEXT NOT NULL DEFAULT 'other',
    -- the interval the owner chose for it alone, in seconds
    interval   INTEGER,
    -- where the owner sends what concerns them; the model decides when none
    route      TEXT,
    enabled    INTEGER NOT NULL DEFAULT 1,
    last_run   INTEGER
);
CREATE TABLE IF NOT EXISTS lookout_items (
    id        INTEGER PRIMARY KEY,
    source    TEXT NOT NULL,
    dedup_key TEXT NOT NULL UNIQUE,
    at        INTEGER NOT NULL,
    title     TEXT NOT NULL,
    body      TEXT NOT NULL,
    outcome   TEXT NOT NULL,
    reason    TEXT NOT NULL DEFAULT '',
    feedback  TEXT
);
CREATE TABLE IF NOT EXISTS triage_items (
    id        INTEGER PRIMARY KEY,
    source    TEXT NOT NULL,
    dedup_key TEXT NOT NULL UNIQUE,
    at        INTEGER NOT NULL,
    title     TEXT NOT NULL,
    addressed INTEGER NOT NULL,
    reason    TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS lookout_heard (
    id     INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    title  TEXT NOT NULL,
    body   TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS slack_taken (
    workspace TEXT NOT NULL,
    channel   TEXT NOT NULL,
    ts        TEXT NOT NULL,
    PRIMARY KEY (workspace, channel, ts)
);
CREATE TABLE IF NOT EXISTS dreams (
    id        INTEGER PRIMARY KEY,
    at        INTEGER NOT NULL,
    commit_id TEXT,
    status    TEXT NOT NULL,
    summary   TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS record_looks (
    kind        TEXT NOT NULL,
    id          TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    looked_on   TEXT NOT NULL,
    verdict     TEXT NOT NULL,
    PRIMARY KEY (kind, id)
);
";

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Sender, Source};

    fn input(ts: &str) -> Input {
        Input {
            sender: Sender::Owner,
            source: Source::Slack {
                workspace: "w1".into(),
                channel: "D1".into(),
                ts: ts.into(),
                thread_ts: None,
                user: "U1".into(),
                unmentioned: false,
            },
            text: format!("message {ts}"),
        }
    }

    #[test]
    fn an_input_waits_as_pending_until_it_is_delivered() {
        let store = Store::open_in_memory().unwrap();
        let id = store.enqueue(&input("1")).unwrap().unwrap();
        let pending = store.next_pending().unwrap().unwrap();
        assert_eq!((pending.id, pending.status), (id, Status::Pending));
        assert_eq!(pending.input, input("1"));

        assert!(store.start_turn(id, "work").unwrap());
        assert_eq!(store.next_pending().unwrap(), None);
        assert_eq!(store.get(id).unwrap().unwrap().status, Status::Running);
        assert_eq!(store.end_turn().unwrap(), Some(id));
        assert_eq!(store.get(id).unwrap().unwrap().status, Status::Done);
    }

    #[test]
    fn a_slack_input_kept_before_unmentioned_was_marked_reads_as_mentioned() {
        let store = Store::open_in_memory().unwrap();
        let id = store.enqueue(&input("1")).unwrap().unwrap();
        store
            .conn()
            .execute(
                "UPDATE inputs SET body = json_remove(body, '$.source.unmentioned')",
                [],
            )
            .unwrap();

        assert_eq!(store.get(id).unwrap().unwrap().input, input("1"));
        assert_eq!(store.recent_inputs(0, 10).unwrap()[0].0.input, input("1"));
    }

    #[test]
    fn an_arrival_kept_before_it_said_what_it_is_about_reads_as_about_nothing() {
        let store = Store::open_in_memory().unwrap();
        let arrival = Input {
            sender: Sender::Other,
            source: Source::Arrival {
                connection: "adroit".into(),
                receiver: "asks".into(),
                id: "1".into(),
                about: String::new(),
            },
            text: "asks".into(),
        };
        store.enqueue(&arrival).unwrap();
        store
            .conn()
            .execute(
                "UPDATE inputs SET body = json_remove(body, '$.source.about')",
                [],
            )
            .unwrap();

        assert_eq!(store.recent_inputs(0, 10).unwrap()[0].0.input, arrival);
    }

    #[test]
    fn only_one_turn_runs_and_it_carries_its_workgroup() {
        let store = Store::open_in_memory().unwrap();
        let a = store.enqueue(&input("a")).unwrap().unwrap();
        let b = store.enqueue(&input("b")).unwrap().unwrap();
        assert!(store.start_turn(a, "work").unwrap());
        assert!(
            !store.start_turn(b, "personal").unwrap(),
            "a second turn is refused"
        );
        assert_eq!(
            store.running().unwrap(),
            Some(Turn {
                input: a,
                workgroup: "work".into()
            })
        );
        store.end_turn().unwrap();
        assert!(store.start_turn(b, "personal").unwrap());
        assert!(
            !store.start_turn(b, "work").unwrap(),
            "a running input is not started again"
        );
    }

    #[test]
    fn a_turn_cut_off_puts_its_input_back_marked_and_a_failed_hand_over_unmarked() {
        let store = Store::open_in_memory().unwrap();
        let a = store.enqueue(&input("a")).unwrap().unwrap();
        store.start_turn(a, "work").unwrap();
        store.undo_start(a).unwrap();
        assert!(!store.next_pending().unwrap().unwrap().interrupted);
        store.start_turn(a, "work").unwrap();
        assert_eq!(store.interrupt_turn().unwrap(), Some(a));
        let again = store.next_pending().unwrap().unwrap();
        assert_eq!(again.id, a);
        assert!(again.interrupted);
        assert_eq!(store.running().unwrap(), None);
        assert_eq!(store.interrupt_turn().unwrap(), None);
    }

    #[test]
    fn a_held_input_waits_for_the_owner_and_then_goes_on_or_is_dropped() {
        let store = Store::open_in_memory().unwrap();
        let a = store.enqueue(&input("a")).unwrap().unwrap();
        let b = store.enqueue(&input("b")).unwrap().unwrap();
        store.start_turn(a, "work").unwrap();
        assert_eq!(store.hold_turn().unwrap(), Some(a));
        assert_eq!(
            store.next_pending().unwrap().unwrap().id,
            b,
            "a held input is not given again by itself"
        );
        assert_eq!(store.held().unwrap()[0].id, a);
        assert!(store.answer_held(a, true).unwrap());
        assert!(!store.answer_held(a, true).unwrap(), "answered once");
        assert_eq!(store.get(a).unwrap().unwrap().status, Status::Pending);
        store.start_turn(b, "work").unwrap();
        store.hold_turn().unwrap();
        assert!(store.answer_held(b, false).unwrap());
        assert_eq!(store.get(b).unwrap().unwrap().status, Status::Failed);
    }

    #[test]
    fn inputs_arriving_together_each_get_their_own_id() {
        let store = Store::open_in_memory().unwrap();
        std::thread::scope(|scope| {
            for thread in 0..8 {
                let store = store.clone();
                scope.spawn(move || {
                    for n in 0..200 {
                        let input = input(&format!("{thread}.{n}"));
                        let id = store.enqueue(&input).unwrap().unwrap();
                        assert_eq!(store.get(id).unwrap().unwrap().input, input);
                    }
                });
            }
        });
    }

    #[test]
    fn inputs_are_delivered_in_the_order_they_arrived() {
        let store = Store::open_in_memory().unwrap();
        let first = store.enqueue(&input("1")).unwrap().unwrap();
        store.enqueue(&input("2")).unwrap().unwrap();
        assert_eq!(store.next_pending().unwrap().unwrap().id, first);
    }

    #[test]
    fn recent_inputs_come_newest_first_and_pending_ones_are_counted() {
        let store = Store::open_in_memory().unwrap();
        let first = store.enqueue(&input("1")).unwrap().unwrap();
        let second = store.enqueue(&input("2")).unwrap().unwrap();
        store.start_turn(first, "all").unwrap();
        store.end_turn().unwrap();
        let recent: Vec<i64> = store
            .recent_inputs(0, 10)
            .unwrap()
            .into_iter()
            .map(|(row, _)| row.id)
            .collect();
        assert_eq!(recent, [second, first]);
        assert_eq!(store.count_pending().unwrap(), 1);
    }

    #[test]
    fn recent_inputs_stop_at_the_limit_keeping_the_newest() {
        let store = Store::open_in_memory().unwrap();
        let ids: Vec<i64> = (1..=4)
            .map(|n| store.enqueue(&input(&n.to_string())).unwrap().unwrap())
            .collect();
        let recent: Vec<i64> = store
            .recent_inputs(0, 2)
            .unwrap()
            .into_iter()
            .map(|(row, _)| row.id)
            .collect();
        assert_eq!(recent, [ids[3], ids[2]]);
    }

    #[test]
    fn recent_inputs_before_an_id_are_the_older_ones_only() {
        let store = Store::open_in_memory().unwrap();
        let ids: Vec<i64> = (1..=4)
            .map(|n| store.enqueue(&input(&n.to_string())).unwrap().unwrap())
            .collect();
        let older: Vec<i64> = store
            .recent_inputs(ids[2], 10)
            .unwrap()
            .into_iter()
            .map(|(row, _)| row.id)
            .collect();
        assert_eq!(older, [ids[1], ids[0]]);
    }

    #[test]
    fn a_resent_delivery_is_kept_once() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.enqueue(&input("1")).unwrap().is_some());
        assert_eq!(store.enqueue(&input("1")).unwrap(), None);
    }

    #[test]
    fn state_is_overwritten_in_place() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.state("session_id").unwrap(), None);
        store.set_state("session_id", "a").unwrap();
        store.set_state("session_id", "b").unwrap();
        assert_eq!(store.state("session_id").unwrap().as_deref(), Some("b"));
    }

    #[test]
    fn pending_inputs_survive_reopening_the_database() {
        let dir = std::env::temp_dir().join(format!("famulen-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite3");
        let _ = std::fs::remove_file(&path);
        let id = Store::open(&path)
            .unwrap()
            .enqueue(&input("1"))
            .unwrap()
            .unwrap();
        assert_eq!(
            Store::open(&path)
                .unwrap()
                .next_pending()
                .unwrap()
                .unwrap()
                .id,
            id
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
