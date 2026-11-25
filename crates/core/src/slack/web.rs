//! What the channel does on Slack, through the gateway: every post, update
//! and reaction is a call of the Slack connection's tools, so it is judged
//! and logged like any other call.

use std::future::Future;

use serde_json::{Value, json};

use crate::connections::Connections;
use crate::gateway::Gateway;
use crate::slack::looks::Looks;

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

    /// Post to `channel` for `user` alone, until they reload Slack. It cannot
    /// be replaced later.
    fn post_ephemeral(
        &self,
        channel: &str,
        user: &str,
        message: Outgoing,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

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
    looks: Looks,
}

/// Slack answered a call by refusing it: it did nothing.
#[derive(Debug)]
struct Refused {
    tool: String,
    why: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed: {}", self.tool, self.why)
    }
}

impl std::error::Error for Refused {}

/// The refusals a post meets for carrying a look the bot may not set.
const NO_LOOK: [&str; 3] = ["missing_scope", "no_permission", "not_allowed_token_type"];

impl GatewaySlack {
    pub fn new(
        gateway: Gateway,
        connections: Connections,
        connection: impl Into<String>,
        looks: Looks,
    ) -> Self {
        Self {
            gateway,
            connections,
            connection: connection.into(),
            looks,
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
            let Some(why) = answer.get("error").and_then(Value::as_str) else {
                anyhow::bail!("{tool} failed: {}", result.text);
            };
            return Err(Refused {
                tool: tool.to_owned(),
                why: why.to_owned(),
            }
            .into());
        }
        Ok(answer)
    }

    /// Post with the secretary's look, or without one where the bot may not
    /// set it: a workspace the app was installed in before it asked to.
    async fn post_with_look(&self, tool: &str, mut args: Value) -> anyhow::Result<Value> {
        let Some(look) = self.looks.of(&self.connection) else {
            return self.call(tool, args).await;
        };
        let bare = args.clone();
        args["icon"] = json!(look.icon);
        args["name"] = json!(look.name);
        let error = match self.call(tool, args).await {
            Ok(answer) => return Ok(answer),
            Err(error) => error,
        };
        // Only a refusal says that nothing was posted; after any other
        // failure a second post could double the first.
        if !error
            .downcast_ref::<Refused>()
            .is_some_and(|r| NO_LOOK.contains(&r.why.as_str()))
        {
            return Err(error);
        }
        let answer = self.call(tool, bare).await?;
        tracing::warn!(
            connection = %self.connection,
            %error,
            "the workspace refuses the secretary's icon and name; posting without them until the app is installed again"
        );
        self.looks.refuse(&self.connection);
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
            .post_with_look("post_message", message_args(channel, message))
            .await?;
        Ok(answer
            .get("ts")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    async fn post_ephemeral(
        &self,
        channel: &str,
        user: &str,
        message: Outgoing,
    ) -> anyhow::Result<()> {
        let mut args = message_args(channel, message);
        args["user"] = json!(user);
        self.post_with_look("post_ephemeral", args).await?;
        Ok(())
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
    use crate::persona::Gender;
    use crate::store::Store;

    const SLACK: &str = include_str!("../../../gateway/services/slack/slack.toml");

    type Seen = Arc<Mutex<Vec<(String, Value)>>>;

    async fn slack_api(answer: Value) -> (String, Seen) {
        slack_api_with(move |_, _| answer.clone()).await
    }

    /// A Slack that answers each call as `answer` says, and what it was asked.
    async fn slack_api_with(
        answer: impl Fn(&str, &Value) -> Value + Clone + Send + Sync + 'static,
    ) -> (String, Seen) {
        let seen: Seen = Arc::default();
        let app = axum::Router::new()
            .route(
                "/api/{method}",
                axum::routing::post(
                    |axum::extract::State(seen): axum::extract::State<Seen>,
                     axum::extract::Path(method): axum::extract::Path<String>,
                     body: String| async move {
                        let body = serde_json::from_str(&body).unwrap_or(Value::Null);
                        let answer = answer(&method, &body);
                        seen.lock().unwrap().push((method, body));
                        answer.to_string()
                    },
                ),
            )
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, seen)
    }

    fn slack(base: &str) -> GatewaySlack {
        slack_with(base, Looks::new(Store::open_in_memory().unwrap()))
    }

    fn slack_with(base: &str, looks: Looks) -> GatewaySlack {
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
        GatewaySlack::new(gateway, connections, "slack", looks)
    }

    fn says(text: &str) -> Outgoing {
        Outgoing {
            text: text.into(),
            ..Default::default()
        }
    }

    const MALE: &str =
        "https://raw.githubusercontent.com/lambdalisue/famulen/main/brand/male/avatar-512.png";
    const FEMALE: &str =
        "https://raw.githubusercontent.com/lambdalisue/famulen/main/brand/female/avatar-512.png";

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
            json!({ "channel": "D1", "text": "hi", "thread_ts": "1.0", "icon_url": MALE, "username": "ファム" })
        );
    }

    #[tokio::test]
    async fn an_ephemeral_post_names_the_one_who_sees_it_and_its_thread() {
        let (base, seen) = slack_api(json!({ "ok": true, "message_ts": "1.5" })).await;

        let message = Outgoing {
            thread_ts: Some("1.0".into()),
            ..says("hi")
        };
        slack(&base)
            .post_ephemeral("C1", "U1", message)
            .await
            .unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "chat.postEphemeral");
        assert_eq!(
            seen[0].1,
            json!({ "channel": "C1", "user": "U1", "text": "hi", "thread_ts": "1.0", "icon_url": MALE, "username": "ファム" })
        );
    }

    #[tokio::test]
    async fn a_posts_icon_follows_the_gender_chosen_at_that_moment() {
        let (base, seen) = slack_api(json!({ "ok": true, "ts": "1.5" })).await;
        let store = Store::open_in_memory().unwrap();
        let slack = slack_with(&base, Looks::new(store.clone()));

        slack.post("D1", says("before")).await.unwrap();
        Gender::Female.save(&store).unwrap();
        slack.post("D1", says("after")).await.unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].1["icon_url"], MALE);
        assert_eq!(seen[1].1["icon_url"], FEMALE);
    }

    #[tokio::test]
    async fn a_posts_name_is_the_secretarys_at_that_moment() {
        let (base, seen) = slack_api(json!({ "ok": true, "ts": "1.5" })).await;
        let store = Store::open_in_memory().unwrap();
        let slack = slack_with(&base, Looks::new(store.clone()));

        store.set_state("persona.name", "ミナ").unwrap();
        slack.post("D1", says("before")).await.unwrap();
        store.set_state("persona.name", "レン").unwrap();
        slack.post("D1", says("after")).await.unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].1["username"], "ミナ");
        assert_eq!(seen[1].1["username"], "レン");
    }

    #[test]
    fn the_icons_posted_are_files_of_the_brand() {
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (gender, url) in [(Gender::Male, MALE), (Gender::Female, FEMALE)] {
            let store = Store::open_in_memory().unwrap();
            gender.save(&store).unwrap();
            assert_eq!(Looks::new(store).of("slack").unwrap().icon, url);
            let (_, path) = url.split_once("/main/").unwrap();
            assert!(repository.join(path).is_file(), "{path}");
        }
    }

    #[tokio::test]
    async fn an_update_carries_no_icon_and_no_name() {
        let (base, seen) = slack_api(json!({ "ok": true })).await;

        slack(&base).update("D1", "1.0", says("hi")).await.unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].1.get("icon_url"), None);
        assert_eq!(seen[0].1.get("username"), None);
    }

    /// A workspace the app was installed in before it asked to set icons and names.
    async fn without_the_scope() -> (String, Seen) {
        slack_api_with(|_, body| match body.get("icon_url").or(body.get("username")) {
            Some(_) => {
                json!({ "ok": false, "error": "missing_scope", "needed": "chat:write.customize" })
            }
            None => json!({ "ok": true, "ts": "1.5" }),
        })
        .await
    }

    fn looks_sent(seen: &Seen) -> Vec<bool> {
        let seen = seen.lock().unwrap();
        seen.iter()
            .map(|(_, b)| {
                let icon = b.get("icon_url").is_some();
                assert_eq!(icon, b.get("username").is_some(), "both or neither");
                icon
            })
            .collect()
    }

    #[tokio::test]
    async fn a_workspace_without_the_scope_gets_each_message_once_and_is_not_offered_the_look_again()
     {
        let (base, seen) = without_the_scope().await;
        let slack = slack(&base);

        let first = slack.post("D1", says("one")).await.unwrap();
        slack.clone().post("D1", says("two")).await.unwrap();

        assert_eq!(first, "1.5");
        assert_eq!(looks_sent(&seen), [true, false, false]);
    }

    #[tokio::test]
    async fn a_workspace_installed_again_is_offered_the_look_again() {
        let (base, seen) = without_the_scope().await;
        let looks = Looks::new(Store::open_in_memory().unwrap());
        let slack = slack_with(&base, looks.clone());

        slack.post("D1", says("one")).await.unwrap();
        looks.installed("slack");
        slack.post("D1", says("two")).await.unwrap();

        assert_eq!(looks_sent(&seen), [true, false, true, false]);
    }

    #[tokio::test]
    async fn a_refusal_that_is_not_about_the_look_is_not_posted_again() {
        let (base, seen) = slack_api(json!({ "ok": false, "error": "channel_not_found" })).await;

        let error = slack(&base).post("D1", says("hi")).await.unwrap_err();

        assert_eq!(error.to_string(), "post_message failed: channel_not_found");
        assert_eq!(looks_sent(&seen), [true]);
    }

    #[tokio::test]
    async fn a_failure_that_does_not_say_whether_it_posted_is_not_posted_again() {
        let (base, seen) = slack_api(json!("gateway timeout missing_scope")).await;

        slack(&base).post("D1", says("hi")).await.unwrap_err();

        assert_eq!(looks_sent(&seen), [true]);
    }

    #[tokio::test]
    async fn a_workspace_that_refuses_the_bare_message_too_keeps_its_look() {
        let (base, seen) = slack_api(json!({ "ok": false, "error": "missing_scope" })).await;
        let slack = slack(&base);

        slack.post("D1", says("one")).await.unwrap_err();
        slack.post("D1", says("two")).await.unwrap_err();

        assert_eq!(looks_sent(&seen), [true, false, true, false]);
    }

    #[tokio::test]
    async fn slacks_refusal_is_an_error_that_says_why() {
        let (base, _) = slack_api(json!({ "ok": false, "error": "channel_not_found" })).await;

        let error = slack(&base).react("C1", "1.0", "eyes").await.unwrap_err();

        assert_eq!(error.to_string(), "add_reaction failed: channel_not_found");
    }
}
