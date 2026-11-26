//! The one door between the program using the gateway and the outside. Every
//! connection is served to a caller's model CLI as an MCP server of its own
//! that the gateway answers, so the CLI never reaches a service by itself.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::connections::{
    ApiRequest, Connection, ConnectionTool, Connections, LOGIN_AGAIN, api_call,
};
use crate::mcp::{McpClient, McpError, ToolResult};

mod accounts;
mod apps;
mod bundled;
mod commands;
mod defined;
mod judge;
mod keeping;
mod keyless;
mod logins;
mod model_keys;
mod public;
mod push;
mod receive;
mod redefining;
mod secrets;
mod vault;
pub use commands::{Choice, Question};
pub use defined::*;
pub use judge::*;
pub use keeping::{DELETE_SECRET, DESCRIBE_SECRET, Holder, PUT_SECRET, VAULT};
pub use model_keys::model_key_name;
pub use public::{PublicPage, Wanted, is_local_name, is_public};
pub use push::{Pushed, Subscription};
pub use receive::{Arrival, Destination, Receipt};
pub use redefining::{CHANGE_DEFINITION, DEFINITIONS, DefinitionChange};
pub use vault::*;

/// The HTTP methods an API connection offers, each as `http_<method>`.
const API_METHODS: &[&str] = &["get", "post", "put", "patch", "delete"];

#[derive(Clone)]
pub struct Gateway {
    store: crate::store::Store,
    connections: Connections,
    http: reqwest::Client,
    /// One live client per MCP connection, opened on first use.
    clients: Arc<Mutex<HashMap<String, Arc<McpClient>>>>,
    /// Where secrets are kept; none until the owner gives a vault key.
    vault: Option<Vault>,
    /// Connection ids the program using the gateway keeps for itself.
    reserved: Vec<String>,
    /// The service definitions connections are made from.
    definitions: Arc<std::sync::RwLock<crate::definitions::Definitions>>,
    /// Where they were read from, to read them again after a change.
    definitions_from: Arc<redefining::Sources>,
    /// Told the id of every definition as a change to it goes in.
    redefined: tokio::sync::broadcast::Sender<String>,
    /// Where an app's values are read from: the process environment.
    env: Env,
    destinations: receive::Destinations,
    running: receive::Running,
    /// The OpenAPI descriptions read, by where they were read from.
    openapi: Arc<std::sync::RwLock<HashMap<String, Arc<Vec<crate::openapi::Operation>>>>>,
    endpoints: receive::Endpoints,
    /// Whether the owner is at the machine.
    presence: PresenceNow,
    /// Held while a connection is added that must not be there twice.
    adding: Arc<std::sync::Mutex<()>>,
    /// The answers that finished a login, with the connection each logged
    /// in. Held while one is being finished, so two of the same wait.
    finished: Arc<Mutex<HashMap<String, String>>>,
    /// Held, by connection, while a token is traded for, so no value is
    /// handed over twice.
    exchanging: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    /// The keys issuers publish, with when they were fetched.
    jwks: Arc<std::sync::RwLock<HashMap<String, (i64, jsonwebtoken::jwk::JwkSet)>>>,
    /// Told of every ticket as it is held.
    held: tokio::sync::broadcast::Sender<Ticket>,
    /// Weighs what the auto permission leaves to a model; set by the program
    /// using the gateway.
    assessor: Arc<std::sync::RwLock<Option<Arc<dyn Assessor>>>>,
    public: public::Public,
    push: push::Push,
}

/// Tells whether the owner is at the machine.
type PresenceNow = Arc<dyn Fn() -> crate::presence::Presence + Send + Sync>;

/// Reads one environment variable.
type Env = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

impl Gateway {
    pub fn new(
        store: crate::store::Store,
        connections: Connections,
        http: reqwest::Client,
        vault: Option<Vault>,
    ) -> Self {
        Self {
            store,
            connections,
            http,
            clients: Arc::default(),
            vault,
            reserved: Vec::new(),
            definitions: Arc::new(std::sync::RwLock::new(
                crate::definitions::Definitions::load(None, &[]),
            )),
            definitions_from: Arc::default(),
            redefined: tokio::sync::broadcast::channel(16).0,
            env: Arc::new(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty())),
            destinations: Default::default(),
            running: Default::default(),
            openapi: Arc::default(),
            finished: Arc::default(),
            exchanging: Arc::default(),
            adding: Arc::default(),
            presence: Arc::new(crate::presence::now),
            endpoints: Default::default(),
            jwks: Arc::default(),
            held: tokio::sync::broadcast::channel(64).0,
            assessor: Arc::default(),
            public: Default::default(),
            push: Default::default(),
        }
    }

    /// Reach the services' receiving ends at `endpoints` instead.
    #[cfg(test)]
    fn with_endpoints(mut self, endpoints: receive::Endpoints) -> Self {
        self.endpoints = endpoints;
        self
    }

    /// Use these service definitions: the bundled ones, `user`'s, and the
    /// files in `dir`, a later one winning.
    pub fn with_definitions(
        mut self,
        dir: Option<&std::path::Path>,
        user: &[(String, String)],
    ) -> Self {
        self.definitions_from = Arc::new(redefining::Sources {
            dir: dir.map(std::path::Path::to_path_buf),
            user: user.to_vec(),
        });
        self.read_definitions();
        self
    }

    /// Read app values from `env` instead of the process environment.
    #[cfg(test)]
    pub(crate) fn with_env(
        mut self,
        env: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.env = Arc::new(env);
        self
    }

    /// Tell whether the owner is at the machine with `presence`.
    #[cfg(test)]
    pub(crate) fn with_presence(
        mut self,
        presence: impl Fn() -> crate::presence::Presence + Send + Sync + 'static,
    ) -> Self {
        self.presence = Arc::new(presence);
        self
    }

    /// The MCP client config of a connection, from its definition when it
    /// is made from one.
    fn server_config(&self, connection: &Connection) -> Option<crate::mcp::ServerConfig> {
        match &connection.transport {
            crate::connections::Transport::Defined { .. } => self.defined_server_config(connection),
            _ => logins::server_config(connection),
        }
    }

    /// Keep `ids` from being given to new connections: the program using the
    /// gateway names its own tools with them.
    pub fn with_reserved(mut self, ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.reserved = ids.into_iter().map(Into::into).collect();
        self
    }

    /// The gateway's database, shared with what reads it beside the gateway.
    pub fn store(&self) -> &crate::store::Store {
        &self.store
    }

    /// The connections with their secrets opened from, and kept into, the vault.
    fn sealed(&self) -> secrets::Sealed {
        secrets::Sealed {
            connections: self.connections.clone(),
            vault: self.vault.clone(),
        }
    }

    /// Move the secrets connections hold from when there was no vault key
    /// into the vault; how many connections were moved.
    pub fn seal_existing(&self) -> anyhow::Result<usize> {
        self.sealed().seal_all()
    }

    /// The vault, to put secrets in and take them out; none without a key.
    pub fn vault(&self) -> Option<&Vault> {
        self.vault.as_ref()
    }

    /// The connections served to a model CLI, by id: each is a server the
    /// gateway answers through [`Self::handle`].
    pub fn served(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.connections.list()?.into_iter().map(|c| c.id).collect())
    }

    /// Answer one JSON-RPC message `actor`'s model CLI sent to `server`; a
    /// tool call is judged, with `ask` saying why to ask even when its permission
    /// allows it. A notification gets `None`.
    pub async fn handle(
        &self,
        actor: &str,
        ask: Option<String>,
        server: &str,
        message: &Value,
    ) -> Option<Value> {
        let by = Call {
            actor: actor.to_owned(),
            ask,
            ..Call::default()
        };
        self.serve(by, server, message).await
    }

    /// [`Self::handle`] for a call made for `origin`, which its ticket keeps.
    pub async fn handle_from(
        &self,
        origin: Option<String>,
        actor: &str,
        ask: Option<String>,
        server: &str,
        message: &Value,
    ) -> Option<Value> {
        let by = Call {
            actor: actor.to_owned(),
            ask,
            origin,
            ..Call::default()
        };
        self.serve(by, server, message).await
    }

    /// [`Self::handle`] for an `actor` who cannot wait for the owner's answer:
    /// a call that would be held is refused instead.
    pub async fn handle_unasked(
        &self,
        actor: &str,
        ask: Option<String>,
        server: &str,
        message: &Value,
    ) -> Option<Value> {
        let by = Call {
            actor: actor.to_owned(),
            ask,
            never_ask: true,
            ..Call::default()
        };
        self.serve(by, server, message).await
    }

    /// Answer one JSON-RPC message; its tool calls are judged as `by` says.
    async fn serve(&self, by: Call, server: &str, message: &Value) -> Option<Value> {
        let id = message.get("id").cloned()?;
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match self.sealed().get(server) {
            Ok(Some(connection)) => self.answer(by, &connection, method, &params).await,
            Ok(None) => {
                Err(json!({ "code": -32602, "message": format!("no connection {server}") }))
            }
            Err(error) => Err(json!({ "code": -32603, "message": error.to_string() })),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        })
    }

    async fn answer(
        &self,
        by: Call,
        connection: &Connection,
        method: &str,
        params: &Value,
    ) -> Result<Value, Value> {
        let failed = |error: anyhow::Error| json!({ "code": -32603, "message": error.to_string() });
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": params.get("protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": connection.id, "version": env!("CARGO_PKG_VERSION") },
            })),
            "ping" => Ok(json!({})),
            "tools/list" => self
                .list_tools(connection)
                .await
                .map(|tools| json!({ "tools": tools }))
                .map_err(failed),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                let call = Call {
                    connection: connection.id.clone(),
                    tool: name.to_owned(),
                    input: arguments,
                    ..by
                };
                Ok(match self.call(&call).await {
                    Ok(Outcome::Done(result)) => result_value(&result),
                    Ok(Outcome::Held(ticket)) => json!({
                        "content": [{ "type": "text", "text": held_message(ticket.id) }],
                        "isError": true,
                        "_meta": { NOT_MADE: true },
                    }),
                    Err(error) => {
                        let mut value = json!({
                            "content": [{ "type": "text", "text": error.to_string() }],
                            "isError": true,
                        });
                        if error.is::<Refused>() {
                            value["_meta"] = json!({ NOT_MADE: true });
                        }
                        value
                    }
                })
            }
            _ => Err(json!({ "code": -32601, "message": format!("method not found: {method}") })),
        }
    }

    /// The tools of connection `id`, as `tools/list` offers them.
    pub async fn tools_of(&self, id: impl AsRef<str>) -> anyhow::Result<Vec<Value>> {
        let id = id.as_ref();
        let connection = self.sealed().require(id)?;
        self.list_tools(&connection).await
    }

    /// A connection's tools as `tools/list` offers them, kept as they are
    /// read for the owner to see and for judging their calls.
    async fn list_tools(&self, connection: &Connection) -> anyhow::Result<Vec<Value>> {
        let def = match self.definition_of(connection) {
            Some(Ok(def)) => Some(def),
            Some(Err(why)) => {
                self.connections
                    .set_status(&connection.id, format!("つながらない（{why}）"))?;
                anyhow::bail!("{why}");
            }
            None => None,
        };
        let tools: Vec<Value> = match (self.server_config(connection), &def) {
            (None, Some(def)) if def.transport.http.is_some() => {
                self.defined_tools(connection, def).await?
            }
            (None, Some(def)) if def.transport.command.is_some() => Self::command_tools(def),
            (Some(_), _) => {
                let tools = self
                    .with_client(
                        connection,
                        |client| async move { client.list_tools().await },
                    )
                    .await?;
                tools
                    .into_iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description.unwrap_or_default(),
                            "inputSchema": t.input_schema,
                            "annotations": { "readOnlyHint": t.read_only, "destructiveHint": t.always_ask },
                        })
                    })
                    .collect()
            }
            (None, _) => api_tools(&connection.name),
        };
        let kept = listed(&tools);
        self.connections.record_tools(&connection.id, &kept)?;
        if let Some(def) = &def {
            let defaults: Vec<(String, crate::permissions::Permission)> = kept
                .iter()
                .filter_map(|t| Some((t.name.clone(), (*def.permissions.get(&t.name)?).into())))
                .collect();
            crate::permissions::Permissions::new(self.store.clone())
                .set_defaults(&connection.id, &defaults)?;
        }
        Ok(tools)
    }

    /// Call one of a connection's tools for `call.actor`: judged, made when
    /// allowed, and logged with how it went. A refusal is an error with its
    /// reason.
    pub async fn call(&self, call: &Call) -> anyhow::Result<Outcome<ToolResult>> {
        let connection = self.sealed().require(&call.connection)?;
        let reason = match self.judge_assessed(call).await? {
            Verdict::Allow { reason } => reason,
            Verdict::Hold(ticket) => return Ok(Outcome::Held(ticket)),
            Verdict::Deny { reason } => return Err(Refused(reason).into()),
        };
        let started = std::time::Instant::now();
        let result = self
            .call_unjudged(&connection, &call.tool, call.input.clone(), call.whole)
            .await;
        let (outcome, answer) = match &result {
            Ok(r) if r.is_error => ("error".to_owned(), r.text.as_str()),
            Ok(r) => ("ok".to_owned(), r.text.as_str()),
            Err(error) => (format!("failed: {error:#}"), ""),
        };
        self.log().write(&crate::log::LogEntry {
            actor: call.actor.clone(),
            connection: call.connection.clone(),
            tool: call.tool.clone(),
            decision: "allow".into(),
            reason,
            secret: connection
                .secret
                .in_vault
                .iter()
                .map(|field| secrets::vault_name(&connection.id, field))
                .collect::<Vec<_>>()
                .join(","),
            outcome,
            digest: format!(
                "in {}; out {}",
                crate::log::digest(call.input.to_string().as_bytes()),
                crate::log::digest(answer.as_bytes())
            ),
            ms: started.elapsed().as_millis() as i64,
            ..Default::default()
        })?;
        result.map(Outcome::Done)
    }

    /// Call a tool as the owner: for what the owner set up to run on their
    /// behalf. A forbidden tool is still refused.
    pub async fn call_as_owner(
        &self,
        connection: &Connection,
        tool: &str,
        arguments: Value,
    ) -> anyhow::Result<ToolResult> {
        match self
            .call(&Call::new(OWNER, &connection.id, tool, arguments))
            .await?
        {
            Outcome::Done(result) => Ok(result),
            Outcome::Held(ticket) => anyhow::bail!("held as #{}", ticket.id),
        }
    }

    async fn call_unjudged(
        &self,
        connection: &Connection,
        tool: &str,
        arguments: Value,
        whole: bool,
    ) -> anyhow::Result<ToolResult> {
        let connection = &self
            .sealed()
            .get(&connection.id)?
            .unwrap_or_else(|| connection.clone());
        match self.definition_of(connection) {
            Some(Err(why)) => anyhow::bail!("{why}"),
            Some(Ok(def)) if def.transport.command.is_some() => {
                return self
                    .call_command(connection, &def, tool, arguments, whole)
                    .await;
            }
            Some(Ok(def)) if def.transport.http.is_some() => {
                return self.call_defined(connection, &def, tool, arguments).await;
            }
            _ => {}
        }
        if self.server_config(connection).is_some() {
            let tool = tool.to_owned();
            return self
                .with_client(connection, |client| {
                    let (tool, arguments) = (tool.clone(), arguments.clone());
                    async move { client.call_tool(tool, arguments).await }
                })
                .await;
        }
        let method = api_method(tool)
            .ok_or_else(|| anyhow::anyhow!("{} has no tool {tool}", connection.id))?;
        let request = ApiRequest {
            method: method.to_owned(),
            path: arguments
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            query: arguments.get("query").cloned(),
            body: arguments.get("body").cloned(),
        };
        let (status, text) = api_call(&self.http, connection, &request).await?;
        Ok(text_result(text, status >= 400))
    }

    /// Keep a connection's API key: in the vault when there is one, and
    /// never again in the connection itself.
    pub fn set_api_key(
        &self,
        conn: &str,
        key: impl AsRef<str>,
        header: impl AsRef<str>,
    ) -> anyhow::Result<()> {
        let sealed = self.sealed();
        let connection = sealed.require(conn)?;
        let mut secret = connection.secret.clone();
        secret.api_key = Some(key.as_ref().trim().to_owned()).filter(|k| !k.is_empty());
        secret.api_key_header = Some(header.as_ref().trim().to_owned()).filter(|h| !h.is_empty());
        sealed.set_secret(conn, &secret)?;
        self.log_change_by(OWNER, conn, "connection.key", "")
    }

    /// Keep the variables a local MCP server starts with: in the vault when
    /// there is one, since most of them are keys.
    pub fn set_environment(
        &self,
        conn: &str,
        env: std::collections::BTreeMap<String, String>,
    ) -> anyhow::Result<()> {
        let sealed = self.sealed();
        let connection = sealed.require(conn)?;
        let mut secret = connection.secret.clone();
        secret.account = env;
        sealed.set_secret(conn, &secret)?;
        self.log_change_by(OWNER, conn, "connection.env", "")
    }

    /// Run `f` on the connection's live client, opening one when there is
    /// none. A client that fails is dropped; one that lost its server is
    /// opened again once, so a server that restarted or a token that was
    /// refreshed is picked up. What the server answered is not sent again.
    async fn with_client<T, F, Fut>(&self, connection: &Connection, f: F) -> anyhow::Result<T>
    where
        F: Fn(Arc<McpClient>) -> Fut,
        Fut: std::future::Future<Output = Result<T, McpError>>,
    {
        for attempt in 0..2 {
            let client = self.client(connection).await?;
            match f(client).await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    self.clients.lock().await.remove(&connection.id);
                    if attempt > 0 || !error.is_lost() {
                        return Err(error.into());
                    }
                    tracing::debug!(connection = %connection.id, %error, "opening the client again");
                }
            }
        }
        unreachable!("the second attempt returns")
    }

    async fn client(&self, connection: &Connection) -> anyhow::Result<Arc<McpClient>> {
        let mut clients = self.clients.lock().await;
        if let Some(client) = clients.get(&connection.id) {
            return Ok(client.clone());
        }
        let config = self
            .server_config(connection)
            .ok_or_else(|| anyhow::anyhow!("{} is not an MCP connection", connection.id))?;
        let client = match McpClient::connect(&config, connection.secret.bearer.as_deref()).await {
            Ok(client) => Arc::new(client),
            Err(McpError::Unauthorized { .. }) => {
                self.connections.set_status(&connection.id, LOGIN_AGAIN)?;
                anyhow::bail!("{} needs a login", connection.id);
            }
            Err(error) => return Err(error.into()),
        };
        clients.insert(connection.id.clone(), client.clone());
        Ok(client)
    }
}

/// The `http_<method>` tools of an API that describes none of its own.
fn api_tools(name: &str) -> Vec<Value> {
    API_METHODS
        .iter()
        .map(|m| {
            json!({
                "name": format!("http_{m}"),
                "description": format!("{} request to {name}", m.to_ascii_uppercase()),
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "The path under the API's base URL" },
                        "query": { "type": "object", "description": "Query parameters" },
                        "body": { "description": "The JSON body" },
                    },
                    "required": ["path"],
                },
                "annotations": { "readOnlyHint": *m == "get", "destructiveHint": *m == "delete" },
            })
        })
        .collect()
}

/// The method an `http_<method>` tool sends.
fn api_method(tool: &str) -> Option<&str> {
    tool.strip_prefix("http_")
        .filter(|m| API_METHODS.contains(m))
}

/// Tools `tools/list` offers, as they are kept.
fn listed(tools: &[Value]) -> Vec<ConnectionTool> {
    tools
        .iter()
        .map(|t| ConnectionTool {
            name: t["name"].as_str().unwrap_or_default().to_owned(),
            description: t["description"].as_str().unwrap_or_default().to_owned(),
            read_only: t["annotations"]["readOnlyHint"].as_bool(),
            always_ask: ["destructiveHint", "openWorldHint"]
                .iter()
                .any(|hint| t["annotations"][hint] == true),
        })
        .collect()
}

/// A text answer as a tool result.
fn text_result(text: String, is_error: bool) -> ToolResult {
    ToolResult {
        content: vec![json!({ "type": "text", "text": text })],
        structured_content: None,
        is_error,
        text,
    }
}

/// A tool result as `tools/call` answers it.
const NOT_MADE: &str = "famulenNotMade";

/// A call the judge refused, so nothing was sent.
#[derive(Debug)]
struct Refused(String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// Whether the gateway answered `response` to a call without sending it: it
/// is held, or it needed an approval its caller cannot ask for. What a tool
/// answers cannot say so.
pub fn not_made(response: &Value) -> bool {
    response["result"]["_meta"][NOT_MADE] == true
}

fn result_value(result: &ToolResult) -> Value {
    let mut value = json!({
        "content": if result.content.is_empty() {
            json!([{ "type": "text", "text": result.text }])
        } else {
            json!(result.content)
        },
        "isError": result.is_error,
    });
    if let Some(structured) = &result.structured_content {
        value["structuredContent"] = structured.clone();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::{Secret, Transport};
    use crate::store::Store;

    #[tokio::test]
    async fn a_call_is_judged_and_logged_without_its_body() {
        let gateway = gateway();
        add_fake(&gateway, "fake-work");
        gateway.grant("secretary", ANY, ANY).unwrap();
        gateway
            .set_permission(
                "owner",
                "conn:fake-work",
                "fail",
                Some(crate::permissions::Permission::Forbid),
            )
            .unwrap();
        let call = |tool: &str| {
            request(
                "tools/call",
                json!({ "name": tool, "arguments": { "text": "a secret body" } }),
            )
        };
        gateway
            .handle(
                "secretary",
                None,
                "fake-work",
                &request("tools/list", json!({})),
            )
            .await
            .unwrap();

        let ok = gateway
            .handle("secretary", None, "fake-work", &call("echo"))
            .await
            .unwrap();
        let refused = gateway
            .handle("secretary", None, "fake-work", &call("fail"))
            .await
            .unwrap();
        let held = gateway
            .handle(
                "secretary",
                Some("outside the workgroup".into()),
                "fake-work",
                &call("echo"),
            )
            .await
            .unwrap();

        assert_eq!(ok["result"]["content"][0]["text"], "a secret body");
        assert_eq!(refused["result"]["isError"], true);
        assert!(!not_made(&ok));
        assert!(not_made(&refused), "{refused}");
        assert!(not_made(&held), "{held}");
        assert!(
            refused["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("forbade")
        );
        assert!(
            held["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("Held for the owner's approval as request #")
        );
        let log = gateway.log_entries(10).unwrap();
        let summary: Vec<(&str, &str, &str)> = log
            .iter()
            .map(|e| (e.tool.as_str(), e.decision.as_str(), e.actor.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                ("echo", "confirm", "secretary"),
                ("fail", "deny", "secretary"),
                ("echo", "allow", "secretary"),
                ("permission.set", "allow", "owner"),
            ]
        );
        let allowed = &log[2];
        assert_eq!(allowed.outcome, "ok");
        assert!(
            allowed.digest.starts_with("in sha256:"),
            "{}",
            allowed.digest
        );
        assert!(
            !serde_json::to_string(&log)
                .unwrap()
                .contains("a secret body")
        );
    }

    fn gateway() -> Gateway {
        gateway_with(Store::open_in_memory().unwrap(), None)
    }

    fn gateway_with(store: Store, vault: Option<Vault>) -> Gateway {
        Gateway::new(
            store.clone(),
            Connections::new(store),
            reqwest::Client::new(),
            vault,
        )
    }

    fn add_fake(gateway: &Gateway, id: &str) {
        let crate::mcp::ServerConfig::Stdio { command, args, env } =
            crate::mcp::fake::stdio_server("legacy")
        else {
            unreachable!()
        };
        gateway
            .connections
            .add(
                id,
                "Fake",
                "fake",
                Transport::McpLocal { command, args, env },
                Secret::default(),
                "",
            )
            .unwrap();
    }

    fn request(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params })
    }

    #[test]
    fn every_connection_is_a_server_famulen_answers() {
        let gateway = gateway();
        add_fake(&gateway, "fake-work");
        gateway
            .connections
            .add(
                "billing",
                "Billing",
                "billing",
                Transport::Api {
                    base_url: "https://api.example.com".into(),
                    openapi_url: String::new(),
                },
                Secret::default(),
                "",
            )
            .unwrap();

        let mut served = gateway.served().unwrap();
        served.sort();

        assert_eq!(served, ["billing", "fake-work"]);
    }

    #[tokio::test]
    async fn a_servers_tools_are_listed_with_their_schemas() {
        let gateway = gateway();
        add_fake(&gateway, "fake-work");

        let answer = gateway
            .handle(OWNER, None, "fake-work", &request("tools/list", json!({})))
            .await
            .unwrap();

        let tools = answer["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names, ["echo", "fail", "seen"]);
        assert_eq!(tools[0]["inputSchema"], json!({ "type": "object" }));
        assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
        let kept = gateway.connections.tools("fake-work").unwrap();
        assert_eq!(kept.len(), 3);
    }

    #[tokio::test]
    async fn a_call_goes_through_to_the_server_and_comes_back() {
        let gateway = gateway();
        add_fake(&gateway, "fake-work");

        let ok = gateway
            .handle(
                OWNER,
                None,
                "fake-work",
                &request(
                    "tools/call",
                    json!({ "name": "echo", "arguments": { "text": "hello" } }),
                ),
            )
            .await
            .unwrap();
        let failed = gateway
            .handle(
                OWNER,
                None,
                "fake-work",
                &request("tools/call", json!({ "name": "fail", "arguments": {} })),
            )
            .await
            .unwrap();

        assert_eq!(ok["id"], 7);
        assert_eq!(ok["result"]["content"][0]["text"], "hello");
        assert_eq!(ok["result"]["isError"], false);
        assert_eq!(failed["result"]["isError"], true);
    }

    #[tokio::test]
    async fn an_unknown_server_is_an_error_and_a_notification_gets_nothing() {
        let gateway = gateway();

        let unknown = gateway
            .handle(OWNER, None, "nope", &request("tools/list", json!({})))
            .await
            .unwrap();
        let note = gateway
            .handle(
                OWNER,
                None,
                "nope",
                &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            )
            .await;

        assert!(
            unknown["error"]["message"]
                .as_str()
                .unwrap()
                .contains("nope")
        );
        assert!(note.is_none());
    }

    #[tokio::test]
    async fn an_api_key_is_kept_in_the_vault_and_still_reaches_the_service() {
        let store = Store::open_in_memory().unwrap();
        let vault = vault::tests::scratch();
        let gateway = gateway_with(store, Some(vault.clone()));
        let app = axum::Router::new().route(
            "/me",
            axum::routing::get(|headers: axum::http::HeaderMap| async move {
                headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned()
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let connection = gateway
            .connections
            .add(
                "billing",
                "Billing",
                "billing",
                Transport::Api {
                    base_url: base,
                    openapi_url: String::new(),
                },
                Secret::default(),
                "",
            )
            .unwrap();

        gateway.set_api_key("billing", "k-1234567890", "").unwrap();
        let answer = gateway
            .call_as_owner(&connection, "http_get", json!({ "path": "/me" }))
            .await
            .unwrap();

        assert_eq!(answer.text, "Bearer k-1234567890");
        let kept = gateway.connections.get("billing").unwrap().unwrap();
        assert_eq!(kept.secret.api_key, None);
        assert_eq!(vault.list().unwrap()[0].used_by, "billing");
    }

    #[tokio::test]
    async fn a_remote_mcp_servers_key_is_sent_in_its_header() {
        use crate::mcp::fake::{Era, FakeServer};
        let gateway = gateway();
        let fake = FakeServer::start(Era::Modern, Some("k-1")).await;
        let add = |id: &str| {
            gateway
                .connections
                .add(
                    id,
                    id,
                    "notes",
                    Transport::McpRemote { url: fake.url() },
                    Secret::default(),
                    "",
                )
                .unwrap()
        };
        add("notes");
        gateway.set_api_key("notes", "k-1", "").unwrap();
        assert!(gateway.read_tools("notes").await.unwrap() > 0);

        add("notes-2");
        gateway.set_api_key("notes-2", "k-2", "X-Api-Key").unwrap();
        let _ = gateway.read_tools("notes-2").await;

        let last = fake.received().pop().unwrap();
        assert_eq!(last.header("x-api-key"), Some("k-2"));
        assert_eq!(last.header("authorization"), None);
    }

    #[tokio::test]
    async fn a_call_the_server_answered_with_an_error_goes_out_only_once() {
        use crate::mcp::fake::{Era, FakeServer};
        let gateway = gateway();
        let fake = FakeServer::start(Era::Modern, None).await;
        let notes = gateway
            .connections
            .add(
                "notes",
                "Notes",
                "notes",
                Transport::McpRemote { url: fake.url() },
                Secret::default(),
                "",
            )
            .unwrap();

        let result = gateway.call_as_owner(&notes, "no_such", json!({})).await;

        assert!(result.is_err());
        assert_eq!(fake.received_method("tools/call").len(), 1);
    }

    #[tokio::test]
    async fn a_local_servers_environment_is_kept_in_the_vault_and_still_reaches_it() {
        let vault = vault::tests::scratch();
        let gateway = gateway_with(Store::open_in_memory().unwrap(), Some(vault.clone()));
        let crate::mcp::ServerConfig::Stdio { command, args, env } =
            crate::mcp::fake::stdio_server("modern")
        else {
            unreachable!()
        };
        gateway
            .connections
            .add(
                "fake",
                "Fake",
                "fake",
                Transport::McpLocal {
                    command,
                    args,
                    env: Default::default(),
                },
                Secret::default(),
                "",
            )
            .unwrap();

        gateway.set_environment("fake", env).unwrap();
        gateway.read_tools("fake").await.unwrap();

        let row = gateway.connections.get("fake").unwrap().unwrap();
        assert!(row.secret.account.is_empty());
        assert_eq!(
            vault
                .reveal("connection:fake:account.FAKE_MCP_ERA")
                .unwrap(),
            Some("modern".to_owned())
        );
        let seen = gateway
            .call_as_owner(&row, "seen", json!({}))
            .await
            .unwrap();
        assert!(
            seen.text.contains("server/discover") && !seen.text.contains("initialize"),
            "the server ran as the kept environment says: {}",
            seen.text
        );
    }

    #[tokio::test]
    async fn initialize_names_the_connection() {
        let gateway = gateway();
        add_fake(&gateway, "fake-work");

        let answer = gateway
            .handle(
                OWNER,
                None,
                "fake-work",
                &request("initialize", json!({ "protocolVersion": "2025-11-25" })),
            )
            .await
            .unwrap();

        assert_eq!(answer["result"]["serverInfo"]["name"], "fake-work");
        assert_eq!(answer["result"]["protocolVersion"], "2025-11-25");
        assert!(answer["result"]["capabilities"]["tools"].is_object());
    }
}
