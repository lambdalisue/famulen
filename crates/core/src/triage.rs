//! Triage: one narrow question asked of each message in a channel the
//! secretary is attending — is it said to the secretary, or part of a
//! conversation with it? A function with its own model and cap, apart from
//! the lookout, which reads in batches.

use std::future::Future;

use rusqlite::params;
use serde_json::{Value, json};

use crate::lookout::{Item, dedup_key};
use crate::store::{Store, now};

/// The answer to whether one message is said to the secretary.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "addressed": { "type": "boolean" } },
        "required": ["addressed"],
        "additionalProperties": false
    })
}

/// What triage decided of one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub id: i64,
    /// Unix seconds.
    pub at: i64,
    pub source: String,
    pub title: String,
    pub addressed: bool,
    pub reason: String,
}

#[derive(Clone)]
pub struct Triage {
    store: Store,
}

impl Triage {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Weigh one message. `judge` answers `Some(true)` when it is said to the
    /// secretary and `None` when triage's model is unassigned or over its cap.
    /// Returns `None` for a message weighed before, which is not judged again;
    /// otherwise whether it passes. Over the cap or on a failure it does not.
    pub async fn weigh<J, F>(
        &self,
        source: &str,
        item: &Item,
        judge: J,
    ) -> anyhow::Result<Option<bool>>
    where
        J: FnOnce(Item) -> F,
        F: Future<Output = anyhow::Result<Option<bool>>>,
    {
        let key = dedup_key(source, item);
        let inserted = self.store.conn().execute(
            "INSERT OR IGNORE INTO triage_items (source, dedup_key, at, title, addressed, reason)
             VALUES (?1, ?2, ?3, ?4, 0, 'pending')",
            params![source, key, now(), item.title],
        )?;
        if inserted == 0 {
            return Ok(None);
        }
        let (addressed, reason) = match judge(item.clone()).await {
            Ok(Some(true)) => (true, "秘書に話しかけている".to_owned()),
            Ok(Some(false)) => (false, "秘書宛てではない".to_owned()),
            Ok(None) => (
                false,
                "トリアージが上限に達した · メンションだけに戻る".to_owned(),
            ),
            Err(error) => (false, format!("トリアージが失敗した（{error}）")),
        };
        self.store.conn().execute(
            "UPDATE triage_items SET addressed = ?2, reason = ?3 WHERE dedup_key = ?1",
            params![key, addressed, reason],
        )?;
        Ok(Some(addressed))
    }

    /// The latest decisions older than the one with id `before`, newest
    /// first; `0` starts from the newest.
    pub fn before(&self, before: i64, limit: usize) -> anyhow::Result<Vec<Decision>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT id, at, source, title, addressed, reason FROM triage_items
             WHERE reason != 'pending' AND (?1 = 0 OR id < ?1) ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![before, limit as i64], |r| {
            Ok(Decision {
                id: r.get(0)?,
                at: r.get(1)?,
                source: r.get(2)?,
                title: r.get(3)?,
                addressed: r.get(4)?,
                reason: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(t: &str) -> Item {
        Item {
            title: t.into(),
            body: format!("{t} body"),
        }
    }

    fn triage() -> Triage {
        Triage::new(Store::open_in_memory().unwrap())
    }

    #[tokio::test]
    async fn a_message_passes_only_when_said_to_the_secretary() {
        let t = triage();
        let yes = |_| std::future::ready(anyhow::Ok(Some(true)));
        let no = |_| std::future::ready(anyhow::Ok(Some(false)));

        assert_eq!(t.weigh("slack", &item("a"), yes).await.unwrap(), Some(true));
        assert_eq!(t.weigh("slack", &item("b"), no).await.unwrap(), Some(false));

        let recent = t.before(0, 10).unwrap();
        let addressed = |title: &str| recent.iter().find(|d| d.title == title).unwrap().addressed;
        assert!(addressed("a"));
        assert!(!addressed("b"));
    }

    #[tokio::test]
    async fn decisions_before_a_cursor_are_only_the_older_ones() {
        let t = triage();
        for title in ["a", "b", "c"] {
            let no = |_| std::future::ready(anyhow::Ok(Some(false)));
            t.weigh("slack", &item(title), no).await.unwrap();
        }
        let newest = t.before(0, 1).unwrap();
        let older = t.before(newest[0].id, 10).unwrap();
        assert_eq!(newest[0].title, "c");
        assert_eq!(
            older.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
    }

    #[tokio::test]
    async fn a_message_weighed_before_is_not_judged_again() {
        let t = triage();
        let yes = |_| std::future::ready(anyhow::Ok(Some(true)));
        assert_eq!(t.weigh("slack", &item("a"), yes).await.unwrap(), Some(true));

        let judged = std::cell::Cell::new(false);
        let again = t
            .weigh("slack", &item("a"), |_| {
                judged.set(true);
                std::future::ready(anyhow::Ok(Some(true)))
            })
            .await
            .unwrap();

        assert!(again.is_none() && !judged.get());
    }

    #[tokio::test]
    async fn over_the_cap_or_failing_a_message_does_not_pass() {
        let t = triage();
        let over = |_| std::future::ready(anyhow::Ok(None));
        let failed = |_| std::future::ready(Err(anyhow::anyhow!("down")));

        assert_eq!(
            t.weigh("slack", &item("a"), over).await.unwrap(),
            Some(false)
        );
        assert_eq!(
            t.weigh("slack", &item("b"), failed).await.unwrap(),
            Some(false)
        );
    }
}
