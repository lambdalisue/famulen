//! The console's side of the conversation: what was said there, kept in the
//! store and pushed to every open console.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::attachments::{self, Attached};
use crate::store::{Store, now};
use crate::workgroups::ALL;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    Owner,
    Aide,
}

impl Who {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Aide => "aide",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub at: i64,
    pub who: Who,
    pub text: String,
    /// Where it came from, which tab to open, and similar.
    pub meta: Value,
}

impl Message {
    /// The message it replies to.
    pub fn reply_to(&self) -> Option<i64> {
        self.meta.get("reply_to").and_then(Value::as_i64)
    }

    /// The message its thread begins with, when it replies to one.
    pub fn thread(&self) -> Option<i64> {
        self.meta.get("thread").and_then(Value::as_i64)
    }

    /// The input it is kept as, for what came in on Slack.
    pub fn input(&self) -> Option<i64> {
        self.meta.get("input").and_then(Value::as_i64)
    }

    /// Whether it was said on Slack and is shown here as well.
    pub fn on_slack(&self) -> bool {
        self.meta
            .get("origin")
            .and_then(Value::as_str)
            .is_some_and(|origin| origin.starts_with("Slack"))
    }

    /// Whether it shows while `workgroup` is chosen: one said in another
    /// workgroup is left out, one said in すべて is kept.
    pub fn shown_in(&self, workgroup: &str) -> bool {
        workgroup.is_empty()
            || workgroup == ALL
            || self
                .meta
                .get("workgroup")
                .and_then(Value::as_str)
                .is_none_or(|of| of == workgroup || of == ALL)
    }

    /// The files that came with it.
    pub fn attached(&self) -> Vec<Attached> {
        self.meta
            .get("attached")
            .and_then(|v| Vec::deserialize(v).ok())
            .unwrap_or_default()
    }

    /// What was said, with the names of the files that came with it.
    pub fn gist(&self) -> String {
        attachments::with_names(&self.text, &self.attached())
    }

    /// The first `limit` characters of it, marked when cut.
    pub fn excerpt(&self, limit: usize) -> String {
        let gist = self.gist();
        match gist.char_indices().nth(limit) {
            Some((end, _)) => format!("{}…", &gist[..end]),
            None => gist,
        }
    }
}

/// Something every open console should show.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Message(Message),
    /// An approval was asked or answered.
    Approvals,
    /// What the secretary is doing for the owner may have changed.
    Working,
    /// Records changed; sections showing them may reload.
    Records,
}

/// How far back a message is looked for.
const RECENT: usize = 1000;

/// How many rows are read at a time, so that the store is not held from
/// others while a long conversation is searched.
const CHUNK: usize = 500;

#[derive(Clone)]
pub struct ConsoleLog {
    store: Store,
    events: broadcast::Sender<Event>,
}

impl ConsoleLog {
    pub fn new(store: Store) -> Self {
        let (events, _) = broadcast::channel(256);
        Self { store, events }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub fn notify(&self, event: Event) {
        let _ = self.events.send(event);
    }

    pub fn add(&self, who: Who, text: impl AsRef<str>, meta: Value) -> anyhow::Result<Message> {
        let at = now();
        let id = {
            let conn = self.store.conn();
            conn.execute(
                "INSERT INTO console_messages (at, who, text, meta) VALUES (?1, ?2, ?3, ?4)",
                params![at, who.as_str(), text.as_ref(), meta.to_string()],
            )?;
            conn.last_insert_rowid()
        };
        let message = Message {
            id,
            at,
            who,
            text: text.as_ref().to_owned(),
            meta,
        };
        self.notify(Event::Message(message.clone()));
        Ok(message)
    }

    /// Say something in reply to `to`, in its thread.
    pub fn reply(
        &self,
        who: Who,
        text: impl AsRef<str>,
        mut meta: Value,
        to: &Message,
    ) -> anyhow::Result<Message> {
        let thread = to.thread().unwrap_or(to.id);
        meta["reply_to"] = to.id.into();
        meta["thread"] = thread.into();
        self.add(who, text, meta)
    }

    /// Every reply in the thread that begins with the message `root`, oldest
    /// first.
    pub fn replies(&self, root: i64) -> anyhow::Result<Vec<Message>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT id, at, who, text, meta FROM console_messages
             WHERE json_extract(meta, '$.thread') = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![root], row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The messages kept as one of `inputs`, oldest first.
    pub fn of_inputs(&self, inputs: &[i64]) -> anyhow::Result<Vec<Message>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT id, at, who, text, meta FROM console_messages
             WHERE json_extract(meta, '$.input') IN (SELECT value FROM json_each(?1))
             ORDER BY id",
        )?;
        let rows = stmt.query_map([serde_json::to_string(inputs)?], row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Show what came of the files of the message kept as the input `input`.
    pub fn set_attached(
        &self,
        input: i64,
        attached: &[crate::attachments::Attached],
    ) -> anyhow::Result<()> {
        let found = self
            .recent(RECENT)?
            .into_iter()
            .rev()
            .find(|m| m.meta.get("input").and_then(Value::as_i64) == Some(input));
        let Some(mut message) = found else {
            return Ok(());
        };
        let attached = serde_json::to_value(attached)?;
        if message.meta["attached"] == attached {
            return Ok(());
        }
        message.meta["attached"] = attached;
        self.store.conn().execute(
            "UPDATE console_messages SET meta = ?2 WHERE id = ?1",
            params![message.id, message.meta.to_string()],
        )?;
        self.notify(Event::Message(message));
        Ok(())
    }

    /// The message with this id.
    pub fn get(&self, id: i64) -> anyhow::Result<Option<Message>> {
        let found = self.rows(id.saturating_add(1), 1)?.pop();
        Ok(found.filter(|m| m.id == id))
    }

    /// The latest messages, oldest first.
    pub fn recent(&self, limit: usize) -> anyhow::Result<Vec<Message>> {
        self.before(0, limit, |_| true)
    }

    /// The latest messages `shown` says yes to that are older than the one
    /// with id `before`, oldest first; `0` starts from the newest.
    pub fn before(
        &self,
        before: i64,
        limit: usize,
        shown: impl Fn(&Message) -> bool,
    ) -> anyhow::Result<Vec<Message>> {
        let mut from = if before == 0 { i64::MAX } else { before };
        let mut found = vec![];
        while found.len() < limit {
            let rows = self.rows(from, CHUNK)?;
            let Some(last) = rows.last() else { break };
            from = last.id;
            found.extend(rows.into_iter().filter(&shown).take(limit - found.len()));
        }
        found.reverse();
        Ok(found)
    }

    /// The messages older than the one with id `before`, newest first.
    fn rows(&self, before: i64, limit: usize) -> anyhow::Result<Vec<Message>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT id, at, who, text, meta FROM console_messages
             WHERE id < ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![before, limit as i64], row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Message> {
    let who: String = r.get(2)?;
    let meta: String = r.get(4)?;
    Ok(Message {
        id: r.get(0)?,
        at: r.get(1)?,
        who: if who == "aide" { Who::Aide } else { Who::Owner },
        text: r.get(3)?,
        meta: serde_json::from_str(&meta).unwrap_or(Value::Null),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn a_message_is_kept_and_pushed() {
        let log = ConsoleLog::new(Store::open_in_memory().unwrap());
        let mut events = log.subscribe();
        log.add(Who::Aide, "こんにちは", json!({ "open": "calendar" }))
            .unwrap();
        let Event::Message(m) = events.recv().await.unwrap() else {
            panic!("not a message")
        };
        assert_eq!(m.text, "こんにちは");
        assert_eq!(log.recent(10).unwrap(), vec![m]);
    }

    #[test]
    fn a_message_is_found_by_its_id() {
        let log = ConsoleLog::new(Store::open_in_memory().unwrap());
        let m = log.add(Who::Owner, "a", Value::Null).unwrap();
        log.add(Who::Owner, "b", Value::Null).unwrap();
        assert_eq!(log.get(m.id).unwrap(), Some(m));
        assert_eq!(log.get(9).unwrap(), None);
    }

    #[test]
    fn a_reply_to_a_reply_is_in_the_same_thread() {
        let log = ConsoleLog::new(Store::open_in_memory().unwrap());
        let root = log.add(Who::Aide, "a", json!({})).unwrap();
        log.add(Who::Aide, "other", Value::Null).unwrap();
        let first = log.reply(Who::Owner, "b", json!({}), &root).unwrap();
        let second = log.reply(Who::Aide, "c", json!({}), &first).unwrap();

        assert_eq!(
            (second.reply_to(), second.thread()),
            (Some(first.id), Some(root.id))
        );
        assert_eq!(log.replies(root.id).unwrap(), vec![first, second]);
    }

    #[test]
    fn recent_returns_the_latest_oldest_first() {
        let log = ConsoleLog::new(Store::open_in_memory().unwrap());
        for i in 0..5 {
            log.add(Who::Owner, format!("{i}"), Value::Null).unwrap();
        }
        let texts: Vec<_> = log.recent(3).unwrap().into_iter().map(|m| m.text).collect();
        assert_eq!(texts, ["2", "3", "4"]);
    }

    #[test]
    fn before_reads_back_past_what_is_not_shown() {
        let log = ConsoleLog::new(Store::open_in_memory().unwrap());
        let ids: Vec<i64> = (0..6)
            .map(|i| log.add(Who::Owner, format!("{i}"), Value::Null).unwrap().id)
            .collect();
        let even = |m: &Message| m.text.parse::<i64>().unwrap() % 2 == 0;
        let texts = |before, limit| -> Vec<String> {
            let found = log.before(before, limit, even).unwrap();
            found.into_iter().map(|m| m.text).collect()
        };
        assert_eq!(texts(0, 2), ["2", "4"]);
        assert_eq!(texts(ids[2], 2), ["0"]);
        assert!(texts(ids[0], 2).is_empty());
    }
}
