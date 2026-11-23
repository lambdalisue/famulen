//! Logging in to a remote MCP server: the MCP authorization flow on top of
//! OAuth 2.1 with PKCE.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthError {
    /// A metadata document could not be found or is not to be trusted.
    Discovery(String),
    /// The authorization server refused, with an OAuth error code.
    Rejected {
        error: String,
        description: Option<String>,
    },
    /// The server answered with an HTTP status that carries no OAuth error.
    Http {
        status: u16,
        body: String,
    },
    Transport(String),
    /// A URL or a callback that does not hold together.
    Invalid(String),
}

impl fmt::Display for OAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Discovery(message) => write!(f, "discovery failed: {message}"),
            Self::Rejected { error, description } => match description {
                Some(description) => write!(f, "{error}: {description}"),
                None => write!(f, "{error}"),
            },
            Self::Http { status, body } => write!(f, "HTTP {status}: {body}"),
            Self::Transport(message) => write!(f, "transport error: {message}"),
            Self::Invalid(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for OAuthError {}

fn transport(error: reqwest::Error) -> OAuthError {
    OAuthError::Transport(error.to_string())
}

fn parse_url(url: &str) -> Result<Url, OAuthError> {
    Url::parse(url).map_err(|error| OAuthError::Invalid(format!("bad URL {url:?}: {error}")))
}

/// The `Bearer` challenge of a `WWW-Authenticate` header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Challenge {
    pub resource_metadata: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

/// Reads the `Bearer` challenge out of a `WWW-Authenticate` header, which
/// may hold challenges of other schemes too.
pub fn parse_www_authenticate(header: &str) -> Option<Challenge> {
    let mut challenge: Option<Challenge> = None;
    let mut in_bearer = false;
    let mut chars = header.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace() || *c == ',').is_some() {}
        let mut token = String::new();
        while let Some(c) = chars.next_if(|c| !c.is_whitespace() && *c != '=' && *c != ',') {
            token.push(c);
        }
        if token.is_empty() {
            break;
        }
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        if chars.next_if_eq(&'=').is_none() {
            in_bearer = token.eq_ignore_ascii_case("bearer");
            if in_bearer && challenge.is_none() {
                challenge = Some(Challenge::default());
            }
            continue;
        }
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let mut value = String::new();
        if chars.next_if_eq(&'"').is_some() {
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => value.extend(chars.next()),
                    c => value.push(c),
                }
            }
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace() && *c != ',') {
                value.push(c);
            }
        }
        if let (true, Some(challenge)) = (in_bearer, challenge.as_mut()) {
            match token.to_ascii_lowercase().as_str() {
                "resource_metadata" => challenge.resource_metadata = Some(value),
                "scope" => challenge.scope = Some(value),
                "error" => challenge.error = Some(value),
                _ => {}
            }
        }
    }
    challenge
}

/// The URI that names an MCP server as an OAuth resource: no fragment, and
/// no trailing slash on a bare origin.
pub fn canonical_resource(url: impl AsRef<str>) -> Result<String, OAuthError> {
    let mut url = parse_url(url.as_ref())?;
    url.set_fragment(None);
    let text = url.to_string();
    Ok(match (url.path(), url.query()) {
        ("/", None) => text.trim_end_matches('/').to_string(),
        _ => text,
    })
}

fn origin_with_path(url: &Url, path: &str) -> Url {
    let mut url = url.clone();
    url.set_query(None);
    url.set_fragment(None);
    url.set_path(path);
    url
}

fn trimmed_path(url: &Url) -> &str {
    url.path().trim_end_matches('/')
}

/// The protected resource metadata (RFC 9728) of an MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedResource {
    pub resource: String,
    #[serde(default)]
    pub authorization_servers: Vec<String>,
    #[serde(default)]
    pub scopes_supported: Option<Vec<String>>,
}

/// Where to look for protected resource metadata when the server's 401 did
/// not say, most specific first.
pub fn protected_resource_metadata_urls(
    server_url: impl AsRef<str>,
) -> Result<Vec<Url>, OAuthError> {
    let url = parse_url(server_url.as_ref())?;
    let path = trimmed_path(&url);
    let root = origin_with_path(&url, "/.well-known/oauth-protected-resource");
    Ok(if path.is_empty() {
        vec![root]
    } else {
        vec![
            origin_with_path(
                &url,
                &format!("/.well-known/oauth-protected-resource{path}"),
            ),
            root,
        ]
    })
}

/// Fetches the protected resource metadata of the server at `server_url`,
/// from the URL its 401 named or from the well-known locations, and checks
/// that it describes that server.
pub async fn discover_protected_resource(
    http: &reqwest::Client,
    server_url: impl AsRef<str>,
    resource_metadata: Option<&str>,
) -> Result<ProtectedResource, OAuthError> {
    let server_url = server_url.as_ref();
    let candidates = match resource_metadata {
        Some(url) => vec![parse_url(url)?],
        None => protected_resource_metadata_urls(server_url)?,
    };
    for candidate in candidates {
        let response = http
            .get(candidate.clone())
            .send()
            .await
            .map_err(transport)?;
        if !response.status().is_success() {
            tracing::debug!(url = %candidate, status = %response.status(), "no protected resource metadata");
            continue;
        }
        let mut metadata: ProtectedResource = response.json().await.map_err(|error| {
            OAuthError::Discovery(format!(
                "unreadable protected resource metadata at {candidate}: {error}"
            ))
        })?;
        if !resource_covers(&metadata.resource, server_url)? {
            return Err(OAuthError::Discovery(format!(
                "the metadata at {candidate} describes {}, not {server_url}",
                metadata.resource
            )));
        }
        if metadata.authorization_servers.is_empty() {
            return Err(OAuthError::Discovery(format!(
                "the metadata at {candidate} names no authorization server"
            )));
        }
        metadata.resource = canonical_resource(&metadata.resource)?;
        return Ok(metadata);
    }
    Err(OAuthError::Discovery(format!(
        "{server_url} publishes no protected resource metadata"
    )))
}

/// Whether `resource` names the server at `server_url` or a path above it on
/// the same origin.
fn resource_covers(resource: &str, server_url: &str) -> Result<bool, OAuthError> {
    let resource = parse_url(resource)?;
    let server = parse_url(server_url)?;
    if resource.origin() != server.origin() {
        return Ok(false);
    }
    let prefix = trimmed_path(&resource);
    let path = trimmed_path(&server);
    Ok(path == prefix || path.starts_with(&format!("{prefix}/")))
}

/// The metadata of an OAuth authorization server (RFC 8414 or OpenID
/// Connect Discovery).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthServer {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Option<Vec<String>>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    pub client_id_metadata_document_supported: bool,
    #[serde(default)]
    pub authorization_response_iss_parameter_supported: bool,
}

/// Where an issuer tells of the user a token is for, when it speaks OpenID
/// Connect; only an address on the issuer's own origin counts.
pub async fn discover_userinfo(
    http: &reqwest::Client,
    issuer: impl AsRef<str>,
) -> Result<Url, OAuthError> {
    let issuer = issuer.as_ref();
    let origin = parse_url(issuer)?.origin();
    for candidate in authorization_server_metadata_urls(issuer)? {
        let Ok(response) = http.get(candidate).send().await else {
            continue;
        };
        let Ok(metadata) = response.json::<serde_json::Value>().await else {
            continue;
        };
        if metadata["issuer"] != issuer {
            continue;
        }
        if let Some(url) = metadata["userinfo_endpoint"]
            .as_str()
            .and_then(|u| parse_url(u).ok())
            .filter(|u| u.origin() == origin)
        {
            return Ok(url);
        }
    }
    Err(OAuthError::Discovery(format!(
        "{issuer} names no userinfo endpoint"
    )))
}

/// Where to look for an issuer's metadata, in the order the MCP
/// authorization spec gives.
pub fn authorization_server_metadata_urls(issuer: impl AsRef<str>) -> Result<Vec<Url>, OAuthError> {
    let url = parse_url(issuer.as_ref())?;
    let path = trimmed_path(&url);
    Ok(if path.is_empty() {
        vec![
            origin_with_path(&url, "/.well-known/oauth-authorization-server"),
            origin_with_path(&url, "/.well-known/openid-configuration"),
        ]
    } else {
        vec![
            origin_with_path(
                &url,
                &format!("/.well-known/oauth-authorization-server{path}"),
            ),
            origin_with_path(&url, &format!("/.well-known/openid-configuration{path}")),
            origin_with_path(&url, &format!("{path}/.well-known/openid-configuration")),
        ]
    })
}

/// Fetches and checks the metadata of `issuer`. A server whose metadata
/// names another issuer, or that cannot do PKCE with S256, is refused.
pub async fn discover_auth_server(
    http: &reqwest::Client,
    issuer: impl AsRef<str>,
) -> Result<AuthServer, OAuthError> {
    let issuer = issuer.as_ref();
    for candidate in authorization_server_metadata_urls(issuer)? {
        let response = http
            .get(candidate.clone())
            .send()
            .await
            .map_err(transport)?;
        if !response.status().is_success() {
            tracing::debug!(url = %candidate, status = %response.status(), "no authorization server metadata");
            continue;
        }
        let server: AuthServer = response.json().await.map_err(|error| {
            OAuthError::Discovery(format!(
                "unreadable authorization server metadata at {candidate}: {error}"
            ))
        })?;
        if server.issuer != issuer {
            return Err(OAuthError::Discovery(format!(
                "the metadata at {candidate} names issuer {}, not {issuer}",
                server.issuer
            )));
        }
        if !server
            .code_challenge_methods_supported
            .iter()
            .any(|method| method == "S256")
        {
            return Err(OAuthError::Discovery(format!(
                "{issuer} does not support PKCE with S256"
            )));
        }
        return Ok(server);
    }
    Err(OAuthError::Discovery(format!(
        "{issuer} publishes no authorization server metadata"
    )))
}

/// The scopes to ask for: what the server's challenge named, otherwise what
/// the resource lists, plus `offline_access` when the authorization server
/// offers it so that a refresh token comes back.
pub fn select_scope(
    challenge: Option<&str>,
    resource: &ProtectedResource,
    server: &AuthServer,
) -> Option<String> {
    let mut scopes: Vec<String> = match challenge {
        Some(scope) => scope.split_whitespace().map(String::from).collect(),
        None => resource.scopes_supported.clone().unwrap_or_default(),
    };
    let offline = server
        .scopes_supported
        .iter()
        .flatten()
        .any(|scope| scope == "offline_access");
    if offline && !scopes.iter().any(|scope| scope == "offline_access") {
        scopes.push("offline_access".into());
    }
    (!scopes.is_empty()).then(|| scopes.join(" "))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientRegistration {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub token_endpoint_auth_method: Option<String>,
}

/// The dynamic client registration (RFC 7591) request for a public client
/// that receives the code at `redirect_uri`.
pub fn registration_request(redirect_uri: impl AsRef<str>, client_name: impl AsRef<str>) -> Value {
    let redirect_uri = redirect_uri.as_ref();
    let local = Url::parse(redirect_uri)
        .ok()
        .and_then(|url| url.host_str().map(String::from))
        .is_some_and(|host| matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]"));
    json!({
        "client_name": client_name.as_ref(),
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": if local { "native" } else { "web" },
    })
}

/// Registers the gateway as a client of `server`.
pub async fn register_client(
    http: &reqwest::Client,
    server: &AuthServer,
    redirect_uri: impl AsRef<str>,
    client_name: impl AsRef<str>,
) -> Result<ClientRegistration, OAuthError> {
    let endpoint = server.registration_endpoint.as_deref().ok_or_else(|| {
        OAuthError::Discovery(format!(
            "{} does not offer client registration",
            server.issuer
        ))
    })?;
    let response = http
        .post(endpoint)
        .json(&registration_request(redirect_uri, client_name))
        .send()
        .await
        .map_err(transport)?;
    let mut registration: ClientRegistration = read_json(response).await?;
    if registration.token_endpoint_auth_method.is_none() {
        registration.token_endpoint_auth_method = Some(
            if registration.client_secret.is_some() {
                "client_secret_basic"
            } else {
                "none"
            }
            .into(),
        );
    }
    Ok(registration)
}

/// A PKCE code verifier and its S256 challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        Self::from_verifier(random_token())
    }

    pub fn from_verifier(verifier: impl Into<String>) -> Self {
        let verifier = verifier.into();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier,
            challenge,
        }
    }
}

/// An unguessable value for the `state` parameter.
pub fn generate_state() -> String {
    random_token()
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Debug, Clone, Copy)]
pub struct AuthorizationRequest<'a> {
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub pkce: &'a Pkce,
    pub state: &'a str,
    /// The canonical URI of the MCP server the token is for.
    pub resource: &'a str,
    pub scope: Option<&'a str>,
}

/// The URL to send the owner's browser to.
pub fn authorization_url(
    server: &AuthServer,
    request: &AuthorizationRequest<'_>,
) -> Result<Url, OAuthError> {
    let mut url = parse_url(&server.authorization_endpoint)?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", request.client_id)
            .append_pair("redirect_uri", request.redirect_uri)
            .append_pair("code_challenge", &request.pkce.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", request.state)
            .append_pair("resource", request.resource);
        if let Some(scope) = request.scope {
            query.append_pair("scope", scope);
        }
    }
    Ok(url)
}

/// The authorization code from the query of the redirect back,
/// once the issuer (RFC 9207) and the state check out.
pub fn authorization_code(
    server: &AuthServer,
    expected_state: impl AsRef<str>,
    query: impl AsRef<str>,
) -> Result<String, OAuthError> {
    let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_ref().as_bytes())
        .into_owned()
        .collect();
    let param = |name: &str| {
        params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    match param("iss") {
        Some(iss) if iss != server.issuer => {
            return Err(OAuthError::Invalid(format!(
                "the callback came from issuer {iss}, not {}",
                server.issuer
            )));
        }
        None if server.authorization_response_iss_parameter_supported => {
            return Err(OAuthError::Invalid("the callback is missing iss".into()));
        }
        _ => {}
    }
    if param("state") != Some(expected_state.as_ref()) {
        return Err(OAuthError::Invalid(
            "the callback state does not match".into(),
        ));
    }
    if let Some(error) = param("error") {
        return Err(OAuthError::Rejected {
            error: error.into(),
            description: param("error_description").map(String::from),
        });
    }
    param("code")
        .map(String::from)
        .ok_or_else(|| OAuthError::Invalid("the callback has no code".into()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub token_type: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
}

impl TokenResponse {
    fn into_tokens(self, now: DateTime<Utc>) -> Tokens {
        Tokens {
            access_token: self.access_token,
            token_type: self.token_type.unwrap_or_else(|| "Bearer".into()),
            refresh_token: self.refresh_token,
            expires_at: self
                .expires_in
                .map(|seconds| now + chrono::Duration::seconds(seconds)),
            scope: self.scope,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CodeExchange<'a> {
    pub code: &'a str,
    pub verifier: &'a str,
    pub redirect_uri: &'a str,
    pub resource: &'a str,
}

/// Trades the authorization code for tokens.
pub async fn exchange_code(
    http: &reqwest::Client,
    server: &AuthServer,
    client: &ClientRegistration,
    exchange: &CodeExchange<'_>,
) -> Result<Tokens, OAuthError> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", exchange.code),
        ("code_verifier", exchange.verifier),
        ("redirect_uri", exchange.redirect_uri),
        ("resource", exchange.resource),
    ];
    request_tokens(http, server, client, &form).await
}

/// Gets a new access token. A server that does not rotate refresh tokens
/// leaves the old one in place.
pub async fn refresh_tokens(
    http: &reqwest::Client,
    server: &AuthServer,
    client: &ClientRegistration,
    refresh_token: impl AsRef<str>,
    resource: impl AsRef<str>,
) -> Result<Tokens, OAuthError> {
    let refresh_token = refresh_token.as_ref();
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("resource", resource.as_ref()),
    ];
    let mut tokens = request_tokens(http, server, client, &form).await?;
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = Some(refresh_token.into());
    }
    Ok(tokens)
}

async fn request_tokens(
    http: &reqwest::Client,
    server: &AuthServer,
    client: &ClientRegistration,
    form: &[(&str, &str)],
) -> Result<Tokens, OAuthError> {
    let mut form = form.to_vec();
    let mut request = http.post(&server.token_endpoint);
    match (
        &client.client_secret,
        client.token_endpoint_auth_method.as_deref(),
    ) {
        (Some(secret), Some("client_secret_basic")) => {
            request = request.basic_auth(&client.client_id, Some(secret));
        }
        (Some(secret), _) => {
            form.push(("client_id", &client.client_id));
            form.push(("client_secret", secret));
        }
        (None, _) => form.push(("client_id", &client.client_id)),
    }
    let response = request.form(&form).send().await.map_err(transport)?;
    let response: TokenResponse = read_json(response).await?;
    Ok(response.into_tokens(Utc::now()))
}

async fn read_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, OAuthError> {
    let status = response.status();
    let body = response.text().await.map_err(transport)?;
    if !status.is_success() {
        let error: Option<Value> = serde_json::from_str(&body).ok();
        return Err(
            match error
                .as_ref()
                .and_then(|error| error.get("error")?.as_str())
            {
                Some(code) => OAuthError::Rejected {
                    error: code.into(),
                    description: error
                        .as_ref()
                        .and_then(|error| error.get("error_description")?.as_str())
                        .map(String::from),
                },
                None => OAuthError::Http {
                    status: status.as_u16(),
                    body,
                },
            },
        );
    }
    serde_json::from_str(&body)
        .map_err(|error| OAuthError::Invalid(format!("unreadable answer: {error}: {body}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> AuthServer {
        AuthServer {
            issuer: "https://auth.example.com".into(),
            authorization_endpoint: "https://auth.example.com/authorize".into(),
            token_endpoint: "https://auth.example.com/token".into(),
            registration_endpoint: None,
            scopes_supported: None,
            code_challenge_methods_supported: vec!["S256".into()],
            client_id_metadata_document_supported: false,
            authorization_response_iss_parameter_supported: false,
        }
    }

    fn strings(urls: Vec<Url>) -> Vec<String> {
        urls.into_iter().map(String::from).collect()
    }

    #[test]
    fn the_bearer_challenge_is_found_among_others() {
        let header = r#"Basic realm="x", Bearer error="invalid_token", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource", scope="files:read files:write""#;

        assert_eq!(
            parse_www_authenticate(header),
            Some(Challenge {
                resource_metadata: Some(
                    "https://mcp.example.com/.well-known/oauth-protected-resource".into()
                ),
                scope: Some("files:read files:write".into()),
                error: Some("invalid_token".into()),
            })
        );
    }

    #[test]
    fn a_header_without_bearer_has_no_challenge() {
        assert_eq!(parse_www_authenticate(r#"Basic realm="x""#), None);
        assert_eq!(parse_www_authenticate("Bearer"), Some(Challenge::default()));
        assert_eq!(
            parse_www_authenticate(r#"bearer scope=read, realm="a \"b\"""#),
            Some(Challenge {
                scope: Some("read".into()),
                ..Challenge::default()
            })
        );
    }

    #[test]
    fn the_canonical_resource_drops_fragments_and_a_bare_trailing_slash() {
        assert_eq!(
            canonical_resource("HTTPS://MCP.Example.com/").unwrap(),
            "https://mcp.example.com"
        );
        assert_eq!(
            canonical_resource("https://mcp.example.com:8443/mcp#x").unwrap(),
            "https://mcp.example.com:8443/mcp"
        );
        assert!(canonical_resource("mcp.example.com").is_err());
    }

    #[test]
    fn protected_resource_metadata_is_looked_for_under_the_path_then_at_the_root() {
        assert_eq!(
            strings(
                protected_resource_metadata_urls("https://example.com/public/mcp?x=1").unwrap()
            ),
            vec![
                "https://example.com/.well-known/oauth-protected-resource/public/mcp",
                "https://example.com/.well-known/oauth-protected-resource",
            ]
        );
        assert_eq!(
            strings(protected_resource_metadata_urls("https://example.com/").unwrap()),
            vec!["https://example.com/.well-known/oauth-protected-resource"]
        );
    }

    #[test]
    fn authorization_server_metadata_is_looked_for_in_the_spec_order() {
        assert_eq!(
            strings(
                authorization_server_metadata_urls("https://auth.example.com/tenant1").unwrap()
            ),
            vec![
                "https://auth.example.com/.well-known/oauth-authorization-server/tenant1",
                "https://auth.example.com/.well-known/openid-configuration/tenant1",
                "https://auth.example.com/tenant1/.well-known/openid-configuration",
            ]
        );
        assert_eq!(
            strings(authorization_server_metadata_urls("https://auth.example.com").unwrap()),
            vec![
                "https://auth.example.com/.well-known/oauth-authorization-server",
                "https://auth.example.com/.well-known/openid-configuration",
            ]
        );
    }

    #[test]
    fn a_resource_covers_its_own_path_and_paths_below_it_on_the_same_origin() {
        assert!(
            resource_covers("https://mcp.example.com/mcp", "https://mcp.example.com/mcp").unwrap()
        );
        assert!(resource_covers("https://mcp.example.com", "https://mcp.example.com/mcp").unwrap());
        assert!(
            !resource_covers("https://mcp.example.com/mc", "https://mcp.example.com/mcp").unwrap()
        );
        assert!(
            !resource_covers(
                "https://evil.example.com/mcp",
                "https://mcp.example.com/mcp"
            )
            .unwrap()
        );
    }

    #[test]
    fn the_challenge_scope_wins_over_the_resource_scopes() {
        let resource = ProtectedResource {
            resource: "https://mcp.example.com".into(),
            authorization_servers: vec![],
            scopes_supported: Some(vec!["a".into(), "b".into()]),
        };
        let mut offline = server();
        offline.scopes_supported = Some(vec!["offline_access".into()]);

        assert_eq!(
            select_scope(Some("files:read"), &resource, &server()).as_deref(),
            Some("files:read")
        );
        assert_eq!(
            select_scope(None, &resource, &server()).as_deref(),
            Some("a b")
        );
        assert_eq!(
            select_scope(None, &resource, &offline).as_deref(),
            Some("a b offline_access")
        );
        let bare = ProtectedResource {
            scopes_supported: None,
            ..resource
        };
        assert_eq!(select_scope(None, &bare, &server()), None);
    }

    #[test]
    fn the_pkce_challenge_is_the_s256_of_the_verifier() {
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");

        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_generated_verifier_is_long_enough_and_fresh_each_time() {
        let one = Pkce::generate();
        let two = Pkce::generate();

        assert_eq!(one.verifier.len(), 43);
        assert_ne!(one.verifier, two.verifier);
        assert_eq!(Pkce::from_verifier(one.verifier.clone()), one);
        assert_ne!(generate_state(), generate_state());
    }

    #[test]
    fn the_authorization_url_carries_pkce_state_resource_and_scope() {
        let pkce = Pkce::from_verifier("verifier");
        let request = AuthorizationRequest {
            client_id: "client-1",
            redirect_uri: "https://famulen.example.com/oauth/callback",
            pkce: &pkce,
            state: "state-1",
            resource: "https://mcp.example.com/mcp",
            scope: Some("files:read offline_access"),
        };

        let url = authorization_url(&server(), &request).unwrap();

        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert_eq!(url.path(), "/authorize");
        assert_eq!(
            query,
            [
                ("response_type", "code"),
                ("client_id", "client-1"),
                ("redirect_uri", "https://famulen.example.com/oauth/callback"),
                ("code_challenge", pkce.challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("state", "state-1"),
                ("resource", "https://mcp.example.com/mcp"),
                ("scope", "files:read offline_access"),
            ]
            .map(|(key, value)| (key.to_string(), value.to_string()))
        );
    }

    #[test]
    fn a_registration_request_is_for_a_public_client_of_the_right_type() {
        let web = registration_request("https://famulen.example.com/cb", "Famulen");
        let native = registration_request("http://127.0.0.1:8080/cb", "Famulen");

        assert_eq!(
            web,
            json!({
                "client_name": "Famulen",
                "redirect_uris": ["https://famulen.example.com/cb"],
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
                "application_type": "web",
            })
        );
        assert_eq!(native["application_type"], json!("native"));
    }

    #[test]
    fn the_callback_yields_the_code_when_state_and_issuer_match() {
        let code = authorization_code(
            &server(),
            "s1",
            "code=abc&state=s1&iss=https%3A%2F%2Fauth.example.com",
        );

        assert_eq!(code.unwrap(), "abc");
    }

    #[test]
    fn the_callback_is_refused_on_a_wrong_state_or_issuer() {
        let mut strict = server();
        strict.authorization_response_iss_parameter_supported = true;

        assert!(authorization_code(&server(), "s1", "code=abc&state=s2").is_err());
        assert!(
            authorization_code(
                &server(),
                "s1",
                "code=abc&state=s1&iss=https%3A%2F%2Fevil.example.com"
            )
            .is_err()
        );
        assert!(authorization_code(&strict, "s1", "code=abc&state=s1").is_err());
        assert_eq!(
            authorization_code(&server(), "s1", "error=access_denied&state=s1"),
            Err(OAuthError::Rejected {
                error: "access_denied".into(),
                description: None
            })
        );
    }

    #[test]
    fn tokens_expire_from_the_time_they_were_received() {
        let now = Utc::now();
        let response = TokenResponse {
            access_token: "a".into(),
            token_type: None,
            refresh_token: None,
            expires_in: Some(3600),
            scope: None,
        };

        let tokens = response.into_tokens(now);

        assert_eq!(tokens.token_type, "Bearer");
        assert_eq!(
            tokens.expires_at,
            Some(now + chrono::Duration::seconds(3600))
        );
    }
}
