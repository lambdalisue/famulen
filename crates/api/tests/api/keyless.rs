//! Connections the secretary adds with its tool.

use famulen_api::{Api, tools::ApiTools};
use famulen_core::hub::{OwnTools, SERVER_NAME};
use famulen_core::permissions::Decision;
use serde_json::{Value, json};

use crate::support::{call, open, temp};

const ADD: &str = "add_keyless_connection";

fn jvn() -> Value {
    json!({ "definition": "rss", "values": { "url": "https://jvn.jp/rss/jvn.rdf" }, "name": "JVN" })
}

async fn listed(api: &Api) -> Vec<Value> {
    let list = call(api, "ConnectionService/ListConnections", json!({})).await;
    list["connections"].as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn the_owner_is_asked_and_sees_what_would_be_added() {
    let dir = temp();
    let api = open(&dir);
    let tools = ApiTools::new(api.clone()).unwrap();
    let app = api.app();
    app.permissions.record_own_tools(&tools.list()).unwrap();

    let name = format!("mcp__{SERVER_NAME}__{ADD}");
    let decided = app.permissions.decide(&name, &jvn()).await.unwrap();

    assert!(matches!(decided, Decision::Held(_)), "{decided:?}");
    let ticket = app.gateway.held_tickets().unwrap().pop().unwrap();
    assert_eq!((ticket.tool.as_str(), &ticket.shown), (ADD, &jvn()));
    assert!(listed(&api).await.is_empty());

    let hopeless = json!({ "definition": "bluesky", "values": { "handle": "me.example" } });
    let decided = app.permissions.decide(&name, &hopeless).await.unwrap();
    assert!(matches!(&decided, Decision::Forbid(why) if why.contains("only the owner")));
    assert_eq!(app.gateway.held_tickets().unwrap().len(), 1);
}

#[tokio::test]
async fn a_feed_the_secretary_added_is_listed_and_looked_out_on_at_once() {
    let dir = temp();
    let api = open(&dir);
    let tools = ApiTools::new(api.clone()).unwrap();

    let added: Value = serde_json::from_str(&tools.call(ADD, jvn()).await.unwrap()).unwrap();

    let id = added["id"].as_str().unwrap();
    let all = listed(&api).await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0]["id"], id);
    assert_eq!(all[0]["name"], "JVN");
    assert_eq!(all[0]["account"], "https://jvn.jp/rss/jvn.rdf");
    assert_eq!(all[0]["ready"], true);
    let lookout: Value = serde_json::from_str(
        &tools
            .call(
                "add_lookout",
                json!({ "name": "JVN", "connection": id, "tool": "get_feed" }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (&lookout["connection"], &lookout["tool"]),
        (&json!(id), &json!("get_feed"))
    );
}

#[tokio::test]
async fn what_needs_a_secret_or_is_there_already_is_not_added() {
    let dir = temp();
    let api = open(&dir);
    let tools = ApiTools::new(api.clone()).unwrap();
    tools.call(ADD, jvn()).await.unwrap();

    for (input, why) in [
        (jvn(), "already connected"),
        (
            json!({ "definition": "bluesky", "values": { "handle": "me.example" } }),
            "only the owner",
        ),
        (
            json!({ "definition": "rss", "values": { "url": "https://127.0.0.1/feed" } }),
            "not a public address",
        ),
    ] {
        let refused = tools.call(ADD, input).await.unwrap_err().to_string();
        assert!(refused.contains(why), "{refused}");
    }
    assert_eq!(listed(&api).await.len(), 1);
}
