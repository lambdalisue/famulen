use std::sync::{Arc, Mutex};

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};

use serde_json::json;

use super::super::logins::query_param;
use super::*;
use crate::connections::Connections;
use crate::permissions::Permission;
use crate::store::Store;

/// Every request the fake service saw: method and path, the
/// Authorization header, the query and the body.
type Seen = Arc<Mutex<Vec<(String, String, BTreeMap<String, String>, String)>>>;

async fn service() -> (String, Seen) {
    let seen: Seen = Arc::default();
    async fn record(
        State(seen): State<Seen>,
        method: axum::http::Method,
        uri: axum::http::Uri,
        headers: HeaderMap,
        Query(query): Query<BTreeMap<String, String>>,
        body: String,
    ) -> String {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        seen.lock()
            .unwrap()
            .push((format!("{method} {}", uri.path()), auth, query, body));
        r#"{"ok":false,"error":"channel_not_found"}"#.to_owned()
    }
    async fn token() -> String {
        r#"{"access_token":"xoxb-bot-token","refresh_token":"r-1","expires_in":3600,"authed_user":{"id":"U1","access_token":"xoxp-user-token"},"team":{"id":"T1","name":"Acme"}}"#.to_owned()
    }
    let app = axum::Router::new()
        .route("/oauth/token", post(token))
        .route("/api/{*rest}", get(record).post(record))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, seen)
}

fn chat(base: &str) -> String {
    format!(
        r#"
format = 1
id = "chat"
name = "Chat"
setup = "README.md"

[account]
display = "{{account.team_name}}"
region = {{ label = "Region", optional = true }}
phrase = {{ label = "Phrase", optional = true, secret = true }}

[app]
id = "chat"
client_id = {{ env = "CHAT_CLIENT_ID" }}
client_secret = {{ env = "CHAT_CLIENT_SECRET", secret = true }}

[login.oauth2]
authorize = "{base}/oauth/authorize"
token = "{base}/oauth/token"
scopes = ["chat:write"]
query = {{ user_scope = "search:read" }}
tokens = {{ bot = "{{/access_token}}", user = "{{/authed_user/access_token}}" }}
values = {{ team = "{{/team/id}}", team_name = "{{/team/name}}", user = "{{/authed_user/id}}" }}
same = ["team"]
refresh = true

[transport]
http = {{ base = "{base}/api", auth = "bot" }}

[[tools]]
name = "post_message"
description = "Post a message."
method = "POST"
path = "/chat.postMessage"
body = {{ channel = "{{args.channel}}", text = "{{args.text}}", thread_ts = "{{args.thread}}" }}
ok = "{{/ok}}"

[[tools]]
name = "search"
description = "Search as the owner."
method = "GET"
path = "/search"
query = {{ q = "{{args.q}}" }}
auth = "user"

[[tools]]
name = "knock"
description = "Knock with the phrase."
method = "GET"
path = "/knock"
query = {{ phrase = "{{account.phrase}}", region = "{{account.region}}" }}

[permissions]
post_message = "confirm"
search = "allow"
"#
    )
}

fn gateway(base: &str, env: bool) -> Gateway {
    let store = Store::open_in_memory().unwrap();
    Gateway::new(
        store.clone(),
        Connections::new(store),
        reqwest::Client::new(),
        Some(super::super::vault::tests::scratch()),
    )
    .with_definitions(None, &[("chat.toml".into(), chat(base))])
    .with_env(move |name| match (env, name) {
        (true, "CHAT_CLIENT_ID") => Some("cid".into()),
        (true, "CHAT_CLIENT_SECRET") => Some("csecret".into()),
        _ => None,
    })
}

async fn logged_in(g: &Gateway, base: &str) -> String {
    let c = g.add_defined("chat", BTreeMap::new(), None).unwrap();
    let url = g.begin_defined_login(&c.id, "https://console/cb").unwrap();
    assert!(url.starts_with(&format!("{base}/oauth/authorize?")));
    for part in [
        "client_id=cid",
        "code_challenge=",
        "code_challenge_method=S256",
        "scope=chat%3Awrite",
        "user_scope=search%3Aread",
    ] {
        assert!(url.contains(part), "{url} lacks {part}");
    }
    let state = query_param(url.split_once('?').unwrap().1, "state").unwrap();
    assert!(
        g.is_defined_login(&format!("state={state}&code=c1"))
            .unwrap()
    );
    g.finish_defined_login(&format!("code=c1&state={state}"))
        .await
        .unwrap()
}

#[tokio::test]
async fn an_oauth2_login_keeps_each_named_token_in_the_vault_and_the_values_on_the_connection() {
    let (base, _) = service().await;
    let g = gateway(&base, true);

    let id = logged_in(&g, &base).await;

    let row = g.connections.get(&id).unwrap().unwrap();
    assert!(row.secret.tokens.is_empty(), "no token stays in the row");
    assert!(row.secret.in_vault.contains("token.bot"));
    assert!(row.secret.in_vault.contains("token.user"));
    assert_eq!(row.account, "Acme");
    assert_eq!(
        row.secret.in_vault.iter().collect::<Vec<_>>(),
        ["refresh_token", "token.bot", "token.user"],
        "only tokens are sealed, not the login's values"
    );
    let Transport::Defined { values, .. } = &row.transport else {
        panic!("made from a definition")
    };
    assert_eq!(
        (values["team"].as_str(), values["user"].as_str()),
        ("T1", "U1"),
        "the login's values sit on the connection as they are"
    );
    let opened = g.sealed().get(&id).unwrap().unwrap();
    assert_eq!(opened.secret.tokens["bot"], "xoxb-bot-token");
    assert_eq!(row.status, "つながっている");
    let values = g.account_values(&id).unwrap();
    assert_eq!(
        (values["team"].as_str(), values["user"].as_str()),
        ("T1", "U1")
    );
    assert!(
        !values.values().any(|v| v.starts_with("xox")),
        "the account values carry no token"
    );
}

#[tokio::test]
async fn a_secret_account_field_is_sealed_and_filled_only_into_what_goes_out() {
    let (base, seen) = service().await;
    let g = gateway(&base, true);

    let c = g
        .add_defined(
            "chat",
            BTreeMap::from([
                ("region".into(), "eu".into()),
                ("phrase".into(), "open-sesame".into()),
            ]),
            None,
        )
        .unwrap();

    let Transport::Defined { values, .. } = &c.transport else {
        panic!("made from a definition")
    };
    assert_eq!(values.keys().collect::<Vec<_>>(), ["region"]);
    assert!(c.secret.in_vault.contains("account.phrase"));
    assert_eq!(
        g.account_values(&c.id).unwrap().keys().collect::<Vec<_>>(),
        ["region"],
        "a secret field is not an account value anyone reads"
    );
    g.call_as_owner(&c, "knock", json!({})).await.unwrap();
    let (_, _, query, _) = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(query["phrase"], "open-sesame");
    assert_eq!(
        g.account_names(&c.id)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        ["phrase", "region"],
        "a sealed field still counts as set"
    );
}

#[tokio::test]
async fn a_generated_account_value_is_made_once_kept_in_the_vault_and_not_set_by_hand() {
    let (base, seen) = service().await;
    let text = chat(&base).replace(
        "phrase = { label = \"Phrase\", optional = true, secret = true }",
        "phrase = { generate = true }",
    );
    let g = gateway(&base, true).with_definitions(None, &[("chat.toml".into(), text)]);
    let c = g.add_defined("chat", BTreeMap::new(), None).unwrap();
    let given = BTreeMap::from([("phrase".to_owned(), "kaze".to_owned())]);
    let other = g.add_defined("chat", given, None).unwrap();

    let mut phrases = Vec::new();
    for c in [&c, &other, &c] {
        g.generate_account_values(&c.id).unwrap();
        g.call_as_owner(c, "knock", json!({})).await.unwrap();
        let (_, _, query, _) = seen.lock().unwrap().last().cloned().unwrap();
        phrases.push(query["phrase"].clone());
    }

    assert!(c.secret.in_vault.contains("account.phrase"));
    assert!(phrases[0].len() >= 32);
    assert_ne!(phrases[1], "kaze", "one given is not taken");
    assert_ne!(phrases[0], phrases[1], "each connection has its own");
    assert_eq!(phrases[0], phrases[2], "one that is made is kept");
    assert!(
        g.set_account_value(OWNER, &c.id, "phrase", "kaze")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_connection_added_before_its_definition_generates_a_value_gets_it() {
    let (base, seen) = service().await;
    let g = gateway(&base, true);
    let c = g.add_defined("chat", BTreeMap::new(), None).unwrap();
    let text = chat(&base).replace(
        "phrase = { label = \"Phrase\", optional = true, secret = true }",
        "phrase = { generate = true }",
    );
    let g = g.with_definitions(None, &[("chat.toml".into(), text)]);

    g.sync_connections().unwrap();

    g.call_as_owner(&c, "knock", json!({})).await.unwrap();
    let (_, _, query, _) = seen.lock().unwrap().last().cloned().unwrap();
    assert!(query["phrase"].len() >= 32);
}

#[tokio::test]
async fn an_account_value_can_be_set_again_after_adding() {
    let (base, seen) = service().await;
    let g = gateway(&base, true);
    let c = g.add_defined("chat", BTreeMap::new(), None).unwrap();

    g.set_account_value(OWNER, &c.id, "region", "jp")
        .await
        .unwrap();
    g.set_account_value(OWNER, &c.id, "phrase", "kaze")
        .await
        .unwrap();

    assert_eq!(g.account_values(&c.id).unwrap()["region"], "jp");
    let c = g.connections.get(&c.id).unwrap().unwrap();
    assert!(c.secret.in_vault.contains("account.phrase"));
    g.call_as_owner(&c, "knock", json!({})).await.unwrap();
    let (_, _, query, _) = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        (query["region"].as_str(), query["phrase"].as_str()),
        ("jp", "kaze")
    );
    let err = g
        .set_account_value(OWNER, &c.id, "colour", "red")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("colour"), "{err}");
    let logged = g
        .log()
        .page(&crate::log::LogQuery {
            limit: 50,
            ..Default::default()
        })
        .unwrap();
    let changes: Vec<_> = logged
        .iter()
        .filter(|(_, e)| e.tool == "account.set")
        .map(|(_, e)| e.reason.as_str())
        .collect();
    assert_eq!(
        changes,
        ["phrase", "region"],
        "logged by name, never the value"
    );
}

#[tokio::test]
async fn logging_in_the_same_account_again_replaces_its_tokens_and_drops_the_new_connection() {
    let (base, _) = service().await;
    let g = gateway(&base, true);

    let first = logged_in(&g, &base).await;
    let second = logged_in(&g, &base).await;

    assert_eq!(first, second);
    assert_eq!(g.connections.list().unwrap().len(), 1);
}

#[tokio::test]
async fn logging_in_the_same_account_again_keeps_its_generated_value() {
    let (base, seen) = service().await;
    let text = chat(&base).replace(
        "phrase = { label = \"Phrase\", optional = true, secret = true }",
        "phrase = { generate = true }",
    );
    let g = gateway(&base, true).with_definitions(None, &[("chat.toml".into(), text)]);

    let mut phrases = Vec::new();
    for _ in 0..2 {
        let id = logged_in(&g, &base).await;
        let c = g.connections.get(&id).unwrap().unwrap();
        g.call_as_owner(&c, "knock", json!({})).await.unwrap();
        let (_, _, query, _) = seen.lock().unwrap().last().cloned().unwrap();
        phrases.push(query["phrase"].clone());
    }

    assert_eq!(phrases[0], phrases[1]);
}

#[tokio::test]
async fn a_tool_fills_its_request_and_carries_the_token_it_names() {
    let (base, seen) = service().await;
    let g = gateway(&base, true);
    let id = logged_in(&g, &base).await;
    let c = g.connections.get(&id).unwrap().unwrap();

    let posted = g
        .call_as_owner(&c, "post_message", json!({ "channel": "C1", "text": "hi" }))
        .await
        .unwrap();
    g.call_as_owner(&c, "search", json!({ "q": "budget" }))
        .await
        .unwrap();

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen[0].0, "POST /api/chat.postMessage");
    assert_eq!(seen[0].1, "Bearer xoxb-bot-token");
    let body: Value = serde_json::from_str(&seen[0].3).unwrap();
    assert_eq!(body, json!({ "channel": "C1", "text": "hi" }));
    assert!(posted.is_error, "ok is not true");
    assert_eq!(seen[1].0, "GET /api/search");
    assert_eq!(seen[1].1, "Bearer xoxp-user-token");
    assert_eq!(seen[1].2["q"], "budget");
}

#[tokio::test]
async fn the_tools_are_listed_from_the_definition_with_its_defaults_below_the_owners() {
    let (base, _) = service().await;
    let g = gateway(&base, true);
    let id = logged_in(&g, &base).await;
    let c = g.connections.get(&id).unwrap().unwrap();

    let tools = g.list_tools(&c).await.unwrap();

    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(names, ["post_message", "search", "knock"]);
    assert_eq!(tools[1]["annotations"]["readOnlyHint"], true);
    let permissions = crate::permissions::Permissions::new(g.store.clone());
    assert_eq!(
        permissions.default_of(&id, "post_message").unwrap(),
        Permission::Confirm
    );
    assert_eq!(
        permissions.default_of(&id, "search").unwrap(),
        Permission::Allow
    );
}

#[tokio::test]
async fn a_login_without_the_apps_environment_shows_the_setup() {
    let (base, _) = service().await;
    let g = gateway(&base, false);
    let c = g.add_defined("chat", BTreeMap::new(), None).unwrap();

    let error = g
        .begin_defined_login(&c.id, "https://console/cb")
        .unwrap_err();

    let text = error.to_string();
    assert!(text.contains("CHAT_CLIENT_ID"), "{text}");
    assert!(text.contains("README.md"), "{text}");
}

#[tokio::test]
async fn a_connection_whose_definition_does_not_read_says_why() {
    let (base, _) = service().await;
    let g =
        gateway(&base, true).with_definitions(None, &[("chat.toml".into(), "format = 1".into())]);
    let c = g
        .connections
        .add(
            "chat",
            "Chat",
            "chat",
            Transport::Defined {
                definition: "chat".into(),
                values: BTreeMap::new(),
            },
            Secret::default(),
            "",
        )
        .unwrap();

    let error = g
        .call_as_owner(&c, "post_message", json!({}))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("missing"), "{error}");
    assert!(g.list_tools(&c).await.is_err());
    assert!(
        g.connections
            .get("chat")
            .unwrap()
            .unwrap()
            .status
            .starts_with("つながらない")
    );
}

#[tokio::test]
async fn an_openapi_description_gives_one_tool_per_operation_that_fills_its_request() {
    let (base, seen) = service().await;
    let doc = json!({
        "openapi": "3.1.0",
        "paths": {
            "/pets/{petId}": {
                "get": {
                    "operationId": "showPet",
                    "summary": "Show one pet.",
                    "parameters": [
                        { "name": "petId", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "fields", "in": "query", "schema": { "type": "string" } }
                    ]
                }
            },
            "/pets": {
                "post": {
                    "operationId": "createPet",
                    "requestBody": { "content": { "application/json": { "schema": { "type": "object" } } } }
                }
            }
        }
    });
    let served = doc.to_string();
    let docs = axum::Router::new().route(
        "/openapi.json",
        get(move || {
            let served = served.clone();
            async move { served }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let docs_base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, docs).await.unwrap() });
    let pets = format!(
        r#"
format = 1
id = "pets"
name = "Pets"

[login]
key = {{ label = "Key", header = "Authorization", value = "Bearer {{token}}" }}

[transport]
http = {{ base = "{base}/api", openapi = "{docs_base}/openapi.json" }}

[[tools]]
name = "create_pet"
description = "Written by hand."
method = "POST"
path = "/pets/new"

[permissions]
show_pet = "allow"
"#
    );
    let g = gateway(&base, true).with_definitions(None, &[("pets.toml".into(), pets)]);
    let c = g.add_defined("pets", BTreeMap::new(), None).unwrap();
    g.set_api_key(&c.id, "k-1", "").unwrap();

    let tools = g.list_tools(&c).await.unwrap();
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(names, ["show_pet", "create_pet"]);
    assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
    assert_eq!(tools[1]["description"], "Written by hand.", "[[tools]] win");

    let result = g
        .call_as_owner(&c, "show_pet", json!({ "petId": "p 1", "fields": "name" }))
        .await
        .unwrap();
    assert!(!result.is_error);
    let seen = seen.lock().unwrap();
    let (target, auth, query, _) = seen.last().unwrap();
    assert_eq!(target, "GET /api/pets/p%201");
    assert_eq!(auth, "Bearer k-1");
    assert_eq!(query["fields"], "name");
}

#[tokio::test]
async fn a_remote_mcp_definition_sends_its_headers_and_the_logins_token() {
    use crate::mcp::fake::{Era, FakeServer};
    let (base, _) = service().await;
    let fake = FakeServer::start(Era::Modern, Some("xoxb-bot-token")).await;
    let code = format!(
        r#"
format = 1
id = "code"
name = "Code"
setup = "README.md"

[app]
id = "chat"
client_id = {{ env = "CHAT_CLIENT_ID" }}
client_secret = {{ env = "CHAT_CLIENT_SECRET", secret = true }}

[login.oauth2]
authorize = "{base}/oauth/authorize"
token = "{base}/oauth/token"

[transport]
mcp_remote = {{ url = "{}", headers = {{ X-Toolsets = "repos,issues" }} }}
"#,
        fake.url()
    );
    let g = gateway(&base, true).with_definitions(None, &[("code.toml".into(), code)]);
    let c = g.add_defined("code", BTreeMap::new(), None).unwrap();
    let url = g.begin_defined_login(&c.id, "https://console/cb").unwrap();
    let state = query_param(url.split_once('?').unwrap().1, "state").unwrap();
    g.finish_defined_login(&format!("code=c1&state={state}"))
        .await
        .unwrap();

    assert!(g.read_tools(&c.id).await.unwrap() > 0);

    let listed = fake.received_method("tools/list").pop().unwrap();
    assert_eq!(listed.header("x-toolsets"), Some("repos,issues"));
    assert_eq!(
        listed.header("authorization"),
        Some("Bearer xoxb-bot-token")
    );
}

#[tokio::test]
async fn a_session_login_trades_the_accounts_values_for_a_token_and_asks_again_before_it_ends() {
    async fn open(
        State(opened): State<Arc<Mutex<u32>>>,
        axum::Json(body): axum::Json<Value>,
    ) -> (axum::http::StatusCode, axum::Json<Value>) {
        if body["password"] != "app-pass" {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                axum::Json(json!({ "error": "AuthenticationRequired" })),
            );
        }
        let mut opened = opened.lock().unwrap();
        *opened += 1;
        (
            axum::http::StatusCode::OK,
            axum::Json(json!({ "accessJwt": format!("jwt-{opened}"), "did": "did:1" })),
        )
    }
    let opened = Arc::new(Mutex::new(0));
    let login = axum::Router::new()
        .route("/session", post(open))
        .with_state(opened.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let door = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, login).await.unwrap() });
    let (base, seen) = service().await;
    let sky = format!(
        r#"
format = 1
id = "sky"
name = "Sky"

[account]
handle = {{ label = "Handle" }}
password = {{ label = "App password", secret = true }}

[login.session]
post = "{door}/session"
body = {{ identifier = "{{account.handle}}", password = "{{account.password}}" }}
tokens = {{ access = "{{/accessJwt}}" }}
values = {{ did = "{{/did}}" }}
lifetime = "1h"

[transport]
http = {{ base = "{base}/api" }}

[[tools]]
name = "timeline"
description = "Read the timeline."
method = "GET"
path = "/timeline"
query = {{ as = "{{account.did}}" }}
"#
    );
    let g = gateway(&base, true).with_definitions(None, &[("sky.toml".into(), sky)]);
    let add = |password: &str| {
        g.add_defined(
            "sky",
            BTreeMap::from([
                ("handle".into(), "me".into()),
                ("password".into(), password.into()),
            ]),
            None,
        )
        .unwrap()
        .id
    };

    let wrong = g.open_session(&add("nope")).await.unwrap_err().to_string();
    assert!(wrong.contains("AuthenticationRequired"), "{wrong}");

    let id = add("app-pass");
    g.open_session(&id).await.unwrap();
    let c = g.connections.get(&id).unwrap().unwrap();
    assert!(c.secret.tokens.is_empty(), "the token is in the vault");
    g.call_as_owner(&c, "timeline", json!({})).await.unwrap();
    {
        let seen = seen.lock().unwrap();
        let (_, auth, query, _) = seen.last().unwrap();
        assert_eq!(auth, "Bearer jwt-1");
        assert_eq!(query["as"], "did:1");
    }
    g.refresh_due().await.unwrap();
    assert_eq!(*opened.lock().unwrap(), 1, "it has not come near its end");

    let mut secret = g.sealed().get(&id).unwrap().unwrap().secret;
    secret.expires_at = Some(chrono::Utc::now().timestamp() + 60);
    g.sealed().set_secret(&id, &secret).unwrap();
    g.refresh_due().await.unwrap();
    assert_eq!(*opened.lock().unwrap(), 2);
}

#[tokio::test]
async fn a_login_that_could_not_be_refreshed_for_a_while_is_connected_once_it_is() {
    let (base, _) = service().await;
    let g = gateway(&base, true);
    let id = logged_in(&g, &base).await;
    let mut secret = g.sealed().require(&id).unwrap().secret;
    secret.expires_at = Some(0);
    g.sealed().set_secret(&id, &secret).unwrap();
    g.connections
        .set_status(&id, "ログインし直す（unreachable）")
        .unwrap();

    assert!(g.refresh_due().await.unwrap());

    assert_eq!(g.connections.require(&id).unwrap().status, "つながっている");
}

#[tokio::test]
async fn the_same_answer_twice_is_the_login_it_already_finished() {
    let (base, _) = service().await;
    let g = gateway(&base, true);
    let c = g.add_defined("chat", BTreeMap::new(), None).unwrap();
    let url = g.begin_defined_login(&c.id, "https://console/cb").unwrap();
    let state = query_param(url.split_once('?').unwrap().1, "state").unwrap();
    let answer = format!("code=c1&state={state}");

    let (first, second) = tokio::join!(g.finish_login(&answer), g.finish_login(&answer));

    assert_eq!(first.unwrap(), (c.id.clone(), true));
    assert_eq!(second.unwrap(), (c.id.clone(), false));
    assert!(
        g.finish_login(&format!("code=c2&state={state}"))
            .await
            .is_err(),
        "another answer to the same login finishes nothing"
    );
}

#[tokio::test]
async fn a_login_asks_for_json_and_says_who_is_asking() {
    async fn token(headers: axum::http::HeaderMap) -> String {
        let sent = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        if sent("accept") == Some("application/json")
            && sent("user-agent").is_some_and(|v| v.starts_with("famulen-gateway/"))
        {
            r#"{"access_token":"xoxb-bot-token","authed_user":{"id":"U1","access_token":"xoxp"},"team":{"id":"T1","name":"Acme"}}"#.into()
        } else {
            "access_token=xoxb-bot-token".into()
        }
    }
    let app = axum::Router::new().route("/oauth/token", post(token));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let g = gateway(&base, true);

    let id = logged_in(&g, &base).await;

    assert_eq!(g.connections.get(&id).unwrap().unwrap().account, "Acme");
}

#[tokio::test]
async fn a_plain_http_tool_sends_the_key_only_below_the_services_base() {
    let (base, seen) = service().await;
    let (elsewhere, seen_elsewhere) = service().await;
    let plain = format!(
        r#"
format = 1
id = "plain"
name = "Plain"

[login]
key = {{ label = "Key", header = "Authorization", value = "Bearer {{token}}" }}

[transport]
http = {{ base = "{base}/api" }}
"#
    );
    let g = gateway(&base, true).with_definitions(None, &[("plain.toml".into(), plain)]);
    let c = g.add_defined("plain", BTreeMap::new(), None).unwrap();
    g.set_api_key(&c.id, "k-1", "").unwrap();

    for path in [
        format!("{elsewhere}/api/steal"),
        format!(
            "HTTP://{}/api/steal",
            elsewhere.trim_start_matches("http://")
        ),
    ] {
        let error = g
            .call_as_owner(&c, "http_get", json!({ "path": path }))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("path"), "{error}");
    }
    let result = g
        .call_as_owner(&c, "http_get", json!({ "path": "/me" }))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(seen_elsewhere.lock().unwrap().is_empty());
    let seen = seen.lock().unwrap();
    let (target, auth, _, _) = seen.last().unwrap();
    assert_eq!(target, "GET /api/me");
    assert_eq!(auth, "Bearer k-1");
}

/// A service whose `admin` token is traded for with a ticket that is spent
/// in doing so; it answers each trade with the next ticket, or refuses.
async fn trading(refuse: bool) -> (String, Seen) {
    let seen: Seen = Arc::default();
    async fn record(
        State((seen, refuse)): State<(Seen, bool)>,
        uri: axum::http::Uri,
        headers: HeaderMap,
        body: String,
    ) -> String {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let mut seen = seen.lock().unwrap();
        seen.push((uri.path().to_owned(), auth, BTreeMap::new(), body));
        if uri.path() != "/trade" {
            return r#"{"ok":true}"#.to_owned();
        }
        let n = seen.iter().filter(|s| s.0 == "/trade").count();
        if refuse {
            r#"{"ok":false,"error":"ticket_spent"}"#.to_owned()
        } else {
            json!({ "ok": true, "token": format!("admin-{n}"), "ticket": format!("t-{n}") })
                .to_string()
        }
    }
    let app = axum::Router::new()
        .route("/{*rest}", post(record))
        .with_state((seen.clone(), refuse));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, seen)
}

fn trader(base: &str) -> Gateway {
    let text = chat(base)
        .replace(
            "[app]",
            "ticket = { label = \"Ticket\", optional = true, secret = true }\n\n[app]",
        )
        .replace(
            "[transport]",
            &format!(
                "[login.exchange.admin]\npost = \"{base}/trade\"\nform = {{ ticket = \"{{account.ticket}}\" }}\ntoken = \"{{/token}}\"\nrenews = {{ ticket = \"{{/ticket}}\" }}\n\n[transport]"
            ),
        )
        .replace(
            "[permissions]",
            "[[tools]]\nname = \"rule\"\ndescription = \"Rule.\"\nmethod = \"POST\"\npath = \"/rule\"\nauth = \"admin\"\nok = \"{/ok}\"\n\n[permissions]",
        );
    gateway(base, true).with_definitions(None, &[("chat.toml".into(), text)])
}

#[tokio::test]
async fn an_exchanged_token_goes_out_and_the_spent_value_is_replaced_in_the_vault() {
    let (base, seen) = trading(false).await;
    let g = trader(&base);
    let c = g.add_defined("chat", BTreeMap::new(), None).unwrap();
    g.set_account_value(OWNER, &c.id, "ticket", "t-0")
        .await
        .unwrap();

    let (one, two) = tokio::join!(
        g.call_as_owner(&c, "rule", json!({})),
        g.call_as_owner(&c, "rule", json!({}))
    );

    assert!(!one.unwrap().is_error && !two.unwrap().is_error);
    let seen = seen.lock().unwrap();
    let of = |path: &str| -> Vec<(String, String)> {
        seen.iter()
            .filter(|s| s.0 == path)
            .map(|s| (s.1.clone(), s.3.clone()))
            .collect()
    };
    assert_eq!(
        of("/trade"),
        [
            (String::new(), "ticket=t-0".to_owned()),
            (String::new(), "ticket=t-1".to_owned())
        ],
        "no ticket is handed over twice"
    );
    let mut ruled: Vec<String> = of("/api/rule").into_iter().map(|r| r.0).collect();
    ruled.sort();
    assert_eq!(ruled, ["Bearer admin-1", "Bearer admin-2"]);
    let row = g.connections.require(&c.id).unwrap();
    assert!(row.secret.account.is_empty() && row.secret.tokens.is_empty());
    let opened = g.sealed().require(&c.id).unwrap();
    assert_eq!(opened.secret.account["ticket"], "t-2");
    assert!(opened.secret.tokens.is_empty(), "the token is not kept");
}

#[tokio::test]
async fn a_refused_exchange_says_why_sends_nothing_on_and_keeps_the_value() {
    let (base, seen) = trading(true).await;
    let g = trader(&base);
    let c = g.add_defined("chat", BTreeMap::new(), None).unwrap();
    let unset = g.call_as_owner(&c, "rule", json!({})).await.unwrap_err();
    assert!(seen.lock().unwrap().is_empty(), "{unset}");
    g.set_account_value(OWNER, &c.id, "ticket", "t-secret")
        .await
        .unwrap();

    let error = g.call_as_owner(&c, "rule", json!({})).await.unwrap_err();

    let error = error.to_string();
    assert!(error.contains("ticket_spent"), "{error}");
    assert!(!error.contains("t-secret"), "{error}");
    assert_eq!(seen.lock().unwrap().len(), 1);
    let opened = g.sealed().require(&c.id).unwrap();
    assert_eq!(opened.secret.account["ticket"], "t-secret");
}

#[tokio::test]
async fn connecting_the_same_account_again_keeps_the_secret_values_set_on_it() {
    let (base, _) = service().await;
    let g = gateway(&base, true);
    let id = logged_in(&g, &base).await;
    g.set_account_value(OWNER, &id, "phrase", "kaze")
        .await
        .unwrap();

    assert_eq!(logged_in(&g, &base).await, id);

    let opened = g.sealed().require(&id).unwrap();
    assert_eq!(opened.secret.account["phrase"], "kaze");
}

#[test]
fn without_a_vault_a_secret_field_counts_as_set_too() {
    let store = Store::open_in_memory().unwrap();
    let g = Gateway::new(
        store.clone(),
        Connections::new(store),
        reqwest::Client::new(),
        None,
    )
    .with_definitions(None, &[("chat.toml".into(), chat("http://127.0.0.1:9"))]);

    let c = g
        .add_defined(
            "chat",
            BTreeMap::from([("phrase".into(), "kaze".into())]),
            None,
        )
        .unwrap();

    assert!(g.account_names(&c.id).unwrap().contains("phrase"));
}

#[tokio::test]
async fn what_is_set_on_a_connection_while_it_logs_in_again_is_kept() {
    let g = {
        let store = Store::open_in_memory().unwrap();
        Gateway::new(
            store.clone(),
            Connections::new(store),
            reqwest::Client::new(),
            Some(super::super::vault::tests::scratch()),
        )
    };
    let (asked, was_asked) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    let (go, done) = (asked.clone(), was_asked.clone());
    let app = axum::Router::new().route(
        "/oauth/token",
        post(move || async move {
            done.notify_one();
            go.notified().await;
            r#"{"access_token":"b","authed_user":{"id":"U1","access_token":"u"},"team":{"id":"T1","name":"Acme"}}"#
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let g = g
        .with_definitions(None, &[("chat.toml".into(), chat(&base))])
        .with_env(|name| name.starts_with("CHAT_").then(|| "v".to_owned()));
    let c = g
        .add_defined(
            "chat",
            BTreeMap::from([("phrase".into(), "old".into())]),
            None,
        )
        .unwrap();
    let url = g.begin_defined_login(&c.id, "https://console/cb").unwrap();
    let state = query_param(url.split_once('?').unwrap().1, "state").unwrap();

    let answer = format!("code=c1&state={state}");
    let (id, ()) = tokio::join!(g.finish_defined_login(&answer), async {
        was_asked.notified().await;
        g.set_account_value(OWNER, &c.id, "phrase", "new")
            .await
            .unwrap();
        asked.notify_one();
    });

    let opened = g.sealed().require(id.unwrap()).unwrap();
    assert_eq!(opened.secret.account["phrase"], "new");
}

#[tokio::test]
async fn a_trade_whose_caller_went_away_still_keeps_the_next_value() {
    let (base, seen) = trading(false).await;
    let g = trader(&base);
    let c = g.add_defined("chat", BTreeMap::new(), None).unwrap();
    g.set_account_value(OWNER, &c.id, "ticket", "t-0")
        .await
        .unwrap();

    {
        let call = g.call_as_owner(&c, "rule", json!({}));
        tokio::pin!(call);
        // Begun, then dropped before the service answers.
        assert!(futures_util::poll!(call.as_mut()).is_pending());
    }

    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while g.sealed().require(&c.id).unwrap().secret.account["ticket"] != "t-1" {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
}
