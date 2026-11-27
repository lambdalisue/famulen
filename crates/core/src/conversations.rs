//! The secretary's conversations. Each is a Claude Code session of its own, so
//! that a turn reads what its thread is about and not everything ever said.
//! Which conversation an input belongs to is kept here, never left to the
//! model to remember.

use rusqlite::{OptionalExtension, params};

use crate::console_log::{ConsoleLog, Message, Who};
use crate::input::{Input, Reach, Sender, Source};
use crate::store::{Store, StoredInput, now};
use crate::workgroups::ALL;

/// How long after the last word in a conversation what the owner says outside
/// any thread goes on with it.
pub const GOES_ON_SECS: i64 = 10 * 60;

/// How far back what was said elsewhere is handed on.
const LATELY_SECS: i64 = 24 * 60 * 60;
/// The most that is handed on of what was said elsewhere.
const ELSEWHERE: Bound = Bound {
    messages: 12,
    each: 400,
    all: 4000,
};
/// The most that is handed on of what was said in the conversation itself,
/// when its session is gone or never was.
const ITS_OWN: Bound = Bound {
    messages: 30,
    each: 800,
    all: 8000,
};
/// How many of the latest messages are looked through for it.
const LOOKED_AT: usize = 200;

/// How often what comes back may have come back before.
const MOST_STEPS: usize = 100;
/// What stands for the notices where a conversation's id is asked for.
pub const NOTICES: i64 = 0;
// Of a conversation `c`: whether an input of it is something the owner said.
const SPOKEN: &str = "EXISTS (SELECT 1 FROM conversation_keys k
    JOIN inputs i ON i.id = CAST(substr(k.key, 7) AS INTEGER)
    WHERE k.conversation = c.id AND k.key LIKE 'input:%'
        AND json_extract(i.body, '$.source.kind') IN ('console', 'slack'))";

/// What the secretary said, each with the conversation `c` it was said in.
const SAID_IN: &str = "console_messages m
    LEFT JOIN conversation_keys own ON own.key = 'message:' || m.id
    LEFT JOIN conversation_keys turn ON turn.key = 'input:' || json_extract(m.meta, '$.input')
    JOIN conversations c ON c.id = coalesce(own.conversation, turn.conversation)";

/// The conversation `c` was begun by something other than what the owner said.
const UNPROMPTED: &str = "(SELECT json_extract(i.body, '$.source.kind') FROM conversation_keys k
    JOIN inputs i ON i.id = CAST(substr(k.key, 7) AS INTEGER)
    WHERE k.conversation = c.id AND k.key LIKE 'input:%'
    ORDER BY i.id LIMIT 1) NOT IN ('console', 'slack')";

struct Bound {
    messages: usize,
    /// Characters.
    each: usize,
    all: usize,
}

/// One conversation of the secretary's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub id: i64,
    /// The Claude Code session it goes on in; none before its first turn.
    pub session: Option<String>,
    /// Unix seconds.
    pub opened_at: i64,
    pub used_at: i64,
    /// How many tokens the model read when it was last called.
    pub context: i64,
    /// The workgroup it began in; empty before its first turn.
    pub workgroup: String,
    /// The newest console message that is not handed to it any more.
    seen: i64,
}

/// Of the input a turn read, what went through the cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cached {
    pub read: u64,
    pub written: u64,
}

impl Conversation {
    /// Whether it shows while `workgroup` is chosen.
    pub fn shown_in(&self, workgroup: &str) -> bool {
        [workgroup, self.workgroup.as_str()]
            .iter()
            .any(|w| w.is_empty() || *w == ALL)
            || self.workgroup == workgroup
    }
}

/// What the turns of conversations used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Spent {
    pub turns: i64,
    pub input_tokens: i64,
    /// Of the input, what was read from the cache.
    pub cached_tokens: i64,
    /// Of the input, what was written to the cache.
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
}

impl std::iter::Sum for Spent {
    fn sum<I: Iterator<Item = Self>>(spent: I) -> Self {
        spent.fold(Self::default(), |sum, s| Self {
            turns: sum.turns + s.turns,
            input_tokens: sum.input_tokens + s.input_tokens,
            cached_tokens: sum.cached_tokens + s.cached_tokens,
            cache_write_tokens: sum.cache_write_tokens + s.cache_write_tokens,
            output_tokens: sum.output_tokens + s.output_tokens,
        })
    }
}

/// The key of a message on the console.
pub fn message_key(id: i64) -> String {
    format!("message:{id}")
}

/// The key of a message on Slack, and of the thread it begins.
pub fn slack_key(workspace: &str, channel: &str, ts: &str) -> String {
    format!("slack:{workspace}:{channel}:{ts}")
}

/// The key of a wake-up.
pub fn wake_key(id: i64) -> String {
    format!("wake:{id}")
}

/// The key of a delegated task, by its record's id.
pub fn task_key(task: &str) -> String {
    format!("task:{task}")
}

fn input_key(id: i64) -> String {
    format!("input:{id}")
}

/// Where the owner talks outside any thread.
fn open_place(input: &Input) -> Option<String> {
    if input.sender != Sender::Owner || input.reach() != Reach::Full {
        return None;
    }
    match &input.source {
        // What the console hands over by itself has no message.
        Source::Console {
            message,
            workgroup,
            reply_to: None,
            ..
        } if *message > 0 => Some(format!("console:{workgroup}")),
        Source::Slack {
            workspace,
            channel,
            thread_ts: None,
            ..
        } => Some(format!("slack:{workspace}:{channel}")),
        _ => None,
    }
}

/// What was said before that a turn is handed, and where it was said.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Said {
    /// Quoted; empty when there is none.
    pub text: String,
    /// The other conversations it was said in.
    pub conversations: Vec<i64>,
    /// The turns it was said in that are of no conversation.
    pub inputs: Vec<i64>,
}

impl Said {
    fn from(&mut self, of: Option<i64>, m: &Message, here: i64) {
        match of {
            Some(of) if of != here => self.conversations.push(of),
            Some(_) => {}
            None => self.inputs.extend(m.input()),
        }
    }
}

/// Clones share one store.
#[derive(Clone)]
pub struct Conversations {
    store: Store,
}

impl Conversations {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Put what was under way in the one conversation there was before they
    /// were told apart into a conversation that goes on in its session.
    pub fn adopt_the_one_before(&self) -> anyhow::Result<()> {
        let Some(session) = self.store.state(BEFORE)? else {
            return Ok(());
        };
        let mut conn = self.store.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO conversations (session, opened_at, used_at) VALUES (?1, ?2, ?2)",
            params![session, now()],
        )?;
        let id = tx.last_insert_rowid();
        for under_way in [
            "SELECT 'input:' || id, ?1 FROM inputs WHERE sender != 'other'",
            "SELECT 'wake:' || id, ?1 FROM wakeups WHERE fired = 0",
            "SELECT 'task:' || task, ?1 FROM delegations WHERE by = 'secretary'",
        ] {
            tx.execute(
                &format!("INSERT OR IGNORE INTO conversation_keys (key, conversation) {under_way}"),
                [id],
            )?;
        }
        tx.execute("DELETE FROM state WHERE key = ?1", [BEFORE])?;
        tx.commit()?;
        Ok(())
    }

    /// The conversation an input is of, opened when there is none yet.
    /// `origin` gives the input a request to the owner was made in.
    pub fn settle(
        &self,
        row: &StoredInput,
        origin: impl Fn(i64) -> Option<i64>,
    ) -> anyhow::Result<Conversation> {
        let found = match self.of(&input_key(row.id))? {
            Some(of) => Some(of),
            None => self.home(row, origin)?,
        };
        let id = match found {
            Some(id) => id,
            None => self.open_for(&row.input)?,
        };
        self.link(id, [input_key(row.id)])?;
        // Another may have settled it meanwhile; the first one holds.
        let id = match self.of(&input_key(row.id))? {
            Some(first) if first != id => {
                self.store.conn().execute(
                    "DELETE FROM conversations WHERE id = ?1
                        AND NOT EXISTS (SELECT 1 FROM conversation_keys WHERE conversation = ?1)",
                    [id],
                )?;
                first
            }
            _ => id,
        };
        self.said(id, &row.input)?;
        match &row.input.source {
            Source::Console {
                message, reply_to, ..
            } => {
                self.link(id, [message_key(*message)])?;
                if let Some(to) = reply_to {
                    self.link(id, [message_key(to.thread)])?;
                }
            }
            Source::Slack {
                workspace,
                channel,
                ts,
                thread_ts,
                ..
            } => {
                let begins = thread_ts.as_deref().unwrap_or(ts);
                self.link(id, [ts, begins].map(|ts| slack_key(workspace, channel, ts)))?;
            }
            _ => {}
        }
        self.get(id)?
            .ok_or_else(|| anyhow::anyhow!("conversation {id} is gone"))
    }

    /// Open a conversation that `input` begins.
    fn open_for(&self, input: &Input) -> anyhow::Result<i64> {
        let workgroup = match &input.source {
            Source::Console { workgroup, .. } => workgroup.as_str(),
            _ => "",
        };
        let conn = self.store.conn();
        conn.execute(
            "INSERT INTO conversations (place, workgroup, said_at, opened_at, used_at)
             VALUES (?1, ?2, ?3, ?3, ?3)",
            params![open_place(input).unwrap_or_default(), workgroup, now()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Open a conversation the owner asked to begin with `input`: it is about
    /// something else, so nothing said before is handed to it.
    pub fn begin_anew(&self, input: &Input) -> anyhow::Result<i64> {
        let id = self.open_for(input)?;
        self.store.conn().execute(
            "UPDATE conversations SET seen = (SELECT COALESCE(MAX(id), 0) FROM console_messages)
             WHERE id = ?1",
            [id],
        )?;
        Ok(id)
    }

    /// What an input can be found by to be of a conversation already there,
    /// best first.
    fn home_keys(
        &self,
        input: &Input,
        origin: impl Fn(i64) -> Option<i64>,
    ) -> anyhow::Result<Vec<String>> {
        Ok(match &input.source {
            // One the owner chose the conversation of is linked already.
            Source::Console {
                message,
                reply_to: Some(to),
                ..
            } => [*message, to.thread, to.message].map(message_key).into(),
            Source::Console { message, .. } => vec![message_key(*message)],
            Source::Slack {
                workspace,
                channel,
                thread_ts: Some(thread),
                ..
            } => vec![slack_key(workspace, channel, thread)],
            Source::Approval { id, .. } => origin(*id).map(input_key).into_iter().collect(),
            Source::Wake { id, .. } => vec![wake_key(*id)],
            Source::Worker { run, .. } => {
                let task: Option<String> = self
                    .store
                    .conn()
                    .query_row("SELECT task FROM worker_runs WHERE id = ?1", [run], |r| {
                        r.get(0)
                    })
                    .optional()?;
                task.map(|task| task_key(&task)).into_iter().collect()
            }
            // What arrives by itself is on its own, and what the owner let be
            // answered is answered without knowing what else is talked of.
            Source::Slack { .. }
            | Source::Asked { .. }
            | Source::Schedule { .. }
            | Source::Arrival { .. }
            | Source::Lookout { .. } => Vec::new(),
        })
    }

    fn home(
        &self,
        row: &StoredInput,
        origin: impl Fn(i64) -> Option<i64>,
    ) -> anyhow::Result<Option<i64>> {
        for key in self.home_keys(&row.input, origin)? {
            if let Some(of) = self.of(&key)? {
                return Ok(Some(of));
            }
        }
        match open_place(&row.input) {
            Some(place) => self.going_on(&place, row.id),
            None => Ok(None),
        }
    }

    /// The conversation the owner last had at `place`, when they spoke on
    /// soon after its last word, or while something is still done in it.
    fn going_on(&self, place: &str, input: i64) -> anyhow::Result<Option<i64>> {
        let last: Option<(i64, bool)> = self
            .store
            .conn()
            .query_row(
                "SELECT c.id, i.created_at - c.said_at <= ?3 FROM conversations c, inputs i
                 WHERE i.id = ?2 AND c.place = ?1
                 ORDER BY c.said_at DESC, c.said_no DESC LIMIT 1",
                params![place, input, GOES_ON_SECS],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match last {
            Some((id, true)) => Ok(Some(id)),
            Some((id, false)) if self.waiting(id)? => Ok(Some(id)),
            _ => Ok(None),
        }
    }

    /// Have what comes under `keys` come to the conversation `id`. A key keeps
    /// the conversation it has.
    pub fn link(&self, id: i64, keys: impl IntoIterator<Item = String>) -> anyhow::Result<()> {
        let conn = self.store.conn();
        for key in keys {
            conn.execute(
                "INSERT OR IGNORE INTO conversation_keys (key, conversation) VALUES (?1, ?2)",
                params![key, id],
            )?;
        }
        Ok(())
    }

    /// Have what comes under `keys` come to the conversation of the running
    /// turn, when that is one of the owner's.
    pub fn link_to_running(&self, keys: impl IntoIterator<Item = String>) -> anyhow::Result<()> {
        let Some((id, row)) = self.running_turn()? else {
            return Ok(());
        };
        let conn = self.store.conn();
        for key in keys {
            conn.execute(
                "INSERT OR IGNORE INTO conversation_keys (key, conversation, input) VALUES (?1, ?2, ?3)",
                params![key, id, row.id],
            )?;
        }
        drop(conn);
        self.said(id, &row.input)
    }

    /// What was ordered anew comes to the conversation it was ordered in last.
    pub fn move_to_running(&self, key: String) -> anyhow::Result<()> {
        let Some((id, row)) = self.running_turn()? else {
            return Ok(());
        };
        self.store.conn().execute(
            "INSERT OR REPLACE INTO conversation_keys (key, conversation, input) VALUES (?1, ?2, ?3)",
            params![key, id, row.id],
        )?;
        Ok(())
    }

    /// The running turn's input with its conversation. One that answers what
    /// the owner let be answered is said to someone else and of none.
    fn running_turn(&self) -> anyhow::Result<Option<(i64, StoredInput)>> {
        let Some(row) = self.store.running_input()? else {
            return Ok(None);
        };
        if !matches!(row.input.reach(), Reach::Full | Reach::Inward) {
            return Ok(None);
        }
        Ok(self.of(&input_key(row.id))?.map(|id| (id, row)))
    }

    pub fn of(&self, key: &str) -> anyhow::Result<Option<i64>> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT conversation FROM conversation_keys WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The conversation a message on the console is in.
    pub fn of_message(&self, m: &Message) -> anyhow::Result<Option<i64>> {
        if let Some(of) = self.of(&message_key(m.id))? {
            return Ok(Some(of));
        }
        match m.input() {
            Some(input) => self.of(&input_key(input)),
            None => Ok(None),
        }
    }

    /// The conversation of the running turn.
    pub fn running(&self) -> anyhow::Result<Option<i64>> {
        match self.store.running()? {
            Some(turn) => self.of(&input_key(turn.input)),
            None => Ok(None),
        }
    }

    pub fn get(&self, id: i64) -> anyhow::Result<Option<Conversation>> {
        Ok(self
            .store
            .conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM conversations WHERE id = ?1"),
                [id],
                conversation,
            )
            .optional()?)
    }

    /// The talks used last, those used before `before` when it is given: the
    /// ones begun in what `workgroup` shows, every one when it is empty.
    pub fn recent(
        &self,
        workgroup: &str,
        before: Option<&Conversation>,
        limit: usize,
    ) -> anyhow::Result<Vec<Conversation>> {
        let (used_at, id) = before.map_or((i64::MAX, i64::MAX), |c| (c.used_at, c.id));
        let workgroup = if workgroup == ALL { "" } else { workgroup };
        let conn = self.store.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM conversations c WHERE (used_at, id) < (?1, ?2)
                AND (?4 = '' OR workgroup IN ('', ?4, 'all')) AND {SPOKEN}
             ORDER BY used_at DESC, id DESC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![used_at, id, limit as i64, workgroup], conversation)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// What the secretary told without being asked, before the owner took
    /// part: the latest of it older than `before` that `shown` says yes to,
    /// oldest first; `0` starts from the newest.
    pub fn notices(
        &self,
        log: &ConsoleLog,
        before: i64,
        limit: usize,
        shown: impl Fn(&Message) -> bool,
    ) -> anyhow::Result<Vec<Message>> {
        // Only what could be a notice is read: the newest of them may be far back,
        // and the list of conversations asks for it each time.
        let candidates: Vec<i64> = {
            let conn = self.store.conn();
            let mut stmt = conn.prepare(&format!(
                "SELECT m.id FROM {SAID_IN}
                 WHERE m.who = ?1 AND m.id < ?2 AND {UNPROMPTED}
                 ORDER BY m.id DESC"
            ))?;
            let from = if before == 0 { i64::MAX } else { before };
            let rows = stmt.query_map(params![Who::Aide.as_str(), from], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        let mut found = Vec::new();
        for id in candidates {
            if found.len() == limit {
                break;
            }
            if let Some(m) = log.get(id)?
                && shown(&m)
                && self.notice_of(log, &m)?.is_some()
            {
                found.push(m);
            }
        }
        found.reverse();
        Ok(found)
    }

    /// The notices `shown` says yes to that the owner has not read, newest
    /// first, each with its conversation. A conversation they spoke in has none:
    /// what they spoke after, they have.
    fn unread_told(
        &self,
        log: &ConsoleLog,
        shown: impl Fn(&Message) -> bool,
    ) -> anyhow::Result<Vec<(Message, i64)>> {
        let after_the_marks: Vec<(i64, i64)> = {
            let conn = self.store.conn();
            let mut stmt = conn.prepare(&format!(
                "SELECT m.id, c.id FROM {SAID_IN}
                 LEFT JOIN conversation_reads r ON r.conversation = c.id
                 WHERE m.who = ?1 AND m.id > coalesce(r.message, 0) AND NOT {SPOKEN}
                 ORDER BY m.id DESC"
            ))?;
            let rows = stmt.query_map([Who::Aide.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        let mut found = Vec::new();
        for (id, of) in after_the_marks {
            if let Some(m) = log.get(id)?
                && shown(&m)
                && self.notice_of(log, &m)?.is_some()
            {
                found.push((m, of));
            }
        }
        Ok(found)
    }

    /// How many of the notices `shown` says yes to the owner has not read, and
    /// the newest of them all.
    pub fn notice_summary(
        &self,
        log: &ConsoleLog,
        shown: impl Fn(&Message) -> bool,
    ) -> anyhow::Result<(usize, Option<Message>)> {
        let unread = self.unread_told(log, &shown)?.len();
        Ok((unread, self.notices(log, 0, 1, shown)?.pop()))
    }

    /// The message up to which the owner has read every notice `shown` says
    /// yes to.
    pub fn notices_read_to(
        &self,
        log: &ConsoleLog,
        shown: impl Fn(&Message) -> bool,
    ) -> anyhow::Result<i64> {
        if let Some((oldest, _)) = self.unread_told(log, &shown)?.last() {
            return Ok(oldest.id - 1);
        }
        Ok(self.notices(log, 0, 1, shown)?.pop().map_or(0, |m| m.id))
    }

    /// The owner was shown the notices `shown` says yes to, up to `message`.
    /// Whether that was more than before is returned.
    pub fn read_notices(
        &self,
        log: &ConsoleLog,
        message: i64,
        shown: impl Fn(&Message) -> bool,
    ) -> anyhow::Result<bool> {
        let mut changed = false;
        for (m, of) in self.unread_told(log, shown)? {
            if m.id <= message {
                changed |= self.read(of, m.id)?;
            }
        }
        Ok(changed)
    }

    /// What the secretary was told by, when `m` is a notice: said in a
    /// conversation the owner neither began nor had taken part in by then.
    pub fn notice_of(&self, log: &ConsoleLog, m: &Message) -> anyhow::Result<Option<Source>> {
        if m.who != Who::Aide {
            return Ok(None);
        }
        let Some(of) = self.of_message(m)? else {
            return Ok(None);
        };
        let Some(first) = self.first_input(of)? else {
            return Ok(None);
        };
        if first.input.is_spoken() {
            return Ok(None);
        }
        let said = self.messages(log, of)?;
        let joined = said.iter().any(|x| x.who == Who::Owner && x.id < m.id);
        Ok((!joined).then_some(first.input.source))
    }

    /// The newest message the owner was shown under `id`, or 0.
    pub fn mark(&self, id: i64) -> anyhow::Result<i64> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT message FROM conversation_reads WHERE conversation = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Whether an input of a conversation waits for its turn or is in it.
    pub fn waiting(&self, id: i64) -> anyhow::Result<bool> {
        for row in self.store.in_hand()? {
            if self.of(&input_key(row.id))? == Some(id) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn numbers(&self, id: i64, prefix: &str) -> anyhow::Result<Vec<i64>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare("SELECT key FROM conversation_keys WHERE conversation = ?1")?;
        let keys = stmt.query_map([id], |r| r.get::<_, String>(0))?;
        let mut found = Vec::new();
        for key in keys {
            found.extend(
                key?.strip_prefix(prefix)
                    .and_then(|n| n.parse::<i64>().ok()),
            );
        }
        Ok(found)
    }

    /// The input a conversation began with.
    pub fn first_input(&self, id: i64) -> anyhow::Result<Option<StoredInput>> {
        let first = self.numbers(id, "input:")?.into_iter().min();
        first.map_or(Ok(None), |id| self.store.get(id))
    }

    /// What the owner asked with, on the console or on Slack, for what `row`
    /// brings back: the turn it was ordered in answered it, or brought back
    /// something of it in turn.
    pub fn asked_with(
        &self,
        row: &StoredInput,
        origin: impl Fn(i64) -> Option<i64>,
    ) -> anyhow::Result<Option<Input>> {
        let mut input = row.input.clone();
        for _ in 0..MOST_STEPS {
            let ordered_in = match &input.source {
                Source::Approval { id, .. } => origin(*id),
                Source::Wake { .. } | Source::Worker { .. } => {
                    let key = self.home_keys(&input, &origin)?.into_iter().next();
                    match key {
                        Some(key) => self.ordered_in(&key)?,
                        None => None,
                    }
                }
                Source::Console { message, .. } if *message > 0 => return Ok(Some(input)),
                Source::Slack { .. }
                    if input.sender == Sender::Owner && input.reach() == Reach::Full =>
                {
                    return Ok(Some(input));
                }
                _ => None,
            };
            let Some(before) = ordered_in
                .map(|id| self.store.get(id))
                .transpose()?
                .flatten()
            else {
                break;
            };
            input = before.input;
        }
        Ok(None)
    }

    fn ordered_in(&self, key: &str) -> anyhow::Result<Option<i64>> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT input FROM conversation_keys WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// The messages on the console that are in a conversation, oldest first.
    pub fn messages(&self, log: &ConsoleLog, id: i64) -> anyhow::Result<Vec<Message>> {
        let mut found = log.of_inputs(&self.numbers(id, "input:")?)?;
        for message in self.numbers(id, "message:")? {
            found.extend(log.get(message)?);
        }
        // A thread of another conversation may begin with a message that is
        // linked here too; it stands once.
        found.sort_by_key(|m| m.id);
        found.dedup_by_key(|m| m.id);
        Ok(found)
    }

    /// The conversation a request to the owner was made in: the secretary's
    /// own, or the one that ordered the delegated task it is of. `origin`
    /// gives the input it was made in.
    pub fn of_request(
        &self,
        request: i64,
        session: Option<&str>,
        origin: impl Fn(i64) -> Option<i64>,
    ) -> anyhow::Result<Option<i64>> {
        if let Some(input) = origin(request)
            && let Some(of) = self.of(&input_key(input))?
        {
            return Ok(Some(of));
        }
        let task: Option<String> = self
            .store
            .conn()
            .query_row(
                "SELECT task FROM worker_runs WHERE ticket = ?1 OR (?2 != '' AND session = ?2)
                 ORDER BY id DESC LIMIT 1",
                params![request, session.unwrap_or_default()],
                |r| r.get(0),
            )
            .optional()?;
        task.map_or(Ok(None), |task| self.of(&task_key(&task)))
    }

    /// The conversation an input is of, once it is settled.
    pub fn of_input(&self, input: i64) -> anyhow::Result<Option<i64>> {
        self.of(&input_key(input))
    }

    /// The owner was shown what is said in a conversation, up to `message`.
    /// Whether that was more than before is returned.
    pub fn read(&self, id: i64, message: i64) -> anyhow::Result<bool> {
        let changed = self.store.conn().execute(
            "INSERT INTO conversation_reads (conversation, message) VALUES (?1, ?2)
             ON CONFLICT (conversation) DO UPDATE SET message = ?2 WHERE message < ?2",
            params![id, message],
        )?;
        Ok(changed > 0)
    }

    /// The newest of `said` in a conversation the owner has read, or 0. What
    /// they spoke after, they have.
    pub fn read_to(&self, id: i64, said: &[Message]) -> anyhow::Result<i64> {
        let read = self.mark(id)?;
        let spoke = said.iter().rfind(|m| m.who == Who::Owner).map(|m| m.id);
        Ok(read.max(spoke.unwrap_or_default()))
    }

    /// How many of `said` in a conversation the owner has not read.
    pub fn unread(&self, id: i64, said: &[Message]) -> anyhow::Result<usize> {
        let after = self.read_to(id, said)?;
        Ok(said
            .iter()
            .filter(|m| m.who == Who::Aide && m.id > after)
            .count())
    }

    /// The session of a conversation, as its process tells it.
    pub fn set_session(&self, id: i64, session: &str) -> anyhow::Result<()> {
        if session.is_empty() {
            return Ok(());
        }
        self.store.conn().execute(
            "UPDATE conversations SET session = ?2 WHERE id = ?1",
            params![id, session],
        )?;
        Ok(())
    }

    /// Its session can be gone on with no more: the next turn begins a new
    /// one, and is handed what was said.
    pub fn forget_session(&self, id: i64) -> anyhow::Result<()> {
        self.store
            .conn()
            .execute("UPDATE conversations SET session = '' WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_context(&self, id: i64, tokens: i64) -> anyhow::Result<()> {
        self.store.conn().execute(
            "UPDATE conversations SET context = ?2 WHERE id = ?1",
            params![id, tokens],
        )?;
        Ok(())
    }

    /// A turn of the conversation began.
    pub fn began(&self, id: i64, input: &Input, workgroup: &str) -> anyhow::Result<()> {
        self.store.conn().execute(
            "UPDATE conversations SET used_at = ?2,
                workgroup = CASE WHEN workgroup = '' THEN ?3 ELSE workgroup END
             WHERE id = ?1",
            params![id, now(), workgroup],
        )?;
        self.said(id, input)
    }

    /// A turn of the conversation ended.
    pub fn ended(&self, id: i64, row: &StoredInput) -> anyhow::Result<()> {
        // Only now is it sure that it was handed what was said elsewhere, which
        // only what is said outside any thread is. What the owner said after
        // it is of turns to come.
        let handed = match open_place(&row.input) {
            Some(_) => self.message_of(row)?,
            None => None,
        };
        self.store.conn().execute(
            "UPDATE conversations SET used_at = ?2, seen = MAX(seen, COALESCE(?3, 0))
             WHERE id = ?1",
            params![id, now(), handed],
        )?;
        self.said(id, &row.input)
    }

    /// The message on the console an input is shown as.
    fn message_of(&self, row: &StoredInput) -> anyhow::Result<Option<i64>> {
        if let Source::Console { message, .. } = &row.input.source {
            return Ok(Some(*message).filter(|m| *m > 0));
        }
        Ok(self.store.conn().query_row(
            "SELECT MAX(id) FROM console_messages WHERE json_extract(meta, '$.input') = ?1",
            [row.id],
            |r| r.get(0),
        )?)
    }

    /// Something was said in the conversation, in the turn of `input` or by
    /// it. Within a thread the owner goes on by replying there, so only what is
    /// said outside any goes on by itself.
    fn said(&self, id: i64, input: &Input) -> anyhow::Result<()> {
        if open_place(input).is_some() {
            self.store.conn().execute(
                "UPDATE conversations SET said_at = ?2,
                    said_no = 1 + (SELECT MAX(said_no) FROM conversations)
                 WHERE id = ?1",
                params![id, now()],
            )?;
        }
        Ok(())
    }

    /// The owner chose to go on with a conversation outside any thread, so
    /// that is where they talk now.
    pub fn choose(&self, id: i64, input: &Input, message: i64) -> anyhow::Result<()> {
        if let Some(place) = open_place(input) {
            self.store.conn().execute(
                "UPDATE conversations SET place = ?2 WHERE id = ?1",
                params![id, place],
            )?;
        }
        self.said(id, input)?;
        self.link(id, [message_key(message)])
    }

    /// Keep what a turn of the conversation used.
    pub fn spent(
        &self,
        id: i64,
        usage: &crate::models::Usage,
        cached: Cached,
    ) -> anyhow::Result<()> {
        let tokens = |t: Option<u64>| t.unwrap_or_default() as i64;
        self.store.conn().execute(
            "INSERT INTO conversation_turns (conversation, at, input_tokens, cached_tokens, cache_write_tokens, output_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                now(),
                tokens(usage.input_tokens),
                tokens(Some(cached.read)),
                tokens(Some(cached.written)),
                tokens(usage.output_tokens),
            ],
        )?;
        Ok(())
    }

    /// What each conversation used from `from` until before `to`, in Unix
    /// seconds, the most input first.
    pub fn spent_between(&self, from: i64, to: i64) -> anyhow::Result<Vec<(i64, Spent)>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT conversation, COUNT(*), SUM(input_tokens), SUM(cached_tokens), SUM(cache_write_tokens), SUM(output_tokens)
             FROM conversation_turns WHERE at >= ?1 AND at < ?2
             GROUP BY conversation ORDER BY 3 DESC, conversation",
        )?;
        let rows = stmt.query_map([from, to], |r| {
            Ok((
                r.get(0)?,
                Spent {
                    turns: r.get(1)?,
                    input_tokens: r.get(2)?,
                    cached_tokens: r.get(3)?,
                    cache_write_tokens: r.get(4)?,
                    output_tokens: r.get(5)?,
                },
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// What a conversation used in all.
    pub fn spent_by(&self, id: i64) -> anyhow::Result<Spent> {
        Ok(self.store.conn().query_row(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(cached_tokens), 0),
                    COALESCE(SUM(cache_write_tokens), 0), COALESCE(SUM(output_tokens), 0)
             FROM conversation_turns WHERE conversation = ?1",
            [id],
            |r| {
                Ok(Spent {
                    turns: r.get(0)?,
                    input_tokens: r.get(1)?,
                    cached_tokens: r.get(2)?,
                    cache_write_tokens: r.get(3)?,
                    output_tokens: r.get(4)?,
                })
            },
        )?)
    }

    /// What was said before that the session of `c` does not know and the
    /// turn of `row` is handed, quoted; empty when there is none. With it, the
    /// other conversations it was said in, and the turns of none it was said in.
    pub fn said_before(
        &self,
        log: &ConsoleLog,
        c: &Conversation,
        row: &StoredInput,
        workgroup: &str,
    ) -> anyhow::Result<Said> {
        // The owner let one question be answered, not their talk be known.
        if row.input.reach() != Reach::Full && row.input.reach() != Reach::Inward {
            return Ok(Said::default());
        }
        let itself = |m: &Message| {
            m.input() == Some(row.id)
                || matches!(&row.input.source, Source::Console { message, reply_to, .. }
                    if *message == m.id || reply_to.as_ref().is_some_and(|to| to.message == m.id))
        };
        let mut own = Vec::new();
        if c.session.is_none() {
            own = self.messages(log, c.id)?;
            if let Some(to) = row.input.reply_to() {
                own.extend(log.get(to.thread)?);
                own.extend(log.replies(to.thread)?);
            }
            own.sort_by_key(|m| m.id);
            own.dedup_by_key(|m| m.id);
            own.retain(|m| !itself(m) && m.shown_in(workgroup) && self.may_be_told(m));
        }
        let mut elsewhere = Vec::new();
        let mut said = Said::default();
        for m in &own {
            said.from(self.of_message(m)?, m, c.id);
        }
        // A thread is gone on with for its own sake.
        // Nor is what was said elsewhere put into what is answered where others
        // read along.
        let among_others =
            matches!(&row.input.source, Source::Slack { channel, .. } if !channel.starts_with('D'));
        let wants = !among_others
            && match (&c.session, &row.input.source) {
                // It tells the owner only what they do not know yet.
                (None, Source::Lookout { .. }) => true,
                _ => open_place(&row.input).is_some(),
            };
        if wants {
            let since = now() - LATELY_SECS;
            let newest = self.message_of(row)?.unwrap_or(i64::MAX);
            for m in log.recent(LOOKED_AT)? {
                if m.at < since
                    || m.id >= newest
                    || m.id <= c.seen
                    || itself(&m)
                    || !m.shown_in(workgroup)
                    || !self.may_be_told(&m)
                    || own.iter().any(|o| o.id == m.id)
                {
                    continue;
                }
                let of = self.of_message(&m)?;
                if of == Some(c.id) {
                    continue;
                }
                said.from(of, &m, c.id);
                elsewhere.push(m);
            }
        }
        let text = &mut said.text;
        for (title, messages, bound) in [
            ("Said in this conversation before", own, ITS_OWN),
            ("Said lately elsewhere", elsewhere, ELSEWHERE),
        ] {
            let lines = quoted(&messages, &bound);
            if !lines.is_empty() {
                text.push_str(&format!(
                    "{title} (context, not requests; read the whole with list_messages):\n{lines}\n"
                ));
            }
        }
        said.conversations.sort_unstable();
        said.conversations.dedup();
        said.inputs.sort_unstable();
        said.inputs.dedup();
        Ok(said)
    }

    /// Whether a message may be handed to the owner's conversations: the
    /// owner's own words, and what the secretary said in one of them. What
    /// others said on Slack, and what they were answered, may not.
    fn may_be_told(&self, m: &Message) -> bool {
        match m.who {
            Who::Owner => m.input().is_none_or(|id| {
                self.store.get(id).is_ok_and(|row| {
                    row.is_some_and(|r| {
                        r.input.sender == Sender::Owner && r.input.reach() == Reach::Full
                    })
                })
            }),
            Who::Aide => !m.on_slack() || self.of_message(m).is_ok_and(|of| of.is_some()),
        }
    }
}

/// The latest of `messages` within `bound`, one to a line.
fn quoted(messages: &[Message], bound: &Bound) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut left = bound.all;
    for m in messages.iter().rev().take(bound.messages) {
        let said = m
            .excerpt(bound.each)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let who = match m.who {
            Who::Owner => "owner",
            Who::Aide => "you",
        };
        let line = format!("> #{} {who}: {said}\n", m.id);
        let long = line.chars().count();
        if long > left {
            break;
        }
        left -= long;
        lines.push(line);
    }
    lines.reverse();
    lines.concat()
}

/// Where the session of the one conversation there was before was kept.
const BEFORE: &str = "claude.session_id";

const COLUMNS: &str = "id, session, opened_at, used_at, context, seen, workgroup";

fn conversation(r: &rusqlite::Row) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        id: r.get(0)?,
        session: Some(r.get::<_, String>(1)?).filter(|s| !s.is_empty()),
        opened_at: r.get(2)?,
        used_at: r.get(3)?,
        context: r.get(4)?,
        seen: r.get(5)?,
        workgroup: r.get(6)?,
    })
}

#[cfg(test)]
mod tests;
