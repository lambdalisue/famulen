//! Connections made from service definitions: their apps, logins and tools
//! come from the definition, and the gateway only fills in and sends.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{Gateway, OWNER};
use crate::connections::{Connection, Secret, Transport};
use crate::definitions::Definition;
use crate::mcp::ServerConfig;
use crate::template::Context;

mod http;
mod login;

/// How the gateway names itself to the services it calls; some refuse a
/// request that does not say.
const AGENT: &str = concat!("famulen-gateway/", env!("CARGO_PKG_VERSION"));

impl Gateway {
    /// The definition a connection is made from, or why it cannot be used.
    pub fn definition_of(&self, connection: &Connection) -> Option<Result<Definition, String>> {
        let Transport::Defined { definition, .. } = &connection.transport else {
            return None;
        };
        let defs = crate::lock::read(&self.definitions);
        Some(match defs.get(definition) {
            Some(def) => Ok(def.clone()),
            None => Err(defs.why_not(definition).unwrap_or_default()),
        })
    }

    /// The definition a connection is made from, or an error saying why
    /// there is none to use.
    pub(super) fn require_definition(&self, connection: &Connection) -> anyhow::Result<Definition> {
        match self.definition_of(connection) {
            Some(Ok(def)) => Ok(def),
            Some(Err(why)) => anyhow::bail!("{why}"),
            None => anyhow::bail!("{} is not made from a definition", connection.id),
        }
    }

    /// The definitions this gateway can use.
    pub fn definitions(&self) -> crate::definitions::Definitions {
        crate::lock::read(&self.definitions).clone()
    }

    /// Add a connection made from a definition, with the account's values.
    pub fn add_defined(
        &self,
        definition: &str,
        values: BTreeMap<String, String>,
        name: Option<&str>,
    ) -> anyhow::Result<Connection> {
        let def = self
            .definitions()
            .get(definition)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no service definition {definition}"))?;
        for field in &def.fields {
            anyhow::ensure!(
                field.optional
                    || values
                        .get(&field.name)
                        .is_some_and(|v| !v.trim().is_empty()),
                "{} is required",
                field.label
            );
        }
        let (secret_values, values): (BTreeMap<_, _>, BTreeMap<_, _>) = values
            .into_iter()
            .filter(|(_, v)| !v.trim().is_empty())
            .partition(|(k, _)| def.fields.iter().any(|f| f.secret && &f.name == k));
        let id = self.fresh_id(&def.id)?;
        let connection = self.connections.add(
            &id,
            name.unwrap_or(&def.name),
            &def.id,
            Transport::Defined {
                definition: def.id.clone(),
                values,
            },
            Secret::default(),
            "",
        )?;
        if !secret_values.is_empty() {
            let secret = Secret {
                account: secret_values,
                ..Secret::default()
            };
            self.sealed().set_secret(&id, &secret)?;
        }
        self.refresh_display(&connection.id)?;
        self.log_change_by(OWNER, &id, "connection.add", &def.id)?;
        self.connections
            .get(&connection.id)?
            .ok_or_else(|| anyhow::anyhow!("connection {id} vanished"))
    }

    /// Set one of a connection's account values after adding it, as `actor`;
    /// a field the definition marks secret goes to the vault. An empty
    /// value clears it. Logged by the field's name only.
    pub async fn set_account_value(
        &self,
        actor: &str,
        id: &str,
        name: &str,
        value: &str,
    ) -> anyhow::Result<()> {
        let connection = self.sealed().require(id)?;
        let def = self.require_definition(&connection)?;
        let field = def
            .fields
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| anyhow::anyhow!("{} has no account field {name}", def.name))?;
        let value = value.trim();
        if field.secret {
            let mut secret = connection.secret.clone();
            if value.is_empty() {
                secret.account.remove(name);
            } else {
                secret.account.insert(name.to_owned(), value.to_owned());
            }
            self.sealed().set_secret(id, &secret)?;
        } else {
            let Transport::Defined {
                definition,
                mut values,
            } = connection.transport
            else {
                unreachable!("a connection with a definition is made from it");
            };
            if value.is_empty() {
                values.remove(name);
            } else {
                values.insert(name.to_owned(), value.to_owned());
            }
            self.connections
                .set_transport(id, &Transport::Defined { definition, values })?;
        }
        self.refresh_display(id)?;
        self.clients.lock().await.remove(id);
        self.log_change_by(actor, id, "account.set", name)?;
        if def.login.as_ref().is_some_and(|l| l.session.is_some()) {
            self.open_session(id).await?;
        }
        Ok(())
    }

    /// The names of a connection's account values that are set, secret
    /// ones included, without opening any.
    pub fn account_names(&self, id: &str) -> anyhow::Result<BTreeSet<String>> {
        let connection = self.connections.require(id)?;
        let Transport::Defined { values, .. } = &connection.transport else {
            return Ok(BTreeSet::new());
        };
        let sealed = connection
            .secret
            .in_vault
            .iter()
            .filter_map(|k| k.strip_prefix("account."));
        Ok(values
            .keys()
            .chain(connection.secret.account.keys())
            .map(String::as_str)
            .chain(sealed)
            .map(str::to_owned)
            .collect())
    }

    /// A connection's account values: those it was added with and those its
    /// login brought. Never its tokens or the app's secrets.
    pub fn account_values(&self, id: &str) -> anyhow::Result<BTreeMap<String, String>> {
        let connection = self.sealed().require(id)?;
        match self.definition_of(&connection) {
            Some(Ok(def)) => Ok(self.context(&connection, &def, Value::Null, false)?.account),
            Some(Err(why)) => anyhow::bail!("{why}"),
            None => Ok(BTreeMap::new()),
        }
    }

    /// Put the definition's `display` into the connection's account.
    fn refresh_display(&self, id: &str) -> anyhow::Result<()> {
        let Some(connection) = self.sealed().get(id)? else {
            return Ok(());
        };
        let Some(Ok(def)) = self.definition_of(&connection) else {
            return Ok(());
        };
        if let Some(display) = &def.display {
            let ctx = self.context(&connection, &def, Value::Null, false)?;
            if let Some(text) = ctx.render_text(display) {
                self.connections.update(id, None, None, Some(&text))?;
            }
        }
        Ok(())
    }

    /// The app's values from the environment, or the message to show when
    /// a required one is not set.
    pub fn app_values(
        &self,
        def: &Definition,
    ) -> Result<(BTreeMap<String, String>, Vec<String>), String> {
        let Some(app) = &def.app_spec else {
            return Ok(Default::default());
        };
        let mut values = BTreeMap::new();
        let mut secrets = Vec::new();
        let mut missing = Vec::new();
        for v in &app.values {
            match (self.env)(&v.env) {
                Some(value) => {
                    values.insert(v.name.clone(), value);
                }
                None if v.optional => {}
                None => missing.push(v.env.clone()),
            }
            if v.secret {
                secrets.push(v.name.clone());
            }
        }
        if !missing.is_empty() {
            return Err(format!(
                "{} のアプリが設定されていない。{} を入れる（{}）",
                def.name,
                missing.join("・"),
                def.setup_place().unwrap_or_default()
            ));
        }
        Ok((values, secrets))
    }

    /// What a connection's definition strings may be filled with.
    pub(super) fn context(
        &self,
        connection: &Connection,
        def: &Definition,
        args: Value,
        secrets: bool,
    ) -> anyhow::Result<Context> {
        let Transport::Defined { values, .. } = &connection.transport else {
            anyhow::bail!("{} is not made from a definition", connection.id);
        };
        let mut account = values.clone();
        if secrets {
            account.extend(connection.secret.account.clone());
        }
        let (app, app_secrets) = self.app_values(def).unwrap_or_default();
        let mut tokens = connection.secret.tokens.clone();
        let login = def.login.as_ref();
        let first_token = if login.is_some_and(|l| l.key.is_some()) {
            if let Some(key) = &connection.secret.api_key {
                tokens.insert("key".into(), key.clone());
            }
            Some("key".to_owned())
        } else {
            login
                .and_then(|l| match (&l.oauth2, &l.session) {
                    (Some(o), _) => o.token_order.first().cloned(),
                    (_, Some(s)) => s.token_order.first().cloned(),
                    _ => None,
                })
                .or_else(|| Some("access_token".to_owned()))
        };
        Ok(Context {
            account,
            app,
            app_secrets,
            tokens,
            first_token,
            args,
            secrets,
            ..Default::default()
        })
    }

    /// The MCP client config of a connection made from a definition.
    pub(super) fn defined_server_config(&self, connection: &Connection) -> Option<ServerConfig> {
        let Ok(def) = self.definition_of(connection)? else {
            return None;
        };
        let ctx = self.context(connection, &def, Value::Null, true).ok()?;
        if let Some(local) = &def.transport.mcp_local {
            return Some(ServerConfig::Stdio {
                command: ctx.render_text(&local.command)?,
                args: local
                    .args
                    .iter()
                    .filter_map(|a| ctx.render_text(a))
                    .collect(),
                env: ctx.render_map(&local.env),
            });
        }
        let remote = def.transport.mcp_remote.as_ref()?;
        let mut headers = ctx.render_map(&remote.headers);
        headers.extend(self.auth_header(&def, &ctx, None));
        Some(ServerConfig::Http {
            url: ctx.render_text(&remote.url)?,
            headers,
        })
    }

    /// The header a request carries, per the definition's login.
    fn auth_header(
        &self,
        def: &Definition,
        ctx: &Context,
        auth: Option<&str>,
    ) -> Option<(String, String)> {
        let login = def.login.as_ref()?;
        if let Some(key) = &login.key {
            return Some((key.header.clone(), ctx.render_text(&key.value)?));
        }
        if login.oauth2.is_none() && login.session.is_none() {
            return None;
        }
        let name = auth
            .map(str::to_owned)
            .or_else(|| ctx.first_token.clone())?;
        let token = ctx.tokens.get(&name)?;
        Some(("Authorization".into(), format!("Bearer {token}")))
    }

    /// Put values a login found onto the connection's account values.
    fn merge_values(&self, id: &str, found: BTreeMap<String, String>) -> anyhow::Result<()> {
        if found.is_empty() {
            return Ok(());
        }
        let Some(connection) = self.connections.get(id)? else {
            return Ok(());
        };
        let Transport::Defined {
            definition,
            mut values,
        } = connection.transport
        else {
            return Ok(());
        };
        values.extend(found);
        self.connections
            .set_transport(id, &Transport::Defined { definition, values })
    }
}

#[cfg(test)]
mod tests;
