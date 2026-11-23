//! What the channel does on Slack, through the gateway: every post, update
//! and reaction is a call of the Slack connection's tools, so it is judged
//! and logged like any other call.

use std::future::Future;

use serde_json::{Value, json};

use crate::connections::Connections;
use crate::gateway::Gateway;

/// A message to post: plain text, and Block Kit blocks when it needs more.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outgoing {
    pub text: String,
    pub blocks: Option<Value>,
    pub thread_ts: Option<String>,
}

/// What Famulen does on Slack. A trait so the rest can be tested without it.
pub trait Slack: Send + Sync {
    /// Post to `channel`. Returns the `ts` of the posted message.
    fn post(
        &self,
        channel: &str,
        message: Outgoing,
    ) -> impl Future<Output = anyhow::Result<String>> + Send;

    /// Replace a message Famulen posted earlier.
    fn update(
        &self,
        channel: &str,
        ts: &str,
        message: Outgoing,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// The direct-message channel with `user`.
    fn open_dm(&self, user: &str) -> impl Future<Output = anyhow::Result<String>> + Send;

    /// Show `status` under the thread of a message; empty takes it away.
    fn set_status(
        &self,
        channel: &str,
        thread: &str,
        status: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Add the reaction `name` (an emoji name such as `eyes`) to a message.
    fn react(
        &self,
        channel: &str,
        ts: &str,
        name: &str,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

/// One Slack connection, reached through the gateway's tools of the bundled
/// Slack definition.
#[derive(Clone)]
pub struct GatewaySlack {
    gateway: Gateway,
    connections: Connections,
    connection: String,
}

impl GatewaySlack {
    pub fn new(gateway: Gateway, connections: Connections, connection: impl Into<String>) -> Self {
        Self {
            gateway,
            connections,
            connection: connection.into(),
        }
    }

    /// Call one tool and read its answer as JSON; a failed call is an error
    /// with what the service said.
    async fn call(&self, tool: &str, args: Value) -> anyhow::Result<Value> {
        let connection = self
            .connections
            .get(&self.connection)?
            .ok_or_else(|| anyhow::anyhow!("no connection {}", self.connection))?;
        let result = self.gateway.call_as_owner(&connection, tool, args).await?;
        let answer: Value = serde_json::from_str(&result.text).unwrap_or(Value::Null);
        if result.is_error {
            let why = answer
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or(result.text);
            anyhow::bail!("{tool} failed: {why}");
        }
        Ok(answer)
    }
}

fn message_args(channel: &str, message: Outgoing) -> Value {
    let mut args = json!({ "channel": channel, "text": message.text });
    if let Some(blocks) = message.blocks {
        args["blocks"] = blocks;
    }
    if let Some(thread) = message.thread_ts {
        args["thread"] = json!(thread);
    }
    args
}

impl Slack for GatewaySlack {
    async fn post(&self, channel: &str, message: Outgoing) -> anyhow::Result<String> {
        let answer = self
            .call("post_message", message_args(channel, message))
            .await?;
        Ok(answer
            .get("ts")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    async fn update(&self, channel: &str, ts: &str, message: Outgoing) -> anyhow::Result<()> {
        let mut args = message_args(channel, message);
        args["ts"] = json!(ts);
        self.call("update_message", args).await?;
        Ok(())
    }

    async fn open_dm(&self, user: &str) -> anyhow::Result<String> {
        let answer = self.call("open_dm", json!({ "user": user })).await?;
        answer
            .pointer("/channel/id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("open_dm answered no channel"))
    }

    async fn set_status(&self, channel: &str, thread: &str, status: &str) -> anyhow::Result<()> {
        self.call(
            "set_status",
            json!({ "channel": channel, "thread": thread, "status": status }),
        )
        .await?;
        Ok(())
    }

    async fn react(&self, channel: &str, ts: &str, name: &str) -> anyhow::Result<()> {
        self.call(
            "add_reaction",
            json!({ "channel": channel, "ts": ts, "name": name }),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use super::*;

    const SLACK: &str = include_str!("../../../gateway/services/slack/slack.toml");

    type Seen = Arc<Mutex<Vec<(String, Value)>>>;

    async fn slack_api(answer: Value) -> (String, Seen) {
        let seen: Seen = Arc::default();
        let app = axum::Router::new()
            .route(
                "/api/{method}",
                axum::routing::post(
                    |axum::extract::State((seen, answer)): axum::extract::State<(Seen, Value)>,
                     axum::extract::Path(method): axum::extract::Path<String>,
                     body: String| async move {
                        seen.lock()
                            .unwrap()
                            .push((method, serde_json::from_str(&body).unwrap_or(Value::Null)));
                        answer.to_string()
                    },
                ),
            )
            .with_state((seen.clone(), answer));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, seen)
    }

    fn slack(base: &str) -> GatewaySlack {
        let store = famulen_gateway::store::Store::open_in_memory().unwrap();
        let connections = Connections::new(store.clone());
        let gateway = Gateway::new(store, connections.clone(), reqwest::Client::new(), None)
            .with_definitions(
                None,
                &[(
                    "slack.toml".into(),
                    SLACK.replace("https://slack.com/api", &format!("{base}/api")),
                )],
            );
        gateway.add_defined("slack", BTreeMap::new(), None).unwrap();
        GatewaySlack::new(gateway, connections, "slack")
    }

    #[tokio::test]
    async fn a_post_calls_the_connections_tool_and_returns_the_ts() {
        let (base, seen) = slack_api(json!({ "ok": true, "ts": "1.5" })).await;

        let ts = slack(&base)
            .post(
                "D1",
                Outgoing {
                    text: "hi".into(),
                    thread_ts: Some("1.0".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(ts, "1.5");
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "chat.postMessage");
        assert_eq!(
            seen[0].1,
            json!({ "channel": "D1", "text": "hi", "thread_ts": "1.0" })
        );
    }

    #[tokio::test]
    async fn slacks_refusal_is_an_error_that_says_why() {
        let (base, _) = slack_api(json!({ "ok": false, "error": "channel_not_found" })).await;

        let error = slack(&base).react("C1", "1.0", "eyes").await.unwrap_err();

        assert_eq!(error.to_string(), "add_reaction failed: channel_not_found");
    }
}
