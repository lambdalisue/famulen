//! Service definitions, receivers' destinations and account values through
//! the API.

use std::sync::Arc;

use famulen_api::Api;
use serde_json::{Value, json};

use crate::support::{self, DataDir, call, define, temp};

const HOOK: &str = r#"
format = 1
id = "hook"
name = "Hook"

[account]
region = { label = "地域", optional = true }
token = { label = "合言葉", secret = true }
made = { generate = true }

[transport]
http = { base = "https://hook.example" }

[[tools]]
name = "ping"
description = "Ping."
method = "GET"
path = "/ping"

[receive.events.http]
verify = { token = { value = "{header.X-Token}", equals = "{account.token}" } }
"#;

fn open(dir: &DataDir) -> Api {
    define(dir, "hook", HOOK);
    define(dir, "broken", "format = 1\n");
    let api = support::open(dir);
    for name in ["lookout", "calendar"] {
        api.app()
            .gateway
            .register_destination(name, Arc::new(|_| {}));
    }
    api
}

async fn add_hook(api: &Api) -> String {
    let hook = call(
        api,
        "ConnectionService/AddConnection",
        json!({"target": "hook", "how": "definition", "env": "token=t-1"}),
    )
    .await;
    hook["connection"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{hook}"))
        .to_owned()
}

#[tokio::test]
async fn the_definitions_are_listed_with_their_receivers_tools_fields_and_what_did_not_read() {
    let dir = temp();
    let api = open(&dir);

    let listed = call(&api, "ConnectionService/ListDefinitions", json!({})).await;

    let hook = listed["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "hook")
        .cloned()
        .unwrap_or_else(|| panic!("{listed}"));
    assert_eq!(hook["receivers"], json!(["events"]));
    assert_eq!(hook["tools"], json!(["ping"]));
    assert_eq!(
        hook["fields"][1],
        json!({"name": "token", "label": "合言葉", "secret": true})
    );
    let google = listed["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "google")
        .cloned()
        .unwrap();
    assert_eq!(
        google.get("fields"),
        None,
        "a Google account is added by its login alone"
    );
    assert_ne!(
        google["appReady"],
        json!(true),
        "no Google app in this test"
    );
    assert!(
        google["appMissing"]
            .as_str()
            .unwrap()
            .contains("FAMULEN_GOOGLE_CLIENT_ID")
    );
    assert!(listed["broken"][0].as_str().unwrap().contains("broken"));
}

#[tokio::test]
async fn a_receivers_destination_is_shown_and_chosen_again() {
    let dir = temp();
    let api = open(&dir);
    let id = add_hook(&api).await;

    let detail = call(&api, "ConnectionService/GetConnection", json!({"id": id})).await;
    assert_eq!(
        detail["receivers"],
        json!([{"name": "events", "how": "HTTP", "destination": "lookout"}])
    );
    assert_eq!(detail["destinations"], json!(["calendar", "lookout"]));
    assert_eq!(
        detail.get("login"),
        None,
        "a definition without a login has nothing to log in to: {detail}"
    );

    let chosen = call(
        &api,
        "ConnectionService/SetDestination",
        json!({"connection": id, "receiver": "events", "destination": "calendar"}),
    )
    .await;
    assert_eq!(chosen["receivers"][0]["destination"], "calendar");
    let unknown = call(
        &api,
        "ConnectionService/SetDestination",
        json!({"connection": id, "receiver": "events", "destination": "elsewhere"}),
    )
    .await;
    assert!(
        unknown["message"].as_str().unwrap().contains("elsewhere"),
        "{unknown}"
    );
    let closed = call(
        &api,
        "ConnectionService/SetDestination",
        json!({"connection": id, "receiver": "events", "destination": ""}),
    )
    .await;
    assert_eq!(
        closed["receivers"][0].get("destination"),
        None,
        "sent nowhere: {closed}"
    );
    let again = call(&api, "ConnectionService/GetConnection", json!({"id": id})).await;
    assert_eq!(
        again["receivers"][0].get("destination"),
        None,
        "a closed receiver stays closed: {again}"
    );

    let log = call(
        &api,
        "GatewayLogService/ListGatewayLog",
        json!({"connection": id}),
    )
    .await;
    assert!(
        log["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["tool"] == "destination.set" && e["reason"] == "events → calendar"),
        "{log}"
    );
}

#[tokio::test]
async fn an_account_value_is_set_again_and_a_secret_one_never_comes_back() {
    let dir = temp();
    let api = open(&dir);
    let id = add_hook(&api).await;

    call(
        &api,
        "ConnectionService/SetAccountValue",
        json!({"connection": id, "name": "region", "value": "jp"}),
    )
    .await;
    let after = call(
        &api,
        "ConnectionService/SetAccountValue",
        json!({"connection": id, "name": "token", "value": "t-2"}),
    )
    .await;

    assert_eq!(
        after["values"],
        json!([
            {"name": "region", "label": "地域", "value": "jp", "set": true, "optional": true},
            {"name": "token", "label": "合言葉", "secret": true, "set": true},
        ])
    );
    let again = call(&api, "ConnectionService/GetConnection", json!({"id": id})).await;
    assert!(
        !after.to_string().contains("t-2") && !again.to_string().contains("t-2"),
        "a secret value is never answered"
    );
}

#[tokio::test]
async fn a_service_is_added_with_its_name_and_its_key_or_environment_kept_in_the_vault() {
    let dir = temp();
    let api = open(&dir);
    let add = async |request: Value| {
        call(&api, "ConnectionService/AddConnection", request).await["connection"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    let local = add(json!({
        "target": "famulen-no-such-server",
        "name": "手元のサーバー",
        "env": "TOKEN=t-secret-1"
    }))
    .await;
    let remote = add(json!({
        "target": "http://127.0.0.1:9/mcp",
        "apiKey": "k-secret-2",
        "header": "X-Api-Key"
    }))
    .await;

    let shown = call(
        &api,
        "ConnectionService/GetConnection",
        json!({ "id": local }),
    )
    .await;
    assert_eq!(shown["connection"]["name"], "手元のサーバー");
    let listed = call(&api, "VaultService/ListSecrets", json!({}))
        .await
        .to_string();
    assert!(
        listed.contains(&format!("connection:{local}:account.TOKEN")),
        "{listed}"
    );
    assert!(
        listed.contains(&format!("connection:{remote}:api_key")),
        "{listed}"
    );
    let db = std::fs::read(dir.join("gateway/gateway.sqlite3")).unwrap();
    for secret in ["t-secret-1", "k-secret-2"] {
        assert!(!db.windows(secret.len()).any(|w| w == secret.as_bytes()));
    }
}
