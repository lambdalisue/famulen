use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use hmac::{KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;

use super::*;
use crate::connections::{Connections, Secret, Transport};
use crate::store::Store;

const HOOK: &str = r#"
format = 1
id = "hook"
name = "Hook"
setup = "README.md"

[account]
channel_token = { label = "Token", optional = true }

[app]
id = "hook"
signing_secret = { env = "HOOK_SECRET", secret = true }

[transport]
http = { base = "https://hook.example" }

[receive.events.http]
verify = { hmac_sha256 = { secret = "{app.signing_secret}", base = "v0:{header.X-Ts}:{body}", signature = "{header.X-Sig}", prefix = "v0=", timestamp = "{header.X-Ts}", max_age = "5m" } }
challenge = "{/challenge}"
"#;

fn gateway(defs: &[(&str, &str)]) -> (Gateway, Arc<Mutex<Vec<Arrival>>>) {
    let store = Store::open_in_memory().unwrap();
    let g = Gateway::new(
        store.clone(),
        Connections::new(store),
        reqwest::Client::new(),
        None,
    )
    .with_definitions(
        None,
        &defs
            .iter()
            .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
            .collect::<Vec<_>>(),
    )
    .with_env(|name| (name == "HOOK_SECRET").then(|| "s3cret".to_owned()));
    let got: Arc<Mutex<Vec<Arrival>>> = Arc::default();
    let sink = got.clone();
    g.register_destination("lookout", Arc::new(move |a| sink.lock().unwrap().push(a)));
    (g, got)
}

fn add(g: &Gateway, id: &str, def: &str, values: &[(&str, &str)]) {
    g.connections
        .add(
            id,
            id,
            def,
            Transport::Defined {
                definition: def.into(),
                values: values
                    .iter()
                    .map(|(k, v)| ((*k).into(), (*v).into()))
                    .collect(),
                public: false,
            },
            Secret::default(),
            "",
        )
        .unwrap();
}

fn signed(body: &str, at: i64, secret: &str) -> BTreeMap<String, String> {
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("v0:{at}:{body}").as_bytes());
    let sig: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    BTreeMap::from([
        ("x-ts".to_owned(), at.to_string()),
        ("x-sig".to_owned(), format!("v0={sig}")),
    ])
}

#[tokio::test]
async fn a_signed_request_is_handed_over_untouched_and_a_forged_or_stale_one_is_refused() {
    let (g, got) = gateway(&[("hook.toml", HOOK)]);
    add(&g, "hook", "hook", &[]);
    g.set_destination(super::super::OWNER, "hook", "events", Some("lookout"))
        .unwrap();
    let body = r#"{"event":{"text":"hello"}}"#;

    let ok = g
        .receive_http("hook", "events", signed(body, now(), "s3cret"), body)
        .await
        .unwrap();
    let forged = g
        .receive_http("hook", "events", signed(body, now(), "wrong"), body)
        .await
        .unwrap();
    let stale = g
        .receive_http("hook", "events", signed(body, now() - 3600, "s3cret"), body)
        .await
        .unwrap();

    assert_eq!((ok.0, forged.0, stale.0), (200, 401, 401));
    let got = got.lock().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(
        (got[0].connection.as_str(), got[0].receiver.as_str()),
        ("hook", "events")
    );
    assert_eq!(
        got[0].payload,
        serde_json::json!({ "event": { "text": "hello" } })
    );
    let receipts = g.receipts(10).unwrap();
    assert_eq!(receipts.len(), 3);
    assert_eq!(receipts.iter().filter(|r| r.verified).count(), 1);
    assert!(receipts.iter().all(|r| r.receiver == "events"));
}

#[tokio::test]
async fn a_challenge_is_answered_and_not_handed_over() {
    let (g, got) = gateway(&[("hook.toml", HOOK)]);
    add(&g, "hook", "hook", &[]);
    g.set_destination(super::super::OWNER, "hook", "events", Some("lookout"))
        .unwrap();
    let body = r#"{"type":"url_verification","challenge":"abc"}"#;

    let answer = g
        .receive_http("hook", "events", signed(body, now(), "s3cret"), body)
        .await
        .unwrap();

    assert_eq!(answer, (200, "abc".to_owned()));
    assert!(got.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_receiver_without_a_destination_or_a_name_its_definition_lacks_is_not_open() {
    let (g, _) = gateway(&[("hook.toml", HOOK)]);
    add(&g, "hook", "hook", &[]);
    let body = "{}";

    let unsent = g
        .receive_http("hook", "events", signed(body, now(), "s3cret"), body)
        .await
        .unwrap();
    let refused = g.set_destination(super::super::OWNER, "hook", "other", Some("lookout"));
    let unnamed = g
        .receive_http("hook", "other", signed(body, now(), "s3cret"), body)
        .await
        .unwrap();

    assert_eq!((unsent.0, unnamed.0), (404, 404));
    assert!(
        refused.is_err(),
        "a receiver the definition lacks is not set"
    );
    assert_eq!(
        g.receivers_of("hook").unwrap(),
        [("events".to_owned(), None)]
    );
}

#[tokio::test]
async fn a_receiver_suggests_the_destination_its_definition_names_without_opening() {
    let def = HOOK.replace(
        "[receive.events.http]",
        "[receive.events]\ndestination = \"channel\"\n\n[receive.events.http]",
    );
    let (g, _) = gateway(&[("hook.toml", &def)]);
    add(&g, "hook", "hook", &[]);

    assert_eq!(
        g.suggested_destination("hook", "events")
            .unwrap()
            .as_deref(),
        Some("channel")
    );
    assert_eq!(g.suggested_destination("hook", "other").unwrap(), None);
    assert_eq!(g.suggested_destination("gone", "events").unwrap(), None);
    assert_eq!(
        g.receivers_of("hook").unwrap(),
        [("events".to_owned(), None)],
        "a suggestion is not a choice: the receiver stays closed"
    );
}

#[tokio::test]
async fn each_receiver_of_one_connection_goes_to_its_own_destination() {
    let def = format!(
        "{HOOK}\n[receive.other.http]\nverify = {{ token = {{ value = \"{{header.X-Token}}\", equals = \"{{account.channel_token}}\" }} }}\n"
    );
    let (g, got) = gateway(&[("hook.toml", &def)]);
    let elsewhere: Arc<Mutex<Vec<Arrival>>> = Arc::default();
    let sink = elsewhere.clone();
    g.register_destination("calendar", Arc::new(move |a| sink.lock().unwrap().push(a)));
    add(&g, "hook", "hook", &[("channel_token", "t-1")]);
    g.set_destination(super::super::OWNER, "hook", "events", Some("lookout"))
        .unwrap();
    g.set_destination(super::super::OWNER, "hook", "other", Some("calendar"))
        .unwrap();
    let body = "{}";

    g.receive_http("hook", "events", signed(body, now(), "s3cret"), body)
        .await
        .unwrap();
    let token = BTreeMap::from([("X-Token".to_owned(), "t-1".to_owned())]);
    g.receive_http("hook", "other", token, body).await.unwrap();

    assert_eq!(got.lock().unwrap().len(), 1);
    assert_eq!(elsewhere.lock().unwrap().len(), 1);
    assert_eq!(elsewhere.lock().unwrap()[0].receiver, "other");
    assert_eq!(g.destination_names(), ["calendar", "lookout"]);
}

#[tokio::test]
async fn a_token_check_compares_the_header_with_the_accounts_value() {
    let def = HOOK.replace(
        r#"verify = { hmac_sha256 = { secret = "{app.signing_secret}", base = "v0:{header.X-Ts}:{body}", signature = "{header.X-Sig}", prefix = "v0=", timestamp = "{header.X-Ts}", max_age = "5m" } }"#,
        r#"verify = { token = { value = "{header.X-Goog-Channel-Token}", equals = "{account.channel_token}" } }"#,
    );
    let (g, got) = gateway(&[("hook.toml", &def)]);
    add(&g, "hook", "hook", &[("channel_token", "t-1")]);
    g.set_destination(super::super::OWNER, "hook", "events", Some("lookout"))
        .unwrap();
    let header = |v: &str| BTreeMap::from([("X-Goog-Channel-Token".to_owned(), v.to_owned())]);

    assert_eq!(
        g.receive_http("hook", "events", header("t-1"), "")
            .await
            .unwrap()
            .0,
        200
    );
    assert_eq!(
        g.receive_http("hook", "events", header("t-2"), "")
            .await
            .unwrap()
            .0,
        401
    );
    assert_eq!(got.lock().unwrap().len(), 1);
    assert!(
        got.lock().unwrap()[0].headers.values().all(|v| v != "t-1"),
        "the shared token checked against is not handed on"
    );
}

#[tokio::test]
async fn a_jwt_check_uses_the_issuers_keys_issuer_and_audience() {
    let def = HOOK.replace(
        r#"verify = { hmac_sha256 = { secret = "{app.signing_secret}", base = "v0:{header.X-Ts}:{body}", signature = "{header.X-Sig}", prefix = "v0=", timestamp = "{header.X-Ts}", max_age = "5m" } }"#,
        r#"verify = { jwt = { value = "{header.Authorization}", issuer = "https://issuer.example", audience = "https://me.example/receive/hook/events" } }"#,
    );
    let (g, got) = gateway(&[("hook.toml", &def)]);
    add(&g, "hook", "hook", &[]);
    g.set_destination(super::super::OWNER, "hook", "events", Some("lookout"))
        .unwrap();
    let keys: jsonwebtoken::jwk::JwkSet = serde_json::from_value(serde_json::json!({
        "keys": [{ "kty": "oct", "kid": "k1", "alg": "HS256", "k": "c2VjcmV0LWtleS1mb3ItdGVzdHMtb25seS0zMmJ5dGVz" }]
    }))
    .unwrap();
    g.set_issuer_keys("https://issuer.example", keys);
    let token = |aud: &str| {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        header.kid = Some("k1".into());
        let claims =
            serde_json::json!({ "iss": "https://issuer.example", "aud": aud, "exp": now() + 600 });
        let key =
            jsonwebtoken::EncodingKey::from_secret(b"secret-key-for-tests-only-32bytes").unwrap();
        let jwt = jsonwebtoken::encode(&header, &claims, &key).unwrap();
        BTreeMap::from([("authorization".to_owned(), format!("Bearer {jwt}"))])
    };

    let ok = g
        .receive_http(
            "hook",
            "events",
            token("https://me.example/receive/hook/events"),
            "{}",
        )
        .await
        .unwrap();
    let other = g
        .receive_http("hook", "events", token("https://else.example"), "{}")
        .await
        .unwrap();

    assert_eq!((ok.0, other.0), (200, 401));
    assert_eq!(got.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_timestamp_beyond_all_bounds_is_refused() {
    let (g, _) = gateway(&[("hook.toml", HOOK)]);
    add(&g, "hook", "hook", &[]);
    g.set_destination(super::super::OWNER, "hook", "events", Some("lookout"))
        .unwrap();

    for at in [i64::MIN, i64::MAX] {
        let answer = g
            .receive_http("hook", "events", signed("{}", at, "s3cret"), "{}")
            .await
            .unwrap();
        assert_eq!(answer.0, 401, "{at}");
    }
}

#[tokio::test]
async fn an_issuer_that_changed_its_keys_is_asked_for_them_again() {
    let key = serde_json::json!({ "keys": [{ "kty": "oct", "kid": "new", "alg": "HS256", "k": "c2VjcmV0LWtleS1mb3ItdGVzdHMtb25seS0zMmJ5dGVz" }] });
    let asked = Arc::new(Mutex::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let place = serde_json::json!({ "jwks_uri": format!("{issuer}/keys") });
    let (count, published) = (asked.clone(), key.clone());
    let app = axum::Router::new()
        .route(
            "/.well-known/openid-configuration",
            axum::routing::get(move || async move { axum::Json(place) }),
        )
        .route(
            "/keys",
            axum::routing::get(move || async move {
                *count.lock().unwrap() += 1;
                axum::Json(published)
            }),
        );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (g, _) = gateway(&[("hook.toml", HOOK)]);
    g.set_issuer_keys(
        &issuer,
        serde_json::from_value(serde_json::json!({ "keys": [] })).unwrap(),
    );

    let keys = g.issuer_keys(&issuer, "new").await.unwrap();
    g.issuer_keys(&issuer, "unknown").await.unwrap();

    assert!(keys.find("new").is_some());
    assert_eq!(*asked.lock().unwrap(), 1, "not asked again for every token");
}

const MAIL: &str = r#"
format = 1
id = "mail"
name = "Mail"

[account]
subscription = { label = "Subscription" }

[login]
key = { label = "Key", header = "Authorization", value = "Bearer {token}" }

[transport]
http = { base = "https://mail.example" }

[receive.mail.connect]
protocol = "pubsub-pull"
subscription = "{account.subscription}"
"#;

async fn pubsub(acked: Arc<Mutex<Vec<Value>>>) -> String {
    use axum::routing::post;
    let app = axum::Router::new().route(
        "/{*rest}",
        post(move |uri: axum::http::Uri, body: String| {
            let seen = acked.clone();
            async move {
                if uri.path().ends_with(":acknowledge") {
                    seen.lock()
                        .unwrap()
                        .push(serde_json::from_str(&body).unwrap());
                    return "{}".to_owned();
                }
                if !seen.lock().unwrap().is_empty() {
                    return "{}".to_owned();
                }
                serde_json::json!({ "receivedMessages": [
                    { "ackId": "a1", "message": { "data": "eyJoaXN0b3J5SWQiOjF9" } },
                    { "ackId": "a2", "message": { "data": "eyJoaXN0b3J5SWQiOjJ9" } }
                ] })
                .to_string()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

#[tokio::test]
async fn a_pubsub_pull_hands_over_each_message_and_acknowledges_them() {
    let acked: Arc<Mutex<Vec<Value>>> = Arc::default();
    let base = pubsub(acked.clone()).await;
    let (g, got) = gateway(&[("mail.toml", MAIL)]);
    let g = g.with_endpoints(Endpoints {
        pubsub: base,
        ..Default::default()
    });
    add(
        &g,
        "mail",
        "mail",
        &[("subscription", "projects/p/subscriptions/s")],
    );
    g.set_api_key("mail", "k-1", "").unwrap();
    g.set_destination(super::super::OWNER, "mail", "mail", Some("lookout"))
        .unwrap();

    assert_eq!(g.pull_once("mail", "mail").await.unwrap(), 2);

    assert_eq!(got.lock().unwrap().len(), 2);
    assert_eq!(got.lock().unwrap()[0].payload["ackId"], "a1");
    assert_eq!(
        acked.lock().unwrap()[0],
        serde_json::json!({ "ackIds": ["a1", "a2"] })
    );
}

#[tokio::test]
async fn connections_open_as_receivers_are_sent_somewhere_and_close_when_they_are_not() {
    let acked: Arc<Mutex<Vec<Value>>> = Arc::default();
    let base = pubsub(acked.clone()).await;
    let (g, got) = gateway(&[("mail.toml", MAIL)]);
    let g = g.with_endpoints(Endpoints {
        pubsub: base,
        ..Default::default()
    });
    add(
        &g,
        "mail",
        "mail",
        &[("subscription", "projects/p/subscriptions/s")],
    );
    g.set_api_key("mail", "k-1", "").unwrap();
    g.sync_connections().unwrap();
    assert_eq!(g.open_connections(), 0, "nothing is sent anywhere yet");

    g.set_destination(super::super::OWNER, "mail", "mail", Some("lookout"))
        .unwrap();
    g.sync_connections().unwrap();
    g.sync_connections().unwrap();
    assert_eq!(g.open_connections(), 1, "opened once, however often synced");
    for _ in 0..100 {
        if got.lock().unwrap().len() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(got.lock().unwrap().len(), 2);

    g.set_destination(super::super::OWNER, "mail", "mail", None)
        .unwrap();
    g.sync_connections().unwrap();
    assert_eq!(g.open_connections(), 0);
}

#[tokio::test]
async fn a_receiver_sent_nowhere_is_closed_and_stays_a_choice() {
    let (g, _) = gateway(&[("hook.toml", HOOK)]);
    add(&g, "hook", "hook", &[]);
    assert!(!g.destination_chosen("hook", "events").unwrap());
    let body = "{}";

    g.set_destination(super::super::OWNER, "hook", "events", None)
        .unwrap();
    let closed = g
        .receive_http("hook", "events", signed(body, now(), "s3cret"), body)
        .await
        .unwrap();

    assert_eq!(closed.0, 404);
    assert_eq!(g.destination_of("hook", "events").unwrap(), None);
    assert!(
        g.destination_chosen("hook", "events").unwrap(),
        "nowhere is the owner's choice, not a receiver never set"
    );
    let log = g.log_entries(1).unwrap();
    assert_eq!(log[0].reason, "events → nowhere");
}

const CHAT: &str = r#"
format = 1
id = "chat"
name = "Chat"

[account]
team = { label = "Team" }

[transport]
http = { base = "https://chat.example" }

[receive.events.connect]
protocol = "slack-socket-mode"
token = "xapp-1"
account = { team = "{/payload/team_id}" }
"#;

#[tokio::test]
async fn events_on_one_socket_go_to_the_connection_whose_account_they_name() {
    let (g, _) = gateway(&[("chat.toml", CHAT)]);
    add(&g, "chat-a", "chat", &[("team", "TA")]);
    add(&g, "chat-b", "chat", &[("team", "TB")]);
    let members = vec![
        ("chat-a".to_owned(), "events".to_owned()),
        ("chat-b".to_owned(), "events".to_owned()),
    ];

    let to_b = g.route_event(
        &members,
        &serde_json::json!({ "payload": { "team_id": "TB" } }),
    );
    let nobody = g.route_event(
        &members,
        &serde_json::json!({ "payload": { "team_id": "TC" } }),
    );

    assert_eq!(to_b, Some(("chat-b".to_owned(), "events".to_owned())));
    assert_eq!(nobody, None);
}

#[tokio::test]
async fn a_socket_event_is_acknowledged_and_handed_over_untouched() {
    use tokio_tungstenite::tungstenite::Message;
    let ws = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_url = format!("ws://{}/", ws.local_addr().unwrap());
    let acks: Arc<Mutex<Vec<Value>>> = Arc::default();
    let seen = acks.clone();
    tokio::spawn(async move {
        let (stream, _) = ws.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let event = serde_json::json!({
            "envelope_id": "e-1",
            "type": "events_api",
            "payload": { "team_id": "TA", "event": { "type": "message", "text": "hi" } }
        });
        socket
            .send(Message::Text(event.to_string().into()))
            .await
            .unwrap();
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            seen.lock()
                .unwrap()
                .push(serde_json::from_str(text.as_str()).unwrap());
        }
    });
    let open = axum::Router::new().route(
        "/open",
        axum::routing::post(move || {
            let url = ws_url.clone();
            async move { serde_json::json!({ "ok": true, "url": url }).to_string() }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let open_url = format!("http://{}/open", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, open).await.unwrap() });
    let (g, got) = gateway(&[("chat.toml", CHAT)]);
    let g = g.with_endpoints(Endpoints {
        socket_open: open_url,
        ..Default::default()
    });
    add(&g, "chat-a", "chat", &[("team", "TA")]);
    g.set_destination(super::super::OWNER, "chat-a", "events", Some("lookout"))
        .unwrap();

    g.sync_connections().unwrap();
    for _ in 0..100 {
        if !got.lock().unwrap().is_empty() && !acks.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let got = got.lock().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].payload["payload"]["event"]["text"], "hi");
    assert_eq!(
        acks.lock().unwrap()[0],
        serde_json::json!({ "envelope_id": "e-1" })
    );
    g.close_connections();
}

/// A chat gateway whose sockets open at `ws`, with everything quiet for
/// long counted in milliseconds.
async fn chat_on(ws: &tokio::net::TcpListener) -> (Gateway, Arc<Mutex<Vec<Arrival>>>) {
    let ws_url = format!("ws://{}/", ws.local_addr().unwrap());
    let open = axum::Router::new().route(
        "/open",
        axum::routing::post(move || {
            let url = ws_url.clone();
            async move { serde_json::json!({ "ok": true, "url": url }).to_string() }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let open_url = format!("http://{}/open", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, open).await.unwrap() });
    let (g, got) = gateway(&[("chat.toml", CHAT)]);
    let g = g.with_endpoints(Endpoints {
        socket_open: open_url,
        socket_quiet: std::time::Duration::from_millis(200),
        socket_retry: std::time::Duration::from_millis(10),
        ..Default::default()
    });
    add(&g, "chat-a", "chat", &[("team", "TA")]);
    g.set_destination(super::super::OWNER, "chat-a", "events", Some("lookout"))
        .unwrap();
    (g, got)
}

fn chat_event(text: &str) -> tokio_tungstenite::tungstenite::Message {
    serde_json::json!({
        "envelope_id": "e-1",
        "type": "events_api",
        "payload": { "team_id": "TA", "event": { "type": "message", "text": text } }
    })
    .to_string()
    .into()
}

#[tokio::test]
async fn a_socket_that_went_dead_without_closing_is_opened_again() {
    let ws = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (g, got) = chat_on(&ws).await;
    tokio::spawn(async move {
        // Never read again, so it neither answers a ping nor closes.
        let (stream, _) = ws.accept().await.unwrap();
        let _dead = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (stream, _) = ws.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        socket.send(chat_event("hi")).await.unwrap();
        while let Some(Ok(_)) = socket.next().await {}
    });

    g.sync_connections().unwrap();
    for _ in 0..250 {
        if !got.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let got = got.lock().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].payload["payload"]["event"]["text"], "hi");
    g.close_connections();
}

#[tokio::test]
async fn a_socket_that_never_finishes_opening_is_opened_again() {
    let ws = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (g, got) = chat_on(&ws).await;
    tokio::spawn(async move {
        let _stuck = ws.accept().await.unwrap();
        let (stream, _) = ws.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        socket.send(chat_event("hi")).await.unwrap();
        while let Some(Ok(_)) = socket.next().await {}
    });

    g.sync_connections().unwrap();
    for _ in 0..250 {
        if !got.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    assert_eq!(got.lock().unwrap().len(), 1);
    g.close_connections();
}

#[tokio::test]
async fn a_quiet_socket_that_answers_stays_open() {
    let ws = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (g, got) = chat_on(&ws).await;
    let opened: Arc<Mutex<usize>> = Arc::default();
    let count = opened.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = ws.accept().await.unwrap();
            *count.lock().unwrap() += 1;
            tokio::spawn(async move {
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                // Reading answers the pings and says nothing else.
                while let Some(Ok(_)) = socket.next().await {}
            });
        }
    });

    g.sync_connections().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    assert_eq!(*opened.lock().unwrap(), 1);
    assert!(got.lock().unwrap().is_empty());
    g.close_connections();
}

#[tokio::test]
async fn each_line_of_a_command_kept_running_is_handed_over_and_it_goes_on_from_the_last() {
    let say = r#"
format = 1
id = "say"
name = "Say"

[transport.command]
program = "perl"

[[commands]]
name = "hi"
description = "Hi."
args = ["-e", "1"]
read_only = true

[receive.changes]
destination = "here"

[receive.changes.connect]
protocol = "command-lines"
args = ["-e", "$|=1; $s = $ARGV[1] // 0; for (1..3) { $n = $s + $_; print qq({\"seq\":$n,\"state\":\"), ($n % 2 ? 'done' : 'working'), qq(\"}\n) } print qq(not json\n)", "--"]
resume = ["--since", "{/seq}"]
when = { "{/state}" = ["done", "waiting"] }
"#;
    let (g, seen) = gateway(&[("say.toml", say)]);
    add(&g, "say", "say", &[]);
    let here = seen.clone();
    g.register_destination("here", Arc::new(move |a| here.lock().unwrap().push(a)));
    g.set_destination("owner", "say", "changes", Some("here"))
        .unwrap();

    g.lines_once("say", "changes").await.unwrap();
    g.lines_once("say", "changes").await.unwrap();

    let seqs: Vec<i64> = seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|a| a.payload["seq"].as_i64())
        .collect();
    assert_eq!(seqs, [1, 3, 5]);
}

#[tokio::test]
async fn a_line_is_handed_over_once_nothing_more_came_about_the_same() {
    let say = r#"
format = 1
id = "say"
name = "Say"

[transport.command]
program = "perl"

[[commands]]
name = "hi"
description = "Hi."
args = ["-e", "1"]
read_only = true

[receive.asks]
destination = "here"

[receive.asks.connect]
protocol = "command-lines"
args = ["-e", "$|=1; sub say_ {{ print qq({{\"seq\":$_[0],\"id\":\"$_[1]\",\"state\":\"$_[2]\"}}\n) }} say_(1,'a','waiting'); say_(2,'b','waiting'); say_(3,'a','working'); sleep 2"]
resume = ["--since", "{/seq}"]
when = { "{/state}" = ["waiting"] }
per = "{/id}"
settle = "1s"
"#;
    let say = say.replace("{{", "{").replace("}}", "}");
    let (g, _) = gateway(&[("say.toml", &say)]);
    add(&g, "say", "say", &[]);
    let seen: Arc<Mutex<Vec<Arrival>>> = Arc::default();
    let here = seen.clone();
    g.register_destination("here", Arc::new(move |a| here.lock().unwrap().push(a)));
    g.set_destination(super::super::OWNER, "say", "asks", Some("here"))
        .unwrap();

    g.lines_once("say", "asks").await.unwrap();

    let ids: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|a| format!("{}{}", a.about, if a.over { " over" } else { "" }))
        .collect();
    assert_eq!(ids, ["a over", "b"]);
    assert_eq!(
        g.store.state("receive.resume.say.asks").unwrap().as_deref(),
        Some(r#"["--since","3"]"#)
    );
}

#[tokio::test]
async fn nothing_waits_to_settle_while_the_owner_is_away() {
    let say = r#"
format = 1
id = "say"
name = "Say"

[transport.command]
program = "perl"

[[commands]]
name = "hi"
description = "Hi."
args = ["-e", "1"]
read_only = true

[receive.asks]
destination = "here"

[receive.asks.connect]
protocol = "command-lines"
args = ["-e", "$|=1; sub say_ {{ print qq({{\"seq\":$_[0],\"id\":\"$_[1]\",\"state\":\"$_[2]\"}}\n) }} say_(1,'a','waiting'); sleep 1"]
resume = ["--since", "{/seq}"]
when = { "{/state}" = ["waiting"] }
per = "{/id}"
settle = "1h"
away = "60s"
front = "app.say"
"#;
    let say = say.replace("{{", "{").replace("}}", "}");
    for (idle, front, told) in [
        (Some(5), Some("app.say"), 0),
        (Some(5), Some("app.other"), 1),
        (Some(5), None, 1),
        (Some(90), Some("app.say"), 1),
        (None, None, 1),
    ] {
        let (g, _) = gateway(&[("say.toml", &say)]);
        let g = g.with_presence(move || crate::presence::Presence {
            idle: idle.map(std::time::Duration::from_secs),
            front: front.map(str::to_owned),
        });
        add(&g, "say", "say", &[]);
        let seen: Arc<Mutex<Vec<Arrival>>> = Arc::default();
        let here = seen.clone();
        g.register_destination("here", Arc::new(move |a| here.lock().unwrap().push(a)));
        g.set_destination(super::super::OWNER, "say", "asks", Some("here"))
            .unwrap();

        g.lines_once("say", "asks").await.unwrap();

        assert_eq!(seen.lock().unwrap().len(), told, "{idle:?} {front:?}");
    }
}
