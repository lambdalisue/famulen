//! Calling a connection that is an HTTP API with no definition.

use serde_json::Value;

use super::{Connection, Transport};

/// A call of an HTTP API connection.
#[derive(Debug, Clone)]
pub struct ApiRequest {
    pub method: String,
    pub path: String,
    pub query: Option<Value>,
    pub body: Option<Value>,
}

/// Call an API connection and return its status and body.
pub async fn api_call(
    http: &reqwest::Client,
    connection: &Connection,
    args: &ApiRequest,
) -> anyhow::Result<(u16, String)> {
    let Transport::Api { base_url, .. } = &connection.transport else {
        anyhow::bail!("{} is not an API connection", connection.id);
    };
    let method = reqwest::Method::from_bytes(args.method.to_ascii_uppercase().as_bytes())?;
    let url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        args.path.trim_start_matches('/')
    );
    // Bounded, so a call that never answers cannot hold the next session
    // back.
    let mut request = http
        .request(method, url)
        .timeout(std::time::Duration::from_secs(60));
    if let Some(query) = &args.query {
        request = request.query(query);
    }
    if let Some(body) = &args.body {
        request = request.json(body);
    }
    if connection.secret.api_key.is_none()
        && let Some(bearer) = &connection.secret.bearer
    {
        request = request.bearer_auth(bearer);
    }
    if let Some((header, value)) = connection.secret.key_header() {
        request = request.header(header, value);
    }
    let response = request.send().await?;
    let status = response.status().as_u16();
    Ok((status, response.text().await?))
}
