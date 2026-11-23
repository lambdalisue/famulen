//! Service connections: one row per service and account, reached as its
//! service definition says, or directly as a remote MCP server, a local MCP
//! server or an HTTP API.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::permissions::scope_of;
use crate::store::{Store, now};

mod api;
mod target;
pub use api::{ApiRequest, api_call};
pub use target::{guess_kind, words};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transport {
    /// A server this machine runs.
    McpLocal {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    /// A server reached over streamable HTTP.
    McpRemote { url: String },
    /// An HTTP API, called with one tool per method.
    Api {
        base_url: String,
        #[serde(default)]
        openapi_url: String,
    },
    /// Reached as its service definition says, with this account's values.
    Defined {
        definition: String,
        #[serde(default)]
        values: BTreeMap<String, String>,
    },
}

impl Transport {
    pub fn label(&self) -> &'static str {
        match self {
            Self::McpLocal { .. } => "ローカル MCP",
            Self::McpRemote { .. } => "リモート MCP",
            Self::Api { .. } => "API",
            Self::Defined { .. } => "サービス定義",
        }
    }
}

/// What the gateway keeps to reach a connection: never shown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Secret {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// The authorization server's metadata, to refresh tokens with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_server: Option<Value>,
    /// The canonical resource the tokens are for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_header: Option<String>,
    /// Tokens a service definition's login named, by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tokens: BTreeMap<String, String>,
    /// Account values a service definition marks secret, by field name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub account: BTreeMap<String, String>,
    /// The fields kept in the gateway's vault instead.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub in_vault: BTreeSet<String>,
}

impl Secret {
    /// Whether the secret has `name` (`bearer`, `api_key`, …), here or in the vault.
    pub fn holds(&self, name: &str) -> bool {
        self.in_vault.contains(name) || self.shown_here(name).is_some()
    }

    /// The header the owner's key is sent in, with its value.
    pub fn key_header(&self) -> Option<(String, String)> {
        let key = self.api_key.as_ref()?;
        let header = self.api_key_header.as_deref().unwrap_or("Authorization");
        let value = if header.eq_ignore_ascii_case("authorization") {
            format!("Bearer {key}")
        } else {
            key.clone()
        };
        Some((header.to_owned(), value))
    }

    fn shown_here(&self, name: &str) -> Option<&str> {
        match name {
            "bearer" => self.bearer.as_deref(),
            "refresh_token" => self.refresh_token.as_deref(),
            "client_secret" => self.client_secret.as_deref(),
            "api_key" => self.api_key.as_deref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub name: String,
    /// The service kind whose defaults it inherits, such as `gmail`.
    pub kind: String,
    pub transport: Transport,
    pub secret: Secret,
    pub account: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConnectionTool {
    pub name: String,
    pub description: String,
    pub read_only: Option<bool>,
    /// It cannot be undone or it reaches other people, so the auto permission
    /// always asks the owner first.
    #[serde(default)]
    pub always_ask: bool,
}

/// The status of a connection that can be used.
pub const CONNECTED: &str = "つながっている";

/// The status of a connection whose login no longer holds; why may follow.
pub const LOGIN_AGAIN: &str = "ログインし直す";

/// The state key `receiver` of connection `id` keeps where to go on from.
pub(crate) fn resume_key(id: &str, receiver: &str) -> String {
    format!("receive.resume.{id}.{receiver}")
}

/// Ids are used inside tool names, so they stay to `[a-z0-9-]`.
pub fn check_id(id: &str) -> anyhow::Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 40
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
        && id != "famulen";
    anyhow::ensure!(
        ok,
        "a connection id is lowercase letters, digits and '-': {id}"
    );
    Ok(())
}

#[derive(Clone)]
pub struct Connections {
    store: Store,
}

impl Connections {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    pub fn add(
        &self,
        id: impl AsRef<str>,
        name: impl AsRef<str>,
        kind: impl AsRef<str>,
        transport: Transport,
        secret: Secret,
        account: impl AsRef<str>,
    ) -> anyhow::Result<Connection> {
        let id = id.as_ref();
        check_id(id)?;
        anyhow::ensure!(
            !name.as_ref().trim().is_empty(),
            "a connection needs a name"
        );
        anyhow::ensure!(self.get(id)?.is_none(), "connection {id} already exists");
        self.store.conn().execute(
            "INSERT INTO connections (id, name, kind, transport, config, secret, account, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                name.as_ref().trim(),
                kind.as_ref().trim().to_ascii_lowercase(),
                transport.label(),
                serde_json::to_string(&transport)?,
                serde_json::to_string(&secret)?,
                account.as_ref(),
                now()
            ],
        )?;
        self.require(id)
    }

    pub fn get(&self, id: impl AsRef<str>) -> anyhow::Result<Option<Connection>> {
        let row = self
            .store
            .conn()
            .query_row(
                "SELECT id, name, kind, config, secret, account, status FROM connections WHERE id = ?1",
                [id.as_ref()],
                connection_row,
            )
            .optional()?;
        row.transpose()
    }

    /// The connection `id`, or an error saying there is none.
    pub fn require(&self, id: impl AsRef<str>) -> anyhow::Result<Connection> {
        let id = id.as_ref();
        self.get(id)?
            .ok_or_else(|| anyhow::anyhow!("no connection {id}"))
    }

    pub fn list(&self) -> anyhow::Result<Vec<Connection>> {
        let rows: Vec<anyhow::Result<Connection>> = {
            let conn = self.store.conn();
            let mut stmt = conn.prepare(
                "SELECT id, name, kind, config, secret, account, status FROM connections ORDER BY kind, name",
            )?;
            stmt.query_map([], connection_row)?
                .collect::<Result<_, _>>()?
        };
        rows.into_iter().collect()
    }

    pub fn update(
        &self,
        id: impl AsRef<str>,
        name: Option<&str>,
        kind: Option<&str>,
        account: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.store.conn();
        if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
            conn.execute(
                "UPDATE connections SET name = ?2 WHERE id = ?1",
                params![id.as_ref(), name.trim()],
            )?;
        }
        if let Some(kind) = kind.filter(|k| !k.trim().is_empty()) {
            conn.execute(
                "UPDATE connections SET kind = ?2 WHERE id = ?1",
                params![id.as_ref(), kind.trim().to_ascii_lowercase()],
            )?;
        }
        if let Some(account) = account {
            conn.execute(
                "UPDATE connections SET account = ?2 WHERE id = ?1",
                params![id.as_ref(), account],
            )?;
        }
        Ok(())
    }

    pub fn set_secret(&self, id: impl AsRef<str>, secret: &Secret) -> anyhow::Result<()> {
        self.store.conn().execute(
            "UPDATE connections SET secret = ?2 WHERE id = ?1",
            params![id.as_ref(), serde_json::to_string(secret)?],
        )?;
        Ok(())
    }

    pub fn set_transport(&self, id: impl AsRef<str>, transport: &Transport) -> anyhow::Result<()> {
        self.store.conn().execute(
            "UPDATE connections SET transport = ?2, config = ?3 WHERE id = ?1",
            params![
                id.as_ref(),
                transport.label(),
                serde_json::to_string(transport)?
            ],
        )?;
        Ok(())
    }

    pub fn set_status(&self, id: impl AsRef<str>, status: impl AsRef<str>) -> anyhow::Result<()> {
        self.store.conn().execute(
            "UPDATE connections SET status = ?2 WHERE id = ?1",
            params![id.as_ref(), status.as_ref()],
        )?;
        Ok(())
    }

    pub fn remove(&self, id: impl AsRef<str>) -> anyhow::Result<()> {
        let id = id.as_ref();
        let conn = self.store.conn();
        conn.execute("DELETE FROM connections WHERE id = ?1", [id])?;
        // Ids are given again, so what stayed would belong to another
        // connection: an old destination would open its receiver.
        for table in ["connection_tools", "tool_defaults", "destinations"] {
            conn.execute(&format!("DELETE FROM {table} WHERE connection = ?1"), [id])?;
        }
        conn.execute(
            "DELETE FROM scoped_permissions WHERE scope = ?1",
            [scope_of(id)],
        )?;
        conn.execute(
            "DELETE FROM state WHERE substr(key, 1, length(?1)) = ?1",
            [resume_key(id, "")],
        )?;
        Ok(())
    }

    /// Remember a connection's tools as its server listed them.
    pub fn record_tools(
        &self,
        id: impl AsRef<str>,
        tools: &[ConnectionTool],
    ) -> anyhow::Result<()> {
        let now = now();
        let conn = self.store.conn();
        for tool in tools {
            conn.execute(
                "INSERT INTO connection_tools (connection, name, description, read_only, always_ask, seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (connection, name) DO UPDATE
                 SET description = ?3, read_only = ?4, always_ask = ?5, seen_at = ?6",
                params![
                    id.as_ref(),
                    tool.name,
                    tool.description,
                    tool.read_only.map(|b| b as i64),
                    tool.always_ask,
                    now
                ],
            )?;
        }
        Ok(())
    }

    pub fn tools(&self, id: impl AsRef<str>) -> anyhow::Result<Vec<ConnectionTool>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT name, description, read_only, always_ask FROM connection_tools
             WHERE connection = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map([id.as_ref()], |r| {
            Ok(ConnectionTool {
                name: r.get(0)?,
                description: r.get(1)?,
                read_only: r.get::<_, Option<i64>>(2)?.map(|v| v != 0),
                always_ask: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The tool names of every connection of a kind, for the kind's page.
    pub fn kind_tools(&self, kind: impl AsRef<str>) -> anyhow::Result<Vec<ConnectionTool>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT t.name, MAX(t.description), MAX(t.read_only), MAX(t.always_ask) FROM connection_tools t
             JOIN connections c ON c.id = t.connection WHERE c.kind = ?1 GROUP BY t.name ORDER BY t.name",
        )?;
        let rows = stmt.query_map([kind.as_ref()], |r| {
            Ok(ConnectionTool {
                name: r.get(0)?,
                description: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                read_only: r.get::<_, Option<i64>>(2)?.map(|v| v != 0),
                always_ask: r.get::<_, Option<bool>>(3)?.unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

fn connection_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<anyhow::Result<Connection>> {
    let id: String = r.get(0)?;
    let name: String = r.get(1)?;
    let kind: String = r.get(2)?;
    let config: String = r.get(3)?;
    let secret: String = r.get(4)?;
    let account: String = r.get(5)?;
    let status: String = r.get(6)?;
    Ok((|| {
        Ok(Connection {
            id,
            name,
            kind,
            transport: serde_json::from_str(&config)?,
            secret: serde_json::from_str(&secret)?,
            account,
            status,
        })
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connections() -> Connections {
        Connections::new(Store::open_in_memory().unwrap())
    }

    fn remote(url: &str) -> Transport {
        Transport::McpRemote { url: url.into() }
    }

    #[test]
    fn two_accounts_of_a_service_are_two_connections_of_one_kind() {
        let c = connections();
        let work = c
            .add(
                "gmail-work",
                "Gmail · 仕事",
                "Gmail",
                remote("https://a"),
                Secret::default(),
                "me@work",
            )
            .unwrap();
        let home = c
            .add(
                "gmail-home",
                "Gmail · 個人",
                "gmail",
                remote("https://a"),
                Secret::default(),
                "me@home",
            )
            .unwrap();
        assert_eq!((work.kind.as_str(), home.kind.as_str()), ("gmail", "gmail"));
        assert_eq!(c.list().unwrap().len(), 2);
    }

    #[test]
    fn ids_fit_inside_tool_names() {
        let c = connections();
        for bad in ["Gmail", "a_b", "", "famulen", "-x"] {
            assert!(
                c.add(bad, "x", "k", remote("https://a"), Secret::default(), "")
                    .is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn tools_are_remembered_per_connection_and_per_kind() {
        let c = connections();
        c.add(
            "gmail-work",
            "a",
            "gmail",
            remote("https://a"),
            Secret::default(),
            "",
        )
        .unwrap();
        c.add(
            "gmail-home",
            "b",
            "gmail",
            remote("https://a"),
            Secret::default(),
            "",
        )
        .unwrap();
        c.record_tools(
            "gmail-work",
            &[ConnectionTool {
                name: "send_message".into(),
                description: "send".into(),
                read_only: Some(false),
                ..ConnectionTool::default()
            }],
        )
        .unwrap();
        c.record_tools(
            "gmail-home",
            &[ConnectionTool {
                name: "search_threads".into(),
                description: "search".into(),
                read_only: Some(true),
                ..ConnectionTool::default()
            }],
        )
        .unwrap();
        assert_eq!(c.tools("gmail-work").unwrap().len(), 1);
        assert_eq!(c.kind_tools("gmail").unwrap().len(), 2);
    }

    #[test]
    fn a_connection_added_with_the_id_of_a_removed_one_keeps_nothing_of_it() {
        let store = Store::open_in_memory().unwrap();
        let c = Connections::new(store.clone());
        let add = || c.add("x", "x", "k", remote("https://a"), Secret::default(), "");
        add().unwrap();
        c.record_tools(
            "x",
            &[ConnectionTool {
                name: "t".into(),
                ..ConnectionTool::default()
            }],
        )
        .unwrap();
        store
            .conn()
            .execute_batch(
                "INSERT INTO scoped_permissions (scope, tool, permission) VALUES ('conn:x', 't', 'allow');
                 INSERT INTO tool_defaults (connection, tool, permission) VALUES ('x', 't', 'allow');
                 INSERT INTO destinations (connection, receiver, name) VALUES ('x', 'mail', 'lookout');
                 INSERT INTO state (key, value) VALUES ('receive.resume.x.mail', '[]');
                 INSERT INTO state (key, value) VALUES ('receive.resume.x-2.mail', '[]');",
            )
            .unwrap();

        c.remove("x").unwrap();
        add().unwrap();

        assert!(c.tools("x").unwrap().is_empty());
        for table in ["scoped_permissions", "tool_defaults", "destinations"] {
            let left: i64 = store
                .conn()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(left, 0, "{table}");
        }
        assert_eq!(store.state("receive.resume.x.mail").unwrap(), None);
        assert!(store.state("receive.resume.x-2.mail").unwrap().is_some());
    }
}
