//! The stream-json protocol Claude Code speaks on stdin and stdout when it is
//! started with `--input-format stream-json --output-format stream-json`.
//!
//! Only the messages famulen acts on are modelled; every other line is kept
//! as [`Inbound::Other`] so it can still be logged.

use serde_json::{Value, json};

use crate::permissions::Caller;

/// A line Claude Code wrote to stdout.
#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    /// The session started or resumed; lists every tool the model can call.
    Init {
        session_id: String,
        tools: Vec<String>,
    },
    /// Claude Code asks the host something and waits for a control response.
    Control {
        request_id: String,
        request: ControlRequest,
    },
    /// A turn ended.
    Result {
        session_id: String,
        is_error: bool,
        usage: crate::models::Usage,
    },
    /// Anything else (assistant messages, tool results, rate limits, ...).
    Other(Value),
}

/// The control requests famulen answers.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlRequest {
    /// May the model use this tool with this input?
    CanUseTool {
        caller: Caller,
        tool_name: String,
        input: Value,
    },
    /// A JSON-RPC message for an MCP server that runs inside the host.
    McpMessage { server_name: String, message: Value },
    /// A request famulen does not know; it is answered with an error.
    Unknown { subtype: String },
}

/// Read one stdout line. Lines that are not JSON objects are an error.
pub fn parse_line(line: &str) -> anyhow::Result<Inbound> {
    let value: Value = serde_json::from_str(line)?;
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let inbound = match kind {
        "system" if str_field(&value, "subtype") == Some("init") => Inbound::Init {
            session_id: str_field(&value, "session_id")
                .unwrap_or_default()
                .to_owned(),
            tools: value
                .get("tools")
                .and_then(Value::as_array)
                .map(|tools| {
                    tools
                        .iter()
                        .filter_map(|t| t.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
        },
        "control_request" => {
            let request_id = str_field(&value, "request_id")
                .unwrap_or_default()
                .to_owned();
            let request = value.get("request").cloned().unwrap_or(Value::Null);
            Inbound::Control {
                request_id,
                request: control_request(&request),
            }
        }
        "result" => Inbound::Result {
            session_id: str_field(&value, "session_id")
                .unwrap_or_default()
                .to_owned(),
            is_error: value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            usage: turn_usage(&value),
        },
        _ => Inbound::Other(value),
    };
    Ok(inbound)
}

/// What a turn used. Cached input counts as input: it is what the turn read.
pub(crate) fn turn_usage(value: &Value) -> crate::models::Usage {
    let tokens = |key: &str| {
        value
            .pointer(&format!("/usage/{key}"))
            .and_then(Value::as_u64)
    };
    let input = [
        "input_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ]
    .iter()
    .filter_map(|k| tokens(k))
    .reduce(|a, b| a + b);
    crate::models::Usage {
        input_tokens: input,
        output_tokens: tokens("output_tokens"),
        calls: 1,
    }
}

fn control_request(request: &Value) -> ControlRequest {
    match str_field(request, "subtype").unwrap_or_default() {
        "can_use_tool" => ControlRequest::CanUseTool {
            caller: match str_field(request, "agent_id") {
                Some(_) => Caller::Subagent,
                None => Caller::Secretary,
            },
            tool_name: str_field(request, "tool_name")
                .unwrap_or_default()
                .to_owned(),
            input: request.get("input").cloned().unwrap_or(Value::Null),
        },
        "mcp_message" => ControlRequest::McpMessage {
            server_name: str_field(request, "server_name")
                .unwrap_or_default()
                .to_owned(),
            message: request.get("message").cloned().unwrap_or(Value::Null),
        },
        other => ControlRequest::Unknown {
            subtype: other.to_owned(),
        },
    }
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// The control request that must open every session.
pub fn initialize_request(request_id: &str) -> Value {
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": { "subtype": "initialize", "hooks": null },
    })
}

/// A user message that starts a turn, or joins the one that is running.
pub fn user_message(text: &str) -> Value {
    json!({
        "type": "user",
        "message": { "role": "user", "content": text },
        "parent_tool_use_id": null,
        "session_id": "default",
    })
}

/// Let the tool run with the input the model gave.
pub fn allow_tool(request_id: &str, input: &Value) -> Value {
    success(
        request_id,
        json!({ "behavior": "allow", "updatedInput": input }),
    )
}

/// Refuse the tool; the model reads `message` as the tool's error.
pub fn deny_tool(request_id: &str, message: &str) -> Value {
    success(
        request_id,
        json!({ "behavior": "deny", "message": message }),
    )
}

/// Answer an MCP message. `response` is `None` for a JSON-RPC notification,
/// which still needs an acknowledgement on the control channel.
pub fn mcp_response(request_id: &str, response: Option<Value>) -> Value {
    let response = response.unwrap_or_else(|| json!({ "jsonrpc": "2.0", "result": {} }));
    success(request_id, json!({ "mcp_response": response }))
}

/// Tell Claude Code a control request could not be handled.
pub fn control_error(request_id: &str, error: &str) -> Value {
    json!({
        "type": "control_response",
        "response": { "subtype": "error", "request_id": request_id, "error": error },
    })
}

fn success(request_id: &str, response: Value) -> Value {
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": request_id, "response": response },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_lists_the_session_and_its_tools() {
        let line = r#"{"type":"system","subtype":"init","session_id":"s1","tools":["Read","mcp__famulen__send_slack"]}"#;
        assert_eq!(
            parse_line(line).unwrap(),
            Inbound::Init {
                session_id: "s1".into(),
                tools: vec!["Read".into(), "mcp__famulen__send_slack".into()],
            }
        );
    }

    #[test]
    fn can_use_tool_carries_the_tool_and_its_input() {
        let line = r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"a"}}}"#;
        assert_eq!(
            parse_line(line).unwrap(),
            Inbound::Control {
                request_id: "r1".into(),
                request: ControlRequest::CanUseTool {
                    caller: Caller::Secretary,
                    tool_name: "Write".into(),
                    input: json!({ "file_path": "a" }),
                },
            }
        );
    }

    #[test]
    fn can_use_tool_with_an_agent_id_comes_from_a_subagent() {
        let line = r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Write","input":{},"agent_id":"a1"}}"#;
        assert_eq!(
            parse_line(line).unwrap(),
            Inbound::Control {
                request_id: "r1".into(),
                request: ControlRequest::CanUseTool {
                    caller: Caller::Subagent,
                    tool_name: "Write".into(),
                    input: json!({}),
                },
            }
        );
    }

    #[test]
    fn mcp_message_names_the_server() {
        let line = r#"{"type":"control_request","request_id":"r2","request":{"subtype":"mcp_message","server_name":"famulen","message":{"jsonrpc":"2.0","id":1,"method":"tools/list"}}}"#;
        let Inbound::Control {
            request:
                ControlRequest::McpMessage {
                    server_name,
                    message,
                },
            ..
        } = parse_line(line).unwrap()
        else {
            panic!("not an mcp message");
        };
        assert_eq!(server_name, "famulen");
        assert_eq!(message["method"], "tools/list");
    }

    #[test]
    fn an_unknown_control_request_keeps_its_subtype() {
        let line =
            r#"{"type":"control_request","request_id":"r3","request":{"subtype":"hook_callback"}}"#;
        assert_eq!(
            parse_line(line).unwrap(),
            Inbound::Control {
                request_id: "r3".into(),
                request: ControlRequest::Unknown {
                    subtype: "hook_callback".into()
                },
            }
        );
    }

    #[test]
    fn a_result_carries_what_the_turn_used() {
        let line = r#"{"type":"result","is_error":false,"session_id":"s1","usage":{"input_tokens":10,"cache_read_input_tokens":90,"output_tokens":7}}"#;
        let Inbound::Result { usage, .. } = parse_line(line).unwrap() else {
            panic!("not a result");
        };
        assert_eq!(
            (usage.input_tokens, usage.output_tokens, usage.calls),
            (Some(100), Some(7), 1)
        );
    }

    #[test]
    fn result_ends_a_turn() {
        let line = r#"{"type":"result","subtype":"success","is_error":false,"session_id":"s1"}"#;
        assert_eq!(
            parse_line(line).unwrap(),
            Inbound::Result {
                session_id: "s1".into(),
                is_error: false,
                usage: crate::models::Usage {
                    input_tokens: None,
                    output_tokens: None,
                    calls: 1
                },
            }
        );
    }

    #[test]
    fn other_lines_are_kept_whole() {
        let line = r#"{"type":"assistant","message":{}}"#;
        assert!(matches!(parse_line(line).unwrap(), Inbound::Other(v) if v["type"] == "assistant"));
    }

    #[test]
    fn a_line_that_is_not_json_is_an_error() {
        assert!(parse_line("not json").is_err());
    }

    #[test]
    fn deny_tells_the_model_why() {
        assert_eq!(
            deny_tool("r1", "held for approval"),
            json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": "r1",
                    "response": { "behavior": "deny", "message": "held for approval" },
                },
            })
        );
    }

    #[test]
    fn allow_passes_the_input_back_unchanged() {
        let input = json!({ "text": "hi" });
        assert_eq!(
            allow_tool("r1", &input)["response"]["response"]["updatedInput"],
            input
        );
    }

    #[test]
    fn a_notification_is_still_acknowledged() {
        assert_eq!(
            mcp_response("r1", None)["response"]["response"]["mcp_response"],
            json!({ "jsonrpc": "2.0", "result": {} })
        );
    }
}
