//! The permissions a call is judged by: what the owner chose per connection and
//! per kind of connection, what a service definition gives, and what a tool says
//! of itself.

use rusqlite::{OptionalExtension, params};

use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Reading runs, a call that cannot be undone or reaches other people asks,
    /// and the assessor weighs the rest.
    Auto,
    /// Runs without asking.
    Allow,
    /// Each call waits for the owner's approval.
    Confirm,
    /// Never runs.
    Forbid,
}

impl Permission {
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::Auto => "auto",
            Permission::Allow => "allow",
            Permission::Confirm => "confirm",
            Permission::Forbid => "forbid",
        }
    }

    pub fn parse(s: &str) -> anyhow::Result<Self> {
        Ok(match s {
            "auto" => Permission::Auto,
            "allow" => Permission::Allow,
            "confirm" => Permission::Confirm,
            "forbid" => Permission::Forbid,
            other => anyhow::bail!("unknown permission: {other}"),
        })
    }
}

/// Where the permission that applies to a tool was set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionSource {
    Connection,
    Kind,
    Default,
}

/// Verbs that mark a tool as reading when its name starts with one
/// (`search_threads`), or with a service prefix and then one (`clickup_get_task`).
const READING_VERBS: &[&str] = &[
    "get", "list", "search", "read", "fetch", "query", "find", "browse", "view", "describe",
    "download",
];

/// Verbs that mark a name as writing, so the word after them is not read as
/// the verb (`delete_search_index`).
const WRITING_VERBS: &[&str] = &[
    "add", "apply", "archive", "complete", "copy", "create", "delete", "edit", "execute",
    "forward", "insert", "label", "mark", "merge", "move", "post", "react", "remove", "reply",
    "request", "respond", "schedule", "send", "set", "share", "start", "stop", "trash", "unlabel",
    "update", "upload", "write",
];

/// Verbs that mark a call as one that cannot be undone or that reaches other
/// people: under the auto permission the owner is always asked first.
const ASKING_VERBS: &[&str] = &[
    "delete", "destroy", "remove", "trash", "purge", "erase", "send", "forward", "reply",
    "respond", "share", "invite", "decline", "cancel", "publish", "pay", "transfer", "purchase",
];

/// The verb of a tool's name: its first word, or the second after a service
/// prefix (`clickup_get_task`) when the first is no verb.
fn verb_in<'a>(tool: &'a str, verbs: &[&str]) -> Option<&'a str> {
    let mut words = tool.split(['_', '-']);
    let first = words.next().unwrap_or_default();
    if verbs.contains(&first) {
        return Some(first);
    }
    if [READING_VERBS, WRITING_VERBS, ASKING_VERBS]
        .iter()
        .any(|known| known.contains(&first))
    {
        return None;
    }
    words.next().filter(|second| verbs.contains(second))
}

/// Whether a tool's name reads as only reading.
pub fn reads_by_name(tool: &str) -> bool {
    verb_in(&tool.to_ascii_lowercase(), READING_VERBS).is_some()
}

/// Whether a tool's name reads as a call that cannot be undone or that
/// reaches other people.
pub fn asks_by_name(tool: &str) -> bool {
    verb_in(&tool.to_ascii_lowercase(), ASKING_VERBS).is_some()
}

/// What a tool says of itself, as the auto permission weighs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Marks {
    /// It only reads.
    pub read_only: Option<bool>,
    /// It cannot be undone or it reaches other people.
    pub always_ask: bool,
}

/// What the auto permission does with a call before any model is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoCourse {
    /// It only reads: it runs.
    Run,
    /// It cannot be undone or it reaches other people: the owner is asked.
    Ask,
    /// The assessor weighs it.
    Assess,
}

/// The course of the auto permission for a tool. A mark that it is always
/// asked about outweighs one that it only reads.
pub fn auto_course(marks: Marks, tool: &str) -> AutoCourse {
    if marks.always_ask || asks_by_name(tool) {
        AutoCourse::Ask
    } else if marks.read_only.unwrap_or_else(|| reads_by_name(tool)) {
        AutoCourse::Run
    } else {
        AutoCourse::Assess
    }
}

/// Split `mcp__<connection>__<tool>`.
pub fn split_tool(tool_name: &str) -> Option<(&str, &str)> {
    tool_name.strip_prefix("mcp__")?.split_once("__")
}

/// The scope of the permissions for the caller's built-in tools: the calls that
/// name no connection.
pub const BUILTINS: &str = "";

/// The scope a connection's own permissions are kept under.
pub fn scope_of(connection: &str) -> String {
    if connection.is_empty() {
        BUILTINS.to_owned()
    } else {
        format!("conn:{connection}")
    }
}

#[derive(Clone)]
pub struct Permissions {
    store: Store,
}

impl Permissions {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// The permission that applies to a connection's tool and where it came from:
    /// the connection, the connection's kind, or none chosen.
    pub fn chosen(
        &self,
        connection: &str,
        tool: &str,
    ) -> anyhow::Result<Option<(Permission, PermissionSource)>> {
        if let Some(permission) = self.scoped(&scope_of(connection), tool)? {
            return Ok(Some((permission, PermissionSource::Connection)));
        }
        if let Some(kind) = self.kind_of(connection)?
            && let Some(permission) = self.scoped(&format!("kind:{kind}"), tool)?
        {
            return Ok(Some((permission, PermissionSource::Kind)));
        }
        Ok(None)
    }

    /// What applies to a connection's tool without a permission of the
    /// connection's own: its kind's, else its default.
    pub fn inherited(&self, connection: &str, tool: &str) -> anyhow::Result<Permission> {
        if let Some(kind) = self.kind_of(connection)?
            && let Some(permission) = self.scoped(&format!("kind:{kind}"), tool)?
        {
            return Ok(permission);
        }
        self.default_of(connection, tool)
    }

    /// The default of a kind's tool: what the service definition gives every
    /// account of the kind, else auto.
    pub fn kind_default_of(&self, kind: &str, tool: &str) -> anyhow::Result<Permission> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT d.permission FROM connections c
             LEFT JOIN tool_defaults d ON d.connection = c.id AND d.tool = ?2 WHERE c.kind = ?1",
        )?;
        let given = stmt
            .query_map(params![kind, tool], |r| r.get::<_, Option<String>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        match given.as_slice() {
            [Some(permission)] => Permission::parse(permission),
            _ => Ok(Permission::Auto),
        }
    }

    /// The default of a connection's tool: what its service definition gives
    /// it, else auto.
    pub fn default_of(&self, connection: &str, tool: &str) -> anyhow::Result<Permission> {
        let defined: Option<String> = self
            .store
            .conn()
            .query_row(
                "SELECT permission FROM tool_defaults WHERE connection = ?1 AND tool = ?2",
                params![connection, tool],
                |r| r.get(0),
            )
            .optional()?;
        defined.map_or(Ok(Permission::Auto), |p| Permission::parse(&p))
    }

    /// What a connection's tool says of itself, as its server or definition
    /// listed it; nothing for a tool not listed.
    pub fn marks_of(&self, connection: &str, tool: &str) -> anyhow::Result<Marks> {
        let marks = self
            .store
            .conn()
            .query_row(
                "SELECT read_only, always_ask FROM connection_tools WHERE connection = ?1 AND name = ?2",
                params![connection, tool],
                |r| {
                    Ok(Marks {
                        read_only: r.get::<_, Option<i64>>(0)?.map(|f| f != 0),
                        always_ask: r.get::<_, i64>(1)? != 0,
                    })
                },
            )
            .optional()?;
        Ok(marks.unwrap_or_default())
    }

    /// Keep the defaults a connection's service definition gives its tools,
    /// replacing the ones kept before. The owner's permissions stay stronger.
    pub fn set_defaults(
        &self,
        connection: &str,
        defaults: &[(String, Permission)],
    ) -> anyhow::Result<()> {
        let conn = self.store.conn();
        conn.execute(
            "DELETE FROM tool_defaults WHERE connection = ?1",
            params![connection],
        )?;
        for (tool, permission) in defaults {
            conn.execute(
                "INSERT INTO tool_defaults (connection, tool, permission) VALUES (?1, ?2, ?3)",
                params![connection, tool, permission.as_str()],
            )?;
        }
        Ok(())
    }

    /// The permission a kind gives a tool and where it came from: the kind's own
    /// permission, or its default.
    pub fn kind_permission_detail(
        &self,
        kind: impl AsRef<str>,
        tool: impl AsRef<str>,
    ) -> anyhow::Result<(Permission, PermissionSource)> {
        let (kind, tool) = (kind.as_ref(), tool.as_ref());
        if let Some(permission) = self.scoped(&format!("kind:{kind}"), tool)? {
            return Ok((permission, PermissionSource::Kind));
        }
        Ok((self.kind_default_of(kind, tool)?, PermissionSource::Default))
    }

    fn kind_of(&self, connection: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT kind FROM connections WHERE id = ?1",
                [connection],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The permission a scope (`conn:<id>` or `kind:<kind>`) sets for a tool, if any.
    pub fn scoped(&self, scope: &str, tool: &str) -> anyhow::Result<Option<Permission>> {
        let permission: Option<String> = self
            .store
            .conn()
            .query_row(
                "SELECT permission FROM scoped_permissions WHERE scope = ?1 AND tool = ?2",
                params![scope, tool],
                |row| row.get(0),
            )
            .optional()?;
        permission.map(|r| Permission::parse(&r)).transpose()
    }

    /// Set the permission for a tool under `conn:<id>` or `kind:<kind>`.
    pub fn set_scoped(
        &self,
        scope: &str,
        tool: &str,
        permission: Option<Permission>,
    ) -> anyhow::Result<()> {
        let conn = self.store.conn();
        match permission {
            Some(permission) => {
                conn.execute(
                    "INSERT INTO scoped_permissions (scope, tool, permission) VALUES (?1, ?2, ?3)
                     ON CONFLICT (scope, tool) DO UPDATE SET permission = ?3",
                    params![scope, tool, permission.as_str()],
                )?;
            }
            None => {
                conn.execute(
                    "DELETE FROM scoped_permissions WHERE scope = ?1 AND tool = ?2",
                    params![scope, tool],
                )?;
            }
        }
        Ok(())
    }

    /// Give a tool of `connection` a default when it has none yet.
    pub fn set_default_if_absent(
        &self,
        connection: &str,
        tool: &str,
        permission: Permission,
    ) -> anyhow::Result<()> {
        self.store.conn().execute(
            "INSERT OR IGNORE INTO tool_defaults (connection, tool, permission) VALUES (?1, ?2, ?3)",
            params![connection, tool, permission.as_str()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_runs_a_reading_tool_by_its_mark_or_its_name() {
        let read = Marks {
            read_only: Some(true),
            ..Marks::default()
        };
        assert_eq!(auto_course(read, "glossary"), AutoCourse::Run);
        assert_eq!(
            auto_course(Marks::default(), "search_threads"),
            AutoCourse::Run
        );
        assert_eq!(
            auto_course(Marks::default(), "clickup_get_task"),
            AutoCourse::Run
        );
    }

    #[test]
    fn auto_has_the_assessor_weigh_a_writing_tool() {
        let write = Marks {
            read_only: Some(false),
            ..Marks::default()
        };
        assert_eq!(auto_course(write, "search_threads"), AutoCourse::Assess);
        assert_eq!(
            auto_course(Marks::default(), "create_task"),
            AutoCourse::Assess
        );
        assert_eq!(
            auto_course(Marks::default(), "glossary"),
            AutoCourse::Assess
        );
    }

    #[test]
    fn auto_always_asks_about_a_tool_marked_or_named_as_beyond_undoing() {
        let marked = Marks {
            read_only: Some(true),
            always_ask: true,
        };
        assert_eq!(auto_course(marked, "list_events"), AutoCourse::Ask);
        for tool in [
            "delete_event",
            "send_message",
            "slack_send_message",
            "clickup_delete_task",
            "trash_thread",
            "forward",
            "cancel_event",
            "http_delete",
        ] {
            assert_eq!(
                auto_course(Marks::default(), tool),
                AutoCourse::Ask,
                "{tool}"
            );
        }
        assert_eq!(
            auto_course(Marks::default(), "delete_search_index"),
            AutoCourse::Ask
        );
        assert_eq!(
            auto_course(Marks::default(), "search_deleted"),
            AutoCourse::Run
        );
    }

    #[test]
    fn a_tool_without_a_chosen_or_defined_permission_is_auto() {
        let p = Permissions::new(Store::open_in_memory().unwrap());
        assert_eq!(
            p.default_of("mail", "send_message").unwrap(),
            Permission::Auto
        );
        assert_eq!(
            p.kind_permission_detail("gmail", "search_threads").unwrap(),
            (Permission::Auto, PermissionSource::Default)
        );
        p.set_defaults("mail", &[("send_message".into(), Permission::Confirm)])
            .unwrap();
        assert_eq!(
            p.default_of("mail", "send_message").unwrap(),
            Permission::Confirm
        );
    }
}
