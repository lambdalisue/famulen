//! The vault through the API: the console and the secretary put secrets in
//! and take them out, and no answer holds a value.

use famulen_api::tools::ApiTools;
use famulen_core::gateway::KEY_VAR;
use famulen_core::hub::OwnTools;
use serde_json::json;

use crate::support::{call, call_as, open, open_with, temp};

#[tokio::test]
async fn the_owner_puts_a_secret_in_sees_it_without_its_value_and_takes_it_out() {
    let dir = temp();
    let api = open(&dir);

    let kept = call(
        &api,
        "VaultService/SetSecretValue",
        json!({ "name": "bank", "value": "correct-horse-battery", "purpose": "銀行" }),
    )
    .await;
    let listed = call(&api, "VaultService/ListSecrets", json!({})).await;

    assert_eq!(kept["name"], "bank", "{kept}");
    assert_eq!(kept["by"], "owner");
    let secret = &listed["secrets"][0];
    assert_eq!(secret["description"], "銀行", "{listed}");
    assert!(
        secret.get("automatic").is_none_or(|a| a == false),
        "{listed}"
    );
    let sealed = std::fs::read(dir.join("gateway/vault/bank.age")).unwrap();
    assert!(sealed.starts_with(b"age-encryption.org/v1"));
    for shown in [kept.to_string(), listed.to_string()] {
        assert!(!shown.contains("correct-horse"), "{shown}");
    }

    let deleted = call(&api, "VaultService/DeleteSecret", json!({ "id": "bank" })).await;
    assert_eq!(deleted, json!({}));
    let listed = call(&api, "VaultService/ListSecrets", json!({})).await;
    assert!(
        listed["secrets"].as_array().is_none_or(Vec::is_empty),
        "{listed}"
    );
    let log = api.app().gateway.log_entries(10).unwrap();
    assert_eq!(
        log.iter()
            .map(|e| (e.tool.as_str(), e.actor.as_str()))
            .collect::<Vec<_>>(),
        [("vault.delete", "owner"), ("vault.put", "owner")]
    );
}

#[tokio::test]
async fn the_secretary_puts_in_and_takes_out_with_its_tools_and_asks_before_taking_out() {
    let dir = temp();
    let api = open(&dir);
    let tools = ApiTools::new(api.clone()).unwrap();
    let defs = tools.list();
    let def = |name: &str| defs.iter().find(|t| t.name == name).cloned();
    assert!(def("list_secrets").is_some_and(|t| t.read_only));
    assert!(
        def("delete_secret").is_some_and(|t| !t.confirm),
        "the gateway asks under the vault's permissions, not the tool"
    );

    let kept = tools
        .call(
            "set_secret_value",
            json!({ "name": "wifi", "value": "a-wifi-password", "purpose": "家の Wi-Fi" }),
        )
        .await
        .unwrap();
    assert!(!kept.contains("a-wifi-password"), "{kept}");
    assert!(kept.contains("\"by\":\"secretary\""), "{kept}");
    let held = match tools.call("delete_secret", json!({ "id": "wifi" })).await {
        Ok(text) => text,
        Err(error) => error.to_string(),
    };
    assert!(held.contains("Held for the owner's approval"), "{held}");
    let gateway = &api.app().gateway;
    let ticket = gateway.held_tickets().unwrap().pop().unwrap();
    assert_eq!(ticket.tool, famulen_core::gateway::DELETE_SECRET);
    gateway
        .answer_ticket(ticket.id, famulen_core::gateway::Answer::Approve)
        .await
        .unwrap();
    let listed = tools.call("list_secrets", json!({})).await.unwrap();
    assert!(!listed.contains("wifi"), "{listed}");
}

#[tokio::test]
async fn describing_a_secret_rewrites_what_it_is_for_keeps_its_value_and_is_logged() {
    let dir = temp();
    let api = open(&dir);
    call(
        &api,
        "VaultService/SetSecretValue",
        json!({ "name": "bank", "value": "correct-horse-battery", "purpose": "銀行" }),
    )
    .await;
    let sealed = std::fs::read(dir.join("gateway/vault/bank.age")).unwrap();

    let described = call(
        &api,
        "VaultService/DescribeSecret",
        json!({ "name": "bank", "description": "銀行のサイトと暗証番号" }),
    )
    .await;

    assert_eq!(
        described["description"], "銀行のサイトと暗証番号",
        "{described}"
    );
    let listed = call(&api, "VaultService/ListSecrets", json!({})).await;
    assert_eq!(
        listed["secrets"][0]["description"],
        "銀行のサイトと暗証番号"
    );
    assert_eq!(
        std::fs::read(dir.join("gateway/vault/bank.age")).unwrap(),
        sealed,
        "the value is left as it was"
    );
    let log = api.app().gateway.log_entries(1).unwrap();
    assert_eq!(
        (log[0].tool.as_str(), log[0].secret.as_str()),
        ("vault.describe", "bank")
    );
    let missing = call(
        &api,
        "VaultService/DescribeSecret",
        json!({ "name": "nothing", "description": "x" }),
    )
    .await;
    assert!(missing["message"].is_string(), "{missing}");
}

#[tokio::test]
async fn the_secretary_describes_a_secret_with_its_tool_under_the_vaults_permission() {
    let dir = temp();
    let api = open(&dir);
    let tools = ApiTools::new(api.clone()).unwrap();
    assert!(
        tools
            .list()
            .iter()
            .any(|t| t.name == "describe_secret" && !t.confirm),
        "the gateway asks under the vault's permissions, not the tool"
    );
    tools
        .call(
            "set_secret_value",
            json!({ "name": "wifi", "value": "a-wifi-password" }),
        )
        .await
        .unwrap();

    let described = tools
        .call(
            "describe_secret",
            json!({ "name": "wifi", "description": "家の Wi-Fi" }),
        )
        .await
        .unwrap();
    assert!(described.contains("家の Wi-Fi"), "{described}");
    assert!(described.contains("\"by\":\"secretary\""), "{described}");

    call(
        &api,
        "VaultService/SetVaultPermission",
        json!({ "change": "describe", "permission": "PERMISSION_CONFIRM" }),
    )
    .await;
    let held = match tools
        .call(
            "describe_secret",
            json!({ "name": "wifi", "description": "別の説明" }),
        )
        .await
    {
        Ok(text) => text,
        Err(error) => error.to_string(),
    };
    assert!(held.contains("Held for the owner's approval"), "{held}");
    let ticket = api.app().gateway.held_tickets().unwrap().pop().unwrap();
    assert_eq!(ticket.tool, famulen_core::gateway::DESCRIBE_SECRET);
}

#[tokio::test]
async fn the_lookout_and_workers_do_not_touch_the_vault() {
    let dir = temp();
    let api = open(&dir);
    for caller in [famulen_api::LOOKOUT, famulen_api::WORKER] {
        for (method, body) in [
            ("ListSecrets", json!({})),
            ("SetSecretValue", json!({ "name": "x", "value": "y" })),
            ("DeleteSecret", json!({ "id": "x" })),
            ("DescribeSecret", json!({ "name": "x", "description": "y" })),
        ] {
            let refused = call_as(&api, caller, &format!("VaultService/{method}"), body).await;
            assert!(
                refused["message"].is_string(),
                "{caller} {method}: {refused}"
            );
        }
    }
    assert!(api.app().gateway.secrets().unwrap().is_empty());
}

#[tokio::test]
async fn without_a_key_the_list_says_why_and_putting_in_is_refused() {
    let dir = temp();
    let api = open_with(&dir, None);

    let listed = call(&api, "VaultService/ListSecrets", json!({})).await;
    let refused = call(
        &api,
        "VaultService/SetSecretValue",
        json!({ "name": "x", "value": "y" }),
    )
    .await;

    assert!(
        listed["unavailable"].as_str().unwrap().contains(KEY_VAR),
        "{listed}"
    );
    assert!(
        refused["message"].as_str().unwrap().contains(KEY_VAR),
        "{refused}"
    );
}

#[tokio::test]
async fn the_owner_chooses_whether_the_secretary_is_asked_before_putting_a_secret_in() {
    let dir = temp();
    let api = open(&dir);

    let permissions = call(&api, "VaultService/GetVaultPermissions", json!({})).await;
    assert_eq!(
        permissions,
        json!({
            "put": "PERMISSION_ALLOW",
            "delete": "PERMISSION_CONFIRM",
            "describe": "PERMISSION_ALLOW"
        })
    );
    let permissions = call(
        &api,
        "VaultService/SetVaultPermission",
        json!({ "change": "put", "permission": "PERMISSION_CONFIRM" }),
    )
    .await;
    assert_eq!(permissions["put"], "PERMISSION_CONFIRM", "{permissions}");

    let held = call_as(
        &api,
        famulen_api::SECRETARY,
        "VaultService/SetSecretValue",
        json!({ "name": "bank", "value": "hunter2-hunter2", "purpose": "銀行" }),
    )
    .await;
    assert!(
        held["message"]
            .as_str()
            .unwrap()
            .contains("Held for the owner's approval"),
        "{held}"
    );
    let listed = call(&api, "VaultService/ListSecrets", json!({})).await;
    assert!(
        listed["secrets"].as_array().is_none_or(Vec::is_empty),
        "{listed}"
    );
    assert!(!plain_anywhere(&dir, "hunter2"));
}

#[tokio::test]
async fn the_owner_reads_the_gateways_log_filtered_and_page_by_page() {
    let dir = temp();
    let api = open(&dir);
    for name in ["a", "b", "c"] {
        call(
            &api,
            "VaultService/SetSecretValue",
            json!({ "name": name, "value": "some-value" }),
        )
        .await;
    }
    call_as(
        &api,
        famulen_api::SECRETARY,
        "VaultService/DeleteSecret",
        json!({ "id": "a" }),
    )
    .await;

    let first = call(
        &api,
        "GatewayLogService/ListGatewayLog",
        json!({ "limit": 2 }),
    )
    .await;
    let entries = first["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "{first}");
    assert_eq!(first["more"], true);
    assert_eq!(entries[0]["decision"], "confirm");
    assert_eq!(entries[0]["actor"], "secretary");
    let before: i64 = entries[1]["id"].as_str().unwrap().parse().unwrap();
    let rest = call(
        &api,
        "GatewayLogService/ListGatewayLog",
        json!({ "limit": 50, "before": before }),
    )
    .await;
    assert_eq!(rest["entries"].as_array().unwrap().len(), 2, "{rest}");
    assert!(rest.get("more").is_none_or(|m| m == false));
    let by_owner = call(
        &api,
        "GatewayLogService/ListGatewayLog",
        json!({ "actor": "owner", "decision": "allow" }),
    )
    .await;
    assert_eq!(by_owner["entries"].as_array().unwrap().len(), 3);
    assert!(!by_owner.to_string().contains("some-value"));
    let tools = ApiTools::new(api).unwrap();
    assert!(
        tools.list().iter().all(|t| t.name != "list_gateway_log"),
        "the log is the owner's to read, not a tool"
    );
}

#[tokio::test]
async fn a_models_key_goes_to_the_vault_shows_nothing_of_itself_and_leaves_with_the_model() {
    let dir = temp();
    let api = open(&dir);
    let secret = "sk-model-0123456789-0123456789-wxyz";

    let registered = call(
        &api,
        "ModelService/RegisterModel",
        json!({
            "name": "Paid",
            "kind": "MODEL_KIND_API",
            "model": "m",
            "endpoint": "http://127.0.0.1:9/v1",
            "apiKey": secret,
        }),
    )
    .await;
    let id = registered["id"].as_str().unwrap().to_owned();
    assert!(!registered.to_string().contains(secret), "{registered}");
    assert_eq!(registered["hasKey"], true, "{registered}");
    let models = call(&api, "ModelService/GetModels", json!({})).await;
    assert!(!models.to_string().contains(secret), "{models}");
    let listed = call(&api, "VaultService/ListSecrets", json!({})).await;
    let entry = &listed["secrets"][0];
    assert_eq!(entry["name"], format!("model:{id}:api_key"), "{listed}");
    assert_eq!(
        (&entry["automatic"], &entry["model"]),
        (&json!(true), &json!("Paid")),
        "{listed}"
    );
    for fragment in [&secret[..4], &secret[secret.len() - 4..]] {
        assert!(
            !listed.to_string().contains(fragment),
            "{fragment}: {listed}"
        );
    }
    assert!(
        !plain_anywhere(&dir, secret),
        "the key is kept only sealed in the vault"
    );
    let tools = ApiTools::new(api.clone()).unwrap();
    assert!(
        tools.list().iter().all(|t| t.name != "register_model"),
        "only the owner registers a model and its key"
    );

    call(&api, "ModelService/UnregisterModel", json!({ "id": id })).await;
    let listed = call(&api, "VaultService/ListSecrets", json!({})).await;
    assert!(!listed.to_string().contains("model:"), "{listed}");
}

fn plain_anywhere(dir: &std::path::Path, needle: &str) -> bool {
    std::fs::read_dir(dir).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        if path.is_dir() {
            plain_anywhere(&path, needle)
        } else {
            String::from_utf8_lossy(&std::fs::read(&path).unwrap()).contains(needle)
        }
    })
}
