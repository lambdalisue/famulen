//! One Claude Code process: inputs go in on stdin, and famulen answers the
//! permission and MCP requests that come back on stdout.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use chrono_tz::Tz;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::guest::{self, Guest};
use super::protocol::{self, ControlRequest, Inbound};
use crate::approvals::Approvals;
use crate::conversations::Conversations;
use crate::disclosure::{self, Asking, Gate, Reader, Taint};
use crate::hub::{self, Hub};
use crate::input::{Place, Reach};
use crate::journal::Journal;
use crate::permissions::{Bound, Decision, Permissions};
use crate::slack::clients::SlackClients;
use crate::slack::web::Slack;
use crate::store::Store;

/// From where on Claude Code compacts a conversation: it does so some way below
/// this many tokens, instead of close to the model's own window.
const COMPACT_WINDOW: &str = "160000";
/// How long a process told that nothing more comes may take to end.
const END_LIMIT: Duration = Duration::from_secs(10);

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

/// Who a process answers.
#[derive(Debug, Clone)]
pub enum Audience {
    /// The owner, in the conversation with this id.
    Owner(i64),
    /// Someone else, for one input; the process knows nothing of the owner's.
    Guest(Guest),
}

impl Audience {
    fn guest(&self) -> Option<&Guest> {
        match self {
            Self::Owner(_) => None,
            Self::Guest(guest) => Some(guest),
        }
    }
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

/// The built-in tools the secretary is offered. Every tool offered is
/// described to the model on every call, whether it is used or not, so the
/// ones for an interactive session (plans, worktrees, schedules, questions in
/// the terminal) are left out; the owner is asked through Famulen instead.
const BUILTINS: &[&str] = &[
    "Bash",
    "PowerShell",
    "Read",
    "Edit",
    "Write",
    "NotebookEdit",
    "WebFetch",
    "WebSearch",
    "Skill",
    "ToolSearch",
    "TodoWrite",
    "TaskOutput",
    "TaskStop",
    "ListMcpResourcesTool",
    "ReadMcpResourceTool",
];

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
        args.push(format!("--tools={}", BUILTINS.join(",")));
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
    /// otherwise join the tools, outside the gateway, and a long conversation
    /// would be read whole at every step.
    pub fn env(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("ENABLE_CLAUDEAI_MCP_SERVERS", "false"),
            ("CLAUDE_CODE_AUTO_COMPACT_WINDOW", COMPACT_WINDOW),
        ]
    }
}

/// What a session needs to answer Claude Code's requests.
pub struct Services<S> {
    pub hub: Hub,
    pub permissions: Permissions,
    pub approvals: Approvals<S>,
    pub journal: Journal,
    pub store: Store,
    pub conversations: Conversations,
    pub memory_dir: PathBuf,
    /// Where each turn's usage is counted, and as which model.
    pub usage: Option<(crate::models::Models, crate::models::ModelSpec)>,
    /// Told of what the session does, to show it on the console.
    pub working: Option<crate::working::Working>,
    /// Told when a turn ends, so inputs held for another workgroup go in.
    pub turn_ended: Option<Arc<tokio::sync::Notify>>,
    pub in_flight: InFlight,
    /// What answers the servers that stand for the owner's connections.
    pub gateway: Option<crate::gateway::Gateway>,
    /// Reads what is about to be said where others hear it; without one, the
    /// owner is asked.
    pub reader: Option<Arc<dyn Reader>>,
    /// The owner's time zone.
    pub tz: Tz,
    /// Where the resident process works.
    pub workdir: PathBuf,
}

impl<S: Slack + Clone> Services<S> {
    /// `owners` are the owner's IDs by workspace: approvals are also asked on
    /// Slack when it is connected; the console always gets them.
    pub fn new(
        slack: Option<&SlackClients<S>>,
        store: Store,
        permissions: Permissions,
        owners: BTreeMap<String, String>,
        memory_dir: impl Into<PathBuf>,
        tools: Arc<dyn crate::hub::OwnTools>,
        console: Option<crate::console_log::ConsoleLog>,
    ) -> Self {
        let memory_dir = memory_dir.into();
        let mut approvals =
            Approvals::new(slack.cloned(), store.clone(), permissions.clone(), owners);
        if let Some(console) = console {
            approvals = approvals.with_console(console);
        }
        Self {
            hub: Hub::new(tools),
            permissions: permissions.with_memory_dir(&memory_dir),
            approvals,
            journal: Journal::new(store.clone()),
            conversations: Conversations::new(store.clone()),
            store,
            memory_dir,
            usage: None,
            working: None,
            turn_ended: None,
            in_flight: InFlight::default(),
            gateway: None,
            reader: None,
            tz: Tz::UTC,
            workdir: PathBuf::new(),
        }
    }

    fn gate(&self) -> Gate<'_, S> {
        Gate {
            store: &self.store,
            memory_dir: &self.memory_dir,
            permissions: &self.permissions,
            approvals: &self.approvals,
            reader: self.reader.as_ref(),
            tz: self.tz,
        }
    }

    /// Mark the conversation of a turn about to begin with what the turn is
    /// given.
    /// Fails when the notebook cannot be recorded: what the turn would write
    /// there could not be marked, so it must not begin.
    pub(crate) async fn mark_begun(&self, row: &crate::store::StoredInput) -> anyhow::Result<()> {
        self.gate().begun_or_held(row.id, &row.input).await;
        self.gate().watch_notebook(row.id, &row.input).await
    }

    /// Take back what turns were marked with while comparing shows they did
    /// not need it. Only while no turn runs.
    pub(crate) async fn compare_written(&self) {
        let compared = disclosure::compare_written(&self.store, self.reader.as_deref()).await;
        if let Err(error) = compared {
            tracing::warn!(%error, "what turns wrote stays marked");
        }
    }

    /// Mark the running turn with what it read. What cannot be marked is not
    /// given to the model either.
    fn mark(&self, taints: Vec<Taint>) -> bool {
        disclosure::note(&self.store, taints)
            .inspect_err(|error| tracing::warn!(%error, "could not mark what was read"))
            .is_ok()
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

/// A running session's input side.
#[derive(Clone)]
pub struct Handle {
    tx: mpsc::UnboundedSender<Value>,
    over: Arc<tokio::sync::Notify>,
}

impl Handle {
    /// Deliver `text` as a user message; it starts a turn or joins the running one.
    pub fn send(&self, text: impl AsRef<str>) -> anyhow::Result<()> {
        self.tx
            .send(protocol::user_message(text.as_ref()))
            .map_err(|_| anyhow::anyhow!("the claude session has ended"))
    }

    /// Have the process end by itself, as it keeps the conversation then, and
    /// wait until it did. One that takes too long is stopped.
    pub async fn end(&self, task: &mut JoinHandle<anyhow::Result<()>>) {
        self.over.notify_one();
        if tokio::time::timeout(END_LIMIT, &mut *task).await.is_err() {
            tracing::warn!("claude did not end by itself; stopping it");
            task.abort();
            let _ = task.await;
        }
    }
}

/// Start Claude Code. The task finishes when the process ends.
pub fn spawn<S: Slack + Clone + 'static>(
    options: &Options,
    services: Arc<Services<S>>,
    audience: Audience,
) -> anyhow::Result<(Handle, JoinHandle<anyhow::Result<()>>)> {
    std::fs::create_dir_all(&options.workdir)?;
    let mut command = Command::new(&options.bin);
    crate::config::hide_secrets(&mut command);
    match audience.guest() {
        None => command.args(options.args()).envs(options.env()),
        Some(_) => guest::configure(&mut command, options),
    };
    let mut child = command
        .current_dir(&options.workdir)
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
    let over = Arc::new(tokio::sync::Notify::new());
    let writer = tokio::spawn(write_lines(stdin, rx, over.clone()));
    let handle = Handle {
        tx: tx.clone(),
        over,
    };

    let task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await? {
            match protocol::parse_line(&line) {
                Ok(Inbound::Init { tools, .. })
                    if audience.guest().is_some() && guest::offered_more(&tools) =>
                {
                    child.start_kill().ok();
                    anyhow::bail!("a guest's session was offered more than its answer: {tools:?}")
                }
                Ok(inbound) => on_inbound(inbound, &tx, &services, &audience),
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
    over: Arc<tokio::sync::Notify>,
) -> anyhow::Result<()> {
    while let Some(message) = tokio::select! {
        message = rx.recv() => message,
        () = over.notified() => None,
    } {
        let mut line = message.to_string();
        line.push('\n');
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
    }
    Ok(())
}

fn on_inbound<S: Slack + Clone + 'static>(
    inbound: Inbound,
    tx: &mpsc::UnboundedSender<Value>,
    services: &Arc<Services<S>>,
    audience: &Audience,
) {
    let store = &services.store;
    let conversations = &services.conversations;
    match inbound {
        // Its conversation and its tools are gone after the one turn.
        Inbound::Init { .. } if audience.guest().is_some() => {}
        Inbound::Init { session_id, tools } => {
            tracing::info!(%session_id, tools = tools.len(), "claude session ready");
            services.journal.record(
                "session",
                format!("ready: {session_id} ({} tools)", tools.len()),
            );
            remember_session(conversations, audience, &session_id);
            if let Err(error) = services.permissions.record_tools(&tools) {
                tracing::warn!(%error, "could not record the tools");
            }
        }
        // No turn ran, so the input waits as if it had not been given.
        Inbound::Result { lost: true, .. } => {
            let Audience::Owner(id) = audience else {
                return;
            };
            tracing::warn!(
                conversation = id,
                "its claude session is gone; beginning a new one"
            );
            services.journal.record(
                "session",
                format!("the session of conversation #{id} is gone; beginning a new one"),
            );
            if let Err(error) = conversations.forget_session(*id) {
                tracing::warn!(%error, "could not forget the session");
            }
            let undone = store.running_input().and_then(|row| match row {
                Some(row) if answers(services, audience) => store.undo_start(row.id),
                _ => Ok(()),
            });
            if let Err(error) = undone {
                tracing::warn!(%error, "could not have the input wait again");
            }
        }
        Inbound::Result {
            session_id,
            is_error,
            usage,
            cached,
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
            remember_session(conversations, audience, &session_id);
            if let Audience::Owner(id) = audience
                && let Err(error) = conversations.spent(*id, &usage, cached)
            {
                tracing::warn!(%error, "could not keep what the turn used");
            }
            if answers(services, audience)
                && let Err(error) =
                    disclosure::settle_notebook(&services.store, &services.memory_dir)
            {
                tracing::warn!(%error, "could not mark what the turn wrote into the notebook");
            }
            // A turn another process runs is not ended by this one's.
            if answers(services, audience)
                && let Err(error) = end_turn(services, audience)
            {
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
        } => answer(request_id, request, tx, services, audience),
        Inbound::Other(value) => {
            let kind = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            tracing::debug!(kind, "claude event");
            if let Some(working) = &services.working {
                working.follow(&value, |tool| {
                    crate::wording::under_way(
                        tool,
                        services.permissions.connection_of(tool).as_ref(),
                    )
                });
            }
            if kind == "assistant" {
                if audience.guest().is_none() {
                    services.mark(read_by_builtins(services, &value));
                }
                record_assistant(&services.journal, &value);
                if let (Audience::Owner(id), Some(tokens)) =
                    (audience, protocol::context_of(&value))
                    && let Err(error) = conversations.set_context(*id, tokens)
                {
                    tracing::warn!(%error, "could not keep how long the conversation is");
                }
            }
            if value["subtype"] == "compact_boundary" {
                services.journal.record(
                    "session",
                    format!("compacted: {}", value["compact_metadata"]),
                );
            }
        }
    }
}

fn remember_session(conversations: &Conversations, audience: &Audience, session_id: &str) {
    if let Audience::Owner(id) = audience
        && let Err(error) = conversations.set_session(*id, session_id)
    {
        tracing::warn!(%error, "could not save the session id");
    }
}

fn end_turn<S>(services: &Services<S>, audience: &Audience) -> anyhow::Result<()> {
    if let Audience::Owner(id) = audience
        && let Some(row) = services.store.running_input()?
    {
        services.conversations.ended(*id, &row)?;
    }
    services.store.end_turn()?;
    Ok(())
}

fn answer<S: Slack + Clone + 'static>(
    request_id: String,
    request: ControlRequest,
    tx: &mpsc::UnboundedSender<Value>,
    services: &Arc<Services<S>>,
    audience: &Audience,
) {
    match request {
        ControlRequest::McpMessage { message, .. }
            if message["method"] == "tools/call" && !answers(services, audience) =>
        {
            let refused = hub::failed(&message, guest::NOT_ITS_TURN);
            let _ = tx.send(protocol::mcp_response(&request_id, Some(refused)));
        }
        // A connection's call is judged where it arrives.
        ControlRequest::McpMessage {
            server_name,
            message,
        } if message["method"] == "tools/call"
            && server_name != hub::SERVER_NAME
            && beyond_answering(&services.store, &format!("mcp__{server_name}__"), &message)
                .is_some() =>
        {
            let refused = hub::failed(&message, ONLY_ANSWER);
            let _ = tx.send(protocol::mcp_response(&request_id, Some(refused)));
        }
        ControlRequest::CanUseTool {
            caller,
            tool_name,
            input,
        } => {
            let tx = tx.clone();
            let services = services.clone();
            let audience = audience.clone();
            let guard = services.in_flight.enter();
            tokio::spawn(async move {
                let _guard = guard;
                let response = permit(
                    &request_id,
                    caller,
                    &tool_name,
                    &input,
                    &services,
                    &audience,
                )
                .await;
                let _ = tx.send(response);
            });
        }
        ControlRequest::McpMessage {
            server_name,
            message,
        } if server_name == hub::SERVER_NAME => {
            let tx = tx.clone();
            let services = services.clone();
            let audience = audience.clone();
            let place = match audience.guest() {
                Some(guest) => guest.place.clone(),
                None => answering(&services.store),
            };
            let guard = services.in_flight.enter();
            tokio::spawn(async move {
                let _guard = guard;
                let hub = match audience.guest() {
                    Some(guest) => Hub::new(guest.tools(services.hub.tools().clone())),
                    None => services.hub.clone(),
                };
                let mut message = message;
                if let Some(place) = &place {
                    pin_thread(&mut message, place);
                }
                let response = hub.handle(&message).await;
                let response = match audience.guest() {
                    None => marked_own(&services, &message, response).await,
                    Some(_) => response,
                };
                let _ = tx.send(protocol::mcp_response(&request_id, response));
            });
        }
        ControlRequest::McpMessage {
            server_name,
            message,
        } => match services
            .gateway
            .clone()
            .filter(|_| audience.guest().is_none())
        {
            Some(gateway) => {
                let tx = tx.clone();
                let guard = services.in_flight.enter();
                let ask = running_workgroup(&services.store)
                    .and_then(|workgroup| services.permissions.ask_in(&server_name, &workgroup))
                    .unwrap_or_else(|error| {
                        tracing::warn!(%error, %server_name, "could not read the workgroup; asking");
                        Some("the workgroup could not be read".into())
                    })
                    .or_else(|| ask_outward_connection(services, &server_name, &message));
                let origin = services.store.call_origin().unwrap_or_else(|error| {
                    tracing::warn!(%error, "could not read what the call is made for");
                    None
                });
                let permissions = services.permissions.clone();
                let services = services.clone();
                tokio::spawn(async move {
                    let _guard = guard;
                    let asking = asks_to_disclose(&services, &server_name, &message).await;
                    let ask = match &asking {
                        Some(asking) => Some(asking.why.clone()),
                        None => ask,
                    };
                    let made_for = origin.clone();
                    let response = match permissions.refused_launch(&server_name, &message) {
                        Some(refused) => Some(refused),
                        None => gateway
                            .handle_from(
                                origin,
                                crate::permissions::SECRETARY,
                                ask,
                                &server_name,
                                &message,
                            )
                            .await
                            .map(|r| permissions.without_launchers(&server_name, r)),
                    };
                    if let Some(asking) = asking
                        && let Some(id) = held_now(&permissions, &made_for, &server_name, &message)
                    {
                        asking.held(&services.store, id);
                    }
                    let response = match response {
                        Some(response) => {
                            let clients = services.approvals.slack();
                            let taints = disclosure::of_answer(
                                clients,
                                &permissions,
                                &server_name,
                                &message,
                                &response,
                            )
                            .await;
                            Some(match services.mark(taints) {
                                true => response,
                                false => hub::failed(&message, UNMARKED),
                            })
                        }
                        None => None,
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

const UNMARKED: &str = "famulen could not keep that this was read. Try again later.";

/// What the built-in tools a model calls read in confidence. Claude Code lets
/// most of them run without asking, so they are seen only here.
fn read_by_builtins<S: Slack + Clone>(services: &Services<S>, message: &Value) -> Vec<Taint> {
    let blocks = message
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let gate = services.gate();
    let running = services.store.running_input().ok().flatten();
    let mut taints = Vec::new();
    for block in blocks.filter(|block| block["type"] == "tool_use") {
        let (tool, input) = (block["name"].as_str().unwrap_or_default(), &block["input"]);
        let (memory, work) = (&services.memory_dir, &services.workdir);
        let read = disclosure::of_builtin(memory, work, tool, input);
        taints.extend(match &running {
            Some(row) => disclosure::attached_in(read, &row.input, work, input),
            None => read,
        });
        // What was written into the notebook in confidence is read with it.
        let marked = match disclosure::read_of_notebook(memory, work, tool, input) {
            Some(Some(text)) => gate.of_marked_in(&text),
            Some(None) => gate.of_all_marked(),
            None => Ok(Vec::new()),
        };
        taints.extend(marked.unwrap_or_else(|error| {
            tracing::warn!(%error, "could not tell what the notebook was written with");
            vec![Taint::new(
                disclosure::Kind::Private,
                "手帳に書き写したもの",
            )]
        }));
    }
    taints
}

/// An own tool's answer, once what it read is marked on the turn and handed on
/// to what it leaves for later.
async fn marked_own<S: Slack + Clone>(
    services: &Services<S>,
    message: &Value,
    response: Option<Value>,
) -> Option<Value> {
    let response = response?;
    if message["method"] != "tools/call" || response["result"]["isError"] == true {
        return Some(response);
    }
    let tool = message["params"]["name"].as_str().unwrap_or_default();
    let arguments = &message["params"]["arguments"];
    let answer = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    let mut taints = disclosure::of_own(&services.memory_dir, tool, arguments, answer);
    let gate = services.gate();
    let read = match tool {
        "list_delegated" => gate.of_runs(answer),
        _ => gate.of_marked_read(tool, arguments, answer),
    };
    match read {
        Ok(read) => taints.extend(read),
        Err(error) => {
            tracing::warn!(%error, %tool, "could not tell what was read before");
            return Some(hub::failed(message, UNMARKED));
        }
    }
    if !services.mark(taints) {
        return Some(hub::failed(message, UNMARKED));
    }
    if let Err(error) = services.gate().hand_on(tool, answer).await {
        tracing::warn!(%error, %tool, "could not hand on what the turn read");
    }
    Some(response)
}

/// Why the owner is asked before a connection's tool says something to others.
async fn asks_to_disclose<S: Slack + Clone>(
    services: &Services<S>,
    server: &str,
    message: &Value,
) -> Option<Asking> {
    if message["method"] != "tools/call" {
        return None;
    }
    let field = |pointer: &str| message.pointer(pointer).and_then(Value::as_str);
    let tool = field("/params/name").unwrap_or_default();
    let arguments = &message["params"]["arguments"];
    if disclosure::says_on_slack(&services.permissions, server, tool) {
        let channel = field("/params/arguments/channel").unwrap_or_default();
        let text = arguments.to_string();
        return services
            .gate()
            .asks(server, channel, &text, arguments)
            .await;
    }
    if !disclosure::reaches_others(&services.permissions, server, tool) {
        return None;
    }
    services.gate().asks_outside(server, arguments).await
}

/// The request a connection's call was just held as, when it was.
fn held_now(
    permissions: &Permissions,
    origin: &Option<String>,
    server: &str,
    message: &Value,
) -> Option<i64> {
    let held = permissions.gateway().held_tickets().ok()?;
    let held = held.into_iter().rfind(|t| {
        t.connection == server
            && message["params"]["name"] == t.tool.as_str()
            && t.shown == message["params"]["arguments"]
            && t.origin == *origin
    });
    held.map(|t| t.id)
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
async fn permit<S: Slack + Clone>(
    request_id: &str,
    caller: crate::permissions::Caller,
    tool_name: &str,
    input: &Value,
    services: &Services<S>,
    audience: &Audience,
) -> Value {
    if answers(services, audience)
        && let Some(guest) = audience.guest()
        && tool_name == crate::approvals::asked::TOOL
    {
        return ask_owner(request_id, guest, input, services);
    }
    let refused = if !answers(services, audience) {
        Some(guest::NOT_ITS_TURN.to_owned())
    } else {
        audience
            .guest()
            .and_then(|guest| guest.refusal(tool_name, input))
            .or_else(|| beyond_answering(&services.store, tool_name, input))
    };
    if let Some(refused) = refused {
        tracing::info!(%tool_name, "refused without the notebook");
        services
            .journal
            .record("permission", format!("{tool_name}: forbidden"));
        return protocol::deny_tool(request_id, &refused);
    }
    let asking = match audience.guest() {
        None if answering(&services.store).is_none() => says(services, tool_name, input).await,
        _ => None,
    };
    let bound = Bound {
        ask: match &asking {
            Some(asking) => Some(asking.why.clone()),
            None => ask_outward(services, tool_name, input).await,
        },
        waive: answering(&services.store).is_some()
            || audience.guest().is_some_and(Guest::carries_the_owners_word),
    };
    let decision = match services
        .permissions
        .decide_bound(caller, tool_name, input, bound)
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
        // Why may tell of the owner's rules, which are not a guest's to know.
        Decision::Forbid(_) if audience.guest().is_some() => {
            protocol::deny_tool(request_id, "This may not be sent.")
        }
        Decision::Forbid(message) => protocol::deny_tool(request_id, &message),
        Decision::Held(id) => {
            if let Some(asking) = asking {
                asking.held(&services.store, id);
            }
            protocol::deny_tool(request_id, &crate::gateway::held_message(id))
        }
    }
}

/// Calls an API connection, whose own tools the gate is not told of.
const CALL_API: &str = "mcp__famulen__call_api";

/// Why the owner is asked before one of Famulen's own tools says something to
/// others.
async fn says<S: Slack + Clone>(
    services: &Services<S>,
    tool_name: &str,
    input: &Value,
) -> Option<Asking> {
    let field = |key| input.get(key).and_then(Value::as_str).unwrap_or_default();
    if tool_name == CALL_API && !field("method").eq_ignore_ascii_case("GET") {
        let server = field("connection");
        return services.gate().asks_outside(server, input).await;
    }
    if tool_name != guest::ANSWER_TOOL {
        return None;
    }
    services
        .gate()
        .asks(field("workspace"), field("channel"), field("text"), input)
        .await
}

/// Whether the running turn is for the process that serves `audience`. When
/// that cannot be read, it is not.
fn answers<S>(services: &Services<S>, audience: &Audience) -> bool {
    services.store.running_input().is_ok_and(|row| {
        let guests = row
            .as_ref()
            .is_some_and(|r| r.input.reach() == Reach::Guest);
        match audience {
            // A process of another conversation may linger; a call made between
            // turns is of none.
            Audience::Owner(id) => {
                !guests
                    // Asked of the row already read: read again, the turn
                    // may have ended in between.
                    && row.is_none_or(|r| {
                        services
                            .conversations
                            .of_input(r.id)
                            .is_ok_and(|of| of == Some(*id))
                    })
            }
            Audience::Guest(guest) => row.is_some_and(|r| guests && r.id == guest.input),
        }
    })
}

/// Put a guest's question to the owner; the call itself is never made.
fn ask_owner<S: Slack>(
    request_id: &str,
    guest: &Guest,
    input: &Value,
    services: &Services<S>,
) -> Value {
    let told = match guest.ask(input).map(|ask| ask.put(&services.permissions)) {
        Ok(Ok(told)) => told,
        Ok(Err(error)) => {
            tracing::warn!(%error, "could not ask the owner for a guest");
            "The owner could not be asked. Say in one line that you cannot answer that here."
        }
        Err(why) => why,
    };
    services
        .journal
        .record("permission", "a guest's question: put to the owner");
    protocol::deny_tool(request_id, told)
}

/// Where the running turn answers the one question the owner allowed, when it
/// is such a turn.
fn answering(store: &Store) -> Option<Place> {
    let row = store.running_input().ok()??;
    (row.input.reach() == Reach::Answering)
        .then(|| row.input.place())
        .flatten()
}

/// What the turn that answers one question may use beside the answer:
/// the notebook and the calendar, not the conversation, the logs or the files
/// of this machine.
const NOTEBOOK_READS: &[&str] = &[
    "search_notebook",
    "read_file",
    "list_kinds",
    "list_records",
    "get_record",
    "list_layers",
    "get_layer",
    "list_party_memories",
    "list_events",
];
const ONLY_ANSWER: &str = "This turn answers one question the owner allowed: it may only read the notebook with Famulen's own tools, and answer with send_slack where it was asked.";

/// Why a call goes beyond answering the one question the running turn is for.
fn beyond_answering(store: &Store, tool_name: &str, input: &Value) -> Option<String> {
    match store.running_input() {
        Ok(Some(row)) if row.input.reach() == Reach::Answering => {}
        Ok(_) => return None,
        Err(_) => return Some(guest::NOT_ITS_TURN.to_owned()),
    }
    let fits = match crate::permissions::split_tool(tool_name) {
        Some((hub::SERVER_NAME, "send_slack")) => {
            answering(store).is_some_and(|place| place.takes(input))
        }
        Some((hub::SERVER_NAME, tool)) => NOTEBOOK_READS.contains(&tool),
        Some(_) => false,
        None => tool_name == "ToolSearch",
    };
    (!fits).then(|| ONLY_ANSWER.to_owned())
}

/// An answer put into its thread at once: which input is answered is read
/// only when it is posted, and another may run by then.
fn pin_thread(message: &mut Value, place: &Place) {
    let Some(params) = message
        .get_mut("params")
        .filter(|p| p["name"] == "send_slack")
    else {
        return;
    };
    if let Some(arguments) = params.get_mut("arguments").and_then(Value::as_object_mut) {
        let named = ["thread_ts", "threadTs"]
            .iter()
            .any(|key| arguments.get(*key).is_some_and(|t| t != ""));
        if !named {
            arguments.remove("threadTs");
            arguments.insert("thread_ts".into(), place.thread().into());
        }
    }
}

/// Why the owner is asked before what is said or done outside in this turn:
/// its material is others' words, and the notebook is at hand.
const OUTWARD: &str = "this turn is about what others wrote, so nothing goes out without the owner";

/// What such a turn changes without asking, beside what only reads: it stays
/// in the notebook or goes to the owner, and has nobody act later. A tool not
/// named here asks, so that a new one does not slip through.
const INSIDE: &[&str] = &[
    "post_message",
    "remember",
    "set_working",
    "close_working",
    "link_account",
    "give_feedback",
    "refresh_calendars",
];
/// Built-in tools that reach outside, or write a file by hand, where no one
/// sees whether it has a worker or a schedule act.
const REACHING: &[&str] = &[
    "Bash",
    "PowerShell",
    "WebFetch",
    "Write",
    "Edit",
    "MultiEdit",
    "NotebookEdit",
];
/// Kinds of record nothing acts on by itself.
const INERT: &[&str] = &[
    "memos",
    "events",
    "calendars",
    "parties",
    "holdings",
    "contracts",
];

/// Whether saving a record has no one act on it later: a worker takes up a
/// delegated task, and a routine runs by itself.
fn inert(record: &Value) -> bool {
    let empty = |key: &str| {
        record.get(key).is_none_or(|v| {
            v.is_null() || v == "" || v.as_array().is_some_and(|items| items.is_empty())
        })
    };
    let fields = record.get("fields").and_then(Value::as_array);
    let fields = || {
        fields.into_iter().flatten().map(|f| {
            let part = |key: &str| f.get(key).and_then(Value::as_str).unwrap_or_default();
            let several = f
                .get("values")
                .and_then(Value::as_array)
                .is_some_and(|v| !v.is_empty());
            (
                part("name"),
                if several { "several" } else { part("value") },
            )
        })
    };
    match record.get("kind").and_then(Value::as_str) {
        Some("tasks") if empty("id") => {
            empty("sections")
                && fields().all(|(name, value)| {
                    !name.eq_ignore_ascii_case("assignee")
                        || matches!(value, "owner" | "secretary" | "")
                })
        }
        // It may be delegated already, so only what is not part of the order.
        Some("tasks") => {
            empty("notes")
                && empty("title")
                && empty("sections")
                && empty("details")
                && fields().all(|(name, _)| matches!(name, "Priority" | "Due" | "Related"))
        }
        Some(kind) => INERT.contains(&kind),
        None => false,
    }
}

fn inward(store: &Store) -> bool {
    // Asking once too often is the safe side.
    store.running_input().map_or(true, |row| {
        row.is_some_and(|r| r.input.reach() == Reach::Inward)
    })
}

fn ask_outward_connection<S>(
    services: &Services<S>,
    server: &str,
    message: &Value,
) -> Option<String> {
    if message["method"] != "tools/call" || !inward(&services.store) {
        return None;
    }
    let tool = message
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let reads = services
        .permissions
        .only_reads(server, tool)
        .unwrap_or_default();
    (!reads).then(|| OUTWARD.to_owned())
}

async fn ask_outward<S: Slack>(
    services: &Services<S>,
    tool_name: &str,
    input: &Value,
) -> Option<String> {
    if !inward(&services.store) {
        return None;
    }
    let asks = match crate::permissions::split_tool(tool_name) {
        Some((hub::SERVER_NAME, "send_slack")) => {
            let field = |key| input.get(key).and_then(Value::as_str).unwrap_or_default();
            !services
                .approvals
                .is_owners_dm(field("workspace"), field("channel"))
                .await
        }
        Some((hub::SERVER_NAME, "save_record")) => !inert(input),
        Some((hub::SERVER_NAME, tool)) => {
            !INSIDE.contains(&tool)
                && !services
                    .hub
                    .tools()
                    .list()
                    .iter()
                    .any(|t| t.name == tool && t.read_only)
        }
        Some(_) => false,
        None => REACHING.contains(&tool_name),
    };
    asks.then(|| OUTWARD.to_owned())
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
    use crate::hub::OwnTools;

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

    struct Tools;

    impl OwnTools for Tools {
        fn list(&self) -> Vec<hub::ToolDef> {
            [
                ("search_notebook", true),
                ("remember", false),
                ("send_slack", false),
                ("delegate_task", false),
                ("save_record", false),
                ("wake_at", false),
                ("list_messages", true),
            ]
            .map(|(name, read_only)| hub::ToolDef {
                name: name.into(),
                description: String::new(),
                input_schema: serde_json::json!({}),
                read_only,
                confirm: false,
            })
            .into()
        }

        fn call(
            &self,
            name: &str,
            _: Value,
        ) -> futures_util::future::BoxFuture<'static, anyhow::Result<String>> {
            let name = name.to_owned();
            Box::pin(async move { Ok(name) })
        }
    }

    /// Services in the turn of an input.
    fn in_turn(
        sender: crate::input::Sender,
        source: crate::input::Source,
    ) -> (Services<&'static crate::slack::fake::FakeSlack>, Audience) {
        let store = Store::open_in_memory().unwrap();
        let input = crate::input::Input {
            sender,
            source,
            text: "x".into(),
        };
        let id = store.enqueue(&input).unwrap().unwrap();
        let row = store.get(id).unwrap().unwrap();
        store.start_turn(id, "all").unwrap();
        let gateway = crate::permissions::test_gateway(
            famulen_gateway::store::Store::open_in_memory().unwrap(),
        );
        let permissions = Permissions::new(store.clone(), gateway);
        permissions.record_own_tools(&Tools.list()).unwrap();
        let services = Services::new(
            None,
            store,
            permissions,
            BTreeMap::new(),
            "/memory",
            Arc::new(Tools),
            None,
        );
        let conversation = services.conversations.settle(&row, |_| None).unwrap().id;
        let audience = Guest::of(&row).map_or(Audience::Owner(conversation), Audience::Guest);
        (services, audience)
    }

    fn slack(others_before: bool) -> crate::input::Source {
        crate::input::Source::Slack {
            workspace: "w1".into(),
            channel: "C1".into(),
            ts: "2.0".into(),
            thread_ts: None,
            user: "U2".into(),
            unmentioned: true,
            others_before,
            attached: Vec::new(),
        }
    }

    async fn allowed(
        turn: &(Services<&'static crate::slack::fake::FakeSlack>, Audience),
        tool: &str,
        input: Value,
    ) -> bool {
        let caller = crate::permissions::Caller::Secretary;
        let response = permit("r1", caller, tool, &input, &turn.0, &turn.1).await;
        response.to_string().contains(r#""behavior":"allow""#)
    }

    const SEARCH: &str = "mcp__famulen__search_notebook";
    const REMEMBER: &str = "mcp__famulen__remember";
    const SEND: &str = "mcp__famulen__send_slack";
    const DELEGATE: &str = "mcp__famulen__delegate_task";

    const SAVE: &str = "mcp__famulen__save_record";

    fn record(kind: &str, assignee: &str) -> Value {
        serde_json::json!({ "kind": kind, "title": "t",
            "fields": [{ "name": "Assignee", "value": assignee }] })
    }

    #[tokio::test]
    async fn a_turn_is_not_begun_while_the_notebook_cannot_be_recorded() {
        let (services, _) = in_turn(crate::input::Sender::Owner, slack(false));
        let row = services.store.running_input().unwrap().unwrap();
        assert!(services.mark_begun(&row).await.is_err());
    }

    #[test]
    fn an_answer_is_put_into_its_thread_before_another_turn_can_run() {
        let place = in_turn(crate::input::Sender::Other, slack(false))
            .0
            .store
            .running_input()
            .unwrap()
            .and_then(|row| row.input.place())
            .unwrap();
        let mut message =
            serde_json::json!({ "params": { "name": "send_slack", "arguments": to("C1") } });
        pin_thread(&mut message, &place);
        assert_eq!(message["params"]["arguments"]["thread_ts"], "2.0");
    }

    #[tokio::test]
    async fn the_owner_is_asked_of_a_guests_answer_unless_they_decided_it_already() {
        use crate::input::{Sender, Source};
        let confirm = |turn: &(Services<&'static crate::slack::fake::FakeSlack>, Audience)| {
            let to = Some(crate::permissions::Permission::Confirm);
            let permissions = &turn.0.permissions;
            permissions
                .set_scoped("owner", "conn:famulen", "send_slack", to)
                .unwrap();
        };
        let guest = in_turn(Sender::Other, slack(false));
        confirm(&guest);
        assert!(!allowed(&guest, SEND, to("C1")).await);
        let held = guest.0.permissions.gateway().held_tickets().unwrap();
        assert_eq!(held.len(), 1);

        let answering = in_turn(
            Sender::Owner,
            Source::Asked {
                request: 1,
                workspace: "w1".into(),
                channel: "C1".into(),
                ts: "2.0".into(),
                thread_ts: None,
                user: "U2".into(),
                granted: true,
            },
        );
        confirm(&answering);
        assert!(allowed(&answering, SEND, to("C1")).await);
    }

    #[test]
    fn a_turn_is_ended_only_by_the_process_that_answers_it() {
        let (services, guest) = in_turn(crate::input::Sender::Other, slack(false));
        let services = Arc::new(services);
        let (tx, _rx) = mpsc::unbounded_channel();
        let ended = || Inbound::Result {
            session_id: "s".into(),
            is_error: false,
            text: String::new(),
            usage: Default::default(),
            cached: Default::default(),
            lost: false,
        };
        on_inbound(ended(), &tx, &services, &Audience::Owner(1));
        assert!(services.store.running().unwrap().is_some());
        on_inbound(ended(), &tx, &services, &guest);
        assert!(services.store.running().unwrap().is_none());
        assert_eq!(services.store.running().unwrap(), None);
    }

    #[test]
    fn a_call_is_of_its_conversation_while_the_turn_ends_beside_it() {
        let (services, audience) = in_turn(crate::input::Sender::Owner, slack(false));
        let store = services.store.clone();
        let id = store.running().unwrap().unwrap().input;
        let done = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                while !done.load(std::sync::atomic::Ordering::Relaxed) {
                    store.interrupt_turn().unwrap();
                    store.start_turn(id, "all").unwrap();
                }
            });
            let refused = (0..20_000)
                .filter(|_| !answers(&services, &audience))
                .count();
            done.store(true, std::sync::atomic::Ordering::Relaxed);
            assert_eq!(refused, 0);
        });
    }

    #[test]
    fn a_turn_is_kept_to_the_conversation_it_is_of() {
        let (services, audience) = in_turn(crate::input::Sender::Owner, slack(false));
        let services = Arc::new(services);
        let Audience::Owner(id) = audience else {
            panic!("not the owner's")
        };
        assert!(answers(&services, &Audience::Owner(id)));
        assert!(
            !answers(&services, &Audience::Owner(id + 1)),
            "a process of another conversation"
        );
        let (tx, _rx) = mpsc::unbounded_channel();
        let result = Inbound::Result {
            session_id: "s".into(),
            is_error: false,
            text: String::new(),
            usage: crate::models::Usage {
                input_tokens: Some(100),
                output_tokens: Some(7),
                calls: 1,
            },
            cached: crate::conversations::Cached {
                read: 90,
                written: 8,
            },
            lost: false,
        };
        on_inbound(result, &tx, &services, &Audience::Owner(id));
        let kept = services.conversations.get(id).unwrap().unwrap();
        assert_eq!(kept.session.as_deref(), Some("s"));
        assert_eq!(
            services.conversations.spent_by(id).unwrap(),
            crate::conversations::Spent {
                turns: 1,
                input_tokens: 100,
                cached_tokens: 90,
                cache_write_tokens: 8,
                output_tokens: 7,
            }
        );
        assert!(services.store.running().unwrap().is_none());
    }

    #[test]
    fn a_session_that_is_gone_is_forgotten_and_its_input_waits_again() {
        let (services, audience) = in_turn(crate::input::Sender::Owner, slack(false));
        let services = Arc::new(services);
        let Audience::Owner(id) = audience else {
            panic!("not the owner's")
        };
        services.conversations.set_session(id, "gone").unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"session_id":"gone","errors":["No conversation found with session ID: gone"]}"#;
        on_inbound(
            protocol::parse_line(line).unwrap(),
            &tx,
            &services,
            &Audience::Owner(id),
        );
        assert_eq!(
            services.conversations.get(id).unwrap().unwrap().session,
            None
        );
        let again = services.store.next_pending().unwrap().unwrap();
        assert!(!again.interrupted, "no turn was cut off");
    }

    #[test]
    fn a_long_conversation_is_compacted_early() {
        assert!(
            options(None)
                .env()
                .contains(&("CLAUDE_CODE_AUTO_COMPACT_WINDOW", COMPACT_WINDOW))
        );
    }

    fn may_send(turn: &(Services<&'static crate::slack::fake::FakeSlack>, Audience)) {
        let allow = Some(crate::permissions::Permission::Allow);
        turn.0
            .permissions
            .set_scoped("owner", "conn:famulen", "send_slack", allow)
            .unwrap();
    }

    fn to(channel: &str) -> Value {
        serde_json::json!({ "workspace": "w1", "channel": channel, "text": "hi" })
    }

    #[tokio::test]
    async fn the_owner_uses_the_notebook_as_ever_and_a_guest_only_answers() {
        use crate::input::Sender;
        let owner = in_turn(Sender::Owner, slack(false));
        for tool in [SEARCH, REMEMBER, "Read"] {
            assert!(allowed(&owner, tool, Value::Null).await, "{tool}");
        }
        for guest in [
            in_turn(Sender::Other, slack(false)),
            in_turn(Sender::Owner, slack(true)),
        ] {
            for tool in [SEARCH, REMEMBER, DELEGATE, "Read", "Bash"] {
                assert!(!allowed(&guest, tool, Value::Null).await, "{tool}");
            }
            may_send(&guest);
            assert!(allowed(&guest, SEND, to("C1")).await);
            assert!(!allowed(&guest, SEND, to("C2")).await);
        }
    }

    #[tokio::test]
    async fn what_a_tool_answered_in_confidence_is_asked_about_before_it_is_sent() {
        struct Nothing;
        impl Reader for Nothing {
            fn read<'a>(
                &'a self,
                _: &'a disclosure::Saying,
            ) -> futures_util::future::BoxFuture<'a, anyhow::Result<disclosure::Reading>>
            {
                Box::pin(async { Ok(Default::default()) })
            }
        }
        let clean = || {
            let mut source = slack(false);
            if let crate::input::Source::Slack { unmentioned, .. } = &mut source {
                *unmentioned = false;
            }
            let mut turn = in_turn(crate::input::Sender::Owner, source);
            turn.0.reader = Some(Arc::new(Nothing));
            let slack = crate::slack::fake::FakeSlack::default();
            let public = crate::slack::web::Room {
                name: "general".into(),
                private: Some(false),
                ..Default::default()
            };
            slack.rooms.lock().unwrap().push(("C1".into(), public));
            let clients =
                SlackClients::new([("w1".to_owned(), &*Box::leak(Box::new(slack)))], "w1");
            turn.0.approvals = Approvals::new(
                Some(clients.unwrap()),
                turn.0.store.clone(),
                turn.0.permissions.clone(),
                BTreeMap::new(),
            );
            turn
        };
        let turn = clean();
        may_send(&turn);
        assert!(allowed(&turn, SEND, to("C1")).await);

        let call = |name: &str| serde_json::json!({ "id": 1, "method": "tools/call", "params": { "name": name, "arguments": { "kind": "parties" } } });
        let answer = Some(
            serde_json::json!({ "result": { "content": [{ "type": "text", "text": "{\"records\":[{}]}" }] } }),
        );
        let read = marked_own(&turn.0, &call("list_records"), answer.clone()).await;
        assert_eq!(read, answer, "the model is given it all the same");
        assert!(!allowed(&turn, SEND, to("C1")).await);
        let held = turn.0.permissions.gateway().held_tickets().unwrap();
        assert!(disclosure::reason_of(&held[0].reason).is_some());

        let turn = clean();
        may_send(&turn);
        let used = serde_json::json!({ "message": { "content": [
            { "type": "tool_use", "name": "Read", "input": { "file_path": "/memory" } },
        ] } });
        assert!(turn.0.mark(read_by_builtins(&turn.0, &used)));
        assert!(!allowed(&turn, SEND, to("C1")).await);
    }

    #[tokio::test]
    async fn the_resident_session_may_use_nothing_while_a_guest_is_answered() {
        let (services, _) = in_turn(crate::input::Sender::Other, slack(false));
        let turn = (services, Audience::Owner(1));
        for tool in [SEARCH, "Read", SEND] {
            assert!(!allowed(&turn, tool, to("C1")).await, "{tool}");
        }
    }

    #[tokio::test]
    async fn a_guests_question_is_put_to_the_owner_instead_of_made() {
        let guest = in_turn(crate::input::Sender::Other, slack(false));
        let ask = serde_json::json!({ "question": "Aさんの給与" });
        assert!(!allowed(&guest, crate::approvals::asked::TOOL, ask).await);
        let held = guest.0.permissions.gateway().held_tickets().unwrap();
        assert_eq!(held[0].tool, "ask_owner");
    }

    #[tokio::test]
    async fn a_question_the_owner_allowed_is_only_read_for_and_answered_there() {
        let turn = in_turn(
            crate::input::Sender::Owner,
            crate::input::Source::Asked {
                request: 1,
                workspace: "w1".into(),
                channel: "C1".into(),
                ts: "2.0".into(),
                thread_ts: None,
                user: "U2".into(),
                granted: true,
            },
        );
        for (tool, input, expected) in [
            (SEARCH, Value::Null, true),
            ("Read", Value::Null, false),
            ("mcp__famulen__list_messages", Value::Null, false),
            (SEND, to("C1"), true),
            (SEND, to("C2"), false),
            (REMEMBER, Value::Null, false),
            (DELEGATE, Value::Null, false),
            ("Bash", Value::Null, false),
            ("WebFetch", Value::Null, false),
            ("mcp__gmail__send_message", Value::Null, false),
        ] {
            assert_eq!(allowed(&turn, tool, input).await, expected, "{tool}");
        }
    }

    #[tokio::test]
    async fn what_the_lookout_passed_on_is_judged_with_the_notebook_and_told_only_inside() {
        let turn = in_turn(
            crate::input::Sender::System,
            crate::input::Source::Lookout {
                item: 1,
                source: "1".into(),
            },
        );
        for tool in [SEARCH, REMEMBER, "Read"] {
            assert!(allowed(&turn, tool, Value::Null).await, "{tool}");
        }
        may_send(&turn);
        assert!(!allowed(&turn, SEND, to("C1")).await);
        for (tool, input) in [
            (DELEGATE, Value::Null),
            ("mcp__famulen__wake_at", Value::Null),
            ("WebFetch", Value::Null),
            ("Write", serde_json::json!({ "file_path": "/memory/x.md" })),
            (SAVE, record("routines", "secretary")),
            (SAVE, record("tasks", "worker")),
        ] {
            assert!(!allowed(&turn, tool, input).await, "{tool}");
        }
        let held = turn.0.permissions.gateway().held_tickets().unwrap();
        assert_eq!(held.len(), 7, "the owner is asked of each");
        assert!(allowed(&turn, SAVE, record("tasks", "secretary")).await);
        assert!(allowed(&turn, SAVE, record("memos", "")).await);
        let mut changed = record("tasks", "secretary");
        changed["id"] = "task-7w1hbn".into();
        changed["notes"] = "別の依頼".into();
        assert!(!allowed(&turn, SAVE, changed).await);

        let message = |method: &str| serde_json::json!({ "method": method, "params": { "name": "send_message" } });
        assert!(ask_outward_connection(&turn.0, "gmail", &message("tools/call")).is_some());
        assert!(ask_outward_connection(&turn.0, "gmail", &message("tools/list")).is_none());
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
    fn only_the_builtins_the_secretary_uses_are_offered() {
        let args = options(None).args();
        let offered: Vec<&str> = args
            .iter()
            .find_map(|a| a.strip_prefix("--tools="))
            .unwrap()
            .split(',')
            .collect();
        for tool in [
            "Bash",
            "Read",
            "Edit",
            "Write",
            "WebFetch",
            "Skill",
            "ToolSearch",
        ] {
            assert!(offered.contains(&tool), "{tool} in {offered:?}");
        }
        for tool in [
            "EnterPlanMode",
            "AskUserQuestion",
            "CronCreate",
            "Monitor",
            "Agent",
        ] {
            assert!(!offered.contains(&tool), "{tool} in {offered:?}");
        }
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
