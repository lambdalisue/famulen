//! What each role runs on, as the console assigns it.

use serde_json::{Value, json};

use crate::support::{open, temp, try_call};

fn role<'a>(models: &'a Value, role: &str) -> &'a Value {
    models["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["role"] == role)
        .unwrap()
}

#[tokio::test]
async fn each_role_keeps_its_own_effort_with_its_model() {
    let dir = temp();
    let api = open(&dir);
    let (_, before) = try_call(&api, "ModelService/GetModels", json!({})).await;
    let dream = role(&before, "ROLE_DREAM")["model"].clone();
    let secretary_before = role(&before, "ROLE_SECRETARY")["effort"].clone();

    try_call(
        &api,
        "ModelService/AssignRole",
        json!({ "role": "ROLE_DREAM", "model": dream, "effort": "max" }),
    )
    .await;
    let (_, after) = try_call(&api, "ModelService/GetModels", json!({})).await;

    assert_eq!(role(&after, "ROLE_DREAM")["effort"], "max");
    assert_eq!(role(&after, "ROLE_SECRETARY")["effort"], secretary_before);
}

#[tokio::test]
async fn taking_the_model_off_a_role_is_refused() {
    let dir = temp();
    let api = open(&dir);
    let (_, before) = try_call(&api, "ModelService/GetModels", json!({})).await;
    for r in ["ROLE_SECRETARY", "ROLE_DREAM", "ROLE_WORKER"] {
        let (status, _) = try_call(
            &api,
            "ModelService/AssignRole",
            json!({ "role": r, "model": "" }),
        )
        .await;
        assert!(!status.is_success(), "{r}: {status}");

        let (_, after) = try_call(&api, "ModelService/GetModels", json!({})).await;
        assert!(role(&after, r)["model"].is_string(), "{r}");
        assert_eq!(role(&after, r)["model"], role(&before, r)["model"], "{r}");
    }
}

#[tokio::test]
async fn an_unknown_effort_is_refused() {
    let dir = temp();
    let api = open(&dir);
    let (_, before) = try_call(&api, "ModelService/GetModels", json!({})).await;
    let dream = role(&before, "ROLE_DREAM")["model"].clone();

    let (status, _) = try_call(
        &api,
        "ModelService/AssignRole",
        json!({ "role": "ROLE_DREAM", "model": dream, "effort": "fast" }),
    )
    .await;

    assert!(!status.is_success(), "{status}");
}

#[tokio::test]
async fn each_role_says_the_effort_it_returns_to() {
    let dir = temp();
    let (_, models) = try_call(&open(&dir), "ModelService/GetModels", json!({})).await;
    for (r, effort) in [
        ("ROLE_SECRETARY", "medium"),
        ("ROLE_DREAM", "xhigh"),
        ("ROLE_LOOKOUT", "medium"),
        ("ROLE_TRIAGE", "low"),
        ("ROLE_ASSESS", "low"),
        ("ROLE_WORKER", "medium"),
    ] {
        assert_eq!(role(&models, r)["defaultEffort"], effort, "{r}");
        assert_eq!(role(&models, r)["effort"], effort, "{r}");
    }
}

#[tokio::test]
async fn a_removed_found_model_is_offered_for_restoring_and_not_to_roles() {
    let dir = temp();
    let api = open(&dir);
    let found = |cli, id: &str| famulen_core::models::ModelSpec {
        id: id.into(),
        name: id.into(),
        kind: famulen_core::models::ModelKind::AgentCli,
        endpoint: None,
        model: id.split_once('-').unwrap().1.into(),
        cli,
        price: Default::default(),
    };
    use famulen_core::models::{Cli, Source};
    let models = &api.app().models;
    models
        .take_in(Source::Claude, vec![found(Cli::Claude, "claude-fable")])
        .unwrap();
    models
        .take_in(Source::Codex, vec![found(Cli::Codex, "codex-gpt")])
        .unwrap();

    let (_, before) = try_call(&api, "ModelService/GetModels", json!({})).await;
    let choices = |m: &Value, r: &str| role(m, r)["choices"].as_array().unwrap().clone();
    assert!(choices(&before, "ROLE_DREAM").contains(&json!("codex-gpt")));
    assert!(!choices(&before, "ROLE_SECRETARY").contains(&json!("codex-gpt")));
    let listed = before["models"].as_array().unwrap();
    let codex = listed.iter().find(|m| m["id"] == "codex-gpt").unwrap();
    assert_eq!(codex["found"], true);
    assert_eq!(codex["cli"], "AGENT_CLI_CODEX");

    try_call(
        &api,
        "ModelService/UnregisterModel",
        json!({ "id": "claude-fable" }),
    )
    .await;
    let (_, removed) = try_call(&api, "ModelService/GetModels", json!({})).await;
    assert!(!choices(&removed, "ROLE_DREAM").contains(&json!("claude-fable")));
    assert_eq!(removed["candidates"][0]["id"], "claude-fable");
    let (status, _) = try_call(
        &api,
        "ModelService/AssignRole",
        json!({ "role": "ROLE_DREAM", "model": "claude-fable" }),
    )
    .await;
    assert!(!status.is_success(), "{status}");

    let (_, restored) = try_call(
        &api,
        "ModelService/RestoreModel",
        json!({ "id": "claude-fable" }),
    )
    .await;
    assert!(choices(&restored, "ROLE_DREAM").contains(&json!("claude-fable")));
    assert!(restored["candidates"].as_array().is_none_or(Vec::is_empty));
}
