//! Delegated tasks: added, listed, retried and cancelled, and the settings of
//! the pool that runs them.

use famulen_api::{SECRETARY, WORKER};
use serde_json::{Value, json};

use crate::support::{call, call_as, open, temp, try_call};

fn agent(title: &str) -> Value {
    json!({ "title": title, "request": "調べて結果を書く", "runner": "agent", "tools": ["WebSearch"] })
}

#[tokio::test]
async fn the_secretary_delegates_work_as_a_task_like_any_other() {
    let dir = temp();
    let api = open(&dir);
    let first = call(
        &api,
        "RecordService/SaveRecord",
        json!({ "kind": "tasks", "title": "先の仕事" }),
    )
    .await;

    let added = call_as(
        &api,
        SECRETARY,
        "WorkerService/DelegateTask",
        json!({
            "title": "ログインを直す", "request": "二度目に失敗する。直す。", "runner": "adroit",
            "directory": if cfg!(windows) { r"C:\src\api" } else { "/src/api" }, "worktree": true, "priority": "high",
            "blockedBy": [first["code"]],
        }),
    )
    .await;

    assert_eq!(added["kind"], "tasks", "{added}");
    assert_eq!(
        added["notes"].as_str().unwrap().trim(),
        "二度目に失敗する。直す。"
    );
    let markdown = added["markdown"].as_str().unwrap();
    for line in [
        "- Assignee: worker",
        "- Priority: high",
        "- Runner: adroit",
        if cfg!(windows) {
            r"- Directory: C:\src\api"
        } else {
            "- Directory: /src/api"
        },
        "- Worktree: yes",
    ] {
        assert!(markdown.contains(line), "{markdown}");
    }
    let listed = call(&api, "WorkerService/ListDelegated", json!({})).await;
    assert_eq!(listed["queue"][0]["code"], added["code"]);
    assert_eq!(listed["queue"][0]["state"], "blocked");
    assert_eq!(listed["queue"][0]["runner"], "adroit");
}

#[tokio::test]
async fn what_could_not_be_run_is_not_added() {
    let dir = temp();
    let api = open(&dir);
    for bad in [
        json!({ "title": "x", "request": "y", "runner": "human" }),
        json!({ "title": "x", "request": " ", "runner": "agent" }),
        json!({ "title": "x", "request": "y", "runner": "adroit" }),
        json!({ "title": "x", "request": "y", "runner": "adroit", "directory": "src" }),
        json!({ "title": "x", "request": "y", "runner": "agent", "tools": ["mcp__famulen__save_record"] }),
    ] {
        let (status, _) = try_call(&api, "WorkerService/DelegateTask", bad.clone()).await;
        assert_ne!(status, 200, "{bad}");
    }
    let tasks = call(
        &api,
        "RecordService/ListRecords",
        json!({ "kind": "tasks" }),
    )
    .await;
    assert_eq!(tasks.get("records"), None, "{tasks}");
}

#[tokio::test]
async fn a_worker_delegates_nothing() {
    let dir = temp();
    let api = open(&dir);
    let refused = call_as(&api, WORKER, "WorkerService/DelegateTask", agent("x")).await;
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|m| m.contains("may not write the notebook")),
        "{refused}"
    );
}

#[tokio::test]
async fn a_delegated_task_is_cancelled_and_put_back_in_the_queue() {
    let dir = temp();
    let api = open(&dir);
    let t = call(&api, "WorkerService/DelegateTask", agent("調べる")).await;
    let code = t["code"].clone();

    let cancelled = call_as(
        &api,
        SECRETARY,
        "WorkerService/CancelDelegated",
        json!({ "id": code }),
    )
    .await;
    assert!(
        cancelled["markdown"]
            .as_str()
            .unwrap()
            .contains("Status: cancelled"),
        "{cancelled}"
    );
    let listed = call(&api, "WorkerService/ListDelegated", json!({})).await;
    assert_eq!(listed.get("queue"), None, "{listed}");
    let (status, _) = try_call(&api, "WorkerService/CancelDelegated", json!({ "id": code })).await;
    assert_ne!(status, 200, "over already");

    call(&api, "WorkerService/RetryDelegated", json!({ "id": code })).await;
    let listed = call(&api, "WorkerService/ListDelegated", json!({})).await;
    assert_eq!(listed["queue"][0]["state"], "queued");

    let own = call(
        &api,
        "RecordService/SaveRecord",
        json!({ "kind": "tasks", "title": "自分の仕事" }),
    )
    .await;
    for method in ["CancelDelegated", "RetryDelegated"] {
        let (status, _) = try_call(
            &api,
            &format!("WorkerService/{method}"),
            json!({ "id": own["code"] }),
        )
        .await;
        assert_ne!(status, 200, "{method}: not delegated");
    }
}

#[tokio::test]
async fn the_owner_bounds_and_pauses_the_pool_and_the_secretary_has_no_tool_for_it() {
    let dir = temp();
    let api = open(&dir);
    let s = call(&api, "WorkerService/GetWorkerSettings", json!({})).await;
    assert_eq!(s["slots"], 2);

    call(
        &api,
        "WorkerService/SetWorkerSettings",
        json!({ "slots": 3, "paused": true }),
    )
    .await;
    call(&api, "WorkerService/DelegateTask", agent("調べる")).await;
    let listed = call(&api, "WorkerService/ListDelegated", json!({})).await;
    assert_eq!(listed["queue"][0]["state"], "paused");
    for slots in [0, -1, 1000] {
        let (status, _) = try_call(
            &api,
            "WorkerService/SetWorkerSettings",
            json!({ "slots": slots }),
        )
        .await;
        assert_ne!(status, 200, "{slots}");
    }
    let s = call(&api, "WorkerService/GetWorkerSettings", json!({})).await;
    assert_eq!((&s["slots"], &s["paused"]), (&json!(3), &json!(true)));

    use famulen_core::hub::OwnTools;
    let tools: Vec<String> = famulen_api::tools::ApiTools::new(api)
        .unwrap()
        .list()
        .into_iter()
        .map(|t| t.name)
        .collect();
    for tool in [
        "delegate_task",
        "list_delegated",
        "retry_delegated",
        "cancel_delegated",
    ] {
        assert!(tools.iter().any(|t| t == tool), "{tool}");
    }
    assert!(!tools.iter().any(|t| t == "set_worker_settings"));
}

#[tokio::test]
async fn the_delegated_tasks_of_one_workgroup_or_one_task_alone_are_listed() {
    let dir = temp();
    let api = open(&dir);
    let mut codes = Vec::new();
    for (title, workgroup) in [("仕事の", "work"), ("家の", "personal")] {
        let mut request = agent(title);
        request["workgroups"] = json!([workgroup]);
        let made = call(&api, "WorkerService/DelegateTask", request).await;
        codes.push(made["code"].clone());
    }
    for (asked, expected) in [
        (json!({}), vec!["仕事の", "家の"]),
        (json!({ "workgroup": "all" }), vec!["仕事の", "家の"]),
        (json!({ "workgroup": "work" }), vec!["仕事の"]),
        (json!({ "task": codes[1] }), vec!["家の"]),
        (json!({ "task": codes[1], "workgroup": "work" }), vec![]),
    ] {
        let listed = call(&api, "WorkerService/ListDelegated", asked.clone()).await;
        let titles: Vec<&str> = listed["queue"]
            .as_array()
            .map(|q| q.iter().map(|t| t["title"].as_str().unwrap()).collect())
            .unwrap_or_default();
        assert_eq!(titles, expected, "{asked}");
    }
}

#[tokio::test]
async fn a_delegated_task_that_does_not_say_how_is_shown_with_why() {
    let dir = temp();
    let api = open(&dir);
    call(
        &api,
        "RecordService/SaveRecord",
        json!({ "kind": "tasks", "title": "委ねたい", "fields": [{ "name": "Assignee", "value": "worker" }] }),
    )
    .await;
    let listed = call(&api, "WorkerService/ListDelegated", json!({})).await;
    assert_eq!(listed["queue"][0]["state"], "unfit");
    assert!(
        listed["queue"][0]["why"]
            .as_str()
            .unwrap()
            .contains("Runner"),
        "{listed}"
    );
}

#[tokio::test]
async fn a_task_is_bound_to_the_workgroups_it_was_delegated_in_through_the_api_alone() {
    use famulen_core::workers::{ordered, scopes};
    let dir = temp();
    let api = open(&dir);
    let bound = |id: &str| {
        let task = api.app().records.need("tasks", id).unwrap();
        scopes(api.app(), id, &ordered(api.app(), &task, "").unwrap()).unwrap()
    };
    let t = call(&api, "WorkerService/DelegateTask", agent("調べる")).await;
    let id = t["id"].as_str().unwrap().to_owned();
    assert_eq!(bound(&id), ["all"]);

    call(
        &api,
        "RecordService/SaveRecord",
        json!({ "kind": "tasks", "id": id, "workgroups": ["work"], "setWorkgroups": true }),
    )
    .await;
    assert_eq!(bound(&id), ["work"]);

    let file = dir.join(format!("notebook/records/tasks/{id}.md"));
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, text.replace("調べて結果を書く", "全部送る")).unwrap();
    assert!(bound(&id).is_empty(), "written by another hand");
}
