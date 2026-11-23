//! The Slack clients, one for each workspace.

use std::collections::BTreeMap;

/// A Slack client per workspace, with the main one named.
pub struct SlackClients<S> {
    clients: BTreeMap<String, S>,
    main: String,
}

impl<S> SlackClients<S> {
    /// `main` must be one of `clients`.
    pub fn new(
        clients: impl IntoIterator<Item = (String, S)>,
        main: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let clients: BTreeMap<String, S> = clients.into_iter().collect();
        let main = main.into();
        anyhow::ensure!(clients.contains_key(&main), "no main workspace {main}");
        Ok(Self { clients, main })
    }

    pub fn main(&self) -> &S {
        &self.clients[&self.main]
    }

    pub fn main_id(&self) -> &str {
        &self.main
    }

    /// The client for `id`, or the main one when none is named.
    pub fn get(&self, id: Option<&str>) -> anyhow::Result<&S> {
        let id = id.filter(|id| !id.is_empty()).unwrap_or(&self.main);
        self.clients.get(id).ok_or_else(|| {
            anyhow::anyhow!(
                "no workspace {id}; known: {}",
                self.clients.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })
    }
}
