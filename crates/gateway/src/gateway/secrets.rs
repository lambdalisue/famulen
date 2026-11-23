//! Connections as the gateway sees them: their tokens and keys come out of
//! the vault when read and go back into it when kept, so the rows hold none.
//! Without a vault, as when the owner gave no key, the rows hold them.

use std::collections::{BTreeMap, BTreeSet};

use crate::connections::{Connection, ConnectionTool, Connections, Secret};

use super::{GATEWAY, Vault};

/// The fields of a [`Secret`] that are kept in the vault.
const SEALED_FIELDS: &[&str] = &["bearer", "refresh_token", "client_secret", "api_key"];

fn field<'a>(secret: &'a mut Secret, name: &str) -> &'a mut Option<String> {
    match name {
        "bearer" => &mut secret.bearer,
        "refresh_token" => &mut secret.refresh_token,
        "client_secret" => &mut secret.client_secret,
        "api_key" => &mut secret.api_key,
        other => unreachable!("{other} is not a sealed field"),
    }
}

/// The prefix of the vault's names for what connections keep in it.
const CONNECTION: &str = "connection:";

/// The vault's name for one field of a connection's secret.
pub(super) fn vault_name(id: &str, name: &str) -> String {
    format!("{CONNECTION}{id}:{name}")
}

/// The connection a secret's name belongs to, and the field, when it is one
/// of a connection's.
pub(super) fn connection_of(name: &str) -> Option<(&str, &str)> {
    name.strip_prefix(CONNECTION)?.split_once(':')
}

/// [`Connections`], with secrets opened and sealed on the way.
#[derive(Clone)]
pub(super) struct Sealed {
    pub(super) connections: Connections,
    pub(super) vault: Option<Vault>,
}

impl Sealed {
    pub(super) fn get(&self, id: impl AsRef<str>) -> anyhow::Result<Option<Connection>> {
        self.connections.get(id)?.map(|c| self.open(c)).transpose()
    }

    /// The connection `id`, opened, or an error saying there is none.
    pub(super) fn require(&self, id: impl AsRef<str>) -> anyhow::Result<Connection> {
        self.open(self.connections.require(id)?)
    }

    pub(super) fn list(&self) -> anyhow::Result<Vec<Connection>> {
        self.connections
            .list()?
            .into_iter()
            .map(|c| self.open(c))
            .collect()
    }

    pub(super) fn set_secret(&self, id: impl AsRef<str>, secret: &Secret) -> anyhow::Result<()> {
        let id = id.as_ref();
        let Some(vault) = &self.vault else {
            return self.connections.set_secret(id, secret);
        };
        let mut row = secret.clone();
        for name in SEALED_FIELDS {
            match field(&mut row, name).take() {
                Some(value) => {
                    vault.put(vault_name(id, name), &value, id, GATEWAY)?;
                    row.in_vault.insert((*name).to_owned());
                }
                None => {
                    vault.delete(vault_name(id, name))?;
                    row.in_vault.remove(*name);
                }
            }
        }
        let tokens = std::mem::take(&mut row.tokens);
        seal_named(vault, id, "token.", tokens, &mut row.in_vault)?;
        let account = std::mem::take(&mut row.account);
        seal_named(vault, id, "account.", account, &mut row.in_vault)?;
        self.connections.set_secret(id, &row)
    }

    pub(super) fn set_status(
        &self,
        id: impl AsRef<str>,
        status: impl AsRef<str>,
    ) -> anyhow::Result<()> {
        self.connections.set_status(id, status)
    }

    pub(super) fn record_tools(
        &self,
        id: impl AsRef<str>,
        tools: &[ConnectionTool],
    ) -> anyhow::Result<()> {
        self.connections.record_tools(id, tools)
    }

    /// Move the secrets rows hold from when there was no vault into it; how
    /// many connections were moved. Nothing moves without a vault.
    pub(super) fn seal_all(&self) -> anyhow::Result<usize> {
        if self.vault.is_none() {
            return Ok(0);
        }
        let mut moved = 0;
        for c in self.connections.list()? {
            let mut plain = c.secret.clone();
            if SEALED_FIELDS
                .iter()
                .any(|name| field(&mut plain, name).is_some())
                || !plain.tokens.is_empty()
                || !plain.account.is_empty()
            {
                let opened = self.open(c.clone())?;
                self.set_secret(&c.id, &opened.secret)?;
                moved += 1;
            }
        }
        Ok(moved)
    }

    fn open(&self, mut c: Connection) -> anyhow::Result<Connection> {
        let Some(vault) = &self.vault else {
            return Ok(c);
        };
        for name in SEALED_FIELDS {
            if c.secret.in_vault.contains(*name) {
                *field(&mut c.secret, name) = vault.reveal(vault_name(&c.id, name))?;
            }
        }
        let tokens = open_named(vault, &c.id, "token.", &c.secret.in_vault)?;
        c.secret.tokens.extend(tokens);
        let account = open_named(vault, &c.id, "account.", &c.secret.in_vault)?;
        c.secret.account.extend(account);
        Ok(c)
    }
}

/// Seal each of `values` under `prefix`, and drop the ones sealed before
/// that `values` no longer holds.
fn seal_named(
    vault: &Vault,
    id: &str,
    prefix: &str,
    values: BTreeMap<String, String>,
    in_vault: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    let stale: Vec<String> = in_vault
        .iter()
        .filter(|k| {
            k.strip_prefix(prefix)
                .is_some_and(|name| !values.contains_key(name))
        })
        .cloned()
        .collect();
    for key in stale {
        vault.delete(vault_name(id, &key))?;
        in_vault.remove(&key);
    }
    for (name, value) in values {
        let key = format!("{prefix}{name}");
        vault.put(vault_name(id, &key), &value, id, GATEWAY)?;
        in_vault.insert(key);
    }
    Ok(())
}

fn open_named(
    vault: &Vault,
    id: &str,
    prefix: &str,
    in_vault: &BTreeSet<String>,
) -> anyhow::Result<BTreeMap<String, String>> {
    let mut opened = BTreeMap::new();
    for key in in_vault.iter().filter(|k| k.starts_with(prefix)) {
        if let Some(value) = vault.reveal(vault_name(id, key))? {
            opened.insert(key[prefix.len()..].to_owned(), value);
        }
    }
    Ok(opened)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::Transport;
    use crate::store::Store;

    fn sealed(store: &Store, vault: bool) -> Sealed {
        let connections = Connections::new(store.clone());
        connections
            .add(
                "notion",
                "Notion",
                "notion",
                Transport::McpRemote {
                    url: "https://mcp.notion.com/mcp".into(),
                },
                Secret::default(),
                "",
            )
            .unwrap();
        Sealed {
            connections,
            vault: vault.then(super::super::vault::tests::scratch),
        }
    }

    fn logged_in() -> Secret {
        Secret {
            bearer: Some("at-1234567890".into()),
            refresh_token: Some("rt-1234567890".into()),
            client_id: Some("client".into()),
            client_secret: Some("cs-1234567890".into()),
            ..Secret::default()
        }
    }

    #[test]
    fn tokens_kept_through_the_gateway_live_only_in_the_vault() {
        let store = Store::open_in_memory().unwrap();
        let s = sealed(&store, true);

        s.set_secret("notion", &logged_in()).unwrap();

        let row = s.connections.get("notion").unwrap().unwrap().secret;
        assert_eq!(
            (
                row.bearer.as_deref(),
                row.refresh_token.as_deref(),
                row.client_secret.as_deref()
            ),
            (None, None, None)
        );
        assert_eq!(row.client_id.as_deref(), Some("client"));
        assert!(row.holds("bearer"));
        assert!(row.in_vault.contains("bearer"));
        let opened = s.get("notion").unwrap().unwrap().secret;
        assert_eq!(opened.bearer.as_deref(), Some("at-1234567890"));
        assert_eq!(opened.refresh_token.as_deref(), Some("rt-1234567890"));
        let entries = s.vault.as_ref().unwrap().list().unwrap();
        assert!(entries.iter().all(|e| e.used_by == "notion"));
    }

    #[test]
    fn a_token_cleared_is_taken_out_of_the_vault() {
        let store = Store::open_in_memory().unwrap();
        let s = sealed(&store, true);
        s.set_secret("notion", &logged_in()).unwrap();

        let mut secret = s.get("notion").unwrap().unwrap().secret;
        secret.refresh_token = None;
        s.set_secret("notion", &secret).unwrap();

        let opened = s.get("notion").unwrap().unwrap().secret;
        assert_eq!(opened.refresh_token, None);
        assert!(!opened.holds("refresh_token"));
        let names: Vec<String> = s
            .vault
            .as_ref()
            .unwrap()
            .list()
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(!names.contains(&"connection:notion:refresh_token".to_owned()));
    }

    #[test]
    fn without_a_vault_the_row_keeps_its_tokens() {
        let store = Store::open_in_memory().unwrap();
        let s = sealed(&store, false);

        s.set_secret("notion", &logged_in()).unwrap();

        let row = s.connections.get("notion").unwrap().unwrap().secret;
        assert_eq!(row.bearer.as_deref(), Some("at-1234567890"));
        assert_eq!(s.seal_all().unwrap(), 0);
    }

    #[test]
    fn tokens_left_in_rows_are_moved_into_the_vault() {
        let store = Store::open_in_memory().unwrap();
        let s = sealed(&store, true);
        s.connections.set_secret("notion", &logged_in()).unwrap();

        assert_eq!(s.seal_all().unwrap(), 1);
        assert_eq!(s.seal_all().unwrap(), 0);

        let row = s.connections.get("notion").unwrap().unwrap().secret;
        assert_eq!(row.bearer, None);
        let opened = s.get("notion").unwrap().unwrap().secret;
        assert_eq!(opened.bearer.as_deref(), Some("at-1234567890"));
    }
}
