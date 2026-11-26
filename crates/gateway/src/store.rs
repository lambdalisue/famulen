//! The gateway's own database: everything it keeps but the secrets and the
//! service definitions.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS connections (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL,
    transport  TEXT NOT NULL,
    config     TEXT NOT NULL,
    secret     TEXT NOT NULL DEFAULT '{}',
    account    TEXT NOT NULL DEFAULT '',
    status     TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS connection_tools (
    connection  TEXT NOT NULL,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    read_only   INTEGER,
    always_ask  INTEGER NOT NULL DEFAULT 0,
    seen_at     INTEGER NOT NULL,
    PRIMARY KEY (connection, name)
);
CREATE TABLE IF NOT EXISTS scoped_permissions (
    scope TEXT NOT NULL,
    tool  TEXT NOT NULL,
    permission  TEXT NOT NULL,
    PRIMARY KEY (scope, tool)
);
CREATE TABLE IF NOT EXISTS tool_defaults (
    connection TEXT NOT NULL,
    tool       TEXT NOT NULL,
    permission       TEXT NOT NULL,
    PRIMARY KEY (connection, tool)
);
CREATE TABLE IF NOT EXISTS destinations (
    connection TEXT NOT NULL,
    receiver   TEXT NOT NULL,
    name       TEXT NOT NULL,
    PRIMARY KEY (connection, receiver)
);
CREATE TABLE IF NOT EXISTS receipts (
    id         INTEGER PRIMARY KEY,
    at         INTEGER NOT NULL,
    connection TEXT NOT NULL,
    receiver   TEXT NOT NULL,
    verified   INTEGER NOT NULL,
    digest     TEXT NOT NULL,
    size       INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS log (
    id         INTEGER PRIMARY KEY,
    at         INTEGER NOT NULL,
    actor      TEXT NOT NULL,
    connection TEXT NOT NULL,
    tool       TEXT NOT NULL,
    decision   TEXT NOT NULL,
    reason     TEXT NOT NULL,
    secret     TEXT NOT NULL,
    outcome    TEXT NOT NULL,
    digest     TEXT NOT NULL,
    ms         INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS tickets (
    id         INTEGER PRIMARY KEY,
    actor      TEXT NOT NULL,
    connection TEXT NOT NULL,
    tool       TEXT NOT NULL,
    input      TEXT NOT NULL,
    shown      TEXT NOT NULL,
    sealed     TEXT NOT NULL DEFAULT '',
    reason     TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'held',
    created_at INTEGER NOT NULL,
    answered_at INTEGER
);
CREATE TABLE IF NOT EXISTS ticket_origins (
    ticket INTEGER PRIMARY KEY,
    origin TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS passes (
    ticket     INTEGER PRIMARY KEY,
    actor      TEXT NOT NULL,
    connection TEXT NOT NULL,
    tool       TEXT NOT NULL,
    input      TEXT NOT NULL,
    used       INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS definition_changes (
    id         INTEGER PRIMARY KEY,
    definition TEXT NOT NULL,
    kind       TEXT NOT NULL,
    before     TEXT,
    after      TEXT,
    previous   TEXT,
    actor      TEXT NOT NULL,
    why        TEXT NOT NULL,
    at         INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS actor_grants (
    actor      TEXT NOT NULL,
    connection TEXT NOT NULL,
    tool       TEXT NOT NULL,
    PRIMARY KEY (actor, connection, tool)
);
CREATE TABLE IF NOT EXISTS carried_out (
    ticket INTEGER PRIMARY KEY,
    ok     INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS actor_deadlines (
    actor TEXT PRIMARY KEY,
    until INTEGER NOT NULL
);
";

/// Seconds since the Unix epoch.
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// A handle to the gateway's database. Clones share one connection.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> anyhow::Result<Self> {
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        crate::lock::lock(&self.conn)
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

    pub fn set_state(&self, key: impl AsRef<str>, value: impl AsRef<str>) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO state (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key.as_ref(), value.as_ref()],
        )?;
        Ok(())
    }

    pub fn clear_state(&self, key: impl AsRef<str>) -> anyhow::Result<()> {
        self.conn()
            .execute("DELETE FROM state WHERE key = ?1", params![key.as_ref()])?;
        Ok(())
    }
}
