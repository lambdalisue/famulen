//! JSON-RPC messages as the client sees them.

use serde_json::{Value, json};

use super::error::{METHOD_NOT_FOUND, McpError, RpcError};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Incoming {
    Response {
        id: Value,
        outcome: Result<Value, RpcError>,
    },
    /// A request from the server; only servers of the handshake era send them.
    Request {
        id: Value,
        method: String,
    },
    Notification {
        method: String,
    },
}

impl Incoming {
    pub(crate) fn parse(text: &str) -> Result<Self, McpError> {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| McpError::protocol(format!("not JSON: {error}")))?;
        Self::from_value(value)
    }

    pub(crate) fn from_value(value: Value) -> Result<Self, McpError> {
        let id = value.get("id").filter(|id| !id.is_null()).cloned();
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .map(String::from);
        match (id, method) {
            (Some(id), Some(method)) => Ok(Self::Request { id, method }),
            (None, Some(method)) => Ok(Self::Notification { method }),
            (id, None) => {
                let outcome = if let Some(error) = value.get("error") {
                    Err(RpcError::from_value(error)
                        .ok_or_else(|| McpError::protocol("malformed error object"))?)
                } else if let Some(result) = value.get("result") {
                    Ok(result.clone())
                } else {
                    return Err(McpError::protocol(
                        "a message with neither method nor result",
                    ));
                };
                Ok(Self::Response {
                    id: id.unwrap_or(Value::Null),
                    outcome,
                })
            }
        }
    }
}

pub(crate) fn request(id: u64, method: &str, params: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

pub(crate) fn notification(method: &str) -> Value {
    json!({"jsonrpc": "2.0", "method": method})
}

/// The reply to a request from the server. The client offers no features to
/// servers, so only `ping` gets a result.
pub(crate) fn reply_to_server(id: &Value, method: &str) -> Value {
    if method == "ping" {
        json!({"jsonrpc": "2.0", "id": id, "result": {}})
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": METHOD_NOT_FOUND, "message": format!("method not supported: {method}")},
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_is_a_response() {
        let message = Incoming::parse(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#).unwrap();

        assert_eq!(
            message,
            Incoming::Response {
                id: json!(1),
                outcome: Ok(json!({"ok": true}))
            }
        );
    }

    #[test]
    fn an_error_is_a_response_that_failed() {
        let message =
            Incoming::parse(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"bad"}}"#)
                .unwrap();

        assert_eq!(
            message,
            Incoming::Response {
                id: json!(2),
                outcome: Err(RpcError {
                    code: -32602,
                    message: "bad".into(),
                    data: None
                })
            }
        );
    }

    #[test]
    fn a_method_with_an_id_is_a_request_and_without_one_a_notification() {
        let request = Incoming::parse(r#"{"jsonrpc":"2.0","id":"s1","method":"roots/list"}"#);
        let notification =
            Incoming::parse(r#"{"jsonrpc":"2.0","method":"notifications/progress"}"#);

        assert_eq!(
            request.unwrap(),
            Incoming::Request {
                id: json!("s1"),
                method: "roots/list".into()
            }
        );
        assert_eq!(
            notification.unwrap(),
            Incoming::Notification {
                method: "notifications/progress".into()
            }
        );
    }

    #[test]
    fn garbage_is_a_protocol_error() {
        assert!(matches!(
            Incoming::parse("hello"),
            Err(McpError::Protocol(_))
        ));
        assert!(matches!(
            Incoming::parse(r#"{"jsonrpc":"2.0","id":1}"#),
            Err(McpError::Protocol(_))
        ));
    }

    #[test]
    fn the_server_gets_a_pong_for_ping_and_an_error_for_anything_else() {
        assert_eq!(
            reply_to_server(&json!(7), "ping"),
            json!({"jsonrpc": "2.0", "id": 7, "result": {}})
        );
        assert_eq!(
            reply_to_server(&json!(8), "sampling/createMessage")["error"]["code"],
            json!(METHOD_NOT_FOUND)
        );
    }
}
