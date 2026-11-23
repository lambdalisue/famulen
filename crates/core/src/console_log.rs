//! The console's side of the conversation: what was said there, kept in the
//! store and pushed to every open console.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::store::{Store, now};

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

/// Something every open console should show.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Message(Message),
    /// An approval was asked or answered.
    Approvals,
    /// Records changed; sections showing them may reload.
    Records,
}

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

    /// The latest messages, oldest first.
    pub fn recent(&self, limit: usize) -> anyhow::Result<Vec<Message>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT id, at, who, text, meta FROM console_messages ORDER BY id DESC LIMIT ?1",
        )?;
        let mut rows: Vec<Message> = stmt
            .query_map([limit as i64], |r| {
                let who: String = r.get(2)?;
                let meta: String = r.get(4)?;
                Ok(Message {
                    id: r.get(0)?,
                    at: r.get(1)?,
                    who: if who == "aide" { Who::Aide } else { Who::Owner },
                    text: r.get(3)?,
                    meta: serde_json::from_str(&meta).unwrap_or(Value::Null),
                })
            })?
            .collect::<Result<_, _>>()?;
        rows.reverse();
        Ok(rows)
    }
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
    fn recent_returns_the_latest_oldest_first() {
        let log = ConsoleLog::new(Store::open_in_memory().unwrap());
        for i in 0..5 {
            log.add(Who::Owner, format!("{i}"), Value::Null).unwrap();
        }
        let texts: Vec<_> = log.recent(3).unwrap().into_iter().map(|m| m.text).collect();
        assert_eq!(texts, ["2", "3", "4"]);
    }
}
