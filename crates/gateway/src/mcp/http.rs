//! The streamable HTTP transport: one POST per message, answered with JSON
//! or with an SSE stream.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, WWW_AUTHENTICATE};
use reqwest::{Response, StatusCode};
use serde_json::Value;

use super::error::{McpError, RpcError};
use super::jsonrpc::{Incoming, reply_to_server};
use super::oauth::parse_www_authenticate;
use super::sse::SseParser;

pub(crate) const SESSION_HEADER: &str = "mcp-session-id";
pub(crate) const PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";

pub(crate) struct HttpTransport {
    client: reqwest::Client,
    url: String,
    headers: HeaderMap,
    session: Mutex<Option<String>>,
}

impl HttpTransport {
    pub(crate) fn new(
        url: &str,
        headers: &BTreeMap<String, String>,
        bearer: Option<&str>,
    ) -> Result<Self, McpError> {
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            map.insert(header_name(name)?, header_value(value)?);
        }
        if let Some(token) = bearer {
            map.insert(
                reqwest::header::AUTHORIZATION,
                header_value(&format!("Bearer {token}"))?,
            );
        }
        let client = reqwest::Client::builder()
            .build()
            .map_err(McpError::transport)?;
        Ok(Self {
            client,
            url: url.into(),
            headers: map,
            session: Mutex::new(None),
        })
    }

    pub(crate) async fn request(
        &self,
        id: u64,
        message: &Value,
        extra: &[(String, String)],
        timeout: Duration,
    ) -> Result<Value, McpError> {
        tokio::time::timeout(timeout, async {
            let response = self.post(message, extra).await?;
            if response.status() == StatusCode::ACCEPTED {
                return Err(McpError::protocol(
                    "the server accepted a request without answering it",
                ));
            }
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if content_type.starts_with("text/event-stream") {
                self.read_stream(id, response, extra).await
            } else {
                let body = response.text().await.map_err(McpError::transport)?;
                match Incoming::parse(&body)? {
                    Incoming::Response { outcome, .. } => outcome.map_err(McpError::Rpc),
                    _ => Err(McpError::protocol(
                        "the server answered with something other than a response",
                    )),
                }
            }
        })
        .await
        .map_err(|_| McpError::Timeout)?
    }

    pub(crate) async fn notify(
        &self,
        message: &Value,
        extra: &[(String, String)],
    ) -> Result<(), McpError> {
        self.post(message, extra).await.map(drop)
    }

    /// Ends the session the server opened, if it opened one.
    pub(crate) async fn close(&self, extra: &[(String, String)]) {
        let Some(session) = self.session.lock().unwrap().take() else {
            return;
        };
        let mut request = self
            .client
            .delete(&self.url)
            .headers(self.headers.clone())
            .header(SESSION_HEADER, session);
        for (name, value) in extra {
            request = request.header(name, value);
        }
        if let Err(error) = request.send().await {
            tracing::debug!(%error, "cannot end the MCP session");
        }
    }

    async fn post(
        &self,
        message: &Value,
        extra: &[(String, String)],
    ) -> Result<Response, McpError> {
        let mut request = self
            .client
            .post(&self.url)
            .headers(self.headers.clone())
            .header(ACCEPT, "application/json, text/event-stream")
            .json(message);
        for (name, value) in extra {
            request = request.header(header_name(name)?, header_value(value)?);
        }
        let session = self.session.lock().unwrap().clone();
        let in_session = session.is_some();
        if let Some(session) = session {
            request = request.header(SESSION_HEADER, session);
        }
        let response = request.send().await.map_err(McpError::transport)?;
        if let Some(session) = response
            .headers()
            .get(SESSION_HEADER)
            .and_then(|value| value.to_str().ok())
        {
            *self.session.lock().unwrap() = Some(session.to_string());
        }
        check(response, in_session).await
    }

    async fn read_stream(
        &self,
        id: u64,
        response: Response,
        extra: &[(String, String)],
    ) -> Result<Value, McpError> {
        let mut parser = SseParser::default();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            for event in parser.push(&chunk.map_err(McpError::transport)?) {
                if let Some(outcome) = self.handle_event(id, &event, extra).await? {
                    return outcome.map_err(McpError::Rpc);
                }
            }
        }
        if let Some(event) = parser.finish()
            && let Some(outcome) = self.handle_event(id, &event, extra).await?
        {
            return outcome.map_err(McpError::Rpc);
        }
        Err(McpError::protocol("the stream ended without a response"))
    }

    async fn handle_event(
        &self,
        id: u64,
        event: &str,
        extra: &[(String, String)],
    ) -> Result<Option<Result<Value, RpcError>>, McpError> {
        if event.trim().is_empty() {
            return Ok(None);
        }
        match Incoming::parse(event)? {
            Incoming::Response { id: got, outcome } if got.as_u64() == Some(id) => {
                Ok(Some(outcome))
            }
            Incoming::Response { id, .. } => {
                tracing::debug!(%id, "response to another request on the stream");
                Ok(None)
            }
            Incoming::Request { id, method } => {
                tracing::debug!(%method, "request from the server");
                let version: Vec<_> = extra
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case(PROTOCOL_VERSION_HEADER))
                    .cloned()
                    .collect();
                self.notify(&reply_to_server(&id, &method), &version)
                    .await?;
                Ok(None)
            }
            Incoming::Notification { method } => {
                tracing::debug!(%method, "notification from the server");
                Ok(None)
            }
        }
    }
}

/// Turns statuses that carry no answer into errors. Inside a session, a 400
/// or 404 says the server no longer knows it, whatever error comes along.
async fn check(response: Response, in_session: bool) -> Result<Response, McpError> {
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        let challenge = response
            .headers()
            .get_all(WWW_AUTHENTICATE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find_map(parse_www_authenticate)
            .unwrap_or_default();
        return Err(McpError::Unauthorized {
            resource_metadata: challenge.resource_metadata,
            scope: challenge.scope,
        });
    }
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    if in_session && matches!(status, StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND) {
        return Err(McpError::Http {
            status: status.as_u16(),
            body,
        });
    }
    match Incoming::parse(&body) {
        Ok(Incoming::Response {
            outcome: Err(error),
            ..
        }) => Err(McpError::Rpc(error)),
        _ => Err(McpError::Http {
            status: status.as_u16(),
            body,
        }),
    }
}

fn header_name(name: &str) -> Result<HeaderName, McpError> {
    HeaderName::try_from(name)
        .map_err(|error| McpError::Transport(format!("bad header name {name:?}: {error}")))
}

fn header_value(value: &str) -> Result<HeaderValue, McpError> {
    HeaderValue::try_from(value)
        .map_err(|error| McpError::Transport(format!("bad header value: {error}")))
}
