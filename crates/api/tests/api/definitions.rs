//! Service definitions changed through the API: the secretary proposes one
//! with its tools, the owner's answer lets it in, and the owner takes it back.

use famulen_api::{Api, tools::ApiTools};
use famulen_core::gateway::{Answer, CHANGE_DEFINITION, DEFINITIONS};
use famulen_core::hub::OwnTools;
use serde_json::json;

use crate::support::{self, DataDir, call, define, temp};

const NOTES: &str = r#"format = 1
id = "notes"
name = "Notes"

[transport]
http = { base = "http://127.0.0.1:9" }

[[tools]]
name = "list_notes"
description = "List notes."
method = "GET"
path = "/notes"
"#;

const REMOVE: &str = r#"
[[tools]]
name = "remove_note"
description = "Remove one note."
method = "DELETE"
path = "/notes/{/id}"
"#;

fn open(dir: &DataDir) -> Api {
    define(dir, "notes", NOTES);
    support::open(dir)
}

#[tokio::test]
async fn the_secretary_proposes_a_change_the_owner_lets_in_and_takes_back() {
    let dir = temp();
    let api = open(&dir);
    let tools = ApiTools::new(api.clone()).unwrap();
    let defs = tools.list();
    let def = |name: &str| defs.iter().find(|t| t.name == name).cloned();
    assert!(def("get_definition_text").is_some_and(|t| t.read_only));
    assert!(
        def("propose_definition_change").is_some_and(|t| !t.confirm),
        "the gateway asks, not the tool"
    );
    assert!(
        def("revert_definition").is_none(),
        "only the owner takes back"
    );

    let current = tools
        .call("get_definition_text", json!({ "id": "notes" }))
        .await
        .unwrap();
    assert!(current.contains("list_notes"), "{current}");
    let proposed = format!("{NOTES}{REMOVE}");
    let held = tools
        .call(
            "propose_definition_change",
            json!({ "definition": "notes", "toml": proposed, "why": "メモを消す道具がない" }),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(held.contains("Held for the owner's approval"), "{held}");

    let gateway = &api.app().gateway;
    let ticket = gateway.held_tickets().unwrap().pop().unwrap();
    assert_eq!(
        (ticket.connection.as_str(), ticket.tool.as_str()),
        (DEFINITIONS, CHANGE_DEFINITION)
    );
    let unchanged = call(
        &api,
        "ConnectionService/GetDefinitionText",
        json!({ "id": "notes" }),
    )
    .await;
    assert_eq!(unchanged["toml"], NOTES);

    gateway
        .answer_ticket(ticket.id, Answer::Approve)
        .await
        .unwrap();
    let changed = call(
        &api,
        "ConnectionService/GetDefinitionText",
        json!({ "id": "notes" }),
    )
    .await;
    assert_eq!(changed["toml"], proposed.as_str());
    let changes = call(
        &api,
        "ConnectionService/ListDefinitionChanges",
        json!({ "definition": "notes" }),
    )
    .await;
    assert_eq!(changes["changes"][0]["actor"], "secretary", "{changes}");
    assert_eq!(changes["changes"][0]["before"], NOTES);
    assert_eq!(changes["changes"][0]["why"], "メモを消す道具がない");

    assert_eq!(
        call(
            &api,
            "ConnectionService/RevertDefinition",
            json!({ "id": "notes" })
        )
        .await,
        json!({})
    );
    let back = call(
        &api,
        "ConnectionService/GetDefinitionText",
        json!({ "id": "notes" }),
    )
    .await;
    assert_eq!(back["toml"], NOTES);
}

#[tokio::test]
async fn a_broken_proposal_is_refused_with_why_and_nothing_waits() {
    let dir = temp();
    let api = open(&dir);
    let tools = ApiTools::new(api.clone()).unwrap();

    let refused = tools
        .call(
            "propose_definition_change",
            json!({ "definition": "notes", "toml": "format = 1", "why": "" }),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(refused.contains("does not read"), "{refused}");
    assert!(api.app().gateway.held_tickets().unwrap().is_empty());
}
