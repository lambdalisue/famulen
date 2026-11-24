//! The connections the gateway opens itself: which it should hold open now,
//! and the sockets and subscriptions it takes things from.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};

use super::super::Gateway;
use super::Arrival;
use crate::connections::Connection;
use crate::definitions::{Connect, Definition, Protocol};
use crate::template::Context;

/// Where the gateway's own connections reach the services' receiving ends.
#[derive(Debug, Clone)]
pub(in crate::gateway) struct Endpoints {
    pub(super) socket_open: String,
    pub(super) pubsub: String,
    /// How long a socket may say nothing before it is asked whether it is
    /// alive, and how long it has to answer.
    pub(super) socket_quiet: Duration,
    /// How long to wait before opening a socket again.
    pub(super) socket_retry: Duration,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            socket_open: "https://slack.com/api/apps.connections.open".into(),
            pubsub: "https://pubsub.googleapis.com/v1".into(),
            socket_quiet: Duration::from_secs(30),
            socket_retry: Duration::from_secs(5),
        }
    }
}

/// One connection the gateway opens, told apart from the others: the
/// socket of a token with the receivers on it, or one subscription pulled.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Opened {
    Socket {
        token: String,
        members: Vec<(String, String)>,
    },
    Pull {
        connection: String,
        receiver: String,
    },
    Lines {
        connection: String,
        receiver: String,
    },
}

/// The connections the gateway keeps open, so they can be matched against
/// the receivers sent somewhere as those change.
#[derive(Clone, Default)]
pub(in crate::gateway) struct Running {
    tasks: Arc<std::sync::Mutex<HashMap<Opened, tokio::task::AbortHandle>>>,
}

impl Gateway {
    /// The connections the gateway should hold open now: one socket per
    /// token, one pull per subscription, for every receiver sent somewhere.
    fn wanted(&self) -> anyhow::Result<BTreeSet<Opened>> {
        let mut sockets: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        let mut out = BTreeSet::new();
        for c in self.sealed().list()? {
            let Some(Ok(def)) = self.definition_of(&c) else {
                continue;
            };
            for (name, receive) in &def.receive {
                let Some(connect) = &receive.connect else {
                    continue;
                };
                if self.destination_of(&c.id, name)?.is_none() {
                    continue;
                }
                match connect.protocol {
                    Protocol::SlackSocketMode => {
                        let ctx = self.context(&c, &def, Value::Null, true)?;
                        let Some(token) = connect.token.as_deref().and_then(|t| ctx.render_text(t))
                        else {
                            continue;
                        };
                        sockets
                            .entry(token)
                            .or_default()
                            .push((c.id.clone(), name.clone()));
                    }
                    Protocol::CommandLines => {
                        out.insert(Opened::Lines {
                            connection: c.id.clone(),
                            receiver: name.clone(),
                        });
                    }
                    Protocol::PubsubPull => {
                        out.insert(Opened::Pull {
                            connection: c.id.clone(),
                            receiver: name.clone(),
                        });
                    }
                }
            }
        }
        out.extend(
            sockets
                .into_iter()
                .map(|(token, members)| Opened::Socket { token, members }),
        );
        Ok(out)
    }

    /// Open the connections the definitions ask the gateway to open (Socket
    /// Mode, Pub/Sub pull) for every receiver sent somewhere, and close
    /// those no longer wanted. Call it again whenever connections, logins
    /// or destinations change; what is already open stays open.
    pub fn sync_connections(&self) -> anyhow::Result<()> {
        let wanted = self.wanted()?;
        let mut tasks = crate::lock::lock(&self.running.tasks);
        tasks.retain(|opened, task| {
            let keep = wanted.contains(opened) && !task.is_finished();
            if !keep {
                task.abort();
            }
            keep
        });
        for opened in wanted {
            if tasks.contains_key(&opened) {
                continue;
            }
            let gateway = self.clone();
            let task = match opened.clone() {
                Opened::Socket { token, members } => {
                    tokio::spawn(async move { gateway.socket_forever(&token, &members).await })
                }
                Opened::Pull {
                    connection,
                    receiver,
                } => {
                    tokio::spawn(async move { gateway.pull_forever(&connection, &receiver).await })
                }
                Opened::Lines {
                    connection,
                    receiver,
                } => {
                    tokio::spawn(async move { gateway.lines_forever(&connection, &receiver).await })
                }
            };
            tasks.insert(opened, task.abort_handle());
        }
        Ok(())
    }

    /// Close every connection the gateway opened.
    pub fn close_connections(&self) {
        let mut tasks = crate::lock::lock(&self.running.tasks);
        for (_, task) in tasks.drain() {
            task.abort();
        }
    }

    /// How many connections the gateway holds open.
    #[cfg(test)]
    pub(super) fn open_connections(&self) -> usize {
        crate::lock::lock(&self.running.tasks).len()
    }

    /// Which of `members` (connection and receiver) an event belongs to, by
    /// the values its receiver's `account` points at. One member takes
    /// everything when it is alone and its receiver names no account.
    pub(in crate::gateway) fn route_event(
        &self,
        members: &[(String, String)],
        event: &Value,
    ) -> Option<(String, String)> {
        for (id, receiver) in members {
            let Ok(Some(c)) = self.sealed().get(id) else {
                continue;
            };
            let Some(Ok(def)) = self.definition_of(&c) else {
                continue;
            };
            let Some(connect) = def.receive.get(receiver).and_then(|r| r.connect.as_ref()) else {
                continue;
            };
            if connect.account.is_empty() {
                if members.len() == 1 {
                    return Some((id.clone(), receiver.clone()));
                }
                continue;
            }
            let Ok(mine) = self.context(&c, &def, Value::Null, false) else {
                continue;
            };
            let found = Context::of(event.clone());
            let all = connect.account.iter().all(|(name, at)| {
                let there = at.each().iter().find_map(|at| found.render_text(at));
                there.is_some() && there.as_ref() == mine.account.get(name)
            });
            if all {
                return Some((id.clone(), receiver.clone()));
            }
        }
        None
    }

    pub(super) fn connect_of(
        &self,
        id: &str,
        receiver: &str,
    ) -> anyhow::Result<(Connection, Definition, Connect)> {
        let c = self.sealed().require(id)?;
        let def = self.require_definition(&c)?;
        let connect = def
            .receive
            .get(receiver)
            .and_then(|r| r.connect.clone())
            .ok_or_else(|| anyhow::anyhow!("{id} has no connection {receiver} to open"))?;
        Ok((c, def, connect))
    }

    async fn socket_forever(&self, token: &str, members: &[(String, String)]) {
        loop {
            if let Err(error) = self.socket_once(token, members).await {
                tracing::warn!(%error, "the socket connection failed; opening it again");
            }
            tokio::time::sleep(self.endpoints.socket_retry).await;
        }
    }

    async fn socket_once(&self, token: &str, members: &[(String, String)]) -> anyhow::Result<()> {
        let quiet = self.endpoints.socket_quiet;
        let opening = async {
            let opened: Value = self
                .http
                .post(&self.endpoints.socket_open)
                .bearer_auth(token)
                .send()
                .await?
                .json()
                .await?;
            let url = opened["url"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("the socket was not opened: {}", opened["error"]))?;
            let (ws, _) = tokio_tungstenite::connect_async(url).await?;
            anyhow::Ok(ws)
        };
        let mut ws = tokio::time::timeout(quiet, opening)
            .await
            .map_err(|_| anyhow::anyhow!("the socket did not open in time"))??;
        use tokio_tungstenite::tungstenite::Message;
        // A network that changed under the socket closes nothing: reading
        // alone would wait on the dead connection forever.
        let mut asked = false;
        loop {
            let frame = match tokio::time::timeout(quiet, ws.next()).await {
                Ok(Some(frame)) => frame?,
                Ok(None) => return Ok(()),
                Err(_) if asked => anyhow::bail!("the socket went dead"),
                Err(_) => {
                    ws.send(Message::Ping(Default::default())).await?;
                    asked = true;
                    continue;
                }
            };
            asked = false;
            let text = match frame {
                Message::Text(text) => text,
                Message::Ping(payload) => {
                    ws.send(Message::Pong(payload)).await?;
                    continue;
                }
                Message::Close(_) => return Ok(()),
                _ => continue,
            };
            let event: Value = serde_json::from_str(text.as_str())?;
            if let Some(envelope) = event.get("envelope_id").and_then(Value::as_str) {
                ws.send(Message::Text(
                    json!({ "envelope_id": envelope }).to_string().into(),
                ))
                .await?;
            }
            if event.get("type").and_then(Value::as_str) == Some("disconnect") {
                return Ok(());
            }
            if let Some((connection, receiver)) = self.route_event(members, &event) {
                self.deliver(Arrival::new(&connection, &receiver, event))?;
            }
        }
    }

    async fn pull_forever(&self, id: &str, receiver: &str) {
        loop {
            match self.pull_once(id, receiver).await {
                Ok(0) => tokio::time::sleep(Duration::from_secs(5)).await,
                Ok(_) => {}
                Err(error) => {
                    tracing::debug!(connection = %id, %receiver, %error, "could not pull");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }
        }
    }

    /// Pull once from the subscription of `receiver` of the connection `id`
    /// and hand over what came; how many messages.
    pub(super) async fn pull_once(&self, id: &str, receiver: &str) -> anyhow::Result<usize> {
        let (c, def, connect) = self.connect_of(id, receiver)?;
        let ctx = self.context(&c, &def, Value::Null, true)?;
        let subscription = connect
            .subscription
            .as_deref()
            .and_then(|s| ctx.render_text(s))
            .ok_or_else(|| anyhow::anyhow!("{id} has no subscription"))?;
        let token_name = connect
            .auth
            .clone()
            .or_else(|| ctx.first_token.clone())
            .unwrap_or_default();
        let token = ctx
            .tokens
            .get(&token_name)
            .ok_or_else(|| anyhow::anyhow!("{id} is not logged in"))?;
        let base = self.endpoints.pubsub.trim_end_matches('/');
        let answer: Value = self
            .http
            .post(format!("{base}/{subscription}:pull"))
            .bearer_auth(token)
            .json(&json!({ "maxMessages": 10 }))
            .send()
            .await?
            .json()
            .await?;
        let messages = answer["receivedMessages"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut acks = Vec::new();
        for m in &messages {
            if let Some(ack) = m["ackId"].as_str() {
                acks.push(ack.to_owned());
            }
            self.deliver(Arrival::new(id, receiver, m.clone()))?;
        }
        if !acks.is_empty() {
            self.http
                .post(format!("{base}/{subscription}:acknowledge"))
                .bearer_auth(token)
                .json(&json!({ "ackIds": acks }))
                .send()
                .await?;
        }
        Ok(messages.len())
    }
}
