//! The owner's permissions for the secretary's built-in tools, through the API.

use famulen_api::Api;
use famulen_core::permissions::{Decision, Permissions};
use serde_json::{Value, json};

use crate::support::{self, DataDir, call, temp};

/// The API, the secretary having been offered a shell and a reader.
fn open(dir: &DataDir) -> (Api, Permissions) {
    let api = support::open(dir);
    let permissions = api.app().permissions.clone();
    permissions
        .record_tools(&[
            "Bash".into(),
            "Read".into(),
            "mcp__famulen__save_record".into(),
        ])
        .unwrap();
    (api, permissions)
}

fn permissions_of(listed: &Value) -> Vec<(String, String, String)> {
    listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_owned(),
                t["permission"]
                    .as_str()
                    .unwrap_or("PERMISSION_UNSPECIFIED")
                    .to_owned(),
                t["effective"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn the_owner_forbids_a_builtin_tool_and_the_secretary_is_refused() {
    let dir = temp();
    let (api, permissions) = open(&dir);
    assert_eq!(
        permissions_of(&call(&api, "ConnectionService/ListBuiltinTools", json!({})).await),
        [
            (
                "Bash".into(),
                "PERMISSION_UNSPECIFIED".into(),
                "PERMISSION_AUTO".into()
            ),
            (
                "Read".into(),
                "PERMISSION_UNSPECIFIED".into(),
                "PERMISSION_AUTO".into()
            ),
        ]
    );
    call(
        &api,
        "ConnectionService/SetPermission",
        json!({ "scope": "", "tool": "Bash", "permission": "PERMISSION_FORBID" }),
    )
    .await;
    assert_eq!(
        permissions_of(&call(&api, "ConnectionService/ListBuiltinTools", json!({})).await)[0],
        (
            "Bash".into(),
            "PERMISSION_FORBID".into(),
            "PERMISSION_FORBID".into()
        )
    );
    assert!(matches!(
        permissions
            .decide("Bash", &json!({ "command": "ls" }))
            .await
            .unwrap(),
        Decision::Forbid(_)
    ));
}

#[tokio::test]
async fn the_owner_chooses_for_every_writing_builtin_tool_at_once() {
    let dir = temp();
    let (api, permissions) = open(&dir);
    call(
        &api,
        "ConnectionService/SetPermissions",
        json!({ "scope": "", "which": "write", "permission": "PERMISSION_ALLOW" }),
    )
    .await;
    assert_eq!(
        permissions_of(&call(&api, "ConnectionService/ListBuiltinTools", json!({})).await),
        [
            (
                "Bash".into(),
                "PERMISSION_ALLOW".into(),
                "PERMISSION_ALLOW".into()
            ),
            (
                "Read".into(),
                "PERMISSION_UNSPECIFIED".into(),
                "PERMISSION_AUTO".into()
            ),
        ]
    );
    assert_eq!(
        permissions
            .decide("Bash", &json!({ "command": "ls" }))
            .await
            .unwrap(),
        Decision::Allow
    );
}
