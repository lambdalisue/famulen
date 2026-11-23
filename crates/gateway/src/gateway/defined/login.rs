//! Logging in as a definition says: through a registered app's OAuth 2.0, or
//! by trading the account's values for a token; and keeping the tokens fresh.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::super::logins::query_param;
use super::super::{GATEWAY, Gateway, OWNER};
use super::AGENT;
use super::http::Outgoing;
use crate::connections::{CONNECTED, Connection, LOGIN_AGAIN, Secret, Transport};
use crate::definitions::{Definition, OAuth2};
use crate::mcp::{Pkce, generate_state};
use crate::store::now;
use crate::template::Context;

/// How long to wait after a session could not be opened again.
const RETRY_SECS: i64 = 900;

/// How long before they end tokens are asked for again.
const BEFORE_SECS: i64 = 300;

/// The state key a login made from a definition waits under.
fn pending_key(state: &str) -> String {
    format!("defined-login.{state}")
}

impl Gateway {
    /// Start a login a definition's OAuth 2.0 describes: the URL to open.
    pub fn begin_defined_login(&self, id: &str, redirect_uri: &str) -> anyhow::Result<String> {
        let connection = self.connections.require(id)?;
        let def = self.require_definition(&connection)?;
        let oauth = def
            .login
            .as_ref()
            .and_then(|l| l.oauth2.as_ref())
            .ok_or_else(|| anyhow::anyhow!("{} has no OAuth 2.0 login", def.id))?;
        let (app, _) = self.app_values(&def).map_err(|m| anyhow::anyhow!(m))?;
        let client_id = app
            .get("client_id")
            .ok_or_else(|| anyhow::anyhow!("{} has no client_id", def.id))?;
        let pkce = Pkce::generate();
        let state = generate_state();
        self.store.set_state(
            pending_key(&state),
            json!({ "connection": id, "verifier": pkce.verifier, "redirect_uri": redirect_uri })
                .to_string(),
        )?;
        let mut url = url::Url::parse(&oauth.authorize)?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("response_type", "code")
                .append_pair("client_id", client_id)
                .append_pair("redirect_uri", redirect_uri)
                .append_pair("state", &state)
                .append_pair("code_challenge", &pkce.challenge)
                .append_pair("code_challenge_method", "S256");
            if !oauth.scopes.is_empty() {
                q.append_pair("scope", &oauth.scopes.join(" "));
            }
            for (k, v) in &oauth.query {
                q.append_pair(k, v);
            }
        }
        Ok(url.into())
    }

    /// Whether `query` is the answer to a login a definition started.
    pub fn is_defined_login(&self, query: &str) -> anyhow::Result<bool> {
        Ok(match query_param(query, "state") {
            Some(state) => self.store.state(pending_key(&state))?.is_some(),
            None => false,
        })
    }

    /// Finish a login a definition started: the connection it logged in.
    /// When the account is one already connected (`same`), that connection
    /// takes the new tokens and the new one is removed.
    pub async fn finish_defined_login(&self, query: &str) -> anyhow::Result<String> {
        if let Some(error) = query_param(query, "error") {
            anyhow::bail!("the service did not log in: {error}");
        }
        let state = query_param(query, "state").ok_or_else(|| anyhow::anyhow!("no state"))?;
        let code = query_param(query, "code").ok_or_else(|| anyhow::anyhow!("no code"))?;
        let pending: Value = serde_json::from_str(
            &self
                .store
                .state(pending_key(&state))?
                .ok_or_else(|| anyhow::anyhow!("this login was not started here"))?,
        )?;
        self.store.clear_state(pending_key(&state))?;
        let id = pending["connection"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let mut connection = self.sealed().require(&id)?;
        let def = self.require_definition(&connection)?;
        let oauth = def
            .login
            .as_ref()
            .and_then(|l| l.oauth2.clone())
            .ok_or_else(|| anyhow::anyhow!("{} has no OAuth 2.0 login", def.id))?;
        let (app, _) = self.app_values(&def).map_err(|m| anyhow::anyhow!(m))?;
        let text = |name: &str| pending[name].as_str().unwrap_or_default().to_owned();
        let answer = self
            .ask_for_tokens(
                &oauth,
                &app,
                vec![
                    ("grant_type", "authorization_code".to_owned()),
                    ("code", code),
                    ("redirect_uri", text("redirect_uri")),
                    ("code_verifier", text("verifier")),
                ],
            )
            .await?;
        let mut secret = connection.secret.clone();
        let mut found_values = self.take_tokens(&oauth, &answer, &mut secret)?;
        connection.secret = secret.clone();
        found_values.extend(self.ask_whoami(&connection, &def).await?);
        let target = self
            .same_account(&def, &oauth.same, &found_values, &id)?
            .unwrap_or(id.clone());
        self.sealed().set_secret(&target, &secret)?;
        self.merge_values(&target, found_values)?;
        self.connections.set_status(&target, CONNECTED)?;
        if target != id {
            self.remove_connection(GATEWAY, &id)?;
        }
        self.refresh_display(&target)?;
        self.log_change_by(OWNER, &target, "login", &def.id)?;
        Ok(target)
    }

    /// Ask the token endpoint for what `form` says, as the registered app.
    async fn ask_for_tokens(
        &self,
        oauth: &OAuth2,
        app: &BTreeMap<String, String>,
        mut form: Vec<(&'static str, String)>,
    ) -> anyhow::Result<Value> {
        for name in ["client_id", "client_secret"] {
            if let Some(value) = app.get(name) {
                form.push((name, value.clone()));
            }
        }
        Ok(self
            .http
            .post(&oauth.token)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::USER_AGENT, AGENT)
            .form(&form)
            .send()
            .await?
            .json()
            .await?)
    }

    /// The account values the definition's `whoami` request tells.
    async fn ask_whoami(
        &self,
        connection: &Connection,
        def: &Definition,
    ) -> anyhow::Result<BTreeMap<String, String>> {
        let Some(whoami) = def.login.as_ref().and_then(|l| l.whoami.as_ref()) else {
            return Ok(BTreeMap::new());
        };
        let Some((method, target)) = whoami.request.target() else {
            return Ok(BTreeMap::new());
        };
        let ctx = self.context(connection, def, Value::Null, true)?;
        let (_, text) = self
            .send_defined(
                def,
                &ctx,
                Outgoing {
                    method,
                    target,
                    query: &whoami.request.query,
                    body: whoami.request.body.as_ref(),
                    auth: None,
                    same_host: false,
                },
            )
            .await?;
        let doc = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(Context::of(doc).render_map(&whoami.values))
    }

    /// Put the tokens a token answer holds into `secret`, per the
    /// definition, and give back the account values it names.
    pub(in crate::gateway) fn take_tokens(
        &self,
        oauth: &OAuth2,
        answer: &Value,
        secret: &mut Secret,
    ) -> anyhow::Result<BTreeMap<String, String>> {
        if let Some(error) = answer.get("error") {
            anyhow::bail!("the service refused the login: {error}");
        }
        let found = Context::of(answer.clone());
        if oauth.tokens.is_empty() {
            let token = found
                .render_text("{/access_token}")
                .ok_or_else(|| anyhow::anyhow!("the answer has no access_token"))?;
            secret.tokens.insert("access_token".into(), token);
        } else {
            for (name, at) in &oauth.tokens {
                if let Some(token) = found.render_text(at) {
                    secret.tokens.insert(name.clone(), token);
                }
            }
        }
        let values = found.render_map(&oauth.values);
        if let Some(refresh) = found.render_text("{/refresh_token}") {
            secret.refresh_token = Some(refresh);
        }
        if let Some(Value::Number(n)) = found.render("{/expires_in}") {
            secret.expires_at = n.as_i64().map(|s| now() + s);
        }
        Ok(values)
    }

    /// Another connection of the same definition whose `same` values match.
    fn same_account(
        &self,
        def: &Definition,
        same: &[String],
        found: &BTreeMap<String, String>,
        except: &str,
    ) -> anyhow::Result<Option<String>> {
        if same.is_empty() {
            return Ok(None);
        }
        let mine: Vec<Option<&String>> = same.iter().map(|k| found.get(k)).collect();
        if mine.iter().any(Option::is_none) {
            return Ok(None);
        }
        for c in self.connections.list()? {
            if c.id == except || c.kind != def.id {
                continue;
            }
            let Transport::Defined { values, .. } = &c.transport else {
                continue;
            };
            let theirs: Vec<Option<&String>> = same.iter().map(|k| values.get(k)).collect();
            if theirs == mine {
                return Ok(Some(c.id));
            }
        }
        Ok(None)
    }

    /// Trade the account's values for the tokens its definition's session
    /// login describes.
    pub async fn open_session(&self, id: &str) -> anyhow::Result<()> {
        let connection = self.sealed().require(id)?;
        let def = self.require_definition(&connection)?;
        let session = def
            .login
            .as_ref()
            .and_then(|l| l.session.as_ref())
            .ok_or_else(|| anyhow::anyhow!("{} has no session login", def.id))?;
        let ctx = self.context(&connection, &def, Value::Null, true)?;
        let response = self
            .http
            .post(&session.post)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::USER_AGENT, AGENT)
            .json(&ctx.render_value(&session.body).unwrap_or(Value::Null))
            .send()
            .await?;
        let status = response.status();
        let answer: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let why = ["message", "error"]
                .iter()
                .find_map(|k| answer[*k].as_str())
                .unwrap_or_default();
            anyhow::bail!("{} refused the login ({status}): {why}", def.name);
        }
        let found = Context::of(answer);
        let mut secret = connection.secret.clone();
        for (name, at) in &session.tokens {
            let token = found
                .render_text(at)
                .ok_or_else(|| anyhow::anyhow!("the answer has no {name} token"))?;
            secret.tokens.insert(name.clone(), token);
        }
        secret.expires_at = Some(now() + session.lifetime_secs()?);
        self.sealed().set_secret(id, &secret)?;
        self.merge_values(id, found.render_map(&session.values))?;
        self.clients.lock().await.remove(id);
        self.connections.set_status(id, CONNECTED)?;
        self.refresh_display(id)
    }

    /// Refresh the tokens of connections made from definitions whose login
    /// asks for it, before they expire. Whether any changed.
    pub(in crate::gateway) async fn refresh_defined(&self) -> anyhow::Result<bool> {
        let opened = self.refresh_sessions().await?;
        Ok(self.refresh_oauth2().await? || opened)
    }

    /// Open again the sessions about to end.
    async fn refresh_sessions(&self) -> anyhow::Result<bool> {
        let mut changed = false;
        for c in self.connections.list()? {
            let Some(Ok(def)) = self.definition_of(&c) else {
                continue;
            };
            if !def.login.as_ref().is_some_and(|l| l.session.is_some())
                || c.secret
                    .expires_at
                    .is_some_and(|at| at > now() + BEFORE_SECS)
            {
                continue;
            }
            match self.open_session(&c.id).await {
                Ok(()) => changed = true,
                Err(error) => {
                    // Services count failed logins; do not ask every round.
                    let mut secret = c.secret.clone();
                    secret.expires_at = Some(now() + BEFORE_SECS + RETRY_SECS);
                    self.connections.set_secret(&c.id, &secret)?;
                    self.connections
                        .set_status(&c.id, format!("{}（{error}）", LOGIN_AGAIN))?;
                }
            }
        }
        Ok(changed)
    }

    async fn refresh_oauth2(&self) -> anyhow::Result<bool> {
        let mut changed = false;
        for c in self.sealed().list()? {
            let Some(Ok(def)) = self.definition_of(&c) else {
                continue;
            };
            let Some(oauth) = def.login.as_ref().and_then(|l| l.oauth2.clone()) else {
                continue;
            };
            let (Some(refresh), Some(expires_at)) = (&c.secret.refresh_token, c.secret.expires_at)
            else {
                continue;
            };
            if !oauth.refresh || expires_at > now() + BEFORE_SECS {
                continue;
            }
            let Ok((app, _)) = self.app_values(&def) else {
                continue;
            };
            let answer = self
                .ask_for_tokens(
                    &oauth,
                    &app,
                    vec![
                        ("grant_type", "refresh_token".to_owned()),
                        ("refresh_token", refresh.clone()),
                    ],
                )
                .await;
            let mut secret = c.secret.clone();
            match answer.and_then(|a| self.take_tokens(&oauth, &a, &mut secret)) {
                Ok(found) => {
                    self.sealed().set_secret(&c.id, &secret)?;
                    self.merge_values(&c.id, found)?;
                    self.connections.set_status(&c.id, CONNECTED)?;
                    self.clients.lock().await.remove(&c.id);
                    changed = true;
                }
                Err(error) => {
                    self.connections
                        .set_status(&c.id, format!("{}（{error}）", LOGIN_AGAIN))?;
                }
            }
        }
        Ok(changed)
    }
}
