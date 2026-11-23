//! Logging in to a remote MCP connection through the MCP authorization flow,
//! keeping its tokens fresh, and reading its tools.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::secrets::Sealed;
use crate::connections::{CONNECTED, Connection, ConnectionTool, LOGIN_AGAIN, Secret, Transport};
use crate::mcp::{
    self, AuthServer, AuthorizationRequest, ClientRegistration, CodeExchange, McpClient, McpError,
    Pkce, ServerConfig,
};
use crate::store::Store;

/// What is kept between sending the owner to log in and their return.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pending {
    connection: String,
    verifier: String,
    redirect_uri: String,
    resource: String,
    server: AuthServer,
    client: ClientRegistration,
}

/// The value of `name` in the query a service sent the owner back with.
pub(super) fn query_param(query: &str, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query.trim_start_matches('?').as_bytes())
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

/// Start logging in: returns the URL to send the owner's browser to.
pub async fn begin(
    store: &Store,
    http: &reqwest::Client,
    connections: &Sealed,
    id: &str,
    redirect_uri: &str,
) -> anyhow::Result<String> {
    let connection = connections.require(id)?;
    let Transport::McpRemote { url } = &connection.transport else {
        anyhow::bail!("{id} is not a remote MCP connection");
    };
    let (resource_metadata, scope) =
        match McpClient::connect(&ServerConfig::http(url.clone()), None).await {
            Err(McpError::Unauthorized {
                resource_metadata,
                scope,
            }) => (resource_metadata, scope),
            Ok(client) => {
                client.close().await;
                anyhow::bail!("{id} needs no login");
            }
            Err(error) => {
                tracing::debug!(%error, "probe before login failed");
                (None, None)
            }
        };
    let resource =
        mcp::discover_protected_resource(http, url, resource_metadata.as_deref()).await?;
    let issuer = resource
        .authorization_servers
        .first()
        .ok_or_else(|| anyhow::anyhow!("{id} names no authorization server"))?;
    let server = mcp::discover_auth_server(http, issuer).await?;
    // The client registered at an earlier login is used again with the same
    // issuer, which knows it.
    let client = match (&connection.secret.client_id, &connection.secret.auth_server) {
        (Some(client_id), Some(saved)) if saved["issuer"] == server.issuer => ClientRegistration {
            client_id: client_id.clone(),
            client_secret: connection.secret.client_secret.clone(),
            token_endpoint_auth_method: None,
        },
        _ => mcp::register_client(http, &server, redirect_uri, "Famulen").await?,
    };
    let pkce = Pkce::generate();
    let state = mcp::generate_state();
    let scope = mcp::select_scope(scope.as_deref(), &resource, &server);
    let canonical = mcp::canonical_resource(url)?;
    let target = mcp::authorization_url(
        &server,
        &AuthorizationRequest {
            client_id: &client.client_id,
            redirect_uri,
            pkce: &pkce,
            state: &state,
            resource: &canonical,
            scope: scope.as_deref(),
        },
    )?;
    let pending = Pending {
        connection: id.to_owned(),
        verifier: pkce.verifier,
        redirect_uri: redirect_uri.to_owned(),
        resource: canonical,
        server,
        client,
    };
    store.set_state(format!("login.{state}"), serde_json::to_string(&pending)?)?;
    Ok(target.to_string())
}

/// Finish logging in with the query the authorization server sent back.
/// Returns the connection id.
pub async fn finish(
    store: &Store,
    http: &reqwest::Client,
    connections: &Sealed,
    query: &str,
) -> anyhow::Result<String> {
    let state = query_param(query, "state")
        .ok_or_else(|| anyhow::anyhow!("the login came back without state"))?;
    let key = format!("login.{state}");
    let pending: Pending = serde_json::from_str(
        &store
            .state(&key)?
            .ok_or_else(|| anyhow::anyhow!("no login is waiting for this state"))?,
    )?;
    store.clear_state(&key)?;
    let code = mcp::authorization_code(&pending.server, &state, query)?;
    let tokens = mcp::exchange_code(
        http,
        &pending.server,
        &pending.client,
        &CodeExchange {
            code: &code,
            verifier: &pending.verifier,
            redirect_uri: &pending.redirect_uri,
            resource: &pending.resource,
        },
    )
    .await?;
    let mut secret = connections
        .get(&pending.connection)?
        .map(|c| c.secret)
        .unwrap_or_default();
    secret.bearer = Some(tokens.access_token);
    secret.refresh_token = tokens.refresh_token;
    secret.expires_at = tokens.expires_at.map(|t| t.timestamp());
    secret.client_id = Some(pending.client.client_id);
    secret.client_secret = pending.client.client_secret;
    secret.auth_server = Some(serde_json::to_value(&pending.server)?);
    secret.resource = Some(pending.resource);
    connections.set_secret(&pending.connection, &secret)?;
    connections.set_status(&pending.connection, CONNECTED)?;
    Ok(pending.connection)
}

/// Refresh every remote connection whose token is about to expire.
pub async fn refresh_due(http: &reqwest::Client, connections: &Sealed) -> anyhow::Result<bool> {
    let mut changed = false;
    for c in connections.list()? {
        let Secret {
            refresh_token: Some(refresh),
            expires_at: Some(expires_at),
            auth_server: Some(server),
            resource: Some(resource),
            client_id: Some(client_id),
            ..
        } = &c.secret
        else {
            continue;
        };
        if *expires_at > Utc::now().timestamp() + 300 {
            continue;
        }
        let server: AuthServer = serde_json::from_value(server.clone())?;
        let client = ClientRegistration {
            client_id: client_id.clone(),
            client_secret: c.secret.client_secret.clone(),
            token_endpoint_auth_method: None,
        };
        match mcp::refresh_tokens(http, &server, &client, refresh, resource).await {
            Ok(tokens) => {
                let mut secret = c.secret.clone();
                secret.bearer = Some(tokens.access_token);
                secret.refresh_token = tokens.refresh_token;
                secret.expires_at = tokens.expires_at.map(|t| t.timestamp());
                connections.set_secret(&c.id, &secret)?;
                connections.set_status(&c.id, CONNECTED)?;
                changed = true;
            }
            Err(error) => {
                connections.set_status(&c.id, format!("{}（{error}）", LOGIN_AGAIN))?;
            }
        }
    }
    Ok(changed)
}

/// The MCP client config for a connection, or none for an API.
pub fn server_config(connection: &Connection) -> Option<ServerConfig> {
    match &connection.transport {
        Transport::McpLocal { command, args, env } => Some(ServerConfig::Stdio {
            command: command.clone(),
            args: args.clone(),
            env: env
                .clone()
                .into_iter()
                .chain(connection.secret.account.clone())
                .collect(),
        }),
        Transport::McpRemote { url } => Some(ServerConfig::Http {
            url: url.clone(),
            headers: connection.secret.key_header().into_iter().collect(),
        }),
        Transport::Api { .. } | Transport::Defined { .. } => None,
    }
}

/// Ask a connection's server for its tools and keep them.
pub async fn read_tools(connections: &Sealed, id: &str) -> anyhow::Result<usize> {
    let connection = connections.require(id)?;
    let tools: Vec<ConnectionTool> = match server_config(&connection) {
        Some(config) => {
            let client =
                match McpClient::connect(&config, connection.secret.bearer.as_deref()).await {
                    Ok(client) => client,
                    Err(McpError::Unauthorized { .. }) => {
                        connections.set_status(id, LOGIN_AGAIN)?;
                        anyhow::bail!("{id} needs a login");
                    }
                    Err(error) => {
                        connections.set_status(id, format!("つながらない（{error}）"))?;
                        return Err(error.into());
                    }
                };
            let tools = client.list_tools().await?;
            client.close().await;
            tools
                .into_iter()
                .map(|t| ConnectionTool {
                    name: t.name,
                    description: t.description.unwrap_or_default(),
                    read_only: t.read_only,
                    always_ask: t.always_ask,
                })
                .collect()
        }
        None => super::listed(&super::api_tools(&connection.name)),
    };
    connections.record_tools(id, &tools)?;
    connections.set_status(id, CONNECTED)?;
    Ok(tools.len())
}
