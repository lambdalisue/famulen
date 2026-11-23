//! Connecting the known remote MCP servers, starting and finishing a login
//! whichever way it goes, and reading a connection's tools.

use crate::catalog::Known;
use crate::connections::{CONNECTED, Connection, Secret, Transport};

use super::{Gateway, OWNER, logins};

impl Gateway {
    /// Add a connection of a known remote MCP server, or take up one whose
    /// login was never finished, and start its login. Returns the
    /// connection, where to send the owner's browser, and whether the
    /// connection is new.
    pub async fn connect_known(
        &self,
        known: &Known,
        given: &std::collections::BTreeMap<String, String>,
        redirect_uri: &str,
    ) -> anyhow::Result<(Connection, String, bool)> {
        let url = known.address(given)?;
        // A connection of this service whose login was never finished is
        // used again, so pressing twice leaves no empty ones behind.
        let unfinished = self.connections.list()?.into_iter().find(|c| {
            c.kind == known.id
                && matches!(&c.transport, Transport::McpRemote { url: u } if *u == url)
                && !c.secret.holds("bearer")
                && !c.secret.holds("api_key")
        });
        let (c, added) = match unfinished {
            Some(c) => (c, false),
            None => {
                let id = self.fresh_id(known.id)?;
                let c = self.connections.add(
                    &id,
                    known.name,
                    known.id,
                    Transport::McpRemote { url },
                    Secret::default(),
                    "",
                )?;
                self.log_change_by(OWNER, &c.id, "connection.add", known.id)?;
                (c, true)
            }
        };
        // A login that cannot start, as at a mistyped team, leaves no
        // connection behind.
        match self.begin_login(&c.id, redirect_uri).await {
            Ok(login_url) => Ok((c, login_url, added)),
            Err(error) => {
                if added {
                    self.remove_connection(super::GATEWAY, &c.id)?;
                }
                Err(error)
            }
        }
    }

    /// An id for a new connection of `base`, not taken by another.
    pub fn fresh_id(&self, base: &str) -> anyhow::Result<String> {
        let taken: Vec<String> = self.connections.list()?.into_iter().map(|c| c.id).collect();
        Ok((1..)
            .map(|n| {
                if n == 1 {
                    base.to_owned()
                } else {
                    format!("{base}-{n}")
                }
            })
            .find(|id| {
                !taken.contains(id)
                    && !self.reserved.contains(id)
                    && id != super::VAULT
                    && id != super::DEFINITIONS
            })
            .unwrap_or_default())
    }

    /// Start logging in to a connection: the URL to send the owner's browser to.
    pub async fn begin_login(&self, id: &str, redirect_uri: &str) -> anyhow::Result<String> {
        logins::begin(&self.store, &self.http, &self.sealed(), id, redirect_uri).await
    }

    /// Finish a login with the query the provider sent back: the connection
    /// id, and whether this is what finished it. A browser may open the
    /// address it was sent back to more than once; the same answer again is
    /// the login it already finished, not a failure.
    pub async fn finish_login(&self, query: &str) -> anyhow::Result<(String, bool)> {
        const KEPT: usize = 64;
        let mut finished = self.finished.lock().await;
        if let Some(id) = finished.get(query)
            && self.connections.get(id)?.is_some()
        {
            return Ok((id.clone(), false));
        }
        let id = if self.is_defined_login(query)? {
            self.finish_defined_login(query).await?
        } else {
            let id = logins::finish(&self.store, &self.http, &self.sealed(), query).await?;
            self.log_change_by(OWNER, &id, "login", "")?;
            id
        };
        if finished.len() >= KEPT {
            finished.clear();
        }
        finished.insert(query.to_owned(), id.clone());
        Ok((id, true))
    }

    /// Refresh the tokens that are about to expire; whether any changed. The
    /// live clients opened with the old tokens are dropped, so the next call
    /// opens one with the new.
    pub async fn refresh_due(&self) -> anyhow::Result<bool> {
        let changed = logins::refresh_due(&self.http, &self.sealed()).await?;
        let changed = self.refresh_defined().await? || changed;
        if changed {
            self.clients.lock().await.clear();
        }
        Ok(changed)
    }

    /// Ask a connection for its tools and keep them; how many there are.
    pub async fn read_tools(&self, id: &str) -> anyhow::Result<usize> {
        self.clients.lock().await.remove(id);
        if let Some(connection) = self.sealed().get(id)?
            && self.definition_of(&connection).is_some()
        {
            let tools = self.list_tools(&connection).await?;
            self.connections.set_status(id, CONNECTED)?;
            return Ok(tools.len());
        }
        let count = logins::read_tools(&self.sealed(), id).await?;
        // Whose account it is only helps the owner tell it apart.
        if let Some(connection) = self.sealed().get(id)?
            && let Err(error) = self.find_account(&connection).await
        {
            tracing::debug!(%error, connection = id, "cannot tell whose account it is");
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::super::logins::query_param;
    use super::*;
    use crate::connections::Connections;
    use crate::mcp::fake::{Era, FakeServer};
    use crate::store::Store;

    #[tokio::test]
    async fn adding_a_known_server_and_logging_in_to_it_are_logged() {
        let fake = FakeServer::start(Era::Modern, Some("at-1")).await;
        let store = Store::open_in_memory().unwrap();
        let g = Gateway::new(
            store.clone(),
            Connections::new(store),
            reqwest::Client::new(),
            None,
        );
        let known = Known {
            url: Box::leak(fake.url().into_boxed_str()),
            ..*crate::catalog::find("notion").unwrap()
        };

        let (c, url, _) = g
            .connect_known(&known, &Default::default(), "http://localhost/cb")
            .await
            .unwrap();
        let query = url.split_once('?').unwrap().1;
        *fake.challenge.lock().unwrap() = query_param(query, "code_challenge");
        let state = query_param(query, "state").unwrap();
        g.finish_login(&format!("code=good-code&state={state}"))
            .await
            .unwrap();

        let log: Vec<_> = g
            .log_entries(10)
            .unwrap()
            .into_iter()
            .map(|e| (e.actor, e.connection, e.tool))
            .collect();
        let entry = |tool: &str| ("owner".to_owned(), c.id.clone(), tool.to_owned());
        assert_eq!(log, [entry("login"), entry("connection.add")]);
    }

    #[tokio::test]
    async fn a_login_that_cannot_start_leaves_no_connection_behind() {
        let store = Store::open_in_memory().unwrap();
        let g = Gateway::new(
            store.clone(),
            Connections::new(store),
            reqwest::Client::new(),
            None,
        );
        let known = Known {
            url: "http://127.0.0.1:9/mcp",
            asks: &[],
            ..*crate::catalog::find("kibela").unwrap()
        };

        assert!(
            g.connect_known(&known, &Default::default(), "http://localhost/cb")
                .await
                .is_err()
        );

        assert!(g.connections.list().unwrap().is_empty());
    }
}
