//! The MCP client: finds out which protocol revision a server speaks, then
//! lists and calls its tools.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::config::ServerConfig;
use super::error::{HEADER_MISMATCH, McpError, UNSUPPORTED_PROTOCOL_VERSION};
use super::headers::{ParamHeader, encode_header_value, param_header_values, param_headers};
use super::http::{HttpTransport, PROTOCOL_VERSION_HEADER};
use super::jsonrpc::{notification, request};
use super::stdio::StdioTransport;

/// The protocol revision the client prefers, which carries its version on
/// every request instead of opening a session.
pub const PROTOCOL_VERSION: &str = "2026-07-28";

/// The revision the client offers in an `initialize` handshake to a server
/// that does not know [`PROTOCOL_VERSION`].
pub const LEGACY_PROTOCOL_VERSION: &str = "2025-11-25";

const LEGACY_PROTOCOL_VERSIONS: &[&str] = &[
    LEGACY_PROTOCOL_VERSION,
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// How the client and the server agreed to talk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Era {
    /// Every request carries its protocol version and capabilities.
    Modern(String),
    /// An `initialize` handshake fixed the version for the connection.
    Legacy(String),
}

impl Era {
    pub fn protocol_version(&self) -> &str {
        match self {
            Self::Modern(version) | Self::Legacy(version) => version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: Value,
    /// The server's `readOnlyHint`; a hint, not a guarantee.
    pub read_only: Option<bool>,
    /// The server said `destructiveHint` or `openWorldHint` outright: the call
    /// cannot be undone or it reaches beyond the server. Left unsaid, neither
    /// counts, though the protocol reads them as true.
    pub always_ask: bool,
}

impl Tool {
    fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(String::from);
        Some(Self {
            name: text("name")?,
            title: text("title"),
            description: text("description"),
            input_schema: value
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"})),
            read_only: value
                .pointer("/annotations/readOnlyHint")
                .and_then(Value::as_bool),
            always_ask: ["/annotations/destructiveHint", "/annotations/openWorldHint"]
                .iter()
                .any(|hint| value.pointer(hint).and_then(Value::as_bool) == Some(true)),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<Value>,
    pub structured_content: Option<Value>,
    /// The tool ran and failed; `text` says why.
    pub is_error: bool,
    /// The text parts of `content`, one per line.
    pub text: String,
}

impl ToolResult {
    fn from_value(value: &Value) -> Self {
        let content = value
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let text = content
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            structured_content: value.get("structuredContent").cloned(),
            is_error: value
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            content,
            text,
        }
    }
}

enum Transport {
    Stdio(StdioTransport),
    Http(HttpTransport),
}

struct Connection {
    transport: Transport,
    next_id: AtomicU64,
}

impl Connection {
    async fn call(
        &self,
        era: Option<&Era>,
        method: &str,
        params: Value,
        param_headers: &[ParamHeader],
        timeout: Duration,
    ) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let params = with_meta(era, params);
        let message = request(id, method, &params);
        let result = match &self.transport {
            Transport::Stdio(stdio) => stdio.request(id, &message, timeout).await?,
            Transport::Http(http) => {
                let headers = request_headers(era, method, &params, param_headers);
                http.request(id, &message, &headers, timeout).await?
            }
        };
        match result.get("resultType").and_then(Value::as_str) {
            None | Some("complete") => Ok(result),
            Some("input_required") => Err(McpError::protocol(format!(
                "{method} needs input from the user, which this client cannot ask for"
            ))),
            Some(other) => Err(McpError::protocol(format!("unknown resultType {other:?}"))),
        }
    }

    async fn notify(&self, era: &Era, method: &str) -> Result<(), McpError> {
        let message = notification(method);
        match &self.transport {
            Transport::Stdio(stdio) => stdio.notify(&message).await,
            Transport::Http(http) => {
                let headers = request_headers(Some(era), method, &Value::Null, &[]);
                http.notify(&message, &headers).await
            }
        }
    }
}

pub struct McpClient {
    connection: Connection,
    era: Era,
    instructions: Option<String>,
    param_headers: Mutex<HashMap<String, Vec<ParamHeader>>>,
}

impl McpClient {
    /// Starts or reaches the server and agrees on a protocol revision: the
    /// current one when the server knows it, otherwise an `initialize`
    /// handshake.
    pub async fn connect(config: &ServerConfig, bearer: Option<&str>) -> Result<Self, McpError> {
        let transport = match config {
            ServerConfig::Stdio { command, args, env } => {
                Transport::Stdio(StdioTransport::spawn(command, args, env)?)
            }
            ServerConfig::Http { url, headers } => {
                Transport::Http(HttpTransport::new(url, headers, bearer)?)
            }
        };
        let connection = Connection {
            transport,
            next_id: AtomicU64::new(1),
        };
        let (era, instructions) = agree(&connection).await?;
        tracing::debug!(?era, "connected to the MCP server");
        Ok(Self {
            connection,
            era,
            instructions,
            param_headers: Mutex::default(),
        })
    }

    #[cfg(test)]
    pub(crate) fn era(&self) -> &Era {
        &self.era
    }

    /// The server's guidance for a model on how to use it.
    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    /// Every tool, across all pages.
    pub async fn list_tools(&self) -> Result<Vec<Tool>, McpError> {
        let mirrors_params = self.mirrors_params();
        let mut tools = Vec::new();
        let mut headers = HashMap::new();
        let mut cursors = HashSet::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(cursor) => json!({"cursor": cursor}),
                None => json!({}),
            };
            let result = self.call("tools/list", params, &[]).await?;
            for value in result
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(tool) = Tool::from_value(value) else {
                    tracing::debug!(%value, "tool without a name");
                    continue;
                };
                if mirrors_params {
                    match param_headers(&tool.input_schema) {
                        Ok(params) => {
                            headers.insert(tool.name.clone(), params);
                        }
                        Err(reason) => {
                            tracing::debug!(tool = %tool.name, %reason, "tool left out");
                            continue;
                        }
                    }
                }
                tools.push(tool);
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(String::from);
            match &cursor {
                None => break,
                Some(next) if !cursors.insert(next.clone()) => {
                    return Err(McpError::protocol(format!(
                        "tools/list repeated cursor {next:?}"
                    )));
                }
                Some(_) => {}
            }
        }
        *self.param_headers.lock().unwrap() = headers;
        Ok(tools)
    }

    pub async fn call_tool(
        &self,
        name: impl AsRef<str>,
        arguments: Value,
    ) -> Result<ToolResult, McpError> {
        let name = name.as_ref();
        let params = json!({"name": name, "arguments": arguments});
        let result = match self.call_with_param_headers(name, &params).await {
            Err(McpError::Rpc(error)) if error.code == HEADER_MISMATCH && self.mirrors_params() => {
                tracing::debug!(tool = %name, "header mismatch; listing tools again");
                self.list_tools().await?;
                self.call_with_param_headers(name, &params).await
            }
            result => result,
        }?;
        Ok(ToolResult::from_value(&result))
    }

    pub async fn close(self) {
        match &self.connection.transport {
            Transport::Stdio(stdio) => stdio.close().await,
            Transport::Http(http) => {
                let version = self.era.protocol_version().to_string();
                http.close(&[(PROTOCOL_VERSION_HEADER.to_string(), version)])
                    .await;
            }
        }
    }

    async fn call_with_param_headers(&self, name: &str, params: &Value) -> Result<Value, McpError> {
        let headers = self
            .param_headers
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .unwrap_or_default();
        self.call("tools/call", params.clone(), &headers).await
    }

    async fn call(
        &self,
        method: &str,
        params: Value,
        param_headers: &[ParamHeader],
    ) -> Result<Value, McpError> {
        self.connection
            .call(
                Some(&self.era),
                method,
                params,
                param_headers,
                REQUEST_TIMEOUT,
            )
            .await
    }

    fn mirrors_params(&self) -> bool {
        matches!(self.connection.transport, Transport::Http(_))
            && matches!(self.era, Era::Modern(_))
    }
}

/// How to talk, and the server's guidance on how to use it.
type Agreement = (Era, Option<String>);

async fn agree(connection: &Connection) -> Result<Agreement, McpError> {
    let modern = Era::Modern(PROTOCOL_VERSION.into());
    let probe = connection
        .call(
            Some(&modern),
            "server/discover",
            json!({}),
            &[],
            PROBE_TIMEOUT,
        )
        .await;
    let supported = match probe {
        Ok(result) => {
            let supported = versions(result.get("supportedVersions"));
            if supported.iter().any(|version| version == PROTOCOL_VERSION) {
                let instructions = result
                    .get("instructions")
                    .and_then(Value::as_str)
                    .map(String::from);
                return Ok((modern, instructions));
            }
            supported
        }
        Err(McpError::Rpc(error)) if error.code == UNSUPPORTED_PROTOCOL_VERSION => {
            let supported = versions(error.data.as_ref().and_then(|data| data.get("supported")));
            if supported.iter().any(|version| version == PROTOCOL_VERSION) {
                return Err(McpError::protocol(format!(
                    "the server rejected {PROTOCOL_VERSION} while listing it as supported"
                )));
            }
            supported
        }
        Err(McpError::Rpc(error)) if error.is_modern() => return Err(McpError::Rpc(error)),
        Err(McpError::Rpc(_) | McpError::Timeout) => Vec::new(),
        Err(McpError::Http { status, .. }) if (400..500).contains(&status) => Vec::new(),
        Err(error) => return Err(error),
    };
    if !supported.is_empty()
        && !supported
            .iter()
            .any(|version| LEGACY_PROTOCOL_VERSIONS.contains(&version.as_str()))
    {
        return Err(McpError::protocol(format!(
            "no protocol version in common; the server supports {supported:?}"
        )));
    }
    initialize(connection).await
}

async fn initialize(connection: &Connection) -> Result<Agreement, McpError> {
    let params = json!({
        "protocolVersion": LEGACY_PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": client_info(),
    });
    let result = connection
        .call(None, "initialize", params, &[], REQUEST_TIMEOUT)
        .await?;
    let version = result
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !LEGACY_PROTOCOL_VERSIONS.contains(&version) {
        return Err(McpError::protocol(format!(
            "the server answered initialize with unsupported version {version:?}"
        )));
    }
    let era = Era::Legacy(version.into());
    connection.notify(&era, "notifications/initialized").await?;
    let instructions = result
        .get("instructions")
        .and_then(Value::as_str)
        .map(String::from);
    Ok((era, instructions))
}

fn versions(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}

fn client_info() -> Value {
    json!({"name": "famulen", "version": env!("CARGO_PKG_VERSION")})
}

/// The params with the per-request metadata the current revision requires.
fn with_meta(era: Option<&Era>, params: Value) -> Value {
    let Some(Era::Modern(version)) = era else {
        return params;
    };
    let mut params = match params {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    let meta = params.entry("_meta").or_insert_with(|| json!({}));
    if let Some(meta) = meta.as_object_mut() {
        meta.insert(
            "io.modelcontextprotocol/protocolVersion".into(),
            json!(version),
        );
        meta.insert("io.modelcontextprotocol/clientInfo".into(), client_info());
        meta.insert(
            "io.modelcontextprotocol/clientCapabilities".into(),
            json!({}),
        );
    }
    Value::Object(params)
}

/// The HTTP headers that mirror a request body.
fn request_headers(
    era: Option<&Era>,
    method: &str,
    params: &Value,
    param_headers: &[ParamHeader],
) -> Vec<(String, String)> {
    let Some(era) = era else {
        return Vec::new();
    };
    let mut headers = vec![(
        PROTOCOL_VERSION_HEADER.to_string(),
        era.protocol_version().to_string(),
    )];
    if let Era::Modern(_) = era {
        headers.push(("mcp-method".into(), method.into()));
        let name = match method {
            "tools/call" | "prompts/get" => params.get("name"),
            "resources/read" => params.get("uri"),
            _ => None,
        };
        if let Some(name) = name.and_then(Value::as_str) {
            headers.push(("mcp-name".into(), encode_header_value(name)));
        }
        if method == "tools/call" {
            let arguments = params.get("arguments").unwrap_or(&Value::Null);
            headers.extend(param_header_values(param_headers, arguments));
        }
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_modern_request_carries_its_version_and_capabilities_in_meta() {
        let era = Era::Modern(PROTOCOL_VERSION.into());

        let params = with_meta(Some(&era), json!({"cursor": "2"}));

        assert_eq!(params["cursor"], json!("2"));
        assert_eq!(
            params["_meta"]["io.modelcontextprotocol/protocolVersion"],
            json!(PROTOCOL_VERSION)
        );
        assert_eq!(
            params["_meta"]["io.modelcontextprotocol/clientCapabilities"],
            json!({})
        );
        assert_eq!(
            params["_meta"]["io.modelcontextprotocol/clientInfo"]["name"],
            json!("famulen")
        );
    }

    #[test]
    fn a_legacy_request_is_left_as_it_is() {
        let era = Era::Legacy(LEGACY_PROTOCOL_VERSION.into());

        assert_eq!(with_meta(Some(&era), json!({"a": 1})), json!({"a": 1}));
        assert_eq!(with_meta(None, json!({"a": 1})), json!({"a": 1}));
    }

    #[test]
    fn a_modern_tool_call_mirrors_method_name_and_params_into_headers() {
        let era = Era::Modern(PROTOCOL_VERSION.into());
        let params = json!({"name": "execute_sql", "arguments": {"region": "us-west1"}});
        let mirrored = [ParamHeader {
            name: "Region".into(),
            path: vec!["region".into()],
        }];

        let headers = request_headers(Some(&era), "tools/call", &params, &mirrored);

        assert_eq!(
            headers,
            vec![
                (
                    "mcp-protocol-version".to_string(),
                    PROTOCOL_VERSION.to_string()
                ),
                ("mcp-method".to_string(), "tools/call".to_string()),
                ("mcp-name".to_string(), "execute_sql".to_string()),
                ("Mcp-Param-Region".to_string(), "us-west1".to_string()),
            ]
        );
    }

    #[test]
    fn a_legacy_request_sends_only_the_negotiated_version_and_initialize_nothing() {
        let era = Era::Legacy("2025-06-18".into());

        assert_eq!(
            request_headers(Some(&era), "tools/call", &json!({"name": "x"}), &[]),
            vec![("mcp-protocol-version".to_string(), "2025-06-18".to_string())]
        );
        assert!(request_headers(None, "initialize", &json!({}), &[]).is_empty());
    }

    #[test]
    fn a_tool_takes_read_only_from_its_annotations() {
        let tool = Tool::from_value(&json!({
            "name": "search",
            "description": "Search mail",
            "inputSchema": {"type": "object"},
            "annotations": {"readOnlyHint": true}
        }))
        .unwrap();
        let bare = Tool::from_value(&json!({"name": "send"})).unwrap();

        assert_eq!(tool.read_only, Some(true));
        assert_eq!(tool.description.as_deref(), Some("Search mail"));
        assert_eq!(bare.read_only, None);
        assert_eq!(bare.input_schema, json!({"type": "object"}));
    }

    #[test]
    fn a_tool_said_to_be_destructive_or_to_reach_outside_is_always_asked_about() {
        let tool = |annotations: Value| {
            Tool::from_value(&json!({ "name": "t", "annotations": annotations }))
                .unwrap()
                .always_ask
        };
        assert!(tool(json!({ "destructiveHint": true })));
        assert!(tool(json!({ "openWorldHint": true })));
        assert!(!tool(
            json!({ "destructiveHint": false, "openWorldHint": false })
        ));
        assert!(!tool(json!({})));
    }

    #[test]
    fn a_tool_result_joins_its_text_parts() {
        let result = ToolResult::from_value(&json!({
            "content": [
                {"type": "text", "text": "one"},
                {"type": "image", "data": "AAAA", "mimeType": "image/png"},
                {"type": "text", "text": "two"}
            ],
            "isError": true
        }));

        assert_eq!(result.text, "one\ntwo");
        assert!(result.is_error);
        assert_eq!(result.content.len(), 3);
    }
}
