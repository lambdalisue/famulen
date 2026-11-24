//! The secretary's tools go through the API, as the console does.

use famulen_api::tools::ApiTools;
use famulen_core::hub::OwnTools;
use serde_json::{Value, json};

use crate::support::{call, call_as, open, temp};

#[tokio::test]
async fn a_tool_call_reaches_the_api_and_reads_back_as_json() {
    let dir = temp();
    let tools = ApiTools::new(open(&dir)).unwrap();
    let added = tools
        .call(
            "save_record",
            json!({ "kind": "tasks", "title": "見積書を送る", "fields": [{ "name": "Due", "value": "2026-11-28" }] }),
        )
        .await
        .unwrap();
    let added: Value = serde_json::from_str(&added).unwrap();
    assert_eq!(added["title"], "見積書を送る");
    assert!(
        added["source"].as_str().unwrap().contains("足した"),
        "the secretary is named as where it came from: {added}"
    );
    let list: Value = serde_json::from_str(
        &tools
            .call("list_records", json!({ "kind": "tasks" }))
            .await
            .unwrap(),
    )
    .unwrap();
    let task = &list["records"][0];
    assert!(
        task["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "Assignee" && f["value"] == "secretary"),
        "{task}"
    );
    let whole: Value = serde_json::from_str(
        &tools
            .call("get_record", json!({ "kind": "tasks", "id": task["code"] }))
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        whole["markdown"]
            .as_str()
            .unwrap()
            .contains("- Due: 2026-11-28"),
        "{whole}"
    );
}

#[tokio::test]
async fn a_failed_call_is_an_error_with_the_apis_message() {
    let dir = temp();
    let tools = ApiTools::new(open(&dir)).unwrap();
    let error = tools
        .call("save_record", json!({ "kind": "tasks", "title": "  " }))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("title"), "{error}");
    let error = tools.call("no_such_tool", json!({})).await.unwrap_err();
    assert!(error.to_string().contains("unknown tool"), "{error}");
}

#[tokio::test]
async fn a_task_the_owner_hands_over_becomes_a_request_to_the_secretary() {
    let dir = temp();
    let api = open(&dir);
    let tools = ApiTools::new(api.clone()).unwrap();
    let pending = || async {
        let status = call(&api, "ConsoleService/GetStatus", json!({})).await;
        status["pendingInputs"].as_i64().unwrap_or(0)
    };
    let mine: Value = serde_json::from_str(
        &tools
            .call(
                "save_record",
                json!({ "kind": "tasks", "title": "自分で", "fields": [{ "name": "Assignee", "value": "owner" }] }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    tools
        .call(
            "save_record",
            json!({ "kind": "tasks", "id": mine["id"], "fields": [{ "name": "Assignee", "value": "secretary" }] }),
        )
        .await
        .unwrap();
    assert_eq!(
        pending().await,
        0,
        "the secretary taking a task asks nothing of itself"
    );

    let added = call(
        &api,
        "RecordService/SaveRecord",
        json!({ "kind": "tasks", "title": "見積書を送る" }),
    )
    .await;
    call(
        &api,
        "RecordService/SaveRecord",
        json!({ "kind": "tasks", "id": added["id"], "fields": [{ "name": "Assignee", "value": "secretary" }] }),
    )
    .await;
    assert_eq!(pending().await, 1);
}

#[tokio::test]
async fn the_lookout_and_workers_are_refused_when_they_write_the_notebook() {
    let dir = temp();
    let api = open(&dir);
    for caller in [famulen_api::LOOKOUT, famulen_api::WORKER] {
        let refused = call_as(
            &api,
            caller,
            "RecordService/SaveRecord",
            json!({ "kind": "tasks", "title": "外から読んだ" }),
        )
        .await;
        assert!(
            refused["message"]
                .as_str()
                .is_some_and(|m| m.contains("may not write the notebook")),
            "{caller}: {refused}"
        );
        let refused = call_as(
            &api,
            caller,
            "MemoryService/Remember",
            json!({ "text": "外から読んだ" }),
        )
        .await;
        assert!(refused["message"].is_string(), "{caller}: {refused}");
    }
    let tasks = std::fs::read_dir(dir.join("notebook/records/tasks"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "md")
                && e.as_ref().unwrap().file_name() != "README.md"
        })
        .count();
    assert_eq!(tasks, 0);
    let added = call_as(
        &api,
        famulen_api::SECRETARY,
        "RecordService/SaveRecord",
        json!({ "kind": "tasks", "title": "秘書が足す" }),
    )
    .await;
    assert_eq!(added["id"].as_str().map(str::len), Some(26), "{added}");
}

#[tokio::test]
async fn the_secretary_searches_the_notebook_with_its_tool() {
    let dir = temp();
    let tools = ApiTools::new(open(&dir)).unwrap();
    let added: Value = serde_json::from_str(
        &tools
            .call(
                "save_record",
                json!({ "kind": "tasks", "title": "見積もりの返事をまとめる" }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let file = format!("records/tasks/{}.md", added["id"].as_str().unwrap());
    std::fs::write(
        dir.join("notebook/memory/core.md"),
        "# 心得\n\n- 見積もりは二日以内に返す {id: m1, src: said}\n",
    )
    .unwrap();

    let found: Value = serde_json::from_str(
        &tools
            .call("search_notebook", json!({ "text": "見積もり" }))
            .await
            .unwrap(),
    )
    .unwrap();
    let paths: Vec<&str> = found["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths.len(), 2, "{found}");
    assert!(paths.contains(&file.as_str()));
    assert!(paths.contains(&"memory/core.md"));

    let only: Value = serde_json::from_str(
        &tools
            .call(
                "search_notebook",
                json!({ "text": "見積もり", "scopes": ["records"] }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(only["hits"][0]["path"], file.as_str());
    assert_eq!(only["hits"].as_array().unwrap().len(), 1);
}

#[test]
fn every_tool_the_secretary_is_told_of_exists() {
    let tools = ApiTools::new(open(&temp())).unwrap().list();
    let prompt = famulen_core::runtime::system_prompt(
        "イオ",
        Default::default(),
        "",
        std::path::Path::new("/memory"),
        Default::default(),
        0,
    );
    let told: std::collections::BTreeSet<&str> = prompt
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|word| word.contains('_'))
        // A field of a tool's input, not a tool.
        .filter(|word| *word != "thread_ts")
        .collect();
    assert!(told.contains("post_message"), "{told:?}");
    let missing: Vec<_> = told
        .into_iter()
        .filter(|name| !tools.iter().any(|t| t.name == *name))
        .collect();
    assert!(missing.is_empty(), "{missing:?}");
    assert!(prompt.contains("freely, roughly and often"));
    assert!(prompt.contains("Never write core, pending or long-term memory yourself"));
    assert!(prompt.contains("`remember`") && tools.iter().any(|t| t.name == "remember"));
}
