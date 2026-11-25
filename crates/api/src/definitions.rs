//! Service definitions: what each gives, its text, the changes made to it,
//! and what a connection made from one has set.

use famulen_core::gateway::definitions::{Definition, Protocol, Receive};

use crate::pb::*;
use crate::{Api, given};

/// The most changes one listing holds.
const CHANGES: usize = 100;

fn receiver_how(r: &Receive) -> &'static str {
    match (&r.http, &r.connect) {
        (Some(_), _) => "HTTP",
        (_, Some(c)) => match c.protocol {
            Protocol::SlackSocketMode => "slack-socket-mode",
            Protocol::PubsubPull => "pubsub-pull",
            Protocol::CommandLines => "command-lines",
        },
        _ => "",
    }
}

pub(crate) fn login_how(def: &Definition) -> &'static str {
    match def.login.as_ref() {
        Some(l) if l.oauth2.is_some() => "oauth2",
        Some(l) if l.session.is_some() => "session",
        Some(l) if l.key.is_some() => "key",
        Some(_) => "mcp",
        None => "",
    }
}

impl Api {
    /// The definitions the gateway read, and the ones it could not.
    pub(crate) fn definition_list(&self) -> ListDefinitionsResponse {
        let gateway = &self.app.gateway;
        let all = gateway.definitions();
        let definitions = all
            .list()
            .map(|d| ServiceDefinition {
                id: d.id.clone(),
                name: d.name.clone(),
                setup: d.setup_place().unwrap_or_default(),
                app_missing: gateway.app_values(d).err().unwrap_or_default(),
                receivers: d.receive.keys().cloned().collect(),
                tools: d
                    .tools
                    .iter()
                    .map(|t| &t.name)
                    .chain(d.commands.iter().map(|t| &t.name))
                    .cloned()
                    .collect(),
                fields: d
                    .fields
                    .iter()
                    .filter(|f| !f.generated)
                    .map(|f| AccountField {
                        name: f.name.clone(),
                        label: f.label.clone(),
                        optional: f.optional,
                        secret: f.secret,
                        ..Default::default()
                    })
                    .collect(),
                login: login_how(d).into(),
                key_label: d
                    .login
                    .as_ref()
                    .and_then(|l| l.key.as_ref())
                    .map(|k| k.label.clone())
                    .unwrap_or_default(),
                only_by_owner: d.only_by_owner().unwrap_or_default().into(),
                about: d.about.clone().unwrap_or_default(),
                ..Default::default()
            })
            .collect();
        let broken = all
            .broken()
            .map(|(file, why)| format!("{file}: {why}"))
            .collect();
        ListDefinitionsResponse {
            definitions,
            broken,
            ..Default::default()
        }
    }

    /// A defined connection's receivers with their destinations, the
    /// destinations to choose from, and its account values with every secret
    /// one left blank.
    pub(crate) fn defined_detail(
        &self,
        id: &str,
        definition: &str,
    ) -> anyhow::Result<(Vec<ReceiverSetting>, Vec<String>, Vec<AccountValue>)> {
        let gateway = &self.app.gateway;
        let Some(def) = gateway.definitions().get(definition).cloned() else {
            return Ok(Default::default());
        };
        let receivers = gateway
            .receivers_of(id)?
            .into_iter()
            .map(|(name, destination)| ReceiverSetting {
                how: def
                    .receive
                    .get(&name)
                    .map(receiver_how)
                    .unwrap_or_default()
                    .into(),
                name,
                destination: destination.unwrap_or_default(),
                ..Default::default()
            })
            .collect();
        let plain = gateway.account_values(id)?;
        let set = gateway.account_names(id)?;
        let mut values: Vec<AccountValue> = def
            .fields
            .iter()
            .filter(|f| !f.generated)
            .map(|f| AccountValue {
                name: f.name.clone(),
                label: f.label.clone(),
                value: if f.secret {
                    String::new()
                } else {
                    plain.get(&f.name).cloned().unwrap_or_default()
                },
                secret: f.secret,
                set: set.contains(&f.name),
                optional: f.optional,
                ..Default::default()
            })
            .collect();
        values.extend(
            plain
                .iter()
                .filter(|(k, _)| def.fields.iter().all(|f| &f.name != *k))
                .map(|(k, v)| AccountValue {
                    name: k.clone(),
                    value: v.clone(),
                    set: true,
                    ..Default::default()
                }),
        );
        Ok((receivers, gateway.destination_names(), values))
    }

    pub(crate) fn definition_text(&self, id: &str) -> anyhow::Result<DefinitionText> {
        let all = self.app.gateway.definitions();
        let def = all
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("{}", all.why_not(id).unwrap_or_default()))?;
        Ok(DefinitionText {
            id: def.id.clone(),
            toml: def.text.clone(),
            source: def.source.clone(),
            ..Default::default()
        })
    }

    pub(crate) fn definition_changes(
        &self,
        definition: &str,
    ) -> anyhow::Result<ListDefinitionChangesResponse> {
        let changes = self
            .app
            .gateway
            .definition_changes(given(definition), CHANGES)?
            .into_iter()
            .map(|c| DefinitionChange {
                id: c.id,
                definition: c.definition,
                kind: c.kind,
                before: c.before.unwrap_or_default(),
                after: c.after.unwrap_or_default(),
                actor: c.actor,
                why: c.why,
                at: c.at,
                ..Default::default()
            })
            .collect();
        Ok(ListDefinitionChangesResponse {
            changes,
            ..Default::default()
        })
    }
}
