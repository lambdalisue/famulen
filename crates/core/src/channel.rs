//! The Slack channel of Famulen: every connection made from the bundled Slack
//! definition is a workspace the secretary's bot is in. The owner there is
//! whoever installed the app, as the login says; one workspace is the main
//! one, where approvals are asked.

use crate::app::App;
use crate::connections::{Connection, Transport};
use crate::slack::clients::SlackClients;
use crate::slack::web::{GatewaySlack, Outgoing, Slack};

/// The bundled definition the channel's workspaces are made from.
pub const DEFINITION: &str = "slack";
const GREETED_KEY: &str = "channel.greeted";
const MAIN_KEY: &str = "channel.main";

/// A Slack workspace the bot is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// The connection's id.
    pub id: String,
    pub name: String,
    /// The owner's member id there, from the login.
    pub owner: String,
    pub main: bool,
    /// How its connection stands.
    pub status: String,
}

/// The destination of the receivers the owner talks to the secretary through.
pub const CHANNEL: &str = "channel";

/// A receiver the owner can talk to the secretary through: one whose
/// definition suggests the channel, whatever the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub connection: String,
    pub receiver: String,
    /// The account, such as the workspace's name, or the connection's name.
    pub name: String,
    /// The definition's name, such as Slack.
    pub service: String,
    pub definition: String,
    /// How its connection stands.
    pub status: String,
    /// Whether what arrives there goes to the channel; the connection stays
    /// either way.
    pub enabled: bool,
}

pub(crate) fn is_slack(c: &Connection) -> bool {
    matches!(&c.transport, Transport::Defined { definition, .. } if definition == DEFINITION)
}

/// What a connection is called to the owner: its account, such as a
/// workspace's name, or its own name.
pub(crate) fn account_name(c: &Connection) -> String {
    let name = if c.account.is_empty() {
        &c.name
    } else {
        &c.account
    };
    name.clone()
}

impl App {
    /// Every channel the connected services give, enabled or not.
    pub fn channels(&self) -> anyhow::Result<Vec<Channel>> {
        let definitions = self.gateway.definitions();
        let mut list = Vec::new();
        for c in self.connections.list()? {
            let Transport::Defined { definition, .. } = &c.transport else {
                continue;
            };
            for (receiver, to) in self.gateway.receivers_of(&c.id)? {
                let suggested = self.gateway.suggested_destination(&c.id, &receiver)?;
                if suggested.as_deref() != Some(CHANNEL) {
                    continue;
                }
                list.push(Channel {
                    connection: c.id.clone(),
                    receiver,
                    name: account_name(&c),
                    service: definitions
                        .get(definition)
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| definition.clone()),
                    definition: definition.clone(),
                    status: c.status.clone(),
                    enabled: to.as_deref() == Some(CHANNEL),
                });
            }
        }
        Ok(list)
    }

    /// Take what arrives at one channel again, or stop taking it: the
    /// receiver is sent to the channel or nowhere, and its connection stays.
    pub fn set_channel_enabled(
        &self,
        actor: &str,
        connection: &str,
        receiver: &str,
        enabled: bool,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.channels()?
                .iter()
                .any(|c| c.connection == connection && c.receiver == receiver),
            "{connection} has no channel {receiver}"
        );
        self.gateway
            .set_destination(actor, connection, receiver, enabled.then_some(CHANNEL))?;
        self.gateway.sync_connections()
    }

    /// The workspaces the bot is in, in the order they were added. Exactly
    /// one is main when there are any.
    pub fn workspaces(&self) -> anyhow::Result<Vec<Workspace>> {
        let mut list: Vec<Workspace> = Vec::new();
        for c in self.connections.list()?.into_iter().filter(is_slack) {
            let owner = self
                .gateway
                .account_values(&c.id)
                .ok()
                .and_then(|v| v.get("user").cloned())
                .unwrap_or_default();
            list.push(Workspace {
                name: account_name(&c),
                status: c.status.clone(),
                id: c.id,
                owner,
                main: false,
            });
        }
        let chosen = self.store.state(MAIN_KEY)?;
        let main = chosen
            .filter(|id| list.iter().any(|w| &w.id == id))
            .or_else(|| list.first().map(|w| w.id.clone()));
        for w in &mut list {
            w.main = Some(&w.id) == main.as_ref();
        }
        Ok(list)
    }

    /// Ask approvals in the workspace `id` from now on.
    pub fn set_main_workspace(&self, id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.workspaces()?.iter().any(|w| w.id == id),
            "no workspace {id}"
        );
        self.store.set_state(MAIN_KEY, id)
    }

    /// How a workspace is shown to the owner: its name, or its id when it
    /// is gone.
    pub fn workspace_label(&self, id: &str) -> String {
        self.workspaces()
            .ok()
            .and_then(|all| all.into_iter().find(|w| w.id == id))
            .map(|w| w.name)
            .unwrap_or_else(|| id.to_owned())
    }

    /// Greet the owner in a DM of the workspace `id`, the first time it is
    /// connected. One that could not be sent is tried at the next login.
    pub async fn greet(&self, id: &str) -> anyhow::Result<()> {
        let Some(workspace) = self.workspaces()?.into_iter().find(|w| w.id == id) else {
            return Ok(());
        };
        let team = self
            .gateway
            .account_values(id)?
            .get("team")
            .cloned()
            .unwrap_or_default();
        let key = format!("{GREETED_KEY}.{team}.{}", workspace.owner);
        if workspace.owner.is_empty() || self.store.state(&key)?.is_some() {
            return Ok(());
        }
        let slack = self.slack_client(id);
        let channel = slack.open_dm(&workspace.owner).await?;
        let text = format!(
            "はじめまして、{}です。このワークスペースにつながりました。\nこの DM か、チャンネルでのメンションで、いつでもお声がけください。",
            self.aide_name()
        );
        slack
            .post(
                &channel,
                Outgoing {
                    text,
                    ..Default::default()
                },
            )
            .await?;
        self.store.set_state(&key, "1")?;
        Ok(())
    }

    fn slack_client(&self, id: &str) -> GatewaySlack {
        GatewaySlack::new(
            self.gateway.clone(),
            self.connections.clone(),
            id,
            self.slack_icons.clone(),
        )
    }

    /// A client per workspace, posting through the gateway, with the main
    /// one named; none without a workspace.
    pub fn slack_clients(&self) -> anyhow::Result<Option<SlackClients<GatewaySlack>>> {
        let list = self.workspaces()?;
        let Some(main) = list.iter().find(|w| w.main) else {
            return Ok(None);
        };
        let clients = list
            .iter()
            .map(|w| (w.id.clone(), self.slack_client(&w.id)));
        Ok(Some(SlackClients::new(clients, main.id.clone())?))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #[test]
    fn each_slack_connection_is_a_workspace_owned_by_its_installer_and_one_is_main() {
        let dir = crate::notebook::testing::TempDir::new("channel-workspaces");
        let app = crate::runtime::testing::app(dir.path());
        for user in ["U1", "U2"] {
            app.gateway
                .add_defined(
                    super::DEFINITION,
                    [("user".to_owned(), user.to_owned())].into(),
                    None,
                )
                .unwrap();
        }
        app.gateway
            .add_defined("google", Default::default(), None)
            .unwrap();

        let first = app.workspaces().unwrap();
        app.set_main_workspace("slack-2").unwrap();
        let then = app.workspaces().unwrap();

        let owners: Vec<&str> = first.iter().map(|w| w.owner.as_str()).collect();
        assert_eq!(
            owners,
            ["U1", "U2"],
            "only Slack connections are workspaces"
        );
        assert!(first[0].main && !first[1].main);
        assert!(!then[0].main && then[1].main);
        assert!(app.set_main_workspace("google").is_err());
        assert_eq!(app.slack_clients().unwrap().unwrap().main_id(), "slack-2");
    }

    #[tokio::test]
    async fn the_owner_is_greeted_once_in_a_dm_of_each_workspace_they_connect() {
        use serde_json::{Value, json};
        use std::sync::{Arc, Mutex};
        type Seen = Arc<Mutex<Vec<(String, Value)>>>;
        let seen: Seen = Arc::default();
        let working = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let router = axum::Router::new()
            .route(
                "/api/{method}",
                axum::routing::post(
                    |axum::extract::State((seen, working)): axum::extract::State<(
                        Seen,
                        Arc<std::sync::atomic::AtomicBool>,
                    )>,
                     axum::extract::Path(method): axum::extract::Path<String>,
                     body: String| async move {
                        if !working.load(std::sync::atomic::Ordering::Relaxed) {
                            return json!({ "ok": false, "error": "fatal_error" }).to_string();
                        }
                        seen.lock()
                            .unwrap()
                            .push((method, serde_json::from_str(&body).unwrap()));
                        json!({ "ok": true, "ts": "1.0", "channel": { "id": "D1" } }).to_string()
                    },
                ),
            )
            .with_state((seen.clone(), working.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let dir = crate::notebook::testing::TempDir::new("channel-greeting");
        let services = dir.path().join("gateway/services");
        std::fs::create_dir_all(&services).unwrap();
        let bundled = include_str!("../../gateway/services/slack/slack.toml");
        std::fs::write(
            services.join("slack.toml"),
            bundled.replace("https://slack.com/api", &format!("{base}/api")),
        )
        .unwrap();
        let app = crate::runtime::testing::app(dir.path());
        app.store.set_state("persona.name", "ミナ").unwrap();
        for team in ["T1", "T2"] {
            let values = [("team", team), ("user", "U1")].map(|(k, v)| (k.into(), v.into()));
            app.gateway
                .add_defined(super::DEFINITION, values.into(), None)
                .unwrap();
        }
        app.gateway
            .add_defined("google", Default::default(), None)
            .unwrap();

        assert!(app.greet("slack").await.is_err(), "Slack is down");
        working.store(true, std::sync::atomic::Ordering::Relaxed);
        for id in ["slack", "slack", "google", "slack-2"] {
            app.greet(id).await.unwrap();
        }

        let seen = seen.lock().unwrap();
        let methods: Vec<&str> = seen.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(
            methods,
            [
                "conversations.open",
                "chat.postMessage",
                "conversations.open",
                "chat.postMessage"
            ],
            "a failed greeting is tried again, a done one is not"
        );
        assert_eq!(seen[0].1["users"], "U1");
        assert_eq!(seen[1].1["channel"], "D1");
        assert!(seen[1].1["text"].as_str().unwrap().contains("ミナ"));
        assert!(
            seen[1].1["icon_url"]
                .as_str()
                .unwrap()
                .ends_with("/male/avatar-512.png")
        );
    }

    /// A definition of the owner's own whose receiver suggests the channel,
    /// as a pair of glasses would.
    const GLASSES: &str = "format = 1\nid = \"glasses\"\nname = \"Glasses\"\n[transport]\nhttp = { base = \"https://glasses.example\" }\n[receive.said]\ndestination = \"channel\"\n[receive.said.http]\nverify = { token = { value = \"{header.X-Token}\", equals = \"{account.token}\" } }\n[receive.seen]\n[receive.seen.http]\nverify = { token = { value = \"{header.X-Token}\", equals = \"{account.token}\" } }\n";

    pub(crate) fn app_with_glasses(dir: &std::path::Path) -> crate::app::App {
        let services = dir.join("gateway").join("services");
        std::fs::create_dir_all(&services).unwrap();
        std::fs::write(services.join("glasses.toml"), GLASSES).unwrap();
        crate::runtime::testing::app(dir)
    }

    #[tokio::test]
    async fn every_receiver_suggesting_the_channel_is_a_channel_whatever_its_service() {
        let dir = crate::notebook::testing::TempDir::new("channel-list");
        let app = app_with_glasses(dir.path());
        let slack = app
            .gateway
            .add_defined(super::DEFINITION, Default::default(), None)
            .unwrap();
        let glasses = app
            .gateway
            .add_defined("glasses", Default::default(), None)
            .unwrap();
        app.gateway
            .add_defined("google", Default::default(), None)
            .unwrap();
        crate::runtime::sync_receivers(&app).unwrap();

        let channels = app.channels().unwrap();

        let mut listed: Vec<(&str, &str, &str, bool)> = channels
            .iter()
            .map(|c| {
                (
                    c.service.as_str(),
                    c.connection.as_str(),
                    c.receiver.as_str(),
                    c.enabled,
                )
            })
            .collect();
        listed.sort();
        assert_eq!(
            listed,
            [
                ("Glasses", glasses.id.as_str(), "said", true),
                ("Slack", slack.id.as_str(), "events", true),
            ],
            "Google's receivers and the receiver suggesting nothing are not channels"
        );
        app.gateway.close_connections();
    }

    #[tokio::test]
    async fn a_disabled_channel_sends_its_receiver_nowhere_and_keeps_the_connection() {
        let dir = crate::notebook::testing::TempDir::new("channel-disable");
        let app = crate::runtime::testing::app(dir.path());
        let slack = app
            .gateway
            .add_defined(super::DEFINITION, Default::default(), None)
            .unwrap();
        crate::runtime::sync_receivers(&app).unwrap();

        app.set_channel_enabled(crate::gateway::OWNER, &slack.id, "events", false)
            .unwrap();
        crate::runtime::sync_receivers(&app).unwrap();
        let off = app.channels().unwrap();
        let closed = app.gateway.destination_of(&slack.id, "events").unwrap();
        app.set_channel_enabled(crate::gateway::OWNER, &slack.id, "events", true)
            .unwrap();
        let on = app.gateway.destination_of(&slack.id, "events").unwrap();

        assert!(!off[0].enabled, "a disabled channel is still listed");
        assert_eq!(closed, None, "and stays closed after the receivers sync");
        assert!(app.connections.get(&slack.id).unwrap().is_some());
        assert_eq!(on.as_deref(), Some(super::CHANNEL));
        assert!(
            app.set_channel_enabled(crate::gateway::OWNER, &slack.id, "nope", true)
                .is_err()
        );
        app.gateway.close_connections();
    }
}
