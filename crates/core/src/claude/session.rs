//! One Claude Code process: inputs go in on stdin, and famulen answers the
//! permission and MCP requests that come back on stdout.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::protocol::{self, ControlRequest, Inbound};
use crate::approvals::Approvals;
use crate::hub::{self, Hub};
use crate::journal::Journal;
use crate::permissions::{Decision, Permissions};
use crate::slack::clients::SlackClients;
use crate::slack::web::Slack;
use crate::store::Store;

const SESSION_ID_KEY: &str = "claude.session_id";

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub bin: String,
    pub model: Option<String>,
    /// `--effort`, when set.
    pub effort: Option<String>,
    pub workdir: PathBuf,
    pub system_prompt: String,
    /// Directories outside the working directory the agent works in.
    pub add_dirs: Vec<PathBuf>,
    /// The conversation to continue; a new one starts when `None`.
    pub resume: Option<String>,
    /// The owner's connections by id. Each is a server Famulen answers
    /// through the gateway; the CLI is never told how to reach a service.
    pub connections: Vec<String>,
    /// Places no shell command may read or write: Claude Code's own sandbox
    /// keeps them out of reach, however a command spells its way there.
    pub sealed: Vec<PathBuf>,
    /// Places outside the working directory shell commands may write in,
    /// as the owner chose them. One inside a sealed place is left out.
    pub writable: Vec<PathBuf>,
    /// The folder of plugins made from the owner's enabled plugins, holding
    /// nothing but their skills.
    pub plugin_dir: Option<PathBuf>,
}

/// The settings that run shell commands in Claude Code's sandbox, which can
/// neither read nor write `sealed`, and write only in `writable` beyond the
/// working directory.
pub(crate) fn sandbox_settings(sealed: &[PathBuf], writable: &[PathBuf]) -> Value {
    let shown = |p: &PathBuf| p.display().to_string();
    // denyWrite already wins inside allowWrite; leaving such places out keeps
    // the settings from even naming a sealed place as writable.
    let writable: Vec<String> = writable
        .iter()
        .filter(|w| !sealed.iter().any(|s| crate::paths::may_be_under(w, s)))
        .map(shown)
        .collect();
    let sealed: Vec<String> = sealed.iter().map(shown).collect();
    // Every shell command still asks famulen; the sandbox only bounds what an
    // allowed one can touch, and refuses to run unsandboxed.
    serde_json::json!({ "sandbox": {
        "enabled": true,
        "failIfUnavailable": true,
        "allowUnsandboxedCommands": false,
        "autoAllowBashIfSandboxed": false,
        "filesystem": {
            "denyRead": sealed,
            "denyWrite": sealed,
            "allowWrite": writable,
        },
    } })
}

impl Options {
    /// The arguments Claude Code is started with.
    ///
    /// Every tool call is put to famulen (`--permission-prompt-tool stdio`
    /// with the `manual` mode), and only project settings are read, so the
    /// host user's own permission rules cannot let a tool run unasked.
    pub fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = [
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
            "--permission-prompt-tool",
            "stdio",
            "--permission-mode",
            "manual",
            "--setting-sources",
            "project",
            "--strict-mcp-config",
        ]
        .map(String::from)
        .into();
        // Work is delegated through tasks; the model is not even offered a tool
        // that starts an agent. The `=` form ends the list of names.
        args.push(format!(
            "--disallowedTools={}",
            crate::permissions::LAUNCHERS.join(",")
        ));
        args.push("--mcp-config".into());
        let mut config = hub::mcp_config();
        for id in &self.connections {
            if id != hub::SERVER_NAME {
                config["mcpServers"][id] = serde_json::json!({ "type": "sdk", "name": id });
            }
        }
        args.push(config.to_string());
        if !self.sealed.is_empty() {
            args.push("--settings".into());
            args.push(sandbox_settings(&self.sealed, &self.writable).to_string());
        }
        args.push("--append-system-prompt".into());
        args.push(self.system_prompt.clone());
        for dir in &self.add_dirs {
            args.push("--add-dir".into());
            args.push(dir.display().to_string());
        }
        if let Some(dir) = &self.plugin_dir {
            args.push("--plugin-dir".into());
            args.push(dir.display().to_string());
        }
        if let Some(model) = &self.model {
            args.push("--model".into());
            args.push(model.clone());
        }
        if let Some(effort) = &self.effort {
            args.push("--effort".into());
            args.push(effort.clone());
        }
        // The `=` form keeps a value that starts with `-` from being read as a flag.
        if let Some(session_id) = &self.resume {
            args.push(format!("--resume={session_id}"));
        }
        args
    }

    /// The environment Claude Code is started with, on top of Famulen's.
    /// The connectors of the claude.ai account it is logged in with would
    /// otherwise join the tools, outside the gateway.
    pub fn env(&self) -> Vec<(&'static str, &'static str)> {
        vec![("ENABLE_CLAUDEAI_MCP_SERVERS", "false")]
    }
}

/// What a session needs to answer Claude Code's requests.
pub struct Services<S> {
    pub hub: Hub,
    pub permissions: Permissions,
    pub approvals: Approvals<S>,
    pub journal: Journal,
    pub store: Store,
    pub memory_dir: PathBuf,
    /// Where each turn's usage is counted, and as which model.
    pub usage: Option<(crate::models::Models, crate::models::ModelSpec)>,
    /// Told when a turn ends, so inputs held for another workgroup go in.
    pub turn_ended: Option<Arc<tokio::sync::Notify>>,
    pub in_flight: InFlight,
    /// What answers the servers that stand for the owner's connections.
    pub gateway: Option<crate::gateway::Gateway>,
}

impl<S: Slack + Clone> Services<S> {
    /// `owner` is the owner's ID in the main workspace, where approvals are
    /// also asked when Slack is connected; the console always gets them.
    pub fn new(
        slack: Option<&SlackClients<S>>,
        store: Store,
        permissions: Permissions,
        owner: impl Into<String>,
        memory_dir: impl Into<PathBuf>,
        tools: Arc<dyn crate::hub::OwnTools>,
        console: Option<crate::console_log::ConsoleLog>,
    ) -> Self {
        let memory_dir = memory_dir.into();
        let (main, main_id) = match &slack {
            Some(s) => (Some(s.main().clone()), s.main_id().to_owned()),
            None => (None, String::new()),
        };
        let mut approvals =
            Approvals::new(main, store.clone(), permissions.clone(), main_id, owner);
        if let Some(console) = console {
            approvals = approvals.with_console(console);
        }
        Self {
            hub: Hub::new(tools),
            permissions: permissions.with_memory_dir(&memory_dir),
            approvals,
            journal: Journal::new(store.clone()),
            store,
            memory_dir,
            usage: None,
            turn_ended: None,
            in_flight: InFlight::default(),
            gateway: None,
        }
    }
}

/// The tool calls a session is answering. They run as tasks of their own and
/// outlive the session they came from; the input of a turn cut off is given
/// again only once they have settled, so a call still on its way (a mail
/// being sent) cannot be made a second time by the next turn.
#[derive(Clone, Default)]
pub struct InFlight {
    count: Arc<std::sync::atomic::AtomicUsize>,
    settled: Arc<tokio::sync::Notify>,
}

impl InFlight {
    fn enter(&self) -> InFlightGuard {
        self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        InFlightGuard(self.clone())
    }

    /// Wait until no tool call is running.
    pub async fn settled(&self) {
        loop {
            let notified = self.settled.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.count.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                return;
            }
            notified.await;
        }
    }
}

struct InFlightGuard(InFlight);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if self
            .0
            .count
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
            == 1
        {
            self.0.settled.notify_waiters();
        }
    }
}

/// The saved conversation to resume, if any.
pub fn saved_session(store: &Store) -> anyhow::Result<Option<String>> {
    store.state(SESSION_ID_KEY)
}

/// A running session's input side.
#[derive(Clone)]
pub struct Handle {
    tx: mpsc::UnboundedSender<Value>,
}

impl Handle {
    /// Deliver `text` as a user message; it starts a turn or joins the running one.
    pub fn send(&self, text: impl AsRef<str>) -> anyhow::Result<()> {
        self.tx
            .send(protocol::user_message(text.as_ref()))
            .map_err(|_| anyhow::anyhow!("the claude session has ended"))
    }
}

/// Start Claude Code. The task finishes when the process ends.
pub fn spawn<S: Slack + 'static>(
    options: &Options,
    services: Arc<Services<S>>,
) -> anyhow::Result<(Handle, JoinHandle<anyhow::Result<()>>)> {
    std::fs::create_dir_all(&options.workdir)?;
    let mut command = Command::new(&options.bin);
    crate::config::hide_secrets(&mut command);
    let mut child = command
        .args(options.args())
        .current_dir(&options.workdir)
        .envs(options.env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("no stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("no stdout"))?;

    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(protocol::initialize_request("famulen-init"))?;
    let writer = tokio::spawn(write_lines(stdin, rx));
    let handle = Handle { tx: tx.clone() };

    let task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await? {
            match protocol::parse_line(&line) {
                Ok(inbound) => on_inbound(inbound, &tx, &services),
                Err(error) => tracing::debug!(%error, %line, "unreadable line from claude"),
            }
        }
        writer.abort();
        let status = child.wait().await?;
        anyhow::bail!("claude exited: {status}")
    });
    Ok((handle, task))
}

async fn write_lines(
    mut stdin: ChildStdin,
    mut rx: mpsc::UnboundedReceiver<Value>,
) -> anyhow::Result<()> {
    while let Some(message) = rx.recv().await {
        let mut line = message.to_string();
        line.push('\n');
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
    }
    Ok(())
}

fn on_inbound<S: Slack + 'static>(
    inbound: Inbound,
    tx: &mpsc::UnboundedSender<Value>,
    services: &Arc<Services<S>>,
) {
    let store = &services.store;
    match inbound {
        Inbound::Init { session_id, tools } => {
            tracing::info!(%session_id, tools = tools.len(), "claude session ready");
            services.journal.record(
                "session",
                format!("ready: {session_id} ({} tools)", tools.len()),
            );
            remember_session(store, &session_id);
            if let Err(error) = services.permissions.record_tools(&tools) {
                tracing::warn!(%error, "could not record the tools");
            }
        }
        Inbound::Result {
            session_id,
            is_error,
            usage,
            ..
        } => {
            if let Some((models, spec)) = &services.usage
                && let Err(error) = models.record(crate::models::Role::Secretary, spec, &usage)
            {
                tracing::warn!(%error, "could not count the turn");
            }
            tracing::info!(%session_id, is_error, "turn finished");
            services.journal.record(
                "turn",
                if is_error {
                    "ended with an error"
                } else {
                    "ended"
                },
            );
            remember_session(store, &session_id);
            if let Err(error) = store.end_turn() {
                tracing::warn!(%error, "could not mark the turn ended");
            }
            if let Some(turn_ended) = &services.turn_ended {
                turn_ended.notify_one();
            }
            if let Err(error) = crate::notebook::commit_all(
                &services.memory_dir,
                &crate::notebook::famulen(),
                "会話の中で書き留めた",
            ) {
                tracing::warn!(%error, "could not commit memory");
            }
        }
        Inbound::Control {
            request_id,
            request,
        } => answer(request_id, request, tx, services),
        Inbound::Other(value) => {
            let kind = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            tracing::debug!(kind, "claude event");
            if kind == "assistant" {
                record_assistant(&services.journal, &value);
            }
        }
    }
}

fn remember_session(store: &Store, session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    if let Err(error) = store.set_state(SESSION_ID_KEY, session_id) {
        tracing::warn!(%error, "could not save the session id");
    }
}

fn answer<S: Slack + 'static>(
    request_id: String,
    request: ControlRequest,
    tx: &mpsc::UnboundedSender<Value>,
    services: &Arc<Services<S>>,
) {
    match request {
        ControlRequest::CanUseTool {
            caller,
            tool_name,
            input,
        } => {
            let tx = tx.clone();
            let services = services.clone();
            let guard = services.in_flight.enter();
            tokio::spawn(async move {
                let _guard = guard;
                let response = permit(&request_id, caller, &tool_name, &input, &services).await;
                let _ = tx.send(response);
            });
        }
        ControlRequest::McpMessage {
            server_name,
            message,
        } if server_name == hub::SERVER_NAME => {
            let tx = tx.clone();
            let services = services.clone();
            let guard = services.in_flight.enter();
            tokio::spawn(async move {
                let _guard = guard;
                let response = services.hub.handle(&message).await;
                let _ = tx.send(protocol::mcp_response(&request_id, response));
            });
        }
        ControlRequest::McpMessage {
            server_name,
            message,
        } => match services.gateway.clone() {
            Some(gateway) => {
                let tx = tx.clone();
                let guard = services.in_flight.enter();
                let ask = running_workgroup(&services.store)
                    .and_then(|workgroup| services.permissions.ask_in(&server_name, &workgroup))
                    .unwrap_or_else(|error| {
                        tracing::warn!(%error, %server_name, "could not read the workgroup; asking");
                        Some("the workgroup could not be read".into())
                    });
                let permissions = services.permissions.clone();
                tokio::spawn(async move {
                    let _guard = guard;
                    let response = match permissions.refused_launch(&server_name, &message) {
                        Some(refused) => Some(refused),
                        None => gateway
                            .handle(crate::permissions::SECRETARY, ask, &server_name, &message)
                            .await
                            .map(|r| permissions.without_launchers(&server_name, r)),
                    };
                    let _ = tx.send(protocol::mcp_response(&request_id, response));
                });
            }
            None => {
                let _ = tx.send(protocol::control_error(
                    &request_id,
                    &format!("unknown mcp server: {server_name}"),
                ));
            }
        },
        ControlRequest::Unknown { subtype } => {
            tracing::debug!(%subtype, "unsupported control request");
            let _ = tx.send(protocol::control_error(
                &request_id,
                &format!("unsupported: {subtype}"),
            ));
        }
    }
}

/// Keep what the model wrote and which tools it called, for the log.
fn record_assistant(journal: &Journal, message: &Value) {
    let blocks = message
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !text.trim().is_empty() {
                    journal.record("text", text);
                }
            }
            Some("tool_use") => {
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let input = block
                    .get("input")
                    .map(|input| shown_input(name, input).to_string())
                    .unwrap_or_default();
                journal.record("tool", format!("{name} {input}"));
            }
            _ => {}
        }
    }
}

/// The tools whose input carries a secret on its way into the vault, and
/// the field that holds it.
const SEALED_INPUTS: &[(&str, &str)] = &[("set_secret_value", "value")];

/// A tool's input as the journal keeps it: a secret going into the vault is
/// left out.
fn shown_input(name: &str, input: &Value) -> Value {
    let tool = name.rsplit("__").next().unwrap_or(name);
    let mut shown = input.clone();
    for (sealed, field) in SEALED_INPUTS {
        if tool == *sealed
            && let Some(value) = shown.get_mut(*field)
        {
            *value = Value::String("…".into());
        }
    }
    shown
}

/// The control response for one permission request. A tool that needs the
/// owner's approval is refused now and put to the owner.
async fn permit<S: Slack>(
    request_id: &str,
    caller: crate::permissions::Caller,
    tool_name: &str,
    input: &Value,
    services: &Services<S>,
) -> Value {
    let decision = match services
        .permissions
        .decide_for(caller, tool_name, input)
        .await
    {
        Ok(decision) => decision,
        Err(error) => {
            tracing::warn!(%error, %tool_name, "could not decide; refusing");
            return protocol::deny_tool(
                request_id,
                "famulen could not check this tool's permission. Try again later.",
            );
        }
    };
    tracing::info!(%tool_name, ?decision, "permission request");
    let verdict = match &decision {
        Decision::Allow => "allowed",
        Decision::Held(_) => "held for approval",
        Decision::Forbid(_) => "forbidden",
    };
    services
        .journal
        .record("permission", format!("{tool_name}: {verdict}"));
    match decision {
        Decision::Allow => protocol::allow_tool(request_id, input),
        Decision::Forbid(message) => protocol::deny_tool(request_id, &message),
        Decision::Held(id) => protocol::deny_tool(request_id, &crate::gateway::held_message(id)),
    }
}

/// The running turn's workgroup. Every tool call comes inside a turn; without
/// one there is nothing to narrow it to.
fn running_workgroup(store: &Store) -> anyhow::Result<String> {
    Ok(store
        .running()?
        .map_or_else(|| crate::workgroups::ALL.to_owned(), |t| t.workgroup))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(resume: Option<&str>) -> Options {
        Options {
            bin: "claude".into(),
            model: Some("opus".into()),
            effort: Some("low".into()),
            workdir: "/w".into(),
            system_prompt: "be helpful".into(),
            add_dirs: vec!["/m".into()],
            resume: resume.map(Into::into),
            connections: Vec::new(),
            sealed: Vec::new(),
            writable: Vec::new(),
            plugin_dir: None,
        }
    }

    #[test]
    fn a_secret_given_to_the_vault_is_not_written_to_the_journal() {
        let journal = Journal::new(Store::open_in_memory().unwrap());
        let message = serde_json::json!({ "message": { "content": [
            { "type": "tool_use", "name": "mcp__famulen__set_secret_value",
              "input": { "name": "bank", "value": "hunter2-hunter2", "purpose": "bank" } },
            { "type": "tool_use", "name": "mcp__famulen__add_memo",
              "input": { "value": "kept as said" } },
        ] } });

        record_assistant(&journal, &message);

        let details: Vec<String> = journal
            .recent(10)
            .unwrap()
            .into_iter()
            .map(|e| e.detail)
            .collect();
        assert!(
            details.iter().all(|d| !d.contains("hunter2")),
            "{details:?}"
        );
        assert!(
            details
                .iter()
                .any(|d| d.contains("set_secret_value") && d.contains("\"bank\""))
        );
        assert!(details.iter().any(|d| d.contains("kept as said")));
    }

    fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    }

    #[tokio::test]
    async fn waiting_for_tool_calls_ends_when_the_last_one_does() {
        let in_flight = InFlight::default();
        in_flight.settled().await;
        let a = in_flight.enter();
        let b = in_flight.enter();
        let waiter = tokio::spawn({
            let in_flight = in_flight.clone();
            async move { in_flight.settled().await }
        });
        drop(a);
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished(), "one call still runs");
        drop(b);
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn every_permission_request_is_put_to_famulen() {
        let args = options(None).args();
        assert_eq!(
            value_after(&args, "--permission-prompt-tool"),
            Some("stdio")
        );
        assert_eq!(value_after(&args, "--permission-mode"), Some("manual"));
    }

    #[test]
    fn shell_commands_run_in_a_sandbox_that_cannot_read_or_write_the_sealed_places() {
        let mut options = options(None);
        options.sealed = vec!["/data/gateway".into(), "/srv/.env".into()];
        let args = options.args();
        let settings: Value =
            serde_json::from_str(value_after(&args, "--settings").unwrap()).unwrap();
        let sandbox = &settings["sandbox"];
        assert_eq!(sandbox["enabled"], true);
        assert_eq!(sandbox["failIfUnavailable"], true);
        assert_eq!(sandbox["allowUnsandboxedCommands"], false);
        assert_eq!(sandbox["autoAllowBashIfSandboxed"], false);
        let sealed = serde_json::json!(["/data/gateway", "/srv/.env"]);
        assert_eq!(sandbox["filesystem"]["denyRead"], sealed);
        assert_eq!(sandbox["filesystem"]["denyWrite"], sealed);
    }

    fn sandbox_filesystem(options: &Options) -> Value {
        let settings: Value =
            serde_json::from_str(value_after(&options.args(), "--settings").unwrap()).unwrap();
        settings["sandbox"]["filesystem"].clone()
    }

    #[test]
    fn shell_commands_may_write_in_the_places_the_owner_added() {
        let mut options = options(None);
        options.sealed = vec!["/data/gateway".into()];
        options.writable = vec!["/home/me/notes".into(), "/srv/out".into()];
        assert_eq!(
            sandbox_filesystem(&options)["allowWrite"],
            serde_json::json!(["/home/me/notes", "/srv/out"])
        );
    }

    #[test]
    fn a_sealed_place_stays_unreadable_and_unwritable_when_added_as_writable() {
        let mut options = options(None);
        options.sealed = vec!["/data/gateway".into(), "/srv/.env".into()];
        options.writable = vec![
            "/data/gateway".into(),
            "/data/gateway/vault".into(),
            "/data".into(),
            "/srv/.env".into(),
        ];
        let filesystem = sandbox_filesystem(&options);
        let sealed = serde_json::json!(["/data/gateway", "/srv/.env"]);
        assert_eq!(filesystem["denyRead"], sealed);
        assert_eq!(filesystem["denyWrite"], sealed);
        assert_eq!(filesystem["allowWrite"], serde_json::json!(["/data"]));
    }

    #[test]
    fn without_writable_places_no_write_is_allowed_beyond_the_default() {
        let mut options = options(None);
        options.sealed = vec!["/data/gateway".into()];
        assert_eq!(
            sandbox_filesystem(&options)["allowWrite"],
            serde_json::json!([])
        );
    }

    #[test]
    fn the_plugins_made_from_the_owners_skills_are_loaded() {
        let mut options = options(None);
        assert_eq!(value_after(&options.args(), "--plugin-dir"), None);
        options.plugin_dir = Some("/w/.claude/owner-plugins".into());
        assert_eq!(
            value_after(&options.args(), "--plugin-dir"),
            Some("/w/.claude/owner-plugins")
        );
        assert_eq!(
            value_after(&options.args(), "--setting-sources"),
            Some("project")
        );
    }

    #[test]
    fn without_sealed_places_no_sandbox_is_asked_for() {
        assert_eq!(value_after(&options(None).args(), "--settings"), None);
    }

    #[test]
    fn the_model_is_offered_no_tool_that_starts_an_agent() {
        let args = options(None).args();
        let denied = args
            .iter()
            .find_map(|a| a.strip_prefix("--disallowedTools="))
            .unwrap();
        assert!(denied.split(',').any(|t| t == "Task"), "{denied}");
        assert!(denied.split(',').any(|t| t == "Agent"), "{denied}");
    }

    #[test]
    fn the_host_users_settings_are_not_read() {
        assert_eq!(
            value_after(&options(None).args(), "--setting-sources"),
            Some("project")
        );
    }

    #[test]
    fn famulen_tools_are_offered_through_the_mcp_config() {
        let args = options(None).args();
        let config: Value =
            serde_json::from_str(value_after(&args, "--mcp-config").unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["famulen"]["type"], "sdk");
    }

    #[test]
    fn the_owners_connections_are_servers_famulen_answers_and_cannot_replace_famulen() {
        let mut options = options(None);
        options.connections = vec!["gmail-work".into(), "famulen".into()];
        let args = options.args();
        let config: Value =
            serde_json::from_str(value_after(&args, "--mcp-config").unwrap()).unwrap();
        assert_eq!(
            config["mcpServers"]["gmail-work"],
            serde_json::json!({ "type": "sdk", "name": "gmail-work" })
        );
        assert_eq!(config["mcpServers"]["famulen"]["name"], "famulen");
        assert_eq!(config["mcpServers"].as_object().unwrap().len(), 2);
    }

    #[test]
    fn no_other_mcp_server_reaches_the_cli() {
        let options = options(None);
        assert!(options.args().contains(&"--strict-mcp-config".to_string()));
        assert!(
            options
                .env()
                .contains(&("ENABLE_CLAUDEAI_MCP_SERVERS", "false"))
        );
    }

    #[test]
    fn extra_directories_are_added() {
        assert_eq!(value_after(&options(None).args(), "--add-dir"), Some("/m"));
    }

    #[test]
    fn a_saved_conversation_is_resumed() {
        assert!(
            options(Some("s1"))
                .args()
                .contains(&"--resume=s1".to_string())
        );
        assert!(
            !options(None)
                .args()
                .iter()
                .any(|a| a.starts_with("--resume"))
        );
    }

    #[test]
    fn the_model_and_system_prompt_are_passed() {
        let args = options(None).args();
        assert_eq!(value_after(&args, "--model"), Some("opus"));
        assert_eq!(value_after(&args, "--effort"), Some("low"));
        assert_eq!(
            value_after(&args, "--append-system-prompt"),
            Some("be helpful")
        );
    }
}
