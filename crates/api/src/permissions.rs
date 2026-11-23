//! What each tool is permitted, as the console shows and sets it: for one
//! account, for a kind of service, and for the model CLI's built-in tools.

use famulen_core::gateway::permissions::{BUILTINS, PermissionSource};
use famulen_core::permissions::Permission as CorePermission;
use famulen_core::permissions::{AutoCourse, Marks, auto_course, builtin_marks};

use crate::Api;
use crate::connections::connection;
use crate::pb::{self, *};

pub(crate) fn permission(p: CorePermission) -> pb::Permission {
    match p {
        CorePermission::Auto => pb::Permission::PERMISSION_AUTO,
        CorePermission::Allow => pb::Permission::PERMISSION_ALLOW,
        CorePermission::Confirm => pb::Permission::PERMISSION_CONFIRM,
        CorePermission::Forbid => pb::Permission::PERMISSION_FORBID,
    }
}

pub(crate) fn permission_from(p: pb::Permission) -> Option<CorePermission> {
    match p {
        pb::Permission::PERMISSION_AUTO => Some(CorePermission::Auto),
        pb::Permission::PERMISSION_ALLOW => Some(CorePermission::Allow),
        pb::Permission::PERMISSION_CONFIRM => Some(CorePermission::Confirm),
        pb::Permission::PERMISSION_FORBID => Some(CorePermission::Forbid),
        _ => None,
    }
}

/// The owner's own choice, unspecified when there is none.
fn chosen(own: Option<CorePermission>) -> pb::Permission {
    own.map_or(pb::Permission::PERMISSION_UNSPECIFIED, permission)
}

/// What the console says of where a permission came from.
fn source_label(source: PermissionSource) -> &'static str {
    match source {
        PermissionSource::Connection => "このアカウント",
        PermissionSource::Kind => "種類の既定",
        PermissionSource::Default => "まだ決めていない",
    }
}

impl Api {
    /// The tools of a scope with whether each only reads.
    pub(crate) fn scope_tools(&self, scope: &str) -> anyhow::Result<Vec<(String, bool)>> {
        Ok(match scope.split_once(':') {
            Some(("conn", id)) => self
                .app
                .connections
                .tools(id)?
                .into_iter()
                .map(|t| (t.name, t.read_only == Some(true)))
                .collect(),
            Some(("kind", kind)) => self
                .app
                .connections
                .kind_tools(kind)?
                .into_iter()
                .map(|t| (t.name, t.read_only == Some(true)))
                .collect(),
            _ if scope == BUILTINS => self
                .app
                .permissions
                .builtin_tools()?
                .into_iter()
                .map(|t| (t.name, t.reads))
                .collect(),
            _ => anyhow::bail!("unknown scope {scope}"),
        })
    }

    pub(crate) fn kind_detail(&self, kind: &str) -> anyhow::Result<KindDetail> {
        let scope = format!("kind:{kind}");
        let tools = self
            .app
            .connections
            .kind_tools(kind)?
            .into_iter()
            .map(|t| {
                let (effective, source) =
                    self.app.permissions.kind_permission_detail(kind, &t.name)?;
                let always_ask = auto_course(
                    Marks {
                        read_only: t.read_only,
                        always_ask: t.always_ask,
                    },
                    &t.name,
                ) == AutoCourse::Ask;
                let own = self.app.permissions.scoped(&scope, &t.name)?;
                Ok(ToolSetting {
                    permission: chosen(own).into(),
                    effective: permission(effective).into(),
                    inherited: permission(self.app.permissions.kind_default_of(kind, &t.name)?)
                        .into(),
                    source: source_label(source).into(),
                    read_only: t.read_only == Some(true),
                    description: t.description,
                    always_ask,
                    name: t.name,
                    ..Default::default()
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let connections = self
            .app
            .connections
            .list()?
            .iter()
            .filter(|c| c.kind == kind)
            .map(connection)
            .collect();
        Ok(KindDetail {
            kind: kind.to_owned(),
            tools,
            connections,
            ..Default::default()
        })
    }

    pub(crate) fn tool_setting(
        &self,
        conn: &str,
        name: &str,
        description: &str,
        read_only: bool,
    ) -> anyhow::Result<ToolSetting> {
        let full = format!("mcp__{conn}__{name}");
        let (mut effective, source) = self.app.permissions.permission_detail(&full)?;
        // Whatever is chosen, the owner is asked of every such call.
        if effective != CorePermission::Forbid && self.app.gateway.asks_each_time(conn, name)? {
            effective = CorePermission::Confirm;
        }
        let own = self.app.permissions.scoped(&format!("conn:{conn}"), name)?;
        Ok(ToolSetting {
            name: name.to_owned(),
            description: description.to_owned(),
            read_only,
            permission: chosen(own).into(),
            effective: permission(effective).into(),
            inherited: permission(self.app.permissions.inherited(&full)?).into(),
            source: source_label(source).into(),
            always_ask: self.app.permissions.always_asked(&full)?,
            ..Default::default()
        })
    }

    pub(crate) fn builtin_tools(&self) -> anyhow::Result<Vec<ToolSetting>> {
        Ok(self
            .app
            .permissions
            .builtin_tools()?
            .into_iter()
            .map(|t| {
                let source = if t.permission.is_some() {
                    PermissionSource::Connection
                } else {
                    PermissionSource::Default
                };
                ToolSetting {
                    always_ask: auto_course(builtin_marks(&t.name), &t.name) == AutoCourse::Ask,
                    name: t.name,
                    read_only: t.reads,
                    permission: chosen(t.permission).into(),
                    effective: permission(t.effective).into(),
                    inherited: pb::Permission::PERMISSION_AUTO.into(),
                    source: source_label(source).into(),
                    ..Default::default()
                }
            })
            .collect())
    }
}
