//! Watching the outside world: sources polled through connections, and the
//! one model call per batch that passes only what concerns the owner on to
//! the secretary.
//!
//! The model reads a batch at once and answers, for every item, whether it
//! concerns the owner and how soon, and puts the related items about one
//! subject together with where each group goes. Only "act" and "notify"
//! reach the secretary at once; the rest of what concerns the owner waits
//! for the morning digest.

mod feed;
mod reading;
mod screen;
mod sources;
mod verdict;

pub use feed::*;
pub use reading::*;
pub use sources::*;
pub use verdict::*;

use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

pub use famulen_gateway::Item;

use crate::input::{Input, Sender, Source};
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The model found it does not concern the owner.
    Unrelated,
    /// Concerns the owner, who only wants it in the log.
    Logged,
    /// Kept for the morning digest.
    Digest,
    /// Passed to the secretary to tell the owner.
    Notify,
    /// Passed to the secretary to act on.
    Act,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unrelated => "unrelated",
            Self::Logged => "logged",
            Self::Digest => "digest",
            Self::Notify => "notify",
            Self::Act => "act",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "unrelated" => Self::Unrelated,
            "logged" => Self::Logged,
            "digest" => Self::Digest,
            "notify" => Self::Notify,
            "act" => Self::Act,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unrelated => "関わりがない",
            Self::Logged => "ログだけ",
            Self::Digest => "朝のまとめへ",
            Self::Notify => "秘書へ · 知らせる",
            Self::Act => "秘書へ · 動く",
        }
    }
}

/// Where the owner sends what a source finds to concern them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    Secretary,
    Digest,
    Log,
}

impl Route {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Secretary => "secretary",
            Self::Digest => "digest",
            Self::Log => "log",
        }
    }

    pub(super) fn parse(s: &str) -> Option<Self> {
        [Self::Secretary, Self::Digest, Self::Log]
            .into_iter()
            .find(|r| r.as_str() == s)
    }

    /// Where something the model sends to `outcome` goes.
    fn instead_of(self, outcome: Outcome) -> Outcome {
        match (self, outcome) {
            (_, Outcome::Unrelated) => Outcome::Unrelated,
            (Self::Secretary, Outcome::Act) => Outcome::Act,
            (Self::Secretary, _) => Outcome::Notify,
            (Self::Digest, _) => Outcome::Digest,
            (Self::Log, _) => Outcome::Logged,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub id: i64,
    pub source: String,
    pub at: i64,
    pub title: String,
    pub outcome: Outcome,
    pub reason: String,
    pub feedback: Option<String>,
}

/// An item as it was read, and what was decided of it.
#[derive(Debug, Clone, PartialEq)]
pub struct SeenItem {
    pub item: Item,
    pub at: i64,
    pub outcome: Outcome,
    pub reason: String,
}

/// What the lookout watches and what it decided.
#[derive(Clone)]
pub struct Lookout {
    store: Store,
}

impl Lookout {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// The items kept for the digest since the last one, as one input, and
    /// marks them as digested.
    pub fn digest(&self) -> anyhow::Result<Option<Input>> {
        let rows: Vec<(i64, String, String, String)> = {
            let conn = self.store.conn();
            let mut stmt = conn.prepare(
                "SELECT id, source, title, reason FROM lookout_items WHERE outcome = 'digest' ORDER BY id",
            )?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<Result<_, _>>()?
        };
        if rows.is_empty() {
            return Ok(None);
        }
        let first = rows[0].0;
        let last = rows[rows.len() - 1].0;
        let lines: Vec<String> = rows
            .iter()
            .map(|(id, source, title, reason)| format!("- #{id} [{source}] {title} ({reason})"))
            .collect();
        self.store.conn().execute(
            "UPDATE lookout_items SET outcome = 'digested' WHERE outcome = 'digest' AND id BETWEEN ?1 AND ?2",
            params![first, last],
        )?;
        Ok(Some(Input {
            sender: Sender::System,
            source: Source::Lookout {
                item: last,
                source: "digest".to_owned(),
            },
            text: format!(
                "Morning digest from the lookout. Tell the owner briefly what matters, or nothing if none of it does:\n{}",
                lines.join("\n")
            ),
        }))
    }

    /// The latest decisions, newest first.
    pub fn recent(&self, limit: usize) -> anyhow::Result<Vec<Decision>> {
        self.before(0, limit)
    }

    /// The decisions older than the one with id `before`, newest first; `0`
    /// starts from the newest.
    pub fn before(&self, before: i64, limit: usize) -> anyhow::Result<Vec<Decision>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT id, source, at, title, outcome, reason, feedback FROM lookout_items
             WHERE outcome != 'pending' AND (?1 = 0 OR id < ?1) ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![before, limit as i64], decision)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The latest item decided on, read through `connection`, whose body starts with `start` and whose
    /// first line ends with `end`, its line break with it; an empty `end`
    /// leaves the line as it is.
    pub fn seen(
        &self,
        connection: &str,
        start: &str,
        end: &str,
    ) -> anyhow::Result<Option<SeenItem>> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT title, body, at, outcome, reason FROM lookout_items
                 WHERE outcome != 'pending'
                   AND source IN (SELECT CAST(id AS TEXT) FROM lookout_sources WHERE connection = ?3)
                   AND substr(body, 1, length(?1)) = ?1
                   AND (?2 = '' OR substr(substr(body, 1, instr(body, char(10))), -length(?2)) = ?2)
                 ORDER BY id DESC LIMIT 1",
                params![start, end, connection],
                |r| {
                    let outcome: String = r.get(3)?;
                    Ok(SeenItem {
                        item: Item {
                            title: r.get(0)?,
                            body: r.get(1)?,
                        },
                        at: r.get(2)?,
                        outcome: Outcome::parse(&outcome).unwrap_or(Outcome::Digest),
                        reason: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// Items since `since`: received, found to concern the owner, passed to
    /// the secretary.
    pub fn funnel(&self, since: i64) -> anyhow::Result<[i64; 3]> {
        let conn = self.store.conn();
        let count =
            |sql: &str| -> anyhow::Result<i64> { Ok(conn.query_row(sql, [since], |r| r.get(0))?) };
        Ok([
            count("SELECT COUNT(*) FROM lookout_items WHERE at >= ?1")?,
            count(
                "SELECT COUNT(*) FROM lookout_items WHERE at >= ?1 AND outcome NOT IN ('pending', 'unrelated')",
            )?,
            count(
                "SELECT COUNT(*) FROM lookout_items WHERE at >= ?1 AND outcome IN ('act', 'notify')",
            )?,
        ])
    }

    /// The owner's 要らなかった / 拾ってほしかった on a decision.
    pub fn feedback(&self, id: i64, wanted: bool) -> anyhow::Result<Option<Decision>> {
        Ok(self
            .store
            .conn()
            .query_row(
                "UPDATE lookout_items SET feedback = ?2 WHERE id = ?1 AND outcome != 'pending'
                 RETURNING id, source, at, title, outcome, reason, feedback",
                params![id, if wanted { "wanted" } else { "unwanted" }],
                decision,
            )
            .optional()?)
    }
}

fn decision(row: &Row<'_>) -> rusqlite::Result<Decision> {
    let outcome: String = row.get(4)?;
    Ok(Decision {
        id: row.get(0)?,
        source: row.get(1)?,
        at: row.get(2)?,
        title: row.get(3)?,
        // What was in a digest keeps that as its outcome once it was sent.
        outcome: Outcome::parse(&outcome).unwrap_or(Outcome::Digest),
        reason: row.get(5)?,
        feedback: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn item(t: &str) -> Item {
        Item {
            title: t.into(),
            body: format!("{t} body"),
        }
    }

    pub(super) fn lookout() -> Lookout {
        Lookout::new(Store::open_in_memory().unwrap())
    }

    pub(super) type Answer = std::future::Ready<anyhow::Result<Option<Verdict>>>;

    pub(super) fn related(urgency: Urgency) -> Screened {
        Screened {
            related: true,
            urgency,
        }
    }

    pub(super) fn unrelated() -> Screened {
        Screened {
            related: false,
            urgency: Urgency::Later,
        }
    }

    pub(super) fn group(items: &[usize], outcome: Outcome, title: &str, reason: &str) -> Group {
        Group {
            items: items.to_vec(),
            outcome,
            title: title.into(),
            reason: reason.into(),
        }
    }

    /// Every item related and later, each a group of its own going to `outcome`.
    pub(super) fn each_alone(outcome: Outcome) -> impl FnOnce(Vec<Item>) -> Answer {
        move |items| {
            std::future::ready(Ok(Some(Verdict {
                items: items.iter().map(|_| related(Urgency::Later)).collect(),
                groups: (0..items.len())
                    .map(|i| group(&[i], outcome, "", "r"))
                    .collect(),
            })))
        }
    }

    #[tokio::test]
    async fn decisions_before_a_cursor_are_only_the_older_ones() {
        let w = lookout();
        for title in ["a", "b", "c"] {
            w.screen("x", vec![item(title)], |_: Vec<Item>| {
                std::future::ready(Ok(Some(Verdict {
                    items: vec![related(Urgency::Later)],
                    groups: vec![group(&[0], Outcome::Digest, "", "r")],
                })))
            })
            .await
            .unwrap();
        }
        let newest = w.before(0, 1).unwrap();
        let older = w.before(newest[0].id, 10).unwrap();
        assert_eq!(newest[0].title, "c");
        assert_eq!(
            older.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
    }

    #[tokio::test]
    async fn the_funnel_counts_what_was_seen_related_and_passed_on_and_feedback_is_kept() {
        let w = lookout();
        w.screen("x", vec![item("a"), item("b"), item("c")], |_| {
            std::future::ready(Ok(Some(Verdict {
                items: vec![
                    related(Urgency::Today),
                    unrelated(),
                    related(Urgency::Later),
                ],
                groups: vec![
                    group(&[0], Outcome::Notify, "", "r"),
                    group(&[2], Outcome::Digest, "", "r"),
                ],
            })))
        })
        .await
        .unwrap();
        assert_eq!(w.funnel(0).unwrap(), [3, 2, 1]);
        let id = w.before(0, 1).unwrap()[0].id;
        assert_eq!(
            w.feedback(id, false).unwrap().unwrap().feedback.as_deref(),
            Some("unwanted")
        );
    }

    #[tokio::test]
    async fn feedback_is_taken_on_a_decision_however_old() {
        let w = lookout();
        for batch in 0..3 {
            let items = (0..200).map(|n| item(&format!("{batch}-{n}"))).collect();
            w.screen("x", items, each_alone(Outcome::Logged))
                .await
                .unwrap();
        }

        let decision = w.feedback(1, true).unwrap().unwrap();

        assert_eq!(decision.title, "0-0");
        assert_eq!(decision.feedback.as_deref(), Some("wanted"));
    }
}
