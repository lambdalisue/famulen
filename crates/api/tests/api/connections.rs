//! Connections: the permissions of their tools, and calls of an HTTP API.

use famulen_api::{Api, tools::ApiTools};
use famulen_core::approvals::Approvals;
use famulen_core::hub::OwnTools;
use famulen_core::permissions::{Bound, Caller, Decision};
use famulen_core::slack::web::GatewaySlack;
use serde_json::{Value, json};

use crate::support::{call, open, temp};

/// An HTTP API connection: its tools are known without reaching it.
async fn add_api(api: &Api, kind: &str) -> String {
    let added = call(
        api,
        "ConnectionService/AddConnection",
        json!({ "target": "http://127.0.0.1:9/", "how": "api" }),
    )
    .await;
    let id = added["connection"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{added}"))
        .to_owned();
    call(
        api,
        "ConnectionService/EditConnection",
        json!({ "id": id, "kind": kind }),
    )
    .await;
    id
}

fn tool<'a>(detail: &'a Value, name: &str) -> &'a Value {
    detail["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("no tool {name} in {detail}"))
}

#[tokio::test]
async fn a_kind_permission_applies_to_every_account_of_the_kind_unless_the_account_overrides_it() {
    let dir = temp();
    let api = open(&dir);
    let work = add_api(&api, "e2e-shared").await;
    let home = add_api(&api, "e2e-shared").await;
    call(&api, "ConnectionService/SetPermission",
        json!({ "scope": "kind:e2e-shared", "tool": "http_post", "permission": "PERMISSION_ALLOW" }),
    )
    .await;
    call(&api, "ConnectionService/SetPermission",
        json!({ "scope": format!("conn:{home}"), "tool": "http_post", "permission": "PERMISSION_FORBID" }),
    )
    .await;

    let kind = call(
        &api,
        "ConnectionService/GetKind",
        json!({ "id": "e2e-shared" }),
    )
    .await;
    let post = tool(&kind, "http_post");
    assert_eq!(post["permission"], "PERMISSION_ALLOW");
    assert_eq!(post["inherited"], "PERMISSION_AUTO");
    assert_eq!(post["effective"], "PERMISSION_ALLOW");
    assert_eq!(post["source"], "種類の既定");
    let get = tool(&kind, "http_get");
    assert_eq!(
        get["permission"],
        Value::Null,
        "the kind sets nothing for it"
    );
    assert_eq!(get["effective"], "PERMISSION_AUTO");
    assert_eq!(get["readOnly"], true);
    assert_eq!(get["source"], "まだ決めていない");
    let mut sharing: Vec<&str> = kind["connections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    sharing.sort();
    let mut expected = vec![work.as_str(), home.as_str()];
    expected.sort();
    assert_eq!(sharing, expected);

    let at_work = call(
        &api,
        "ConnectionService/GetConnection",
        json!({ "id": work }),
    )
    .await;
    let post = tool(&at_work, "http_post");
    assert_eq!(post["permission"], Value::Null);
    assert_eq!(post["effective"], "PERMISSION_ALLOW");
    assert_eq!(post["source"], "種類の既定");
    let at_home = call(
        &api,
        "ConnectionService/GetConnection",
        json!({ "id": home }),
    )
    .await;
    let post = tool(&at_home, "http_post");
    assert_eq!(post["permission"], "PERMISSION_FORBID");
    assert_eq!(post["inherited"], "PERMISSION_ALLOW");
    assert_eq!(post["effective"], "PERMISSION_FORBID");
    assert_eq!(post["source"], "このアカウント");
}

#[tokio::test]
async fn a_kind_no_account_has_lists_nothing() {
    let dir = temp();
    let api = open(&dir);
    let kind = call(&api, "ConnectionService/GetKind", json!({ "id": "nobody" })).await;
    assert_eq!(kind["kind"], "nobody");
    assert_eq!(kind["tools"], Value::Null);
    assert_eq!(kind["connections"], Value::Null);
}

/// An HTTP API that answers the key each request came with, as a connection.
async fn add_echo(api: &Api) -> String {
    let router = axum::Router::new().fallback(|headers: http::HeaderMap| async move {
        headers
            .get("x-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("no key")
            .to_owned()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let added = call(
        api,
        "ConnectionService/AddConnection",
        json!({ "target": target, "how": "api", "apiKey": "k-12345678", "header": "X-Key" }),
    )
    .await;
    added["connection"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn the_secretary_calls_an_api_with_the_key_the_vault_keeps_and_the_gateway_logs_it() {
    let dir = temp();
    let api = open(&dir);
    let id = add_echo(&api).await;
    let tools = ApiTools::new(api.clone()).unwrap();

    let answer = tools
        .call(
            "call_api",
            json!({ "connection": id, "method": "GET", "path": "/me" }),
        )
        .await
        .unwrap();

    assert!(answer.contains("k-12345678"), "{answer}");
    let log = api.app().gateway.log_entries(1).unwrap();
    assert_eq!(
        (log[0].actor.as_str(), log[0].tool.as_str()),
        ("secretary", "http_get")
    );
}

#[tokio::test]
async fn an_api_call_is_made_only_as_its_tools_permission_lets_it() {
    let dir = temp();
    let api = open(&dir);
    let id = add_echo(&api).await;
    let tools = ApiTools::new(api.clone()).unwrap();
    let post = async || {
        tools
            .call(
                "call_api",
                json!({ "connection": id, "method": "POST", "path": "/me" }),
            )
            .await
            .unwrap_err()
            .to_string()
    };
    for (permission, refusal) in [
        ("PERMISSION_FORBID", "forbid"),
        ("PERMISSION_CONFIRM", "Held for the owner's approval"),
    ] {
        call(
            &api,
            "ConnectionService/SetPermission",
            json!({ "scope": format!("conn:{id}"), "tool": "http_post", "permission": permission }),
        )
        .await;
        let refused = post().await;
        assert!(refused.contains(refusal), "{permission}: {refused}");
    }
}

#[tokio::test]
async fn a_call_allowing_would_not_stop_is_shown_as_asked_once_with_why() {
    let dir = temp();
    let api = open(&dir);
    let lookout = Bound {
        ask: Some("this turn is about what others wrote".to_owned()),
        waive: false,
    };
    let decision = api
        .app()
        .permissions
        .decide_bound(
            Caller::Secretary,
            "Bash",
            &json!({ "command": "ls" }),
            lookout,
        )
        .await
        .unwrap();
    assert!(matches!(decision, Decision::Held(_)));

    let app = api.app();
    Approvals::<GatewaySlack>::new(
        None,
        app.store.clone(),
        app.permissions.clone(),
        Default::default(),
    )
    .present_held()
    .await
    .unwrap();
    let inbox = call(&api, "ConsoleService/GetInbox", json!({})).await;

    let held = &inbox["approvals"][0];
    assert_eq!(held["once"], true, "{inbox}");
    assert!(
        held["stillAsked"]
            .as_str()
            .is_some_and(|why| why.contains("許可にしていても聞きます")),
        "{inbox}"
    );
}
