//! MCP servers and an OAuth authorization server that record what the
//! client does with them, for tests.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::Router;
use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::config::ServerConfig;

/// A stdio MCP server in Perl, which every CI runner has. `FAKE_MCP_ERA`
/// picks `modern` or `legacy`; the `seen` tool reports the methods it got.
const STDIO_SERVER: &str = r#"
use strict; use warnings; use JSON::PP;
$| = 1;
my $json = JSON::PP->new->canonical;
my $era = $ENV{FAKE_MCP_ERA} // 'legacy';
my @seen;
sub send_msg { print $json->encode($_[0]), "\n" }
sub reply { send_msg({jsonrpc => "2.0", id => $_[0], result => $_[1]}) }
sub fail {
  my ($id, $code, $message, $data) = @_;
  my $error = {code => $code, message => $message};
  $error->{data} = $data if defined $data;
  send_msg({jsonrpc => "2.0", id => $id, error => $error});
}
my @pages = (
  {tools => [{name => "echo", description => "Echo text", inputSchema => {type => "object"},
              annotations => {readOnlyHint => JSON::PP::true}}], nextCursor => "page-2"},
  {tools => [{name => "fail", inputSchema => {type => "object"}},
             {name => "seen", inputSchema => {type => "object"}}]},
);
while (my $line = <STDIN>) {
  my $msg = eval { $json->decode($line) } or next;
  my $method = $msg->{method};
  my $id = $msg->{id};
  if (!defined $method) {
    push @seen, "response:$id:" . ($msg->{error} ? $msg->{error}{code} : "ok");
    next;
  }
  push @seen, $method;
  next unless defined $id;
  if ($era eq 'modern') {
    my $version = $msg->{params}{_meta}{'io.modelcontextprotocol/protocolVersion'} // '';
    if ($method eq 'initialize') { fail($id, -32601, "Method not found"); next }
    if ($version ne '2026-07-28') {
      fail($id, -32022, "Unsupported protocol version", {supported => ["2026-07-28"], requested => $version});
      next;
    }
    if ($method eq 'server/discover') {
      reply($id, {resultType => "complete", supportedVersions => ["2026-07-28"], capabilities => {tools => {}},
                  _meta => {"io.modelcontextprotocol/serverInfo" => {name => "fake", version => "1"}},
                  instructions => "Be kind."});
      next;
    }
  } else {
    if ($method eq 'server/discover') { fail($id, -32601, "Method not found"); next }
    if ($method eq 'initialize') {
      reply($id, {protocolVersion => "2025-11-25", capabilities => {tools => {}},
                  serverInfo => {name => "fake", version => "1"}});
      next;
    }
  }
  if ($method eq 'tools/list') {
    send_msg({jsonrpc => "2.0", id => "from-server", method => "roots/list"}) if $era eq 'legacy';
    send_msg({jsonrpc => "2.0", method => "notifications/message", params => {level => "info", data => "listing"}});
    my $cursor = $msg->{params}{cursor} // '';
    reply($id, $cursor eq 'page-2' ? $pages[1] : $pages[0]);
    next;
  }
  if ($method eq 'tools/call') {
    my $name = $msg->{params}{name};
    if ($name eq 'echo') {
      reply($id, {content => [{type => "text", text => $msg->{params}{arguments}{text}}], isError => JSON::PP::false});
    } elsif ($name eq 'fail') {
      reply($id, {content => [{type => "text", text => "it broke"}], isError => JSON::PP::true});
    } elsif ($name eq 'seen') {
      reply($id, {content => [{type => "text", text => join(",", @seen)}]});
    } else {
      fail($id, -32602, "Unknown tool: $name");
    }
    next;
  }
  fail($id, -32601, "Method not found");
}
"#;

pub(crate) fn stdio_server(era: &str) -> ServerConfig {
    ServerConfig::Stdio {
        command: "perl".into(),
        args: vec!["-e".into(), STDIO_SERVER.into()],
        env: BTreeMap::from([("FAKE_MCP_ERA".into(), era.into())]),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Era {
    Modern,
    Legacy,
}

/// A request the fake received, with header names in lower case.
#[derive(Debug, Clone)]
pub(crate) struct Received {
    pub(crate) http_method: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Value,
}

impl Received {
    pub(crate) fn method(&self) -> Option<&str> {
        self.body.get("method").and_then(Value::as_str)
    }

    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

pub(crate) struct FakeServer {
    era: Era,
    token: Option<String>,
    base: OnceLock<String>,
    received: Mutex<Vec<Received>>,
    /// The PKCE challenge the token endpoint holds the verifier to.
    pub(crate) challenge: Mutex<Option<String>>,
    pub(crate) registrations: Mutex<Vec<Value>>,
    pub(crate) token_requests: Mutex<Vec<BTreeMap<String, String>>>,
}

impl FakeServer {
    /// Serves an MCP endpoint at `/mcp`, its protected resource metadata
    /// and an authorization server on one origin. With a `token`, `/mcp`
    /// wants it as a bearer.
    pub(crate) async fn start(era: Era, token: Option<&str>) -> Arc<Self> {
        let fake = Arc::new(Self {
            era,
            token: token.map(String::from),
            base: OnceLock::new(),
            received: Mutex::default(),
            challenge: Mutex::default(),
            registrations: Mutex::default(),
            token_requests: Mutex::default(),
        });
        let app = Router::new()
            .route("/mcp", post(mcp).delete(end_session))
            .route(
                "/.well-known/oauth-protected-resource/mcp",
                get(protected_resource),
            )
            .route(
                "/.well-known/oauth-authorization-server",
                get(authorization_server),
            )
            .route(
                "/tenant/.well-known/openid-configuration",
                get(tenant_configuration),
            )
            .route(
                "/.well-known/oauth-authorization-server/impostor",
                get(impostor),
            )
            .route("/tenant/me", get(userinfo))
            .route("/register", post(register))
            .route("/token", post(token_endpoint))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        fake.base
            .set(format!("http://{}", listener.local_addr().unwrap()))
            .unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        fake
    }

    pub(crate) fn base(&self) -> &str {
        self.base.get().unwrap()
    }

    pub(crate) fn url(&self) -> String {
        format!("{}/mcp", self.base())
    }

    pub(crate) fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }

    pub(crate) fn received_method(&self, method: &str) -> Vec<Received> {
        self.received()
            .into_iter()
            .filter(|received| received.method() == Some(method))
            .collect()
    }
}

fn lowercase(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(status: StatusCode, id: &Value, code: i64, message: &str) -> Response {
    let body = json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}});
    (status, axum::Json(body)).into_response()
}

fn sse(messages: &[Value]) -> Response {
    let body: String = messages
        .iter()
        .map(|message| format!(": keep-alive\nevent: message\ndata: {message}\n\n"))
        .collect();
    ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response()
}

async fn mcp(State(fake): State<Arc<FakeServer>>, headers: HeaderMap, body: String) -> Response {
    let received = Received {
        http_method: "POST".into(),
        headers: lowercase(&headers),
        body: serde_json::from_str(&body).unwrap_or(Value::Null),
    };
    fake.received.lock().unwrap().push(received.clone());

    if let Some(token) = &fake.token
        && received.header("authorization") != Some(&format!("Bearer {token}"))
    {
        let challenge = format!(
            r#"Bearer resource_metadata="{}/.well-known/oauth-protected-resource/mcp", scope="files:read""#,
            fake.base()
        );
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, challenge)],
        )
            .into_response();
    }
    let Some(method) = received.method().map(String::from) else {
        return StatusCode::ACCEPTED.into_response();
    };
    let Some(id) = received.body.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let params = received.body.get("params").cloned().unwrap_or(Value::Null);
    match fake.era {
        Era::Modern => modern(&received, &method, &id, &params),
        Era::Legacy => legacy(&received, &method, &id, &params),
    }
}

fn modern(received: &Received, method: &str, id: &Value, params: &Value) -> Response {
    let version = params
        .pointer("/_meta/io.modelcontextprotocol~1protocolVersion")
        .and_then(Value::as_str);
    if version != Some("2026-07-28") || received.header("mcp-protocol-version") != version {
        return rpc_error(
            StatusCode::BAD_REQUEST,
            id,
            -32022,
            "Unsupported protocol version",
        );
    }
    if received.header("mcp-method") != Some(method) {
        return rpc_error(StatusCode::BAD_REQUEST, id, -32020, "Mcp-Method mismatch");
    }
    match method {
        "server/discover" => axum::Json(rpc_result(
            id,
            json!({
                "resultType": "complete",
                "supportedVersions": ["2026-07-28", "2025-11-25"],
                "capabilities": {"tools": {}},
                "_meta": {"io.modelcontextprotocol/serverInfo": {"name": "fake-http", "version": "1"}},
            }),
        ))
        .into_response(),
        "tools/list" => {
            let page = if params.get("cursor").and_then(Value::as_str) == Some("2") {
                json!({"resultType": "complete", "tools": [{"name": "echo", "inputSchema": {"type": "object"}}]})
            } else {
                json!({
                    "resultType": "complete",
                    "tools": [
                        {"name": "execute_sql", "inputSchema": {"type": "object", "properties": {
                            "region": {"type": "string", "x-mcp-header": "Region"},
                            "query": {"type": "string"}
                        }}},
                        {"name": "bad_header", "inputSchema": {"type": "object", "properties": {
                            "n": {"type": "number", "x-mcp-header": "N"}
                        }}}
                    ],
                    "nextCursor": "2"
                })
            };
            sse(&[
                json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progress": 1}}),
                rpc_result(id, page),
            ])
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            if received.header("mcp-name") != Some(name) {
                return rpc_error(StatusCode::BAD_REQUEST, id, -32020, "Mcp-Name mismatch");
            }
            let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
            let text = match name {
                "execute_sql" => {
                    let region = arguments.get("region").and_then(Value::as_str);
                    if received.header("mcp-param-region") != region {
                        return rpc_error(StatusCode::BAD_REQUEST, id, -32020, "Mcp-Param-Region mismatch");
                    }
                    format!("ran in {}", region.unwrap_or_default())
                }
                "echo" => arguments["text"].as_str().unwrap_or_default().to_string(),
                _ => return rpc_error(StatusCode::OK, id, -32602, "Unknown tool"),
            };
            axum::Json(rpc_result(
                id,
                json!({"resultType": "complete", "content": [{"type": "text", "text": text}]}),
            ))
            .into_response()
        }
        _ => rpc_error(StatusCode::NOT_FOUND, id, -32601, "Method not found"),
    }
}

fn legacy(received: &Received, method: &str, id: &Value, params: &Value) -> Response {
    match method {
        "server/discover" => rpc_error(
            StatusCode::BAD_REQUEST,
            &Value::Null,
            -32000,
            "Bad Request: Server not initialized",
        ),
        "initialize" => (
            [("mcp-session-id", "session-1")],
            axum::Json(rpc_result(
                id,
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "fake-legacy", "version": "1"},
                    "instructions": "Old but gold."
                }),
            )),
        )
            .into_response(),
        _ if received.header("mcp-session-id") != Some("session-1")
            || received.header("mcp-protocol-version") != Some("2025-06-18") =>
        {
            (StatusCode::BAD_REQUEST, "missing session or version").into_response()
        }
        "tools/list" => sse(&[
            json!({"jsonrpc": "2.0", "id": "server-1", "method": "roots/list"}),
            rpc_result(
                id,
                json!({"tools": [{"name": "echo", "inputSchema": {"type": "object"}}]}),
            ),
        ]),
        "tools/call" => {
            let text = params
                .pointer("/arguments/text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            sse(&[rpc_result(
                id,
                json!({"content": [{"type": "text", "text": text}], "isError": false}),
            )])
        }
        _ => rpc_error(StatusCode::OK, id, -32601, "Method not found"),
    }
}

async fn end_session(State(fake): State<Arc<FakeServer>>, headers: HeaderMap) -> StatusCode {
    fake.received.lock().unwrap().push(Received {
        http_method: "DELETE".into(),
        headers: lowercase(&headers),
        body: Value::Null,
    });
    StatusCode::OK
}

async fn protected_resource(State(fake): State<Arc<FakeServer>>) -> axum::Json<Value> {
    axum::Json(json!({
        "resource": fake.url(),
        "authorization_servers": [fake.base()],
        "scopes_supported": ["files:read"],
    }))
}

fn auth_metadata(issuer: &str, base: &str) -> Value {
    json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "registration_endpoint": format!("{base}/register"),
        "scopes_supported": ["files:read", "offline_access"],
        "code_challenge_methods_supported": ["S256"],
    })
}

async fn authorization_server(State(fake): State<Arc<FakeServer>>) -> axum::Json<Value> {
    axum::Json(auth_metadata(fake.base(), fake.base()))
}

async fn tenant_configuration(State(fake): State<Arc<FakeServer>>) -> axum::Json<Value> {
    let mut metadata = auth_metadata(&format!("{}/tenant", fake.base()), fake.base());
    metadata["userinfo_endpoint"] = json!(format!("{}/tenant/me", fake.base()));
    axum::Json(metadata)
}

async fn userinfo(State(fake): State<Arc<FakeServer>>, headers: HeaderMap) -> Response {
    let sent = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    match &fake.token {
        Some(token) if sent == Some(&format!("Bearer {token}")) => {
            axum::Json(json!({ "sub": "u-1", "email": "me@example.com" })).into_response()
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

async fn impostor(State(fake): State<Arc<FakeServer>>) -> axum::Json<Value> {
    axum::Json(auth_metadata("https://honest.example", fake.base()))
}

async fn register(
    State(fake): State<Arc<FakeServer>>,
    axum::Json(body): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    fake.registrations.lock().unwrap().push(body.clone());
    (
        StatusCode::CREATED,
        axum::Json(json!({
            "client_id": "client-1",
            "client_name": body["client_name"],
            "redirect_uris": body["redirect_uris"],
            "token_endpoint_auth_method": "none",
        })),
    )
}

async fn token_endpoint(
    State(fake): State<Arc<FakeServer>>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Response {
    fake.token_requests.lock().unwrap().push(form.clone());
    let invalid = || {
        (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": "invalid_grant", "error_description": "no"})),
        )
            .into_response()
    };
    let field = |name: &str| form.get(name).map(String::as_str);
    match field("grant_type") {
        Some("authorization_code") => {
            let verified = field("code_verifier").is_some_and(|verifier| {
                let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
                fake.challenge.lock().unwrap().as_deref() == Some(challenge.as_str())
            });
            if field("code") != Some("good-code") || !verified {
                return invalid();
            }
            axum::Json(json!({
                "access_token": "access-1",
                "token_type": "Bearer",
                "refresh_token": "refresh-1",
                "expires_in": 3600,
                "scope": "files:read offline_access",
            }))
            .into_response()
        }
        Some("refresh_token") if field("refresh_token") == Some("refresh-1") => axum::Json(
            json!({"access_token": "access-2", "token_type": "Bearer", "expires_in": 60}),
        )
        .into_response(),
        _ => invalid(),
    }
}
