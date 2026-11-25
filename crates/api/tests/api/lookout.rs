//! The lookout through the API: what it offers to lookout with one press.

use famulen_api::Api;
use serde_json::json;

use crate::support::{self, DataDir, call, define, temp};

const HOOK: &str = r#"
format = 1
id = "hook"
name = "Hook"

[account]
token = { label = "合言葉" }

[transport]
http = { base = "https://hook.example" }

[receive.events.http]
verify = { token = { value = "{header.X-Token}", equals = "{account.token}" } }
"#;

const PLAIN: &str = r#"
format = 1
id = "plain"
name = "Plain"

[transport]
http = { base = "https://plain.example" }
"#;

fn open(dir: &DataDir) -> Api {
    define(dir, "hook", HOOK);
    define(dir, "plain", PLAIN);
    support::open(dir)
}

#[tokio::test]
async fn a_connection_whose_definition_receives_is_offered_as_a_lookout_of_what_arrives() {
    let dir = temp();
    let api = open(&dir);
    let hook = call(
        &api,
        "ConnectionService/AddConnection",
        json!({"target": "hook", "how": "definition", "env": "token=t-1"}),
    )
    .await;
    let hook = hook["connection"]["id"].as_str().unwrap().to_owned();
    let plain = call(
        &api,
        "ConnectionService/AddConnection",
        json!({"target": "plain", "how": "definition"}),
    )
    .await;
    let plain = plain["connection"]["id"].as_str().unwrap().to_owned();

    let lookout = call(&api, "LookoutService/GetLookout", json!({})).await;
    let presets = lookout["presets"].as_array().cloned().unwrap_or_default();

    let offered: Vec<&str> = presets
        .iter()
        .filter_map(|p| p["connection"].as_str())
        .collect();
    assert!(offered.contains(&hook.as_str()), "{presets:?}");
    assert!(!offered.contains(&plain.as_str()), "{presets:?}");
    let preset = presets.iter().find(|p| p["connection"] == hook).unwrap();
    assert_eq!(preset["tool"], famulen_core::lookout::ARRIVED);
}

#[tokio::test]
async fn a_lookout_calls_only_a_tool_that_says_it_only_reads() {
    let dir = temp();
    let api = open(&dir);
    let added = call(
        &api,
        "ConnectionService/AddConnection",
        json!({"target": "http://127.0.0.1:9/", "how": "api"}),
    )
    .await;
    let id = added["connection"]["id"].as_str().unwrap().to_owned();
    let add = |tool: &'static str| {
        let body =
            json!({"name": tool, "connection": id, "tool": tool, "args": "{\"path\":\"/\"}"});
        call(&api, "LookoutService/AddLookout", body)
    };

    for tool in ["http_post", "http_delete", "no_such_tool"] {
        let refused = add(tool).await;
        assert!(refused.get("code").is_some(), "{tool}: {refused}");
    }
    assert_eq!(add("http_get").await["tool"], "http_get");
    let lookout = call(&api, "LookoutService/GetLookout", json!({})).await;
    assert_eq!(lookout["sources"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn what_a_lookout_calls_is_changed_to_another_tool_that_only_reads() {
    let dir = temp();
    let api = open(&dir);
    let added = call(
        &api,
        "ConnectionService/AddConnection",
        json!({"target": "http://127.0.0.1:9/", "how": "api"}),
    )
    .await;
    let id = added["connection"]["id"].as_str().unwrap().to_owned();
    let source = call(
        &api,
        "LookoutService/AddLookout",
        json!({"name": "x", "connection": id, "tool": "http_get", "args": "{\"path\":\"/a\"}"}),
    )
    .await;
    let set = |tool: &'static str, args: &'static str| {
        let body = json!({"id": source["id"], "tool": tool, "args": args});
        call(&api, "LookoutService/SetLookoutReading", body)
    };

    assert_eq!(
        set("http_get", "{\"path\":\"/b\"}").await["args"],
        "{\"path\":\"/b\"}"
    );
    for (tool, args) in [("http_post", "{}"), ("arrived", "{}"), ("http_get", "{no")] {
        let refused = set(tool, args).await;
        assert!(refused.get("code").is_some(), "{tool}: {refused}");
    }
    let lookout = call(&api, "LookoutService/GetLookout", json!({})).await;
    assert_eq!(lookout["sources"][0]["args"], "{\"path\":\"/b\"}");
}

#[tokio::test]
async fn a_lookouts_interval_is_changed_and_kept() {
    let dir = temp();
    let api = open(&dir);
    let added = call(
        &api,
        "LookoutService/AddLookout",
        json!({"name": "届いたもの", "connection": "hook-1", "intervalMinutes": 15}),
    )
    .await;

    let changed = call(
        &api,
        "LookoutService/SetLookoutInterval",
        json!({"id": added["id"], "intervalMinutes": 45}),
    )
    .await;
    let lookout = call(&api, "LookoutService/GetLookout", json!({})).await;

    assert_eq!(changed["intervalMinutes"], 45);
    assert_eq!(lookout["sources"][0]["intervalMinutes"], 45);
}

#[tokio::test]
async fn a_lookout_added_without_an_interval_starts_at_the_default_for_what_it_reads() {
    let dir = temp();
    let api = open(&dir);
    let added = call(
        &api,
        "LookoutService/AddLookout",
        json!({"name": "届いたもの", "connection": "hook-1"}),
    )
    .await;
    assert_eq!(added["intervalMinutes"], 15);
}

#[tokio::test]
async fn a_lookout_follows_the_default_until_given_its_own_interval_and_back() {
    let dir = temp();
    let api = open(&dir);
    let added = call(
        &api,
        "LookoutService/AddLookout",
        json!({"name": "届いたもの", "connection": "hook-1"}),
    )
    .await;
    assert_eq!(added.get("ownInterval"), None);
    assert_eq!(added["feed"], "other");

    let changed = call(
        &api,
        "LookoutService/SetLookoutDefault",
        json!({"feed": "other", "intervalMinutes": 20}),
    )
    .await;
    assert_eq!(changed["intervalMinutes"], 20);
    let lookout = call(&api, "LookoutService/GetLookout", json!({})).await;
    assert_eq!(lookout["sources"][0]["intervalMinutes"], 20);
    let defaults: Vec<_> = lookout["defaults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["feed"].clone(), d["intervalMinutes"].clone()))
        .collect();
    assert_eq!(
        defaults,
        [
            (json!("mail"), json!(30)),
            (json!("chat"), json!(5)),
            (json!("other"), json!(20))
        ]
    );

    let own = call(
        &api,
        "LookoutService/SetLookoutInterval",
        json!({"id": added["id"], "intervalMinutes": 45}),
    )
    .await;
    assert_eq!(own["ownInterval"], true);
    let back = call(
        &api,
        "LookoutService/SetLookoutInterval",
        json!({"id": added["id"], "intervalMinutes": 0}),
    )
    .await;
    assert_eq!(back.get("ownInterval"), None);
    assert_eq!(back["intervalMinutes"], 20);
}

#[tokio::test]
async fn a_mailbox_is_gone_back_over_from_a_day_on_and_stopped() {
    let dir = temp();
    let api = open(&dir);
    let google = api
        .app()
        .gateway
        .add_defined("google", Default::default(), None)
        .unwrap()
        .id;

    let before = call(&api, "LookoutService/ListPastPurchases", json!({})).await;
    assert_eq!(before["mailboxes"][0]["connection"], google.as_str());
    assert!(before["mailboxes"][0]["state"].is_null(), "{before}");

    let started = call(
        &api,
        "LookoutService/StartPastPurchases",
        json!({"connection": google, "since": "2024-03-01"}),
    )
    .await;
    assert_eq!(started["state"], "PAST_MAIL_STATE_GOING");
    assert_eq!(started["month"], "2024-03-01");

    let stopped = call(
        &api,
        "LookoutService/StopPastPurchases",
        json!({"id": google}),
    )
    .await;
    assert_eq!(stopped["state"], "PAST_MAIL_STATE_STOPPED");
}

#[tokio::test]
async fn a_mailbox_is_gone_back_over_for_contracts_apart_from_what_was_bought() {
    let dir = temp();
    let api = open(&dir);
    let google = api
        .app()
        .gateway
        .add_defined("google", Default::default(), None)
        .unwrap()
        .id;

    let started = call(
        &api,
        "LookoutService/StartPastContracts",
        json!({"connection": google, "since": "2020-01-01"}),
    )
    .await;
    assert_eq!(started["state"], "PAST_MAIL_STATE_GOING");

    let bought = call(&api, "LookoutService/ListPastPurchases", json!({})).await;
    assert!(bought["mailboxes"][0]["state"].is_null(), "{bought}");
    let contracts = call(&api, "LookoutService/ListPastContracts", json!({})).await;
    assert_eq!(contracts["mailboxes"][0]["month"], "2020-01-01");

    let stopped = call(
        &api,
        "LookoutService/StopPastContracts",
        json!({"id": google}),
    )
    .await;
    assert_eq!(stopped["state"], "PAST_MAIL_STATE_STOPPED");
}
