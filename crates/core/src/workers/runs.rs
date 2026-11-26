//! The runs of delegated tasks. A run's row is written before the run
//! starts, so a restart neither loses nor doubles it.

use rusqlite::{OptionalExtension, Row, params};

use super::order::Order;
use crate::store::{Store, now};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Meant to start; whether it did is not known until told.
    Starting,
    /// Its start waits for the owner's approval.
    Asking,
    Running,
    /// The Adroit session waits for the owner.
    Waiting,
    /// The Adroit session ended its turn with tasks of its plan unfinished: it
    /// waits for something to go on with, or for someone to look.
    Idle,
    /// Adroit cannot be asked how it stands, or whether it started.
    Unreachable,
    /// It came to an end that is not yet written to the task and told.
    Ending,
    Ended,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Starting => "starting",
            State::Asking => "asking",
            State::Running => "running",
            State::Waiting => "waiting",
            State::Idle => "idle",
            State::Unreachable => "unreachable",
            State::Ending => "ending",
            State::Ended => "ended",
        }
    }

    fn parse(text: &str) -> Self {
        [
            State::Starting,
            State::Asking,
            State::Running,
            State::Waiting,
            State::Idle,
            State::Unreachable,
            State::Ending,
        ]
        .into_iter()
        .find(|s| s.as_str() == text)
        .unwrap_or(State::Ended)
    }

    /// Whether a run in this state takes one of the slots. One that waits on
    /// the owner does not, however long they take, nor one whose turn ended
    /// with its plan unfinished, nor one that cannot be seen while Adroit is
    /// away.
    pub fn takes_slot(self) -> bool {
        matches!(self, State::Starting | State::Running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Failed,
    /// Its session was closed before every task of its plan was completed:
    /// whether its work was done is for someone to check.
    Unsure,
    /// The owner did not approve its start.
    Refused,
    /// Its task was cancelled, taken back or removed.
    Stopped,
    /// Cut off by a restart, to be run once more.
    Interrupted,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::Failed => "failed",
            Outcome::Unsure => "unsure",
            Outcome::Refused => "refused",
            Outcome::Stopped => "stopped",
            Outcome::Interrupted => "interrupted",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [
            Outcome::Done,
            Outcome::Failed,
            Outcome::Unsure,
            Outcome::Refused,
            Outcome::Stopped,
            Outcome::Interrupted,
        ]
        .into_iter()
        .find(|o| o.as_str() == text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub id: i64,
    /// The task's id.
    pub task: String,
    pub state: State,
    /// Which run of the task it is since one last came to an end.
    pub attempt: u32,
    /// How often its start was tried since the owner was last asked.
    pub tries: u32,
    /// How often the owner was asked of its start.
    pub asks: u32,
    /// Before this a failed start is not tried again.
    pub retry_at: i64,
    pub order: Order,
    /// The Adroit connection it was started through.
    pub connection: String,
    /// The Adroit session's id.
    pub session: String,
    /// The gateway's ticket its start waits on.
    pub ticket: Option<i64>,
    pub outcome: Option<Outcome>,
    /// What the worker answered, or why there is no answer.
    pub result: String,
    /// What last went wrong with its start.
    pub note: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

/// What became of a run after a turn of its session ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Again {
    /// How often it went on after it had ended.
    pub reopened: u32,
    /// How often it was asked to go on after a passing fault.
    pub continued: u32,
    /// Of these, how often in a row.
    pub in_a_row: u32,
    /// When it was last asked to go on, or 0.
    pub continued_at: i64,
    /// When it is asked next, or 0.
    pub due_at: i64,
}

const COLUMNS: &str = "id, task, state, attempt, tries, spec, session, ticket, outcome, result, note, created_at, started_at, ended_at, asks, retry_at, connection";

fn read(row: &Row<'_>) -> rusqlite::Result<Run> {
    let spec: String = row.get(5)?;
    Ok(Run {
        id: row.get(0)?,
        task: row.get(1)?,
        state: State::parse(&row.get::<_, String>(2)?),
        attempt: row.get(3)?,
        tries: row.get(4)?,
        order: serde_json::from_str(&spec).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, e.into())
        })?,
        session: row.get(6)?,
        ticket: row.get(7)?,
        outcome: Outcome::parse(&row.get::<_, String>(8)?),
        result: row.get(9)?,
        note: row.get(10)?,
        created_at: row.get(11)?,
        started_at: row.get(12)?,
        ended_at: row.get(13)?,
        asks: row.get(14)?,
        retry_at: row.get(15)?,
        connection: row.get(16)?,
    })
}

#[derive(Clone)]
pub struct Runs {
    store: Store,
}

impl Runs {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    fn select(&self, rest: &str) -> anyhow::Result<Vec<Run>> {
        let conn = self.store.conn();
        let mut statement = conn.prepare(&format!("SELECT {COLUMNS} FROM worker_runs {rest}"))?;
        let runs = statement
            .query_map([], read)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(runs)
    }

    /// The runs that are not over, oldest first.
    pub fn open(&self) -> anyhow::Result<Vec<Run>> {
        self.select("WHERE state != 'ended' ORDER BY id")
    }

    /// Every run by a session in Adroit, oldest first.
    pub fn on_adroit(&self) -> anyhow::Result<Vec<Run>> {
        self.select("WHERE runner = 'adroit' ORDER BY id")
    }

    /// The newest runs, the newest first.
    pub fn recent(&self, limit: usize) -> anyhow::Result<Vec<Run>> {
        self.select(&format!("ORDER BY id DESC LIMIT {limit}"))
    }

    /// The newest runs that ended, the newest first: those of one task, or
    /// of the tasks in one workgroup, when named.
    pub fn ended_of(
        &self,
        task: Option<&str>,
        workgroup: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<Run>> {
        let conn = self.store.conn();
        let mut statement = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM worker_runs
             WHERE state = 'ended' AND (?1 IS NULL OR task = ?1)
               AND (?2 IS NULL OR EXISTS
                   (SELECT 1 FROM json_each(spec, '$.workgroups') WHERE value = ?2))
             ORDER BY id DESC LIMIT {limit}"
        ))?;
        let runs = statement
            .query_map(params![task, workgroup], read)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(runs)
    }

    /// The newest run of each task.
    pub fn latest(&self) -> anyhow::Result<Vec<Run>> {
        self.select("WHERE id IN (SELECT MAX(id) FROM worker_runs GROUP BY task)")
    }

    pub fn get(&self, id: i64) -> anyhow::Result<Option<Run>> {
        Ok(self.select(&format!("WHERE id = {id}"))?.pop())
    }

    /// Write down that a task is meant to start. It fails when a run of the
    /// task is open.
    pub fn begin(&self, task: &str, order: &Order) -> anyhow::Result<Run> {
        let id = {
            let conn = self.store.conn();
            let before: Option<(u32, String)> = conn
                .query_row(
                    "SELECT attempt, outcome FROM worker_runs WHERE task = ?1 ORDER BY id DESC LIMIT 1",
                    [task],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let attempt = match before {
                Some((n, outcome)) if outcome == Outcome::Interrupted.as_str() => n + 1,
                _ => 1,
            };
            conn.execute(
                "INSERT INTO worker_runs (task, runner, state, attempt, spec, created_at)
                 VALUES (?1, ?2, 'starting', ?3, ?4, ?5)",
                params![
                    task,
                    order.runner.as_str(),
                    attempt,
                    serde_json::to_string(order)?,
                    now()
                ],
            )?;
            conn.last_insert_rowid()
        };
        self.get(id)?
            .ok_or_else(|| anyhow::anyhow!("run #{id} is gone"))
    }

    fn set(
        &self,
        id: i64,
        assignments: &str,
        values: &[&dyn rusqlite::ToSql],
    ) -> anyhow::Result<()> {
        let mut all: Vec<&dyn rusqlite::ToSql> = vec![&id];
        all.extend(values);
        self.store.conn().execute(
            &format!("UPDATE worker_runs SET {assignments} WHERE id = ?1"),
            &all[..],
        )?;
        Ok(())
    }

    /// Count a try at its start, before it is made.
    pub fn trying(&self, id: i64) -> anyhow::Result<()> {
        self.set(id, "state = 'starting', tries = tries + 1", &[])
    }

    pub fn asking(&self, id: i64, ticket: i64) -> anyhow::Result<()> {
        self.set(
            id,
            "state = 'asking', tries = 0, asks = asks + 1, retry_at = 0, ticket = ?2",
            &[&ticket],
        )
    }

    pub fn running(&self, id: i64, session: &str) -> anyhow::Result<()> {
        self.set(
            id,
            "state = 'running', session = ?2, note = '', started_at = COALESCE(started_at, ?3)",
            &[&session, &now()],
        )
    }

    /// The session waits for the owner, or went on.
    pub fn stand(&self, id: i64, state: State) -> anyhow::Result<()> {
        self.set(id, "state = ?2", &[&state.as_str()])
    }

    /// The session it waited in was answered. One that ended meanwhile stays so.
    pub fn answered(&self, id: i64) -> anyhow::Result<()> {
        self.store.conn().execute(
            "UPDATE worker_runs SET state = ?2 WHERE id = ?1 AND state = ?3",
            rusqlite::params![id, State::Running.as_str(), State::Waiting.as_str()],
        )?;
        Ok(())
    }

    /// What went wrong with its start, and when it is tried again.
    pub fn failed_try(&self, id: i64, note: &str, retry_at: i64) -> anyhow::Result<()> {
        self.set(id, "note = ?2, retry_at = ?3", &[&note, &retry_at])
    }

    /// Adroit could not be asked of it.
    pub fn unreachable(&self, id: i64, note: &str) -> anyhow::Result<()> {
        self.set(id, "state = 'unreachable', note = ?2", &[&note])
    }

    /// It did not start after all, and may be tried again.
    pub fn unstarted(&self, id: i64) -> anyhow::Result<()> {
        self.set(id, "state = 'starting', retry_at = 0", &[])
    }

    /// Keep which connection it is started through.
    pub fn through(&self, id: i64, connection: &str) -> anyhow::Result<()> {
        self.set(id, "connection = ?2", &[&connection])
    }

    /// Whether any run, over or not, was on this Adroit session; and whether
    /// one that is not over is.
    pub fn of_session(&self, session: &str) -> anyhow::Result<Option<bool>> {
        if session.is_empty() {
            return Ok(None);
        }
        let open: Option<bool> = self.store.conn().query_row(
            "SELECT MAX(state != 'ended') FROM worker_runs WHERE session = ?1",
            [session],
            |r| r.get(0),
        )?;
        Ok(open)
    }

    /// Keep who delegated a task and in which workgroup, unless what it orders
    /// is what was kept before. Returns whether it was kept.
    pub fn delegated(
        &self,
        task: &str,
        digest: &str,
        by: &str,
        workgroup: &str,
    ) -> anyhow::Result<bool> {
        let changed = self.store.conn().execute(
            "INSERT INTO delegations (task, digest, by, workgroup) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (task) DO UPDATE SET digest = ?2, by = ?3, workgroup = ?4
             WHERE digest != ?2",
            params![task, digest, by, workgroup],
        )?;
        Ok(changed == 1)
    }

    /// Who delegated a task as it is ordered by `digest`, and in which
    /// workgroup; `None` when that is not known.
    pub fn delegation(&self, task: &str, digest: &str) -> anyhow::Result<Option<(String, String)>> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT by, workgroup FROM delegations WHERE task = ?1 AND digest = ?2",
                params![task, digest],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// It came to an end, yet to be written to its task and told.
    pub fn ending(&self, id: i64, outcome: Outcome, result: &str) -> anyhow::Result<()> {
        self.set(
            id,
            "state = 'ending', outcome = ?2, result = ?3, ended_at = ?4",
            &[&outcome.as_str(), &result, &now()],
        )
    }

    pub fn ended(&self, id: i64) -> anyhow::Result<()> {
        self.set(id, "state = 'ended'", &[])
    }

    pub fn again(&self, id: i64) -> anyhow::Result<Again> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT reopened, continued, in_a_row, continued_at, due_at
                 FROM worker_again WHERE run = ?1",
                [id],
                |r| {
                    Ok(Again {
                        reopened: r.get(0)?,
                        continued: r.get(1)?,
                        in_a_row: r.get(2)?,
                        continued_at: r.get(3)?,
                        due_at: r.get(4)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Its session went on after it had ended. It stays ended once a later
    /// run of its task was made. Whether it is open now.
    pub fn reopen(&self, id: i64) -> anyhow::Result<bool> {
        let conn = self.store.conn();
        let both = conn.unchecked_transaction()?;
        let changed = both.execute(
            "UPDATE worker_runs
             SET state = 'running', outcome = '', result = '', note = '', ended_at = NULL
             WHERE id = ?1 AND state = 'ended' AND NOT EXISTS (
                 SELECT 1 FROM worker_runs l WHERE l.task = worker_runs.task AND l.id > ?1)",
            [id],
        )?;
        if changed == 1 {
            both.execute(
                "INSERT INTO worker_again (run, reopened) VALUES (?1, 1)
                 ON CONFLICT (run) DO UPDATE SET reopened = reopened + 1",
                [id],
            )?;
        }
        both.commit()?;
        Ok(changed == 1)
    }

    /// When it is asked to go on after a passing fault; 0 when it is not.
    pub fn due(&self, id: i64, at: i64) -> anyhow::Result<()> {
        self.store.conn().execute(
            "INSERT INTO worker_again (run, due_at) VALUES (?1, ?2)
             ON CONFLICT (run) DO UPDATE SET due_at = ?2 WHERE due_at != ?2",
            [id, at],
        )?;
        Ok(())
    }

    /// Count that it is asked to go on, before it is.
    pub fn continuing(&self, id: i64, in_a_row: u32) -> anyhow::Result<()> {
        self.store.conn().execute(
            "INSERT INTO worker_again (run, continued, in_a_row, continued_at)
             VALUES (?1, 1, ?2, ?3)
             ON CONFLICT (run) DO UPDATE SET
                 continued = continued + 1, in_a_row = ?2, continued_at = ?3, due_at = 0",
            params![id, in_a_row, now()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::workers::Runner;

    pub(in crate::workers) fn order(runner: Runner) -> Order {
        Order {
            code: "task-1".into(),
            title: "x".into(),
            workgroup: "all".into(),
            workgroups: Vec::new(),
            scopes: Vec::new(),
            runner,
            request: "do it".into(),
            connection: String::new(),
            directory: "/src".into(),
            worktree: false,
            base: String::new(),
            agent: String::new(),
            model: String::new(),
            permissions: String::new(),
            tools: Vec::new(),
            files: Vec::new(),
            wait: false,
            notebook: Default::default(),
            search: Vec::new(),
            finish: String::new(),
        }
    }

    #[test]
    fn a_task_has_one_open_run_at_most() {
        let runs = Runs::new(Store::open_in_memory().unwrap());
        let order = order(Runner::Agent);
        let run = runs.begin("t", &order).unwrap();
        assert!(runs.begin("t", &order).is_err());
        runs.ending(run.id, Outcome::Done, "r").unwrap();
        assert!(runs.begin("t", &order).is_err(), "its end is not told yet");
        runs.ended(run.id).unwrap();
        assert!(runs.begin("t", &order).is_ok());
    }

    #[test]
    fn a_run_is_opened_again_only_while_it_is_the_last_of_its_task() {
        let runs = Runs::new(Store::open_in_memory().unwrap());
        let order = order(Runner::Adroit);
        let end = |id| {
            runs.ending(id, Outcome::Failed, "broke").unwrap();
            runs.ended(id).unwrap();
        };
        let first = runs.begin("t", &order).unwrap();
        assert!(!runs.reopen(first.id).unwrap(), "it has not ended");
        end(first.id);
        assert!(runs.reopen(first.id).unwrap());
        let open = runs.get(first.id).unwrap().unwrap();
        assert_eq!((open.state, open.outcome), (State::Running, None));
        assert_eq!((open.result.as_str(), open.ended_at), ("", None));
        assert!(!runs.reopen(first.id).unwrap(), "it is open");
        end(first.id);
        let second = runs.begin("t", &order).unwrap();
        assert!(!runs.reopen(first.id).unwrap());
        end(second.id);
        assert!(!runs.reopen(first.id).unwrap());
        assert_eq!(runs.again(first.id).unwrap().reopened, 1);
    }

    #[test]
    fn attempts_are_counted_only_over_interruptions() {
        let runs = Runs::new(Store::open_in_memory().unwrap());
        let order = order(Runner::Agent);
        for (outcome, next) in [
            (Outcome::Interrupted, 2),
            (Outcome::Failed, 1),
            (Outcome::Done, 1),
        ] {
            let run = runs.begin("t", &order).unwrap();
            runs.ending(run.id, outcome, "").unwrap();
            runs.ended(run.id).unwrap();
            let run = runs.begin("t", &order).unwrap();
            assert_eq!(run.attempt, next, "after {outcome:?}");
            runs.ending(run.id, Outcome::Done, "").unwrap();
            runs.ended(run.id).unwrap();
        }
    }
}
