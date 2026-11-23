//! Famulen's own tools, served to Claude Code as an MCP server that lives
//! inside Famulen and is reached over the control channel. Which tools there
//! are and what they do is given from outside, by the API they stand for.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

/// The name Claude Code knows the server by; its tools are `mcp__famulen__*`.
pub const SERVER_NAME: &str = "famulen";

/// The `--mcp-config` entry that points Claude Code at this server.
pub fn mcp_config() -> Value {
    json!({ "mcpServers": { SERVER_NAME: { "type": "sdk", "name": SERVER_NAME } } })
}

/// One of Famulen's own tools.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// The JSON Schema of its arguments.
    pub input_schema: Value,
    /// It changes nothing.
    pub read_only: bool,
    /// The owner is asked before it runs.
    pub confirm: bool,
}

/// The tools the secretary is given, and how to run one.
pub trait OwnTools: Send + Sync {
    fn list(&self) -> Vec<ToolDef>;
    /// Run a tool; the text is what the model reads back.
    fn call(&self, name: &str, arguments: Value) -> BoxFuture<'static, anyhow::Result<String>>;
}

/// Answers Claude Code's MCP messages for the own tools.
#[derive(Clone)]
pub struct Hub {
    tools: Arc<dyn OwnTools>,
}

impl Hub {
    pub fn new(tools: Arc<dyn OwnTools>) -> Self {
        Self { tools }
    }

    pub fn tools(&self) -> &Arc<dyn OwnTools> {
        &self.tools
    }

    /// Answer one JSON-RPC message. A notification gets `None`.
    pub async fn handle(&self, message: &Value) -> Option<Value> {
        let id = message.get("id").cloned()?;
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params.get("protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": self.listed() })),
            "tools/call" => Ok(self.call(&params).await),
            _ => Err(json!({ "code": -32601, "message": format!("method not found: {method}") })),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        })
    }

    /// The tools as `tools/list` offers them. Each is marked to be always
    /// loaded, so the secretary does not have to search for its own tools.
    fn listed(&self) -> Vec<Value> {
        self.tools
            .list()
            .into_iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.input_schema,
                    "annotations": { "readOnlyHint": t.read_only },
                    "_meta": { "anthropic/alwaysLoad": true },
                })
            })
            .collect()
    }

    async fn call(&self, params: &Value) -> Value {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
        match self.tools.call(name, arguments).await {
            Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
            Err(error) => {
                json!({ "content": [{ "type": "text", "text": error.to_string() }], "isError": true })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl OwnTools for Echo {
        fn list(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: "task_list".into(),
                description: "List tasks.".into(),
                input_schema: json!({ "type": "object" }),
                read_only: true,
                confirm: false,
            }]
        }

        fn call(&self, name: &str, arguments: Value) -> BoxFuture<'static, anyhow::Result<String>> {
            let name = name.to_owned();
            Box::pin(async move {
                anyhow::ensure!(name == "task_list", "unknown tool: {name}");
                Ok(arguments.to_string())
            })
        }
    }

    fn request(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
    }

    #[tokio::test]
    async fn tools_are_listed_always_loaded_with_their_read_only_hint() {
        let hub = Hub::new(Arc::new(Echo));
        let answer = hub.handle(&request("tools/list", json!({}))).await.unwrap();
        let tool = &answer["result"]["tools"][0];
        assert_eq!(tool["name"], "task_list");
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["_meta"]["anthropic/alwaysLoad"], true);
    }

    #[tokio::test]
    async fn a_call_returns_the_tools_text_and_a_failure_is_a_tool_error() {
        let hub = Hub::new(Arc::new(Echo));
        let ok = hub
            .handle(&request(
                "tools/call",
                json!({ "name": "task_list", "arguments": { "workgroup": "work" } }),
            ))
            .await
            .unwrap();
        assert_eq!(
            ok["result"]["content"][0]["text"],
            r#"{"workgroup":"work"}"#
        );
        let bad = hub
            .handle(&request("tools/call", json!({ "name": "nope" })))
            .await
            .unwrap();
        assert_eq!(bad["result"]["isError"], true);
    }

    #[tokio::test]
    async fn a_notification_gets_no_response_and_an_unknown_method_an_error() {
        let hub = Hub::new(Arc::new(Echo));
        assert!(
            hub.handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
                .await
                .is_none()
        );
        let answer = hub.handle(&request("nope", json!({}))).await.unwrap();
        assert_eq!(answer["error"]["code"], -32601);
    }
}
