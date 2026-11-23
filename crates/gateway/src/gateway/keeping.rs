//! Keeping the vault for the owner and the callers: listing, putting in and
//! taking out secrets, each change logged. Nothing here hands a value back.

use serde_json::{Value, json};

use crate::log::LogEntry;
use crate::permissions::Permission;

use super::judge::{Call, Outcome, Ticket, Verdict, is_held_entry};
use super::model_keys::model_of;
use super::secrets::connection_of;
use super::{Gateway, Vault, VaultEntry};

/// The name the vault goes by when its changes are judged and logged.
pub const VAULT: &str = "vault";
/// The vault's changes, as tools whose permissions the owner can choose.
pub const PUT_SECRET: &str = "put_secret";
pub const DESCRIBE_SECRET: &str = "describe_secret";
pub const DELETE_SECRET: &str = "delete_secret";

/// What a secret the gateway put in itself belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holder {
    /// A connection's token or key, by the connection's id.
    Connection(String),
    /// A model's key, by the model's id.
    Model(String),
}

impl VaultEntry {
    /// What the secret belongs to when the gateway put it in itself, from a
    /// login, a connection's key or a model's registration; `None` for one
    /// put in by hand. The name alone tells: a connection's or a model's
    /// name is refused unless the gateway made it.
    pub fn holder(&self) -> Option<Holder> {
        if let Some((id, _)) = connection_of(&self.name) {
            return Some(Holder::Connection(id.to_owned()));
        }
        model_of(&self.name).map(|id| Holder::Model(id.to_owned()))
    }
}

impl Gateway {
    fn vault_or_refuse(&self) -> anyhow::Result<&Vault> {
        self.vault
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("there is no vault: set {}", super::KEY_VAR))
    }

    /// The vault's secrets as anyone outside the gateway sees them.
    pub fn secrets(&self) -> anyhow::Result<Vec<VaultEntry>> {
        Ok(self
            .vault_or_refuse()?
            .list()?
            .into_iter()
            .filter(|e| !is_held_entry(&e.name))
            .collect())
    }

    /// The latest entries of the gateway's log, the newest first.
    pub fn log_entries(&self, limit: usize) -> anyhow::Result<Vec<LogEntry>> {
        self.log().recent(limit)
    }

    /// A page of the gateway's log, for the owner to read.
    pub fn log_page(&self, query: &crate::log::LogQuery) -> anyhow::Result<Vec<(i64, LogEntry)>> {
        self.log().page(query)
    }

    /// Put a secret in for `actor`, or replace one. `purpose` becomes its
    /// description; an empty one keeps the description it had. A
    /// connection's or a model's secret only has its value replaced.
    pub fn put_secret(
        &self,
        actor: impl AsRef<str>,
        name: impl AsRef<str>,
        value: impl AsRef<str>,
        purpose: impl AsRef<str>,
    ) -> anyhow::Result<Outcome<()>> {
        let (actor, name, purpose) = (
            actor.as_ref(),
            name.as_ref().trim(),
            purpose.as_ref().trim(),
        );
        let input = json!({ "name": name, "value": value.as_ref(), "purpose": purpose });
        if let Some(ticket) = self.judge_vault(actor, PUT_SECRET, input, &["value"])? {
            return Ok(Outcome::Held(ticket));
        }
        self.put_now(actor, name, value.as_ref(), purpose)
            .map(Outcome::Done)
    }

    fn put_now(&self, actor: &str, name: &str, value: &str, purpose: &str) -> anyhow::Result<()> {
        let result = self.put_checked(actor, name, value, purpose);
        self.log_vault_change(actor, name, "vault.put", &result)?;
        result
    }

    /// Say what a secret is for, for `actor`, leaving its value as it is. A
    /// connection's or a model's secret can be described too, and keeps its
    /// description when the gateway writes its value again.
    pub fn describe_secret(
        &self,
        actor: impl AsRef<str>,
        name: impl AsRef<str>,
        description: impl AsRef<str>,
    ) -> anyhow::Result<Outcome<()>> {
        let (actor, name, description) = (
            actor.as_ref(),
            name.as_ref().trim(),
            description.as_ref().trim(),
        );
        let input = json!({ "name": name, "description": description });
        if let Some(ticket) = self.judge_vault(actor, DESCRIBE_SECRET, input, &[])? {
            return Ok(Outcome::Held(ticket));
        }
        self.describe_now(actor, name, description)
            .map(Outcome::Done)
    }

    fn describe_now(&self, actor: &str, name: &str, description: &str) -> anyhow::Result<()> {
        let result = self.describe_checked(actor, name, description);
        self.log_vault_change(actor, name, "vault.describe", &result)?;
        result
    }

    fn describe_checked(&self, actor: &str, name: &str, description: &str) -> anyhow::Result<()> {
        let vault = self.vault_or_refuse()?;
        anyhow::ensure!(!is_held_entry(name), "no secret {name}");
        vault.describe(name, description, actor)
    }

    /// The permissions the vault's changes have before the owner chooses:
    /// putting a secret in and describing one run, taking one out asks.
    fn vault_defaults(&self) -> anyhow::Result<()> {
        let permissions = self.permissions();
        for (tool, permission) in [
            (PUT_SECRET, Permission::Allow),
            (DESCRIBE_SECRET, Permission::Allow),
            (DELETE_SECRET, Permission::Confirm),
        ] {
            permissions.set_default_if_absent(VAULT, tool, permission)?;
        }
        Ok(())
    }

    /// Judge a change to the vault; the ticket when it is held. A refusal is
    /// an error with its reason.
    fn judge_vault(
        &self,
        actor: &str,
        tool: &str,
        input: Value,
        secret_fields: &[&str],
    ) -> anyhow::Result<Option<Ticket>> {
        self.vault_defaults()?;
        let call = Call {
            secret_fields: secret_fields.iter().map(|f| (*f).to_owned()).collect(),
            ..Call::new(actor, VAULT, tool, input)
        };
        match self.judge(&call)? {
            Verdict::Allow { .. } => Ok(None),
            Verdict::Hold(ticket) => Ok(Some(ticket)),
            Verdict::Deny { reason } => anyhow::bail!("{reason}"),
        }
    }

    /// Make a held change to the vault the owner approved, as its caller.
    pub(super) fn run_held_vault(&self, ticket: &Ticket, input: &Value) -> anyhow::Result<()> {
        let field = |name: &str| input.get(name).and_then(Value::as_str).unwrap_or_default();
        match ticket.tool.as_str() {
            PUT_SECRET => self.put_now(
                &ticket.actor,
                field("name"),
                field("value"),
                field("purpose"),
            ),
            DESCRIBE_SECRET => {
                self.describe_now(&ticket.actor, field("name"), field("description"))
            }
            DELETE_SECRET => self.delete_now(&ticket.actor, field("name")),
            other => anyhow::bail!("the vault has no change {other}"),
        }
    }

    /// The permission the owner chose for a change to the vault, or its default.
    pub fn vault_permission(&self, tool: impl AsRef<str>) -> anyhow::Result<Permission> {
        self.vault_defaults()?;
        let permissions = self.permissions();
        Ok(match permissions.chosen(VAULT, tool.as_ref())? {
            Some((permission, _)) => permission,
            None => permissions.default_of(VAULT, tool.as_ref())?,
        })
    }

    /// Choose the permission for a change to the vault; `None` goes back to the default.
    pub fn set_vault_permission(
        &self,
        actor: impl AsRef<str>,
        tool: impl AsRef<str>,
        permission: Option<Permission>,
    ) -> anyhow::Result<()> {
        let tool = tool.as_ref();
        anyhow::ensure!(
            [PUT_SECRET, DESCRIBE_SECRET, DELETE_SECRET].contains(&tool),
            "the vault has no change {tool}"
        );
        self.set_permission(actor, format!("conn:{VAULT}"), tool, permission)
    }

    fn put_checked(
        &self,
        actor: &str,
        name: &str,
        value: &str,
        purpose: &str,
    ) -> anyhow::Result<()> {
        let vault = self.vault_or_refuse()?;
        if let Some(model) = model_of(name) {
            let entry = vault.get(name)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "{name} is not a key the vault keeps; a model's key comes with registering the model {model}"
                )
            })?;
            return vault.put(name, value, entry.used_by, actor);
        }
        let Some((id, field)) = connection_of(name) else {
            vault.put(name, value, "", actor)?;
            if purpose.is_empty() {
                return Ok(());
            }
            return vault.describe(name, purpose, actor);
        };
        anyhow::ensure!(
            vault.get(name)?.is_some(),
            "{name} is not a secret {id} keeps; a connection's secrets come from its login or its key"
        );
        let mut row = self.connections.require(id)?.secret;
        vault.put(name, value, id, actor)?;
        row.in_vault.insert(field.to_owned());
        self.connections.set_secret(id, &row)
    }

    /// Take a secret out for `actor`. A secret a connection uses is not taken
    /// out: the connection is removed instead, and its secrets with it.
    pub fn delete_secret(
        &self,
        actor: impl AsRef<str>,
        name: impl AsRef<str>,
    ) -> anyhow::Result<Outcome<()>> {
        let (actor, name) = (actor.as_ref(), name.as_ref().trim());
        if let Some(ticket) =
            self.judge_vault(actor, DELETE_SECRET, json!({ "name": name }), &[])?
        {
            return Ok(Outcome::Held(ticket));
        }
        self.delete_now(actor, name).map(Outcome::Done)
    }

    fn delete_now(&self, actor: &str, name: &str) -> anyhow::Result<()> {
        let result = self.delete_checked(name);
        self.log_vault_change(actor, name, "vault.delete", &result)?;
        result
    }

    fn delete_checked(&self, name: &str) -> anyhow::Result<()> {
        let vault = self.vault_or_refuse()?;
        if let Some(model) = model_of(name) {
            anyhow::bail!(
                "{name} is the key of model {model}; remove the model to take its key out"
            );
        }
        if let Some((id, _)) = connection_of(name)
            && let Some(c) = self.connections.get(id)?
        {
            anyhow::bail!(
                "{} uses {name}; remove the connection to take its secrets out",
                c.name
            );
        }
        anyhow::ensure!(vault.delete(name)?, "no secret {name}");
        Ok(())
    }

    /// Remove a connection for `actor`, and every secret it kept.
    pub fn remove_connection(
        &self,
        actor: impl AsRef<str>,
        id: impl AsRef<str>,
    ) -> anyhow::Result<()> {
        let id = id.as_ref();
        if let Some(vault) = &self.vault {
            for entry in vault.list()? {
                if connection_of(&entry.name).is_some_and(|(of, _)| of == id) {
                    vault.delete(&entry.name)?;
                }
            }
        }
        self.connections.remove(id)?;
        self.log_change_by(actor.as_ref(), id, "connection.remove", "")
    }

    fn log_vault_change(
        &self,
        actor: &str,
        name: &str,
        tool: &str,
        result: &anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let (decision, reason, outcome) = match result {
            Ok(()) => ("allow", String::new(), "ok"),
            Err(error) => ("deny", format!("{error:#}"), ""),
        };
        self.log().write(&LogEntry {
            actor: actor.to_owned(),
            connection: connection_of(name)
                .map(|(id, _)| id.to_owned())
                .unwrap_or_default(),
            tool: tool.to_owned(),
            decision: decision.to_owned(),
            reason,
            secret: name.to_owned(),
            outcome: outcome.to_owned(),
            ..LogEntry::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::{Connections, Secret, Transport};
    use crate::store::Store;

    fn gateway(vault: bool) -> Gateway {
        let store = Store::open_in_memory().unwrap();
        let connections = Connections::new(store.clone());
        Gateway::new(
            store,
            connections,
            reqwest::Client::new(),
            vault.then(super::super::vault::tests::scratch),
        )
    }

    /// A connection with an API key in the vault, as setting its key leaves it.
    fn with_keyed_connection(g: &Gateway) {
        g.connections
            .add(
                "billing",
                "Billing",
                "billing",
                Transport::Api {
                    base_url: "http://127.0.0.1:9".into(),
                    openapi_url: String::new(),
                },
                Secret::default(),
                "",
            )
            .unwrap();
        g.set_api_key("billing", "bk-0123456789-0123456789-0123", "")
            .unwrap();
    }

    #[test]
    fn a_deposited_secret_is_listed_without_its_value_and_logged_without_it() {
        let g = gateway(true);

        g.put_secret("owner", "bank-password", "correct-horse", "bank")
            .unwrap();

        let entries = g.secrets().unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(
            (
                e.name.as_str(),
                e.description.as_str(),
                e.used_by.as_str(),
                e.by.as_str(),
            ),
            ("bank-password", "bank", "", "owner")
        );
        assert_eq!(e.holder(), None, "put in by hand");
        let log = g.log_entries(10).unwrap();
        assert_eq!(log[0].tool, "vault.put");
        assert_eq!(log[0].actor, "owner");
        assert_eq!(log[0].secret, "bank-password");
        assert_eq!(
            (log[0].decision.as_str(), log[0].digest.as_str()),
            ("allow", "")
        );
        let all = serde_json::to_string(&log).unwrap() + &serde_json::to_string(&entries).unwrap();
        assert!(!all.contains("correct-horse"));
    }

    #[test]
    fn describing_a_secret_changes_only_its_description_and_is_logged() {
        let g = gateway(true);
        g.put_secret("owner", "pw", "kept-value", "old").unwrap();

        g.describe_secret("owner", "pw", "the bank's site").unwrap();
        let missing = g.describe_secret("owner", "nothing", "x").unwrap_err();

        let entry = &g.secrets().unwrap()[0];
        assert_eq!(entry.description, "the bank's site");
        assert_eq!(
            g.vault().unwrap().reveal("pw").unwrap().as_deref(),
            Some("kept-value")
        );
        assert!(
            missing.to_string().contains("no secret nothing"),
            "{missing}"
        );
        let log = g.log_entries(10).unwrap();
        assert_eq!(
            log.iter()
                .map(|e| (e.tool.as_str(), e.decision.as_str(), e.secret.as_str()))
                .collect::<Vec<_>>(),
            [
                ("vault.describe", "deny", "nothing"),
                ("vault.describe", "allow", "pw"),
                ("vault.put", "allow", "pw"),
            ]
        );
    }

    #[test]
    fn replacing_a_value_without_a_purpose_keeps_the_description() {
        let g = gateway(true);
        g.put_secret("owner", "pw", "first-value", "the bank")
            .unwrap();

        g.put_secret("owner", "pw", "second-value", "").unwrap();

        assert_eq!(g.secrets().unwrap()[0].description, "the bank");
    }

    #[test]
    fn a_connections_or_a_models_secret_says_what_it_belongs_to_and_can_be_described() {
        let g = gateway(true);
        with_keyed_connection(&g);
        g.put_model_key("owner", "paid", "sk-0123456789").unwrap();

        g.describe_secret("owner", "connection:billing:api_key", "請求書")
            .unwrap();
        g.set_api_key("billing", "bk-9876543210-9876543210-9876", "")
            .unwrap();

        let holders: Vec<_> = g
            .secrets()
            .unwrap()
            .into_iter()
            .map(|e| (e.name.clone(), e.holder(), e.description))
            .collect();
        assert_eq!(
            holders,
            [
                (
                    "connection:billing:api_key".to_owned(),
                    Some(Holder::Connection("billing".into())),
                    "請求書".to_owned()
                ),
                (
                    "model:paid:api_key".to_owned(),
                    Some(Holder::Model("paid".into())),
                    String::new()
                ),
            ],
            "the gateway's own writes keep the description"
        );
    }

    #[tokio::test]
    async fn the_secretary_describes_under_the_vaults_permission_and_is_held_under_confirm() {
        let g = secretary(true);
        g.put_secret("owner", "pw", "kept-value", "").unwrap();
        assert_eq!(
            g.vault_permission(DESCRIBE_SECRET).unwrap(),
            Permission::Allow
        );
        assert_eq!(
            g.describe_secret("secretary", "pw", "first").unwrap(),
            Outcome::Done(())
        );
        g.set_vault_permission("owner", DESCRIBE_SECRET, Some(Permission::Confirm))
            .unwrap();

        let Outcome::Held(ticket) = g.describe_secret("secretary", "pw", "second").unwrap() else {
            panic!("held under confirm");
        };
        assert_eq!(g.secrets().unwrap()[0].description, "first");
        g.answer_ticket(ticket.id, super::super::Answer::Approve)
            .await
            .unwrap();

        let entry = &g.secrets().unwrap()[0];
        assert_eq!(
            (entry.description.as_str(), entry.by.as_str()),
            ("second", "secretary")
        );
    }

    #[test]
    fn deleting_takes_a_secret_out_and_a_missing_one_is_refused_and_logged() {
        let g = gateway(true);
        g.put_secret("owner", "pw", "secret-value", "").unwrap();

        g.delete_secret("owner", "pw").unwrap();
        let error = g.delete_secret("owner", "pw").unwrap_err();

        assert!(g.secrets().unwrap().is_empty());
        assert!(error.to_string().contains("no secret pw"), "{error}");
        let log = g.log_entries(10).unwrap();
        assert_eq!(
            log.iter()
                .map(|e| (e.tool.as_str(), e.decision.as_str(), e.actor.as_str()))
                .collect::<Vec<_>>(),
            [
                ("vault.delete", "deny", "owner"),
                ("vault.delete", "allow", "owner"),
                ("vault.put", "allow", "owner"),
            ]
        );
    }

    fn secretary(vault: bool) -> Gateway {
        let g = gateway(vault);
        g.grant("secretary", super::super::ANY, super::super::ANY)
            .unwrap();
        g
    }

    #[tokio::test]
    async fn the_secretary_puts_a_secret_in_and_is_asked_before_taking_one_out() {
        let g = secretary(true);
        assert_eq!(
            g.put_secret("secretary", "pw", "secret-value", "").unwrap(),
            Outcome::Done(())
        );

        let Outcome::Held(ticket) = g.delete_secret("secretary", "pw").unwrap() else {
            panic!("taking a secret out asks the owner");
        };
        assert_eq!(g.secrets().unwrap().len(), 1);

        let answered = g
            .answer_ticket(ticket.id, super::super::Answer::Approve)
            .await
            .unwrap()
            .unwrap();
        assert!(answered.ran.unwrap().is_ok());
        assert!(g.secrets().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_held_secret_is_sealed_until_the_owner_answers_and_put_in_once_approved() {
        let g = secretary(true);
        g.set_vault_permission("owner", PUT_SECRET, Some(Permission::Confirm))
            .unwrap();

        let Outcome::Held(ticket) = g
            .put_secret("secretary", "bank", "hunter2-hunter2", "bank")
            .unwrap()
        else {
            panic!("held under confirm");
        };

        assert_eq!(ticket.shown["value"], "…");
        assert_eq!(ticket.shown["name"], "bank");
        assert!(
            g.secrets().unwrap().is_empty(),
            "a held secret is not listed"
        );
        let dir = g.vault().unwrap().dir().to_owned();
        let plain_anywhere = walk(&dir)
            .into_iter()
            .any(|f| String::from_utf8_lossy(&std::fs::read(f).unwrap()).contains("hunter2"));
        assert!(!plain_anywhere);
        let stored: String = g
            .store
            .conn()
            .query_row("SELECT input || shown FROM tickets", [], |r| r.get(0))
            .unwrap();
        assert!(!stored.contains("hunter2"), "{stored}");

        g.answer_ticket(ticket.id, super::super::Answer::Approve)
            .await
            .unwrap();
        let names: Vec<String> = g
            .vault()
            .unwrap()
            .list()
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["bank"], "the sealed copy is gone");
        assert_eq!(
            g.vault().unwrap().reveal("bank").unwrap().as_deref(),
            Some("hunter2-hunter2")
        );
        assert_eq!(
            g.vault().unwrap().get("bank").unwrap().unwrap().by,
            "secretary"
        );
    }

    #[tokio::test]
    async fn a_rejected_held_secret_is_dropped() {
        let g = secretary(true);
        g.set_vault_permission("owner", PUT_SECRET, Some(Permission::Confirm))
            .unwrap();
        let Outcome::Held(ticket) = g.put_secret("secretary", "bank", "hunter2", "").unwrap()
        else {
            panic!("held under confirm");
        };

        let answered = g
            .answer_ticket(ticket.id, super::super::Answer::Reject)
            .await
            .unwrap()
            .unwrap();

        assert!(answered.ran.is_none());
        assert!(g.vault().unwrap().list().unwrap().is_empty());
    }

    #[test]
    fn a_caller_without_grants_cannot_touch_the_vault() {
        let g = gateway(true);
        let error = g.put_secret("lookout", "pw", "v", "").unwrap_err();
        assert!(error.to_string().contains("not granted"), "{error}");
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.extend(walk(&path));
            } else {
                files.push(path);
            }
        }
        files
    }

    #[test]
    fn a_secret_a_connection_uses_is_not_deleted_and_the_connection_is_named() {
        let g = gateway(true);
        with_keyed_connection(&g);

        let error = g
            .delete_secret("owner", "connection:billing:api_key")
            .unwrap_err();

        assert!(error.to_string().contains("Billing"), "{error}");
        assert_eq!(g.secrets().unwrap().len(), 1);
        assert_eq!(g.log_entries(1).unwrap()[0].connection, "billing");
    }

    #[test]
    fn a_connections_secret_can_be_replaced_but_not_made() {
        let g = gateway(true);
        with_keyed_connection(&g);

        g.put_secret(
            "owner",
            "connection:billing:api_key",
            "bk-new-new-new-new-new-new-1",
            "x",
        )
        .unwrap();
        let error = g
            .put_secret("owner", "connection:billing:bearer", "made-up", "")
            .unwrap_err();

        let c = g.sealed().get("billing").unwrap().unwrap();
        assert_eq!(
            c.secret.api_key.as_deref(),
            Some("bk-new-new-new-new-new-new-1")
        );
        assert!(c.secret.in_vault.contains("api_key"));
        let entry = g
            .vault()
            .unwrap()
            .get("connection:billing:api_key")
            .unwrap()
            .unwrap();
        assert_eq!(
            (entry.used_by.as_str(), entry.by.as_str()),
            ("billing", "owner")
        );
        assert!(error.to_string().contains("login or its key"), "{error}");
        assert!(
            g.vault()
                .unwrap()
                .get("connection:billing:bearer")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn removing_a_connection_takes_its_secrets_out_and_logs_it() {
        let g = gateway(true);
        with_keyed_connection(&g);
        g.put_secret("owner", "pw", "kept-value", "").unwrap();

        g.remove_connection("owner", "billing").unwrap();

        let names: Vec<String> = g.secrets().unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["pw"]);
        assert!(g.connections.get("billing").unwrap().is_none());
        let last = &g.log_entries(1).unwrap()[0];
        assert_eq!(
            (last.tool.as_str(), last.connection.as_str()),
            ("connection.remove", "billing")
        );
    }

    #[test]
    fn without_a_vault_keeping_is_refused_with_the_key_to_set() {
        let g = gateway(false);

        let error = g.put_secret("owner", "pw", "value", "").unwrap_err();

        assert!(error.to_string().contains(super::super::KEY_VAR), "{error}");
        assert!(g.secrets().is_err());
        assert_eq!(g.log_entries(1).unwrap()[0].decision, "deny");
    }
}
