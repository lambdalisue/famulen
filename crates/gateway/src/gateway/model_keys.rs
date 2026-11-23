//! The keys of the models the program calls. A key is kept in the vault
//! under the model's name and put on a request by the gateway as it sends
//! it, so the caller never holds the key.

use std::time::Instant;

use crate::log::LogEntry;

use super::Gateway;

/// The prefix of the vault's names for model keys.
pub(super) const MODEL: &str = "model:";

/// The vault's name for the key of model `id`.
pub fn model_key_name(id: impl AsRef<str>) -> String {
    format!("{MODEL}{}:api_key", id.as_ref())
}

/// The model a vault name is the key of.
pub(super) fn model_of(name: &str) -> Option<&str> {
    name.strip_prefix(MODEL)?.strip_suffix(":api_key")
}

impl Gateway {
    /// Keep `key` as the key of model `id`, for `actor`. An empty key takes
    /// the model's key out.
    pub fn put_model_key(
        &self,
        actor: impl AsRef<str>,
        id: impl AsRef<str>,
        key: impl AsRef<str>,
    ) -> anyhow::Result<()> {
        let (actor, id, key) = (actor.as_ref(), id.as_ref(), key.as_ref().trim());
        if key.is_empty() {
            return self.remove_model_key(actor, id);
        }
        let name = model_key_name(id);
        let vault = self
            .vault
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("there is no vault: set {}", super::KEY_VAR))?;
        vault.put(&name, key, format!("model {id}"), actor)?;
        self.log_model(actor, id, "model.key.put")
    }

    /// Take the key of model `id` out, as when the model is removed.
    pub fn remove_model_key(
        &self,
        actor: impl AsRef<str>,
        id: impl AsRef<str>,
    ) -> anyhow::Result<()> {
        let id = id.as_ref();
        let Some(vault) = &self.vault else {
            return Ok(());
        };
        if vault.delete(model_key_name(id))? {
            self.log_model(actor.as_ref(), id, "model.key.delete")?;
        }
        Ok(())
    }

    /// Whether model `id` has a key in the vault. Nothing of the key itself
    /// leaves the gateway.
    pub fn has_model_key(&self, id: impl AsRef<str>) -> anyhow::Result<bool> {
        let Some(vault) = &self.vault else {
            return Ok(false);
        };
        Ok(vault.get(model_key_name(id))?.is_some())
    }

    /// Send `request` for model `id`, with its key as a bearer token when it
    /// has one. The call is logged with the key's name, never the key.
    pub async fn send_for_model(
        &self,
        actor: impl AsRef<str>,
        id: impl AsRef<str>,
        request: reqwest::RequestBuilder,
    ) -> anyhow::Result<reqwest::Response> {
        let (actor, id) = (actor.as_ref(), id.as_ref());
        let name = model_key_name(id);
        let key = match &self.vault {
            Some(vault) => vault.reveal(&name)?,
            None => None,
        };
        let request = match &key {
            Some(key) => request.bearer_auth(key),
            None => request,
        };
        let started = Instant::now();
        let response = request.send().await;
        let outcome = match &response {
            Ok(r) => r.status().as_u16().to_string(),
            Err(e) => format!("error: {e}"),
        };
        self.log().write(&LogEntry {
            actor: actor.to_owned(),
            connection: format!("{MODEL}{id}"),
            tool: "model.call".into(),
            decision: "allow".into(),
            secret: key.map(|_| name).unwrap_or_default(),
            outcome,
            ms: started.elapsed().as_millis() as i64,
            ..LogEntry::default()
        })?;
        Ok(response?)
    }

    fn log_model(&self, actor: &str, id: &str, tool: &str) -> anyhow::Result<()> {
        self.log().write(&LogEntry {
            actor: actor.to_owned(),
            connection: format!("{MODEL}{id}"),
            tool: tool.to_owned(),
            decision: "allow".into(),
            secret: model_key_name(id),
            outcome: "ok".into(),
            ..LogEntry::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{Router, extract::State, http::HeaderMap, routing::post};

    use super::*;
    use crate::connections::Connections;
    use crate::store::Store;

    fn gateway() -> Gateway {
        let store = Store::open_in_memory().unwrap();
        let connections = Connections::new(store.clone());
        Gateway::new(
            store,
            connections,
            reqwest::Client::new(),
            Some(super::super::vault::tests::scratch()),
        )
    }

    /// Serves `/v1/chat/completions` and records the authorization it got.
    async fn serve() -> (String, Arc<Mutex<Vec<Option<String>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let app =
            Router::new()
                .route(
                    "/v1/chat/completions",
                    post(
                        |State(seen): State<Arc<Mutex<Vec<Option<String>>>>>,
                         headers: HeaderMap| async move {
                            seen.lock().unwrap().push(
                                headers
                                    .get("authorization")
                                    .map(|v| v.to_str().unwrap().to_owned()),
                            );
                            "{}"
                        },
                    ),
                )
                .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, seen)
    }

    #[tokio::test]
    async fn a_model_key_is_put_on_the_request_by_the_gateway_and_logged_by_name() {
        let g = gateway();
        g.put_model_key("owner", "paid", "sk-0123456789-0123456789")
            .unwrap();
        let (url, seen) = serve().await;

        let request = reqwest::Client::new().post(format!("{url}/chat/completions"));
        let response = g.send_for_model("famulen", "paid", request).await.unwrap();

        assert!(response.status().is_success());
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            [Some("Bearer sk-0123456789-0123456789".to_owned())]
        );
        let log = g.log_entries(1).unwrap();
        assert_eq!(
            (
                log[0].tool.as_str(),
                log[0].connection.as_str(),
                log[0].secret.as_str()
            ),
            ("model.call", "model:paid", "model:paid:api_key")
        );
        assert!(!format!("{log:?}").contains("sk-0123456789"));
    }

    #[tokio::test]
    async fn a_model_without_a_key_is_called_without_authorization() {
        let g = gateway();
        let (url, seen) = serve().await;

        let request = reqwest::Client::new().post(format!("{url}/chat/completions"));
        g.send_for_model("famulen", "local", request).await.unwrap();

        assert_eq!(seen.lock().unwrap().as_slice(), [None]);
    }

    #[test]
    fn a_model_key_is_listed_for_its_model_without_any_part_of_it() {
        let g = gateway();
        g.put_model_key("owner", "paid", "sk-0123456789-0123456789-0123")
            .unwrap();

        let entries = g.secrets().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (entries[0].name.as_str(), entries[0].used_by.as_str()),
            ("model:paid:api_key", "model paid")
        );
        assert!(g.has_model_key("paid").unwrap());
        let listed = serde_json::to_string(&entries).unwrap();
        assert!(
            !listed.contains("sk-0") && !listed.contains("0123\""),
            "{listed}"
        );
    }

    #[test]
    fn a_model_key_is_not_taken_out_from_the_vault_but_with_its_model() {
        let g = gateway();
        g.put_model_key("owner", "paid", "sk-1").unwrap();

        let refused = g.delete_secret("owner", "model:paid:api_key");
        assert!(
            refused
                .unwrap_err()
                .to_string()
                .contains("remove the model"),
        );

        g.remove_model_key("owner", "paid").unwrap();
        assert!(g.secrets().unwrap().is_empty());
        assert!(!g.has_model_key("paid").unwrap());
    }

    #[test]
    fn an_empty_key_takes_the_models_key_out() {
        let g = gateway();
        g.put_model_key("owner", "paid", "sk-1").unwrap();
        g.put_model_key("owner", "paid", "  ").unwrap();
        assert!(g.secrets().unwrap().is_empty());
    }

    #[test]
    fn a_model_key_can_be_replaced_from_the_vault_but_not_made_there() {
        let g = gateway();
        assert!(
            g.put_secret("owner", "model:paid:api_key", "sk-2", "")
                .is_err()
        );
        g.put_model_key("owner", "paid", "sk-1").unwrap();
        g.put_secret("owner", "model:paid:api_key", "sk-2", "")
            .unwrap();
        let entry = &g.secrets().unwrap()[0];
        assert_eq!(entry.used_by, "model paid");
    }
}
