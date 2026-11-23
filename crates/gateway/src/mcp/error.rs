//! What can go wrong while talking to an MCP server.

use std::fmt;

use serde_json::Value;

/// The `UnsupportedProtocolVersion` error code.
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
/// The `MissingRequiredClientCapability` error code.
pub const MISSING_REQUIRED_CLIENT_CAPABILITY: i64 = -32021;
/// The `HeaderMismatch` error code.
pub const HEADER_MISMATCH: i64 = -32020;
/// The JSON-RPC `Method not found` error code.
pub const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    /// Whether the code is one the current protocol revision defines, which
    /// only a server speaking that revision would send.
    pub fn is_modern(&self) -> bool {
        matches!(
            self.code,
            HEADER_MISMATCH | MISSING_REQUIRED_CLIENT_CAPABILITY | UNSUPPORTED_PROTOCOL_VERSION
        )
    }

    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            code: value.get("code")?.as_i64()?,
            message: value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            data: value.get("data").cloned(),
        })
    }
}

#[derive(Debug)]
pub enum McpError {
    /// The server answered HTTP 401: it wants a token, or a better one.
    Unauthorized {
        /// The protected resource metadata URL from `WWW-Authenticate`.
        resource_metadata: Option<String>,
        /// The scopes the challenge asked for.
        scope: Option<String>,
    },
    /// The server answered with a JSON-RPC error.
    Rpc(RpcError),
    /// The server answered with an HTTP status that carries no JSON-RPC error.
    Http { status: u16, body: String },
    /// The server did not answer in time.
    Timeout,
    /// The connection to the server is gone.
    Closed,
    /// The connection could not be made or broke.
    Transport(String),
    /// The server said something the client cannot follow.
    Protocol(String),
}

impl McpError {
    /// Whether the request did not reach a server that would act on it, so
    /// connecting anew may help and sending it again does not do it twice.
    pub fn is_lost(&self) -> bool {
        match self {
            Self::Unauthorized { .. } | Self::Closed | Self::Transport(_) => true,
            Self::Http { status, .. } => (400..500).contains(status),
            Self::Rpc(_) | Self::Timeout | Self::Protocol(_) => false,
        }
    }

    pub(crate) fn transport(error: impl fmt::Display) -> Self {
        Self::Transport(error.to_string())
    }

    pub(crate) fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol(message.into())
    }
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized {
                resource_metadata, ..
            } => match resource_metadata {
                Some(url) => write!(f, "unauthorized (resource metadata: {url})"),
                None => write!(f, "unauthorized"),
            },
            Self::Rpc(error) => write!(f, "server error {}: {}", error.code, error.message),
            Self::Http { status, body } => write!(f, "HTTP {status}: {body}"),
            Self::Timeout => write!(f, "the server did not answer in time"),
            Self::Closed => write!(f, "the connection to the server is closed"),
            Self::Transport(message) => write!(f, "transport error: {message}"),
            Self::Protocol(message) => write!(f, "protocol error: {message}"),
        }
    }
}

impl std::error::Error for McpError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_codes_of_the_current_revision_are_modern() {
        let error = |code| RpcError {
            code,
            message: String::new(),
            data: None,
        };

        assert!(error(UNSUPPORTED_PROTOCOL_VERSION).is_modern());
        assert!(error(HEADER_MISMATCH).is_modern());
        assert!(error(MISSING_REQUIRED_CLIENT_CAPABILITY).is_modern());
        assert!(!error(METHOD_NOT_FOUND).is_modern());
        assert!(!error(-32000).is_modern());
    }
}
