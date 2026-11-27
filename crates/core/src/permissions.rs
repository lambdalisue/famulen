//! Whether the secretary's model may use a tool. The gateway judges every
//! call: the owner's connections' tools when they reach it, and Famulen's own
//! tools and the model CLI's built-in tools here, before the model runs them.
//! What stays in Famulen is what only Famulen knows: the owner's choice for
//! the built-in tools, the memory directory file tools may write in without
//! asking, and which connections the running workgroup takes in.

use std::path::{Component, Path, PathBuf};

use rusqlite::{OptionalExtension, params};
use serde_json::Value;

pub use famulen_gateway::permissions::{
    AutoCourse, Marks, Permission, PermissionSource, auto_course, split_tool,
};
use famulen_gateway::permissions::{BUILTINS, Permissions as GatewayPermissions};
use famulen_gateway::{Call, Gateway, Verdict};

use crate::hub;
use crate::paths;
use crate::sealed::Sealed;
use crate::store::{Store, now};

/// Who the secretary is, to the gateway.
pub const SECRETARY: &str = "secretary";
/// Who starts and runs delegated tasks, to the gateway.
pub const WORKER: &str = "worker";

/// Who a run of work Famulen runs itself is to the gateway: one of the
/// workers, but none other, so that what it was handed and the owner's
/// approval of its call are its alone.
pub fn job_actor(run: i64) -> String {
    format!("{WORKER}:run-{run}")
}

/// Whether a caller is a run of work Famulen runs itself.
pub fn is_job_actor(actor: impl AsRef<str>) -> bool {
    actor
        .as_ref()
        .strip_prefix(WORKER)
        .and_then(|rest| rest.strip_prefix(":run-"))
        .is_some_and(|id| id.parse::<i64>().is_ok())
}

/// Whether a caller is the pool or one of its runs.
pub fn is_worker(actor: impl AsRef<str>) -> bool {
    let actor = actor.as_ref();
    actor == WORKER
        || actor
            .strip_prefix(WORKER)
            .is_some_and(|rest| rest.starts_with(':'))
}

/// Built-in tools that start other agents. No model uses them: work is
/// delegated through tasks, which the worker pool picks up.
pub const LAUNCHERS: &[&str] = &["Task", "Agent", "Workflow", "TeamCreate"];

/// What a model reads when it tries to start an agent by itself.
pub const DELEGATE_INSTEAD: &str = "Agents are not started this way. Add a delegated task with delegate_task (runner adroit or agent): a worker picks it up in priority order, and you are told when it ends.";

/// Who in the secretary's process makes a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    /// The session the owner talks to.
    Secretary,
    /// A subagent in it, which is let use nothing: work is delegated.
    Subagent,
}

/// What a turn adds to the permission of a call made in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bound {
    /// Why the owner is asked even where the permission allows the call.
    pub ask: Option<String>,
    /// The owner said yes to it before, so only a forbid stops it.
    pub waive: bool,
}

/// What came of using the owner's approval of a call run directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pass {
    Taken,
    /// There was none left to use.
    Gone,
    /// It was used, and the call needed the owner again all the same; with
    /// the request it was held as for a moment, to be taken back wherever it
    /// is shown.
    HeldAgain(Option<i64>),
    /// The call is refused now whatever was approved.
    Refused(String),
}

/// What to answer Claude Code for one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Held for the owner's answer as this ticket; the call itself is
    /// refused for now.
    Held(i64),
    /// Refused; the model reads the message as the tool's error.
    Forbid(String),
}

/// Built-in Claude Code tools that only read, or only organise the model's
/// own work.
const READING_BUILTINS: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "LS",
    "WebSearch",
    "WebFetch",
    "ToolSearch",
    "TodoWrite",
    // Only loads a skill's instructions; the copies Famulen gives the
    // secretary grant no tools, so what a skill leads to is still asked.
    "Skill",
    "TaskOutput",
    "ListMcpResourcesTool",
    "ReadMcpResourceTool",
];

/// Famulen's own tools whose change the gateway judges when it reaches it:
/// the vault's, under its own permissions, and a proposed service definition,
/// which always waits for the owner.
const JUDGED_ON_ARRIVAL: &[&str] = &[
    "set_secret_value",
    "describe_secret",
    "delete_secret",
    "propose_definition_change",
];

/// Famulen's own tool that adds a connection which needs no secret.
const ADD_KEYLESS: &str = "add_keyless_connection";

/// Built-in tools the auto permission always asks about: a shell command
/// reaches whatever the shell can, which its input does not show. Claude Code
/// offers `PowerShell` on Windows.
const ASKED_BUILTINS: &[&str] = &["Bash", "PowerShell"];

/// What a built-in tool says of itself, as the auto permission weighs it.
pub fn builtin_marks(tool_name: &str) -> Marks {
    Marks {
        read_only: Some(READING_BUILTINS.contains(&tool_name)),
        always_ask: ASKED_BUILTINS.contains(&tool_name),
    }
}

/// A tool as the console lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolEntry {
    pub name: String,
    /// The owner's choice, if any.
    pub permission: Option<Permission>,
    /// What applies now: the owner's choice, or the default.
    pub effective: Permission,
    /// It only reads, or only organises the model's own work.
    pub reads: bool,
}

/// Built-in tools that write one file, named by this input field.
const FILE_WRITERS: &[(&str, &str)] = &[
    ("Write", "file_path"),
    ("Edit", "file_path"),
    ("MultiEdit", "file_path"),
    ("NotebookEdit", "notebook_path"),
];

#[derive(Clone)]
pub struct Permissions {
    store: Store,
    gateway: Gateway,
    connections: crate::connections::Connections,
    memory_dir: Option<PathBuf>,
    notebook_dir: Option<PathBuf>,
    sealed: Sealed,
}

impl Permissions {
    /// `store` is Famulen's own; `gateway` judges the calls and keeps the
    /// connections and their permissions.
    pub fn new(store: Store, gateway: Gateway) -> Self {
        Self {
            store,
            connections: crate::connections::Connections::new(gateway.store().clone()),
            gateway,
            memory_dir: None,
            notebook_dir: None,
            sealed: Sealed::default(),
        }
    }

    /// Refuse built-in tools that would reach `places`, whatever the permissions
    /// say; relative paths are read from `workdir`.
    pub fn with_sealed(
        mut self,
        places: impl IntoIterator<Item = impl Into<PathBuf>>,
        workdir: impl Into<PathBuf>,
    ) -> Self {
        self.sealed = Sealed::new(places, workdir);
        self
    }

    fn gateway_permissions(&self) -> GatewayPermissions {
        GatewayPermissions::new(self.gateway.store().clone())
    }

    /// Let file tools write inside `dir` without asking, unless the owner
    /// forbade the tool.
    pub fn with_memory_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.memory_dir = Some(dir.into());
        self
    }

    /// Outside `dir`, the secretary's own file tools ask even when the
    /// assessor would let them run, so that it hands the work to another process.
    pub fn with_notebook_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.notebook_dir = Some(dir.into());
        self
    }

    /// Keep Famulen's own tools: each is used without asking unless it is one
    /// the owner is asked about first.
    pub fn record_own_tools(&self, tools: &[crate::hub::ToolDef]) -> anyhow::Result<()> {
        let tools: Vec<crate::connections::ConnectionTool> = tools
            .iter()
            .map(|t| crate::connections::ConnectionTool {
                name: t.name.clone(),
                description: t.description.clone(),
                read_only: Some(!t.confirm),
                always_ask: t.confirm,
            })
            .collect();
        self.connections.record_tools(hub::SERVER_NAME, &tools)
    }

    /// Decide one call the secretary itself is about to make.
    pub async fn decide(&self, tool_name: &str, input: &Value) -> anyhow::Result<Decision> {
        self.decide_for(Caller::Secretary, tool_name, input).await
    }

    /// Decide one call the secretary's model is about to make.
    /// A connection's tool is let through to the gateway, which judges it
    /// when it arrives; every other tool is judged by the gateway here.
    pub async fn decide_for(
        &self,
        caller: Caller,
        tool_name: &str,
        input: &Value,
    ) -> anyhow::Result<Decision> {
        self.decide_bound(caller, tool_name, input, Bound::default())
            .await
    }

    /// [`Permissions::decide_for`], within what the turn lets it do.
    pub async fn decide_bound(
        &self,
        caller: Caller,
        tool_name: &str,
        input: &Value,
        bound: Bound,
    ) -> anyhow::Result<Decision> {
        if caller == Caller::Subagent {
            return Ok(Decision::Forbid(DELEGATE_INSTEAD.to_owned()));
        }
        let call = match split_tool(tool_name) {
            Some((connection, tool)) if self.starts_agents(connection, tool) => {
                return Ok(Decision::Forbid(DELEGATE_INSTEAD.to_owned()));
            }
            Some((connection, _)) if connection != hub::SERVER_NAME => return Ok(Decision::Allow),
            Some((_, tool)) if JUDGED_ON_ARRIVAL.contains(&tool) => return Ok(Decision::Allow),
            Some((connection, tool)) => Call {
                refuse: self.refusal_of_adding(tool, input),
                ..Call::new(SECRETARY, connection, tool, input.clone())
            },
            None if LAUNCHERS.contains(&tool_name) => Call {
                refuse: Some(DELEGATE_INSTEAD.to_owned()),
                ..Call::new(SECRETARY, "", tool_name, input.clone())
            },
            None => Call {
                marks: Some(self.marks_of_builtin(tool_name, input)),
                waive: self.writes_memory(tool_name, input),
                refuse: self.sealed.refusal(tool_name, input),
                ..Call::new(SECRETARY, "", tool_name, input.clone())
            },
        };
        let call = Call {
            origin: self.store.call_origin()?,
            waive: (call.waive && bound.ask.is_none()) || bound.waive,
            ask: bound.ask,
            ..call
        };
        Ok(match self.gateway.judge_assessed(&call).await? {
            Verdict::Allow { .. } => Decision::Allow,
            Verdict::Hold(ticket) => Decision::Held(ticket.id),
            Verdict::Deny { reason } => Decision::Forbid(format!("{tool_name}: {reason}")),
        })
    }

    /// Why a connection could not be added, before the owner is asked of it.
    fn refusal_of_adding(&self, tool: &str, input: &Value) -> Option<String> {
        if tool != ADD_KEYLESS {
            return None;
        }
        let values = input["values"].as_object().into_iter().flatten();
        let values = values
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
            .collect();
        self.gateway
            .check_keyless(
                input["definition"].as_str().unwrap_or_default(),
                values,
                input["name"].as_str(),
            )
            .err()
            .map(|error| format!("{error:#}"))
    }

    /// Decide one call of a hired worker's model, refused when `refuse` says why. A
    /// connection's tool is judged when it reaches the gateway. What would be
    /// asked of the owner is refused: a worker cannot wait for the answer.
    pub async fn decide_worker(
        &self,
        run: i64,
        tool_name: &str,
        input: &Value,
        refuse: Option<String>,
        ask: Option<String>,
    ) -> anyhow::Result<Decision> {
        if refuse.is_none() && split_tool(tool_name).is_some() {
            return Ok(Decision::Allow);
        }
        let call = Call {
            marks: Some(builtin_marks(tool_name)),
            refuse,
            ask,
            never_ask: true,
            ..Call::new(job_actor(run), "", tool_name, input.clone())
        };
        Ok(match self.gateway.judge_assessed(&call).await? {
            Verdict::Allow { .. } => Decision::Allow,
            Verdict::Hold(_) => Decision::Forbid(famulen_gateway::NEEDS_OWNER.to_owned()),
            Verdict::Deny { reason } => Decision::Forbid(format!("{tool_name}: {reason}")),
        })
    }

    /// Decide one built-in call of a model run directly, refused when `refuse`
    /// says why. Judged as a worker's is; what would be asked of the owner
    /// is held for their answer, which the run waits for.
    pub async fn decide_direct(
        &self,
        run: i64,
        tool_name: &str,
        input: &Value,
        refuse: Option<String>,
        ask: Option<String>,
    ) -> anyhow::Result<Decision> {
        let call = Call {
            marks: Some(builtin_marks(tool_name)),
            refuse: refuse.or_else(|| {
                split_tool(tool_name).map(|_| format!("{tool_name} was not handed to you."))
            }),
            ask,
            ..Call::new(job_actor(run), "", tool_name, input.clone())
        };
        Ok(match self.gateway.judge_assessed(&call).await? {
            Verdict::Allow { .. } => Decision::Allow,
            Verdict::Hold(ticket) => Decision::Held(ticket.id),
            Verdict::Deny { reason } => Decision::Forbid(format!("{tool_name}: {reason}")),
        })
    }

    /// Ask the owner whether a run may start, for the reason `why` gives. An
    /// approval lets it start once, as `input` describes it, until `until`.
    pub async fn decide_start(
        &self,
        run: i64,
        input: &Value,
        why: impl Into<String>,
        until: i64,
    ) -> anyhow::Result<Decision> {
        const START: &str = "start_work";
        let actor = job_actor(run);
        self.gateway.grant(&actor, "", START)?;
        self.gateway.expire(&actor, until)?;
        let call = Call {
            marks: Some(builtin_marks(START)),
            ask: Some(why.into()),
            ..Call::new(actor, "", START, input.clone())
        };
        Ok(match self.gateway.judge_assessed(&call).await? {
            Verdict::Allow { .. } => Decision::Allow,
            Verdict::Hold(ticket) => Decision::Held(ticket.id),
            Verdict::Deny { reason } => Decision::Forbid(reason),
        })
    }

    /// Let a run call what it was handed, until `until` at the latest, in
    /// seconds since the epoch. Built-in tools are granted all together:
    /// which of them it was handed is judged before the gateway is asked.
    pub fn grant_job(&self, run: i64, tools: &[String], until: i64) -> anyhow::Result<()> {
        let actor = job_actor(run);
        self.gateway.grant(&actor, "", famulen_gateway::ANY)?;
        for (connection, tool) in tools.iter().filter_map(|t| split_tool(t)) {
            self.gateway.grant(&actor, connection, tool)?;
        }
        self.gateway.expire(&actor, until)
    }

    /// A run is over: it calls nothing any more, and what it asked or was
    /// approved goes with it. The ids of the requests it still waited on.
    pub fn revoke_job(&self, run: i64) -> anyhow::Result<Vec<i64>> {
        self.gateway.revoke(job_actor(run))
    }

    /// Use the one pass the owner's approval of `ticket` left for a built-in
    /// call of a run of work run directly: that ticket's alone, whichever
    /// other approval matches the same call. The call is still refused where
    /// the owner forbids it now; nothing else is asked.
    pub fn take_direct_pass(
        &self,
        run: i64,
        ticket: i64,
        tool_name: &str,
        input: &Value,
    ) -> anyhow::Result<Pass> {
        if !self.gateway.take_pass_of(ticket)? {
            return Ok(Pass::Gone);
        }
        let call = Call {
            marks: Some(builtin_marks(tool_name)),
            waive: true,
            ..Call::new(job_actor(run), "", tool_name, input.clone())
        };
        Ok(match self.gateway.judge(&call)? {
            Verdict::Allow { .. } => Pass::Taken,
            Verdict::Hold(held) => {
                let withdrawn = self.gateway.withdraw_ticket(held.id)?;
                Pass::HeldAgain(withdrawn.then_some(held.id))
            }
            Verdict::Deny { reason } => Pass::Refused(format!("{tool_name}: {reason}")),
        })
    }

    /// Why a connection is asked about when a delegated task uses it: it is
    /// outside one of the workgroups the task is bound to, or there is none.
    pub fn ask_outside(
        &self,
        connection: &str,
        scopes: &[String],
    ) -> anyhow::Result<Option<String>> {
        if scopes.is_empty() {
            return Ok(Some(
                "it is not known from which workgroup the task was delegated".into(),
            ));
        }
        for scope in scopes {
            if let Some(why) = self.ask_in(connection, scope)? {
                return Ok(Some(why));
            }
        }
        Ok(None)
    }

    /// Whether a connection's tool starts an agent.
    pub fn starts_agents(&self, connection: &str, tool: &str) -> bool {
        crate::workers::adroit::LAUNCH.contains(&tool)
            && self
                .connections
                .get(connection)
                .ok()
                .flatten()
                .is_some_and(|c| crate::workers::adroit::is_adroit(&c))
    }

    /// The answer to an MCP message that calls a tool which starts an agent;
    /// `None` for any other message.
    pub fn refused_launch(&self, server: &str, message: &Value) -> Option<Value> {
        let name = message.pointer("/params/name").and_then(Value::as_str)?;
        if message["method"] != "tools/call" || !self.starts_agents(server, name) {
            return None;
        }
        Some(serde_json::json!({
            "jsonrpc": "2.0",
            "id": message.get("id")?,
            "result": { "content": [{ "type": "text", "text": DELEGATE_INSTEAD }], "isError": true },
        }))
    }

    /// An MCP answer whose list of tools leaves out those that start agents.
    pub fn without_launchers(&self, server: &str, mut response: Value) -> Value {
        if let Some(tools) = response
            .pointer_mut("/result/tools")
            .and_then(Value::as_array_mut)
        {
            tools.retain(|t| !self.starts_agents(server, t["name"].as_str().unwrap_or_default()));
        }
        response
    }

    /// Why a call to `connection` made in `workgroup` is asked about even when its
    /// permission allows it: the connection is outside the workgroup.
    pub fn ask_in(&self, connection: &str, workgroup: &str) -> anyhow::Result<Option<String>> {
        if workgroup == crate::workgroups::ALL || connection == hub::SERVER_NAME {
            return Ok(None);
        }
        let inside = self
            .store
            .conn()
            .query_row(
                "SELECT 1 FROM workgroup_members WHERE workgroup = ?1 AND kind = 'connection' AND ref = ?2",
                params![workgroup, connection],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok((!inside).then(|| format!("{connection} is outside the workgroup {workgroup}")))
    }

    fn marks_of_builtin(&self, tool_name: &str, input: &Value) -> Marks {
        let marks = builtin_marks(tool_name);
        let writes_elsewhere = FILE_WRITERS.iter().any(|(name, _)| *name == tool_name)
            && !writes_inside(self.notebook_dir.as_deref(), tool_name, input);
        Marks {
            always_ask: marks.always_ask || writes_elsewhere,
            ..marks
        }
    }

    fn writes_memory(&self, tool_name: &str, input: &Value) -> bool {
        writes_inside(self.memory_dir.as_deref(), tool_name, input)
    }

    pub fn permission(&self, tool_name: &str) -> anyhow::Result<Permission> {
        Ok(self.permission_detail(tool_name)?.0)
    }

    /// The permission that applies and where it came from: the connection (for a
    /// built-in tool, the owner's choice for it), the connection's kind, or
    /// the default.
    pub fn permission_detail(
        &self,
        tool_name: &str,
    ) -> anyhow::Result<(Permission, PermissionSource)> {
        let permissions = self.gateway_permissions();
        let (connection, tool) = split_tool(tool_name).unwrap_or(("", tool_name));
        if let Some(chosen) = permissions.chosen(connection, tool)? {
            return Ok(chosen);
        }
        Ok((
            permissions.default_of(connection, tool)?,
            PermissionSource::Default,
        ))
    }

    /// Whether a connection's tool says that it only reads, and nothing it
    /// does is beyond undoing or reaches other people.
    pub fn only_reads(&self, connection: &str, tool: &str) -> anyhow::Result<bool> {
        let marks = self.gateway_permissions().marks_of(connection, tool)?;
        Ok(marks.read_only == Some(true) && !marks.always_ask)
    }

    /// What applies to a tool when the owner chooses nothing for its connection.
    pub fn inherited(&self, tool_name: &str) -> anyhow::Result<Permission> {
        let (connection, tool) = split_tool(tool_name).unwrap_or(("", tool_name));
        self.gateway_permissions().inherited(connection, tool)
    }

    /// What applies to a kind's tool when the owner chooses nothing for the kind.
    pub fn kind_default_of(&self, kind: &str, tool: &str) -> anyhow::Result<Permission> {
        self.gateway_permissions().kind_default_of(kind, tool)
    }

    /// Whether the auto permission always asks the owner about a tool: one
    /// that cannot be undone or that reaches other people.
    pub fn always_asked(&self, tool_name: &str) -> anyhow::Result<bool> {
        let (connection, tool) = split_tool(tool_name).unwrap_or(("", tool_name));
        let marks = if connection.is_empty() {
            builtin_marks(tool)
        } else {
            self.gateway_permissions().marks_of(connection, tool)?
        };
        Ok(auto_course(marks, tool) == AutoCourse::Ask)
    }

    /// The permission a kind gives a tool and where it came from: the kind's own
    /// permission, or its default.
    pub fn kind_permission_detail(
        &self,
        kind: impl AsRef<str>,
        tool: impl AsRef<str>,
    ) -> anyhow::Result<(Permission, PermissionSource)> {
        self.gateway_permissions()
            .kind_permission_detail(kind, tool)
    }

    /// Set the permission for a tool under `conn:<id>` or `kind:<kind>` for
    /// `actor`; the gateway logs the change.
    pub fn set_scoped(
        &self,
        actor: &str,
        scope: &str,
        tool: &str,
        permission: Option<Permission>,
    ) -> anyhow::Result<()> {
        self.gateway.set_permission(actor, scope, tool, permission)
    }

    /// The permission a scope sets for a tool, if any.
    pub fn scoped(&self, scope: &str, tool: &str) -> anyhow::Result<Option<Permission>> {
        self.gateway_permissions().scoped(scope, tool)
    }

    /// The owner's choice for a built-in tool, kept by the gateway.
    fn owner_permission(&self, tool_name: &str) -> anyhow::Result<Option<Permission>> {
        self.gateway_permissions().scoped(BUILTINS, tool_name)
    }

    /// Remember the tools a session offered, for the console.
    pub fn record_tools(&self, names: &[String]) -> anyhow::Result<()> {
        let now = now();
        let conn = self.store.conn();
        for name in names {
            conn.execute(
                "INSERT INTO tools (name, seen_at) VALUES (?1, ?2)
                 ON CONFLICT (name) DO UPDATE SET seen_at = excluded.seen_at",
                params![name, now],
            )?;
        }
        Ok(())
    }

    /// The model CLI's built-in tools seen so far, by name.
    pub fn builtin_tools(&self) -> anyhow::Result<Vec<ToolEntry>> {
        let names: Vec<String> = {
            let conn = self.store.conn();
            let mut statement = conn.prepare("SELECT name FROM tools ORDER BY name")?;
            statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?
        };
        names
            .into_iter()
            .filter(|name| split_tool(name).is_none() && !LAUNCHERS.contains(&name.as_str()))
            .map(|name| {
                let permission = self.owner_permission(&name)?;
                let effective = permission.unwrap_or(Permission::Auto);
                Ok(ToolEntry {
                    reads: READING_BUILTINS.contains(&name.as_str()),
                    name,
                    permission,
                    effective,
                })
            })
            .collect()
    }

    /// A connection, where it is still there.
    pub fn connection(&self, id: impl AsRef<str>) -> Option<crate::connections::Connection> {
        self.connections.get(id).ok().flatten()
    }

    /// The connection a call of `tool` is made through, where it is still there.
    pub fn connection_of(&self, tool: &str) -> Option<crate::connections::Connection> {
        self.connection(crate::wording::connection_of(tool)?)
    }

    /// The gateway that judges the calls.
    pub fn gateway(&self) -> &Gateway {
        &self.gateway
    }
}

/// Whether a file tool writes inside `dir`, by a path that cannot lead out of
/// it.
fn writes_inside(dir: Option<&Path>, tool_name: &str, input: &Value) -> bool {
    let Some(dir) = dir else {
        return false;
    };
    let Some((_, field)) = FILE_WRITERS.iter().find(|(name, _)| *name == tool_name) else {
        return false;
    };
    let Some(path) = input.get(*field).and_then(Value::as_str) else {
        return false;
    };
    // A Windows `\\?\` path can hide its `..` in one component, so the text is
    // looked at too.
    let (path, dir) = (paths::plain(path.into()), paths::plain(dir.into()));
    let escapes = path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        || path
            .to_string_lossy()
            .split(std::path::is_separator)
            .any(|name| name == "..");
    path.is_absolute()
        && !escapes
        && paths::may_be_within(&path, &dir)
        && real(&path)
            .zip(real(&dir))
            .is_some_and(|(path, dir)| path.starts_with(dir))
}

/// Where `path` leads once its links are followed; what is not there yet stays
/// as written. `None` for a dangling link, whose target a write would make.
fn real(path: &Path) -> Option<PathBuf> {
    if let Ok(real) = path.canonicalize() {
        return Some(real);
    }
    if path.symlink_metadata().is_ok() {
        return None;
    }
    Some(real(path.parent()?)?.join(path.file_name()?))
}

/// A gateway for tests, with the secretary granted everything.
#[cfg(test)]
pub(crate) fn test_gateway(store: famulen_gateway::store::Store) -> Gateway {
    let gateway = Gateway::new(
        store.clone(),
        crate::connections::Connections::new(store),
        reqwest::Client::new(),
        None,
    );
    gateway
        .grant(SECRETARY, famulen_gateway::ANY, famulen_gateway::ANY)
        .unwrap();
    gateway
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn permissions() -> Permissions {
        Permissions::new(
            Store::open_in_memory().unwrap(),
            test_gateway(famulen_gateway::store::Store::open_in_memory().unwrap()),
        )
    }

    /// `path` as a whole path of this system, as Claude Code writes it.
    fn at(path: &str) -> String {
        if cfg!(windows) {
            format!("C:{}", path.replace('/', "\\"))
        } else {
            path.to_owned()
        }
    }

    fn held(d: &Decision) -> bool {
        matches!(d, Decision::Held(_))
    }

    #[test]
    fn a_run_calls_only_what_it_was_handed_and_only_while_it_runs() {
        let p = permissions();
        let judged = |run, connection: &str, tool: &str| {
            let call = Call::new(job_actor(run), connection, tool, json!({}));
            !matches!(p.gateway.judge(&call).unwrap(), Verdict::Deny { .. })
        };
        let handed = ["Read".to_owned(), "mcp__mail__search_threads".to_owned()];
        let later = crate::store::now() + 60;
        p.grant_job(1, &handed, later).unwrap();
        assert!(judged(1, "mail", "search_threads"));
        assert!(judged(1, "", "Read"));
        assert!(!judged(1, "mail", "get_thread"), "another tool");
        assert!(
            !judged(1, "calendar", "search_threads"),
            "another connection"
        );
        assert!(!judged(2, "mail", "search_threads"), "another run");
        p.revoke_job(1).unwrap();
        assert!(!judged(1, "mail", "search_threads"), "once over");
        p.grant_job(1, &handed, crate::store::now() - 1).unwrap();
        assert!(!judged(1, "mail", "search_threads"), "past its time");
    }

    #[tokio::test]
    async fn the_owners_approval_of_a_call_run_directly_is_used_once_and_by_that_run_alone() {
        let p = permissions();
        let call = json!({ "command": "rm -rf build" });
        for run in [1, 2] {
            p.grant_job(run, &[], crate::store::now() + 60).unwrap();
        }
        let ask = |run| p.decide_direct(run, "Bash", &call, None, None);
        let (Decision::Held(first), Decision::Held(second)) =
            (ask(1).await.unwrap(), ask(2).await.unwrap())
        else {
            panic!("both are asked of the owner");
        };
        assert_ne!(first, second);
        let ticket = p.gateway.ticket(first).unwrap().unwrap();
        assert_eq!(ticket.actor, job_actor(1));
        assert!(is_worker(&ticket.actor) && !is_worker("workers") && is_worker(WORKER));
        assert!(is_job_actor(&ticket.actor) && !is_job_actor(WORKER));
        p.gateway
            .answer_ticket(first, famulen_gateway::Answer::Approve)
            .await
            .unwrap();

        assert!(held(&ask(2).await.unwrap()), "another run is still asked");
        assert!(matches!(
            p.take_direct_pass(2, second, "Bash", &call).unwrap(),
            Pass::Gone
        ));
        assert_eq!(
            p.take_direct_pass(1, first, "Bash", &call).unwrap(),
            Pass::Taken
        );
        assert!(
            matches!(
                p.take_direct_pass(1, first, "Bash", &call).unwrap(),
                Pass::Gone
            ),
            "once"
        );
        assert!(held(&ask(1).await.unwrap()), "and asked again after");
        // An approval used while the tool was allowed anyway is spent all the same.
        let Decision::Held(third) = ask(1).await.unwrap() else {
            panic!()
        };
        p.gateway
            .answer_ticket(third, famulen_gateway::Answer::Approve)
            .await
            .unwrap();
        p.set_scoped("owner", "", "Bash", Some(Permission::Allow))
            .unwrap();
        assert_eq!(
            p.take_direct_pass(1, third, "Bash", &call).unwrap(),
            Pass::Taken
        );
        p.set_scoped("owner", "", "Bash", None).unwrap();
        assert!(held(&ask(1).await.unwrap()), "no pass is left behind");
        p.revoke_job(1).unwrap();
        assert!(
            matches!(ask(1).await.unwrap(), Decision::Forbid(_)),
            "nothing once over"
        );
    }

    #[tokio::test]
    async fn an_own_tool_asks_first_only_when_its_definition_says_so() {
        let p = permissions();
        let def = |name: &str, confirm: bool| crate::hub::ToolDef {
            name: name.into(),
            description: String::new(),
            input_schema: json!({}),
            read_only: false,
            confirm,
        };
        p.record_own_tools(&[def("add_task", false), def("send_slack", true)])
            .unwrap();
        assert_eq!(
            p.decide("mcp__famulen__add_task", &json!({}))
                .await
                .unwrap(),
            Decision::Allow
        );
        let tool = "mcp__famulen__send_slack";
        assert!(held(&p.decide(tool, &json!({})).await.unwrap()));
        p.set_scoped(
            "owner",
            "conn:famulen",
            "send_slack",
            Some(Permission::Allow),
        )
        .unwrap();
        assert_eq!(p.decide(tool, &json!({})).await.unwrap(), Decision::Allow);
        p.set_scoped(
            "owner",
            "conn:famulen",
            "send_slack",
            Some(Permission::Forbid),
        )
        .unwrap();
        assert!(matches!(
            p.decide(tool, &json!({})).await.unwrap(),
            Decision::Forbid(_)
        ));
    }

    #[tokio::test]
    async fn a_connections_tool_is_left_to_the_gateway_when_it_arrives() {
        let p = permissions();
        assert_eq!(
            p.decide("mcp__gmail-work__send_message", &json!({}))
                .await
                .unwrap(),
            Decision::Allow
        );
        assert!(p.gateway().log_entries(10).unwrap().is_empty());
    }

    #[tokio::test]
    async fn no_model_starts_an_agent_by_itself_whatever_the_permissions_say() {
        let p = permissions();
        for tool in LAUNCHERS {
            p.set_scoped("owner", BUILTINS, tool, Some(Permission::Allow))
                .unwrap();
            let Decision::Forbid(why) = p.decide(tool, &json!({})).await.unwrap() else {
                panic!("{tool} should be refused");
            };
            assert!(why.contains("delegate_task"), "{why}");
        }
    }

    #[tokio::test]
    async fn the_secretary_is_neither_offered_nor_let_use_the_tool_that_starts_an_adroit_session() {
        let dir = crate::notebook::testing::TempDir::new("permissions-launch");
        let app = crate::runtime::testing::app(dir.path());
        let c = app
            .gateway
            .add_defined("adroit", Default::default(), None)
            .unwrap();
        let p = &app.permissions;
        p.set_scoped(
            "owner",
            &format!("conn:{}", c.id),
            "start_session",
            Some(Permission::Allow),
        )
        .unwrap();

        let tool = format!("mcp__{}__start_session", c.id);
        assert_eq!(
            p.decide(&tool, &json!({})).await.unwrap(),
            Decision::Forbid(DELEGATE_INSTEAD.to_owned())
        );
        let call = |name: &str| {
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": { "name": name, "arguments": {} } })
        };
        let refused = p.refused_launch(&c.id, &call("start_session")).unwrap();
        assert_eq!(refused["result"]["isError"], true);
        assert!(p.refused_launch(&c.id, &call("resume_session")).is_some());
        assert_eq!(p.refused_launch(&c.id, &call("send_to_session")), None);
        let listed = p.without_launchers(
            &c.id,
            json!({ "id": 1, "result": { "tools": [{ "name": "start_session" }, { "name": "send_to_session" }] } }),
        );
        assert_eq!(
            listed["result"]["tools"],
            json!([{ "name": "send_to_session" }])
        );
    }

    #[tokio::test]
    async fn the_vaults_own_tools_are_judged_by_the_vaults_permissions() {
        let p = permissions();
        for (tool, input) in [
            ("mcp__famulen__delete_secret", json!({ "id": "pw" })),
            (
                "mcp__famulen__describe_secret",
                json!({ "name": "pw", "description": "bank" }),
            ),
        ] {
            assert_eq!(
                p.decide(tool, &input).await.unwrap(),
                Decision::Allow,
                "{tool}"
            );
        }
    }

    #[tokio::test]
    async fn a_held_or_refused_call_is_in_the_gateways_log() {
        let p = permissions().with_notebook_dir(at("/data/notebook"));
        p.set_scoped("owner", BUILTINS, "Bash", Some(Permission::Forbid))
            .unwrap();
        let held_call = p
            .decide("Write", &json!({ "file_path": at("/data/notebook/a.md") }))
            .await
            .unwrap();
        p.decide("Bash", &json!({ "command": "ls" })).await.unwrap();
        let Decision::Held(id) = held_call else {
            panic!("held");
        };
        let log = p.gateway().log_entries(10).unwrap();
        assert_eq!(
            log.iter()
                .map(|e| (e.tool.as_str(), e.decision.as_str(), e.actor.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Bash", "deny", SECRETARY),
                ("Write", "confirm", SECRETARY),
                ("Write", "confirm", famulen_gateway::ASSESSOR),
                ("permission.set", "allow", "owner")
            ]
        );
        assert!(p.gateway().ticket(id).unwrap().is_some());
    }

    /// An assessor that lets every call it weighs run.
    struct LetsRun;

    impl famulen_gateway::Assessor for LetsRun {
        fn assess<'a>(
            &'a self,
            _: &'a Call,
        ) -> futures_util::future::BoxFuture<'a, anyhow::Result<famulen_gateway::Assessment>>
        {
            Box::pin(async {
                Ok(famulen_gateway::Assessment::Allow {
                    reason: "as asked".into(),
                })
            })
        }
    }

    #[test]
    fn using_a_skill_only_reads_instructions() {
        assert_eq!(builtin_marks("Skill").read_only, Some(true));
    }

    #[tokio::test]
    async fn every_tool_is_auto_until_the_owner_chooses() {
        let p = permissions();
        for tool in ["Read", "Write", "Bash", "mcp__gmail-work__send_message"] {
            assert_eq!(
                p.permission_detail(tool).unwrap(),
                (Permission::Auto, PermissionSource::Default),
                "{tool}"
            );
        }
    }

    #[tokio::test]
    async fn a_reading_builtin_runs_and_a_writing_one_runs_when_the_assessor_lets_it() {
        let p = permissions().with_notebook_dir(at("/data/notebook"));
        assert_eq!(
            p.decide("WebSearch", &json!({ "query": "x" }))
                .await
                .unwrap(),
            Decision::Allow
        );
        assert!(held(
            &p.decide(
                "Write",
                &json!({ "file_path": at("/data/notebook/records/a.md") })
            )
            .await
            .unwrap()
        ));
        p.gateway().set_assessor(std::sync::Arc::new(LetsRun));
        assert_eq!(
            p.decide(
                "Write",
                &json!({ "file_path": at("/data/notebook/records/b.md") })
            )
            .await
            .unwrap(),
            Decision::Allow
        );
    }

    #[tokio::test]
    async fn a_shell_command_is_asked_about_even_when_the_assessor_would_let_it_run() {
        let p = permissions();
        p.gateway().set_assessor(std::sync::Arc::new(LetsRun));
        assert!(held(
            &p.decide("Bash", &json!({ "command": "ls" })).await.unwrap()
        ));
        assert_eq!(builtin_marks("Read").read_only, Some(true));
        assert!(builtin_marks("Bash").always_ask);
    }

    #[tokio::test]
    async fn the_secretarys_own_write_outside_the_notebook_is_asked_about_even_when_the_assessor_would_let_it_run()
     {
        let p = permissions()
            .with_memory_dir(at("/data/notebook/memory"))
            .with_notebook_dir(at("/data/notebook"));
        p.gateway().set_assessor(std::sync::Arc::new(LetsRun));
        for (tool, input) in [
            ("Write", json!({ "file_path": at("/srv/code/main.rs") })),
            ("Edit", json!({ "file_path": at("/srv/code/main.rs") })),
            ("MultiEdit", json!({ "file_path": at("/srv/code/main.rs") })),
            (
                "NotebookEdit",
                json!({ "notebook_path": at("/srv/code/a.ipynb") }),
            ),
            (
                "Write",
                json!({ "file_path": at("/data/notebook/../code/x") }),
            ),
            ("Write", json!({ "file_path": at("/data/notebook-evil/x") })),
            ("Write", json!({ "file_path": "main.rs" })),
            ("Write", json!({})),
        ] {
            assert!(
                held(&p.decide(tool, &input).await.unwrap()),
                "{tool} {input}"
            );
        }
        for path in ["/data/notebook/records/a.md", "/data/notebook/memory/a.md"] {
            let path = at(path);
            assert_eq!(
                p.decide("Edit", &json!({ "file_path": path }))
                    .await
                    .unwrap(),
                Decision::Allow,
                "{path}"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_inside_the_notebook_does_not_carry_a_write_out_of_it() {
        use std::os::unix::fs::symlink;
        let tmp = crate::notebook::testing::TempDir::new("permissions-links");
        let (book, code) = (tmp.path().join("book"), tmp.path().join("code"));
        std::fs::create_dir_all(book.join("memory")).unwrap();
        std::fs::create_dir_all(&code).unwrap();
        symlink(&code, book.join("link")).unwrap();
        symlink(&code, book.join("memory/link")).unwrap();
        symlink(code.join("new.rs"), book.join("dangling.md")).unwrap();
        symlink(&book, tmp.path().join("vault")).unwrap();
        let p = permissions()
            .with_memory_dir(tmp.path().join("vault/memory"))
            .with_notebook_dir(tmp.path().join("vault"));
        p.gateway().set_assessor(std::sync::Arc::new(LetsRun));
        let write = |path: &str| json!({ "file_path": tmp.path().join("vault").join(path) });
        for path in ["link/main.rs", "memory/link/main.rs", "dangling.md"] {
            assert!(
                held(&p.decide("Write", &write(path)).await.unwrap()),
                "{path}"
            );
        }
        for path in ["records/new/a.md", "memory/a.md"] {
            assert_eq!(
                p.decide("Write", &write(path)).await.unwrap(),
                Decision::Allow,
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn a_subagent_however_it_came_to_be_uses_nothing() {
        let p = permissions().with_notebook_dir(at("/data/notebook"));
        p.gateway().set_assessor(std::sync::Arc::new(LetsRun));
        for (tool, input) in [
            ("Edit", json!({ "file_path": at("/data/notebook/a.md") })),
            ("Read", json!({ "file_path": at("/srv/a") })),
            ("mcp__mail__search", json!({})),
        ] {
            assert_eq!(
                p.decide_for(Caller::Subagent, tool, &input).await.unwrap(),
                Decision::Forbid(DELEGATE_INSTEAD.to_owned()),
                "{tool}"
            );
        }
    }

    #[tokio::test]
    async fn the_owner_allowing_a_writing_builtin_covers_files_outside_the_notebook() {
        let p = permissions().with_notebook_dir(at("/data/notebook"));
        p.set_scoped("owner", BUILTINS, "Edit", Some(Permission::Allow))
            .unwrap();
        assert_eq!(
            p.decide("Edit", &json!({ "file_path": at("/srv/code/main.rs") }))
                .await
                .unwrap(),
            Decision::Allow
        );
    }

    #[tokio::test]
    async fn the_owners_permission_wins_over_the_default() {
        let p = permissions();
        p.set_scoped("owner", BUILTINS, "Read", Some(Permission::Forbid))
            .unwrap();
        p.set_scoped("owner", BUILTINS, "Write", Some(Permission::Allow))
            .unwrap();
        assert!(matches!(
            p.decide("Read", &json!({})).await.unwrap(),
            Decision::Forbid(_)
        ));
        assert_eq!(
            p.decide("Write", &json!({})).await.unwrap(),
            Decision::Allow
        );
    }

    #[tokio::test]
    async fn clearing_the_owners_choice_brings_back_the_default() {
        let p = permissions();
        p.set_scoped("owner", BUILTINS, "Write", Some(Permission::Allow))
            .unwrap();
        p.set_scoped("owner", BUILTINS, "Write", None).unwrap();
        assert_eq!(p.permission("Write").unwrap(), Permission::Auto);
    }

    #[tokio::test]
    async fn writing_inside_the_memory_directory_needs_no_approval() {
        let p = permissions().with_memory_dir(at("/data/memory"));
        let inside = json!({ "file_path": at("/data/memory/owner.md"), "content": "x" });
        assert_eq!(p.decide("Write", &inside).await.unwrap(), Decision::Allow);
        assert_eq!(p.decide("Edit", &inside).await.unwrap(), Decision::Allow);
    }

    #[tokio::test]
    async fn writing_outside_the_memory_directory_still_asks() {
        let p = permissions().with_memory_dir(at("/data/memory"));
        for path in [
            at("/data/memory/../famulen.sqlite3"),
            at("/data/memory-evil/x.md"),
            "memory/x.md".into(),
            at("/data/workspace/x"),
        ] {
            assert!(
                held(
                    &p.decide("Write", &json!({ "file_path": path }))
                        .await
                        .unwrap()
                ),
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn the_owner_forbidding_a_tool_also_covers_the_memory_directory() {
        let p = permissions().with_memory_dir(at("/data/memory"));
        p.set_scoped("owner", BUILTINS, "Write", Some(Permission::Forbid))
            .unwrap();
        let inside = json!({ "file_path": at("/data/memory/owner.md") });
        assert!(matches!(
            p.decide("Write", &inside).await.unwrap(),
            Decision::Forbid(_)
        ));
    }

    fn sealed() -> Permissions {
        permissions()
            .with_memory_dir(at("/data/memory"))
            .with_sealed(
                [at("/data/gateway"), at("/srv/famulen/.env")],
                at("/data/workspace"),
            )
    }

    fn forbidden(decision: &Decision) -> bool {
        matches!(decision, Decision::Forbid(_))
    }

    #[tokio::test]
    async fn file_tools_cannot_reach_the_gateways_data_or_the_env_file_even_when_allowed() {
        let p = sealed();
        for tool in [
            "Read",
            "Write",
            "Edit",
            "MultiEdit",
            "Glob",
            "Grep",
            "LS",
            "Bash",
        ] {
            p.set_scoped("owner", BUILTINS, tool, Some(Permission::Allow))
                .unwrap();
        }
        for (tool, input) in [
            (
                "Read",
                json!({ "file_path": at("/data/gateway/vault/bank.age") }),
            ),
            (
                "Read",
                json!({ "file_path": at("/data/gateway/gateway.sqlite3") }),
            ),
            (
                "Read",
                json!({ "file_path": at("/data/memory/../gateway/vault/x.age") }),
            ),
            ("Read", json!({ "file_path": at("/srv/famulen/.env") })),
            (
                "Write",
                json!({ "file_path": at("/data/gateway/services/x.toml"), "content": "" }),
            ),
            ("Edit", json!({ "file_path": at("/srv/famulen/.env") })),
            ("MultiEdit", json!({ "file_path": at("/data/gateway/x") })),
            (
                "NotebookEdit",
                json!({ "notebook_path": at("/data/gateway/x.ipynb") }),
            ),
            (
                "Glob",
                json!({ "pattern": "**/*.age", "path": at("/data/gateway") }),
            ),
            ("Glob", json!({ "pattern": at("/data/gateway/**/*") })),
            (
                "Glob",
                json!({ "pattern": "**/*.age", "path": at("/data") }),
            ),
            ("Glob", json!({ "pattern": "../gateway/**" })),
            (
                "Grep",
                json!({ "pattern": "token", "path": at("/data/gateway/gateway.sqlite3") }),
            ),
            ("Grep", json!({ "pattern": "token", "path": at("/data") })),
            ("Grep", json!({ "pattern": "token", "path": ".." })),
            ("LS", json!({ "path": at("/data/gateway") })),
            (
                "Bash",
                json!({ "command": format!("cat {}", at("/data/gateway/vault/bank.age")) }),
            ),
            (
                "Bash",
                json!({ "command": "sqlite3 ../gateway/gateway.sqlite3 .dump" }),
            ),
            (
                "Bash",
                json!({ "command": format!("cat {}", at("/srv/famulen/.env")) }),
            ),
            ("Bash", json!({ "command": "printenv FAMULEN_VAULT_KEY" })),
        ] {
            assert!(
                forbidden(&p.decide(tool, &input).await.unwrap()),
                "{tool} {input}"
            );
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_sealed_place_is_sealed_in_every_spelling_windows_has_for_it() {
        let p = sealed();
        for tool in ["Read", "Glob", "Grep", "LS", "Bash", "PowerShell"] {
            p.set_scoped("owner", BUILTINS, tool, Some(Permission::Allow))
                .unwrap();
        }
        for path in [
            "c:/data/gateway/x",
            r"C:\DATA\Gateway\x",
            r"\data\gateway\x",
            "/data/gateway/x",
            "/c/data/gateway/x",
            r"C:..\gateway\x",
            r"\\?\C:\data\gateway\x",
            r"\\?\c:\data\GATEWAY\x",
            r"\\.\C:\data\gateway\x",
            r"\\localhost\c$\data\gateway\x",
            r"C:\data\gateway.\x",
            r"D:gateway\x",
            r"\\?\Volume{4c1b02c1-d990-11dc-99ae-806e6f6e6963}\data\gateway\x",
            r"\\server\c$\data\gateway\x",
            r"C:\data\gateway::$INDEX_ALLOCATION\x",
            r"..\Gateway\x",
        ] {
            for (tool, input) in [
                ("Read", json!({ "file_path": path })),
                ("LS", json!({ "path": path })),
                ("Grep", json!({ "pattern": "x", "path": path })),
                ("Glob", json!({ "pattern": format!("{path}/**") })),
            ] {
                assert!(
                    forbidden(&p.decide(tool, &input).await.unwrap()),
                    "{tool} {input}"
                );
            }
        }
        for path in ["c:/", r"C:\Data", "/c/data", r"\\?\C:\data"] {
            assert!(
                forbidden(
                    &p.decide("Grep", &json!({ "pattern": "x", "path": path }))
                        .await
                        .unwrap()
                ),
                "{path}"
            );
        }
        for command in [
            "cat /c/data/gateway/x",
            "cat c:/Data/gateway/x",
            r"type C:\data\gateway\x",
            r"type ..\gateway\x",
            r"type \srv\famulen\.ENV",
            "echo $famulen_vault_key",
        ] {
            for tool in ["Bash", "PowerShell"] {
                assert!(
                    forbidden(
                        &p.decide(tool, &json!({ "command": command }))
                            .await
                            .unwrap()
                    ),
                    "{tool} {command}"
                );
            }
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_windows_spelling_does_not_carry_a_write_out_of_the_memory_directory() {
        let p = permissions().with_memory_dir(r"\\?\C:\data\memory");
        for path in [r"C:\data\memory\a.md", "C:/data/memory/a.md"] {
            assert_eq!(
                p.decide("Write", &json!({ "file_path": path }))
                    .await
                    .unwrap(),
                Decision::Allow,
                "{path}"
            );
        }
        for path in [
            r"C:\data\memory\..\x",
            "C:/data/memory/../x",
            r"\\?\C:\data\memory\..\x",
            r"\\?\C:\data\memory\a/../../x",
            r"C:\data\memory.\a.md",
            r"D:\data\memory\a.md",
            r"\data\memory\a.md",
            "C:a.md",
        ] {
            assert!(
                held(
                    &p.decide("Write", &json!({ "file_path": path }))
                        .await
                        .unwrap()
                ),
                "{path}"
            );
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_write_into_the_memory_directory_may_spell_it_in_another_case() {
        let dir = std::env::temp_dir().join(format!("famulen-case-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = permissions().with_memory_dir(&dir);
        let path = dir.join("a.md").display().to_string().to_uppercase();

        let decision = p.decide("Write", &json!({ "file_path": path })).await;

        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(decision.unwrap(), Decision::Allow);
    }

    #[tokio::test]
    async fn file_tools_still_reach_the_notebook_and_the_workspace() {
        let p = sealed();
        for tool in ["Read", "Glob", "Grep", "LS", "Bash"] {
            p.set_scoped("owner", BUILTINS, tool, Some(Permission::Allow))
                .unwrap();
        }
        for (tool, input) in [
            ("Read", json!({ "file_path": at("/data/memory/owner.md") })),
            ("Read", json!({ "file_path": at("/data/gateway-notes.md") })),
            (
                "Bash",
                json!({ "command": format!("cat {}", at("/data/gateway-notes.md")) }),
            ),
            ("Bash", json!({ "command": "cp .env.sample example" })),
            ("Bash", json!({ "command": "grep envelope notes.md" })),
            (
                "Glob",
                json!({ "pattern": "**/*.md", "path": at("/data/memory") }),
            ),
            ("Grep", json!({ "pattern": "x" })),
            ("LS", json!({ "path": at("/data") })),
        ] {
            assert_eq!(
                p.decide(tool, &input).await.unwrap(),
                Decision::Allow,
                "{tool} {input}"
            );
        }
        assert_eq!(
            p.decide(
                "Write",
                &json!({ "file_path": at("/data/memory/owner.md") })
            )
            .await
            .unwrap(),
            Decision::Allow
        );
    }

    #[tokio::test]
    async fn a_refused_reach_says_why_and_is_in_the_gateways_log() {
        let p = sealed();
        let Decision::Forbid(why) = p
            .decide(
                "Read",
                &json!({ "file_path": at("/data/gateway/vault/bank.age") }),
            )
            .await
            .unwrap()
        else {
            panic!("refused");
        };
        assert!(why.contains("no tool may"), "{why}");
        let entry = &p.gateway().log_entries(1).unwrap()[0];
        assert_eq!(
            (entry.tool.as_str(), entry.decision.as_str()),
            ("Read", "deny")
        );
    }

    #[tokio::test]
    async fn bash_is_not_a_file_writer() {
        let p = permissions().with_memory_dir(at("/data/memory"));
        assert!(held(
            &p.decide("Bash", &json!({ "command": "echo > /data/memory/x" }))
                .await
                .unwrap()
        ));
    }

    #[tokio::test]
    async fn an_approval_lets_the_same_call_through_once() {
        let p = permissions();
        let input = json!({ "file_path": "a", "content": "x" });
        let Decision::Held(id) = p.decide("Write", &input).await.unwrap() else {
            panic!("held");
        };
        p.gateway()
            .answer_ticket(id, famulen_gateway::Answer::Approve)
            .await
            .unwrap();
        assert_eq!(p.decide("Write", &input).await.unwrap(), Decision::Allow);
        assert!(held(&p.decide("Write", &input).await.unwrap()));
    }

    #[tokio::test]
    async fn an_approval_does_not_cover_a_different_call_and_ignores_key_order() {
        let p = permissions();
        let input: Value = serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();
        let Decision::Held(id) = p.decide("Write", &input).await.unwrap() else {
            panic!("held");
        };
        p.gateway()
            .answer_ticket(id, famulen_gateway::Answer::Approve)
            .await
            .unwrap();
        assert!(held(&p.decide("Edit", &input).await.unwrap()));
        assert!(held(
            &p.decide("Write", &json!({ "a": 1, "b": 3 })).await.unwrap()
        ));
        let reordered: Value = serde_json::from_str(r#"{"b":2,"a":1}"#).unwrap();
        assert_eq!(
            p.decide("Write", &reordered).await.unwrap(),
            Decision::Allow
        );
    }

    #[tokio::test]
    async fn the_builtin_tool_list_shows_the_owners_choice_and_what_applies() {
        let p = permissions();
        p.record_tools(&[
            "Write".into(),
            "Read".into(),
            "mcp__gmail-work__send_message".into(),
        ])
        .unwrap();
        p.set_scoped("owner", BUILTINS, "Write", Some(Permission::Allow))
            .unwrap();
        assert_eq!(
            p.builtin_tools().unwrap(),
            vec![
                ToolEntry {
                    name: "Read".into(),
                    permission: None,
                    effective: Permission::Auto,
                    reads: true,
                },
                ToolEntry {
                    name: "Write".into(),
                    permission: Some(Permission::Allow),
                    effective: Permission::Allow,
                    reads: false,
                },
            ]
        );
    }

    #[tokio::test]
    async fn the_owners_choice_for_a_builtin_tool_is_kept_by_the_gateway() {
        let p = permissions();
        p.set_scoped("owner", BUILTINS, "Bash", Some(Permission::Forbid))
            .unwrap();
        assert_eq!(
            GatewayPermissions::new(p.gateway().store().clone())
                .scoped(BUILTINS, "Bash")
                .unwrap(),
            Some(Permission::Forbid)
        );
        assert_eq!(
            p.permission_detail("Bash").unwrap(),
            (Permission::Forbid, PermissionSource::Connection)
        );
    }

    fn with_gmail() -> (Store, famulen_gateway::store::Store, Permissions) {
        let store = Store::open_in_memory().unwrap();
        let gateway = famulen_gateway::store::Store::open_in_memory().unwrap();
        let connections = crate::connections::Connections::new(gateway.clone());
        for id in ["gmail-work", "gmail-home"] {
            connections
                .add(
                    id,
                    id,
                    "gmail",
                    crate::connections::Transport::McpRemote {
                        url: "https://a".into(),
                    },
                    Default::default(),
                    "",
                )
                .unwrap();
        }
        (
            store.clone(),
            gateway.clone(),
            Permissions::new(store, test_gateway(gateway)),
        )
    }

    #[tokio::test]
    async fn a_connection_permission_wins_over_the_kind_default() {
        let (_, _, p) = with_gmail();
        p.set_scoped(
            "owner",
            "kind:gmail",
            "send_message",
            Some(Permission::Confirm),
        )
        .unwrap();
        p.set_scoped(
            "owner",
            "conn:gmail-home",
            "send_message",
            Some(Permission::Forbid),
        )
        .unwrap();
        assert_eq!(
            p.permission_detail("mcp__gmail-work__send_message")
                .unwrap(),
            (Permission::Confirm, PermissionSource::Kind)
        );
        assert_eq!(
            p.permission_detail("mcp__gmail-home__send_message")
                .unwrap(),
            (Permission::Forbid, PermissionSource::Connection)
        );
    }

    #[tokio::test]
    async fn the_kind_default_covers_every_connection_of_the_kind() {
        let (_, _, p) = with_gmail();
        p.set_scoped(
            "owner",
            "kind:gmail",
            "create_draft",
            Some(Permission::Allow),
        )
        .unwrap();
        for c in ["gmail-work", "gmail-home"] {
            assert_eq!(
                p.permission(&format!("mcp__{c}__create_draft")).unwrap(),
                Permission::Allow
            );
        }
    }

    #[tokio::test]
    async fn a_kinds_own_permission_is_reported_as_the_kinds_default() {
        let (_, _, p) = with_gmail();
        p.set_scoped(
            "owner",
            "kind:gmail",
            "send_message",
            Some(Permission::Forbid),
        )
        .unwrap();
        p.set_scoped(
            "owner",
            "conn:gmail-home",
            "send_message",
            Some(Permission::Allow),
        )
        .unwrap();
        assert_eq!(
            p.kind_permission_detail("gmail", "send_message").unwrap(),
            (Permission::Forbid, PermissionSource::Kind),
            "an account's own permission does not change the kind's"
        );
    }

    #[tokio::test]
    async fn a_kind_without_its_own_permission_is_auto() {
        let (_, _, p) = with_gmail();
        assert_eq!(
            p.kind_permission_detail("gmail", "send_message").unwrap(),
            (Permission::Auto, PermissionSource::Default)
        );
    }

    #[tokio::test]
    async fn a_kind_without_its_own_permission_shows_what_the_definition_gives_its_accounts() {
        let (_, _, p) = with_gmail();
        let allow = [("send_message".to_owned(), Permission::Allow)];
        for c in ["gmail-work", "gmail-home"] {
            p.gateway_permissions().set_defaults(c, &allow).unwrap();
        }
        assert_eq!(
            p.kind_permission_detail("gmail", "send_message").unwrap(),
            (Permission::Allow, PermissionSource::Default)
        );
    }

    #[tokio::test]
    async fn the_inherited_permission_is_what_applies_without_the_accounts_own() {
        let (_, _, p) = with_gmail();
        let name = "mcp__gmail-work__send_message";
        p.set_scoped(
            "owner",
            "conn:gmail-work",
            "send_message",
            Some(Permission::Forbid),
        )
        .unwrap();
        assert_eq!(p.inherited(name).unwrap(), Permission::Auto);
        p.set_scoped(
            "owner",
            "kind:gmail",
            "send_message",
            Some(Permission::Confirm),
        )
        .unwrap();
        assert_eq!(p.inherited(name).unwrap(), Permission::Confirm);
        assert_eq!(p.permission(name).unwrap(), Permission::Forbid);
    }

    #[tokio::test]
    async fn a_connection_outside_the_workgroup_is_asked_about() {
        let (store, _, p) = with_gmail();
        let (_tmp, records) = crate::records::testing::records("permissions-workgroup");
        let workgroups =
            crate::workgroups::Workgroups::new(store, records.notebook().clone()).unwrap();
        workgroups
            .set_member(
                "work",
                crate::workgroups::MemberKind::Connection,
                "gmail-work",
                true,
            )
            .unwrap();
        assert_eq!(p.ask_in("gmail-work", "work").unwrap(), None);
        assert!(p.ask_in("gmail-home", "work").unwrap().is_some());
        assert_eq!(p.ask_in("gmail-home", "all").unwrap(), None);
        assert_eq!(p.ask_in(hub::SERVER_NAME, "work").unwrap(), None);
    }
}
