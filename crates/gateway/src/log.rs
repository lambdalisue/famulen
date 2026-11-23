//! The gateway's log: what was called, held, refused or changed, and by whom.
//! Entries are only ever added. None holds a body, an answer or a secret.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::store::{Store, now};

/// One entry, in the fields `SPEC.md` names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub at: i64,
    /// Who made the call or the change: `owner`, `secretary`, …
    pub actor: String,
    pub connection: String,
    /// The tool called, or the change made (`vault.put`, `connection.remove`).
    pub tool: String,
    /// `allow`, `confirm` or `deny`.
    pub decision: String,
    pub reason: String,
    /// The name of the secret used or changed; never its value.
    pub secret: String,
    pub outcome: String,
    /// Hashes and sizes of what was sent and answered; empty for a change.
    pub digest: String,
    pub ms: i64,
}

/// A body's hash and size as the log keeps it, in place of the body.
pub fn digest(body: &[u8]) -> String {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(body);
    let hex: String = hash.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex} {}B", body.len())
}

#[derive(Clone)]
pub struct Log {
    store: Store,
}

impl Log {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Add `entry`, at now when it has no time.
    pub fn write(&self, entry: &LogEntry) -> anyhow::Result<()> {
        let at = if entry.at == 0 { now() } else { entry.at };
        self.store.conn().execute(
            "INSERT INTO log (at, actor, connection, tool, decision, reason, secret, outcome, digest, ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                at,
                entry.actor,
                entry.connection,
                entry.tool,
                entry.decision,
                entry.reason,
                entry.secret,
                entry.outcome,
                entry.digest,
                entry.ms,
            ],
        )?;
        Ok(())
    }

    /// The latest `limit` entries, the newest first.
    pub fn recent(&self, limit: usize) -> anyhow::Result<Vec<LogEntry>> {
        Ok(self
            .page(&LogQuery {
                limit,
                ..LogQuery::default()
            })?
            .into_iter()
            .map(|(_, e)| e)
            .collect())
    }

    /// The entries `query` picks, the newest first, each with the place it
    /// holds so the next page can start below it.
    pub fn page(&self, query: &LogQuery) -> anyhow::Result<Vec<(i64, LogEntry)>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT id, at, actor, connection, tool, decision, reason, secret, outcome, digest, ms
             FROM log
             WHERE (?1 = '' OR actor = ?1) AND (?2 = '' OR connection = ?2)
               AND (?3 = '' OR decision = ?3) AND (?4 = 0 OR id < ?4)
             ORDER BY id DESC LIMIT ?5",
        )?;
        let rows = stmt.query_map(
            params![
                query.actor,
                query.connection,
                query.decision,
                query.before,
                query.limit as i64
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    LogEntry {
                        at: row.get(1)?,
                        actor: row.get(2)?,
                        connection: row.get(3)?,
                        tool: row.get(4)?,
                        decision: row.get(5)?,
                        reason: row.get(6)?,
                        secret: row.get(7)?,
                        outcome: row.get(8)?,
                        digest: row.get(9)?,
                        ms: row.get(10)?,
                    },
                ))
            },
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

/// Which entries to read; an empty field picks every value.
#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    pub actor: String,
    pub connection: String,
    pub decision: String,
    /// Only entries placed before this one; 0 for the newest.
    pub before: i64,
    pub limit: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_come_back_newest_first_with_a_time() {
        let log = Log::new(Store::open_in_memory().unwrap());
        for tool in ["vault.put", "vault.delete"] {
            log.write(&LogEntry {
                actor: "owner".into(),
                tool: tool.into(),
                decision: "allow".into(),
                secret: "pw".into(),
                ..LogEntry::default()
            })
            .unwrap();
        }

        let entries = log.recent(10).unwrap();
        assert_eq!(
            entries.iter().map(|e| e.tool.as_str()).collect::<Vec<_>>(),
            ["vault.delete", "vault.put"]
        );
        assert!(entries.iter().all(|e| e.at > 0));
        assert_eq!(log.recent(1).unwrap().len(), 1);
    }

    #[test]
    fn a_page_is_filtered_and_the_next_one_starts_below_it() {
        let log = Log::new(Store::open_in_memory().unwrap());
        for (actor, decision) in [
            ("owner", "allow"),
            ("secretary", "deny"),
            ("secretary", "allow"),
            ("secretary", "allow"),
        ] {
            log.write(&LogEntry {
                actor: actor.into(),
                decision: decision.into(),
                ..LogEntry::default()
            })
            .unwrap();
        }
        let query = LogQuery {
            actor: "secretary".into(),
            decision: "allow".into(),
            limit: 1,
            ..LogQuery::default()
        };

        let first = log.page(&query).unwrap();
        let second = log
            .page(&LogQuery {
                before: first[0].0,
                ..query.clone()
            })
            .unwrap();
        let third = log
            .page(&LogQuery {
                before: second[0].0,
                ..query
            })
            .unwrap();

        assert_eq!((first[0].0, second[0].0), (4, 3));
        assert!(third.is_empty());
    }

    #[test]
    fn a_digest_is_a_short_hash_and_a_size() {
        assert_eq!(digest(b"abc"), "sha256:ba7816bf8f01cfea 3B");
    }
}
