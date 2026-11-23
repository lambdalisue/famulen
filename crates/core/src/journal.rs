//! What happened, in order, for the owner to read on the console: inputs
//! delivered, tools the model called and how famulen answered, what the model
//! wrote, and where each turn ended.

use rusqlite::params;

use crate::store::{Store, now};

/// The longest detail kept for one entry, in characters.
const DETAIL_CHARS: usize = 4000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub kind: String,
    pub detail: String,
    pub created_at: i64,
}

#[derive(Clone)]
pub struct Journal {
    store: Store,
}

impl Journal {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Add an entry. A failure is logged, never raised: the journal must not
    /// stop the work it describes.
    pub fn record(&self, kind: &str, detail: impl AsRef<str>) {
        let detail = truncate(detail.as_ref());
        let written = self.store.conn().execute(
            "INSERT INTO journal (kind, detail, created_at) VALUES (?1, ?2, ?3)",
            params![kind, detail, now()],
        );
        if let Err(error) = written {
            tracing::warn!(%error, kind, "could not write the journal");
        }
    }

    /// The newest entries first.
    pub fn recent(&self, limit: usize) -> anyhow::Result<Vec<Entry>> {
        self.before(0, limit)
    }

    /// The newest entries older than the one with id `before` first; `0`
    /// starts from the newest.
    pub fn before(&self, before: i64, limit: usize) -> anyhow::Result<Vec<Entry>> {
        let conn = self.store.conn();
        let mut statement = conn.prepare(
            "SELECT id, kind, detail, created_at FROM journal
             WHERE ?1 = 0 OR id < ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let entries = statement
            .query_map(params![before, limit as i64], |row| {
                Ok(Entry {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    detail: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(entries)
    }
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= DETAIL_CHARS {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(DETAIL_CHARS).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_come_back_newest_first() {
        let journal = Journal::new(Store::open_in_memory().unwrap());
        journal.record("input", "first");
        journal.record("turn", "second");
        let recent = journal.recent(10).unwrap();
        assert_eq!(
            recent.iter().map(|e| e.detail.as_str()).collect::<Vec<_>>(),
            ["second", "first"]
        );
        assert_eq!(recent[1].kind, "input");
    }

    #[test]
    fn the_limit_keeps_only_the_newest() {
        let journal = Journal::new(Store::open_in_memory().unwrap());
        for i in 0..5 {
            journal.record("text", i.to_string());
        }
        assert_eq!(journal.recent(2).unwrap().len(), 2);
    }

    #[test]
    fn before_reads_only_what_is_older_than_the_cursor() {
        let journal = Journal::new(Store::open_in_memory().unwrap());
        for i in 0..5 {
            journal.record("text", i.to_string());
        }
        let cursor = journal.recent(5).unwrap()[2].id;
        let older = journal.before(cursor, 10).unwrap();
        assert_eq!(
            older.iter().map(|e| e.detail.as_str()).collect::<Vec<_>>(),
            ["1", "0"]
        );
    }

    #[test]
    fn a_long_detail_is_cut_on_a_character_boundary() {
        let journal = Journal::new(Store::open_in_memory().unwrap());
        journal.record("text", "あ".repeat(DETAIL_CHARS + 10));
        let detail = &journal.recent(1).unwrap()[0].detail;
        assert_eq!(detail.chars().count(), DETAIL_CHARS + 1);
        assert!(detail.ends_with('…'));
    }
}
