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
            Source::Console {
                reply_to: Some(to), ..
            } => vec![message_key(to.thread), message_key(to.message)],
            // One the owner chose the conversation of is linked already.
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

    /// The conversations used last, those used before `before` when it is
    /// given: the ones begun in what `workgroup` shows, every one when it is
    /// empty.
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
            "SELECT {COLUMNS} FROM conversations WHERE (used_at, id) < (?1, ?2)
                AND (?4 = '' OR workgroup IN ('', ?4, 'all'))
             ORDER BY used_at DESC, id DESC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![used_at, id, limit as i64, workgroup], conversation)?;
        Ok(rows.collect::<Result<_, _>>()?)
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
    /// turn of `row` is handed, quoted; empty when there is none.
    pub fn said_before(
        &self,
        log: &ConsoleLog,
        c: &Conversation,
        row: &StoredInput,
        workgroup: &str,
    ) -> anyhow::Result<String> {
        // The owner let one question be answered, not their talk be known.
        if row.input.reach() != Reach::Full && row.input.reach() != Reach::Inward {
            return Ok(String::new());
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
                    || self.of_message(&m)? == Some(c.id)
                {
                    continue;
                }
                elsewhere.push(m);
            }
        }
        let mut text = String::new();
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
        Ok(text)
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
