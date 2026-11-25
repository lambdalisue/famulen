//! What the status and usage pages read: recent inputs, pending wake-ups, and
//! what each role used of each model.

use famulen_core::input::{Input, Sender, Source};
use famulen_core::models::{ModelKind, ModelSpec, Price, Role, Usage};
use serde_json::json;

use crate::support::{call, open, temp};

fn from_schedule(text: &str) -> Input {
    Input {
        sender: Sender::System,
        source: Source::Schedule {
            name: "朝のまとめ".into(),
            at: text.into(),
        },
        text: text.into(),
    }
}

#[tokio::test]
async fn list_inputs_gives_the_newest_first_up_to_the_limit_with_where_each_came_from() {
    let dir = temp();
    let api = open(&dir);
    for text in ["一つ目", "二つ目", "三つ目"] {
        api.app().store.enqueue(&from_schedule(text)).unwrap();
    }

    let listed = call(&api, "ConsoleService/ListInputs", json!({ "limit": 2 })).await;

    let inputs = listed["inputs"].as_array().unwrap();
    let texts: Vec<&str> = inputs.iter().map(|i| i["text"].as_str().unwrap()).collect();
    assert_eq!(texts, ["三つ目", "二つ目"]);
    assert_eq!(inputs[0]["source"], "schedule");
    assert_eq!(inputs[0]["detail"], "朝のまとめ");
    assert_eq!(inputs[0]["sender"], "system");
    assert_eq!(inputs[0]["status"], "pending");
}

#[tokio::test]
async fn list_inputs_turns_to_the_older_page_and_says_whether_more_remain() {
    let dir = temp();
    let api = open(&dir);
    for text in ["一つ目", "二つ目", "三つ目"] {
        api.app().store.enqueue(&from_schedule(text)).unwrap();
    }

    let first = call(&api, "ConsoleService/ListInputs", json!({ "limit": 2 })).await;
    assert_eq!(first["more"], true, "{first}");
    let oldest = first["inputs"][1]["id"].clone();

    let rest = call(
        &api,
        "ConsoleService/ListInputs",
        json!({ "limit": 2, "before": oldest }),
    )
    .await;
    let texts: Vec<&str> = rest["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, ["一つ目"]);
    assert!(rest.get("more").is_none_or(|m| m == false), "{rest}");
}

#[tokio::test]
async fn an_input_held_for_the_owner_says_when_it_arrived() {
    let dir = temp();
    let api = open(&dir);
    let store = &api.app().store;
    let id = store.enqueue(&from_schedule("一つ目")).unwrap().unwrap();
    store.enqueue(&from_schedule("二つ目")).unwrap();
    store.start_turn(id, "all").unwrap();
    store.hold_turn().unwrap();

    let inbox = call(&api, "ConsoleService/GetInbox", json!({})).await;
    let listed = call(&api, "ConsoleService/ListInputs", json!({})).await;

    assert_eq!(inbox["held"][0]["text"], "一つ目", "{inbox}");
    assert_eq!(inbox["held"][0]["at"], listed["inputs"][1]["at"]);
    assert!(inbox["held"][0]["at"].is_string(), "{inbox}");
}

#[tokio::test]
async fn list_wakes_shows_only_the_ones_still_waiting_and_a_cancelled_one_is_gone() {
    let dir = temp();
    let api = open(&dir);
    let kept = call(
        &api,
        "RoutineService/WakeAt",
        json!({ "inMinutes": 120, "note": "見積書が届いたか確かめる", "why": "主人が見積書を待っている" }),
    )
    .await;
    let dropped = call(
        &api,
        "RoutineService/WakeAt",
        json!({ "inMinutes": 60, "note": "要らなくなった" }),
    )
    .await;

    call(
        &api,
        "RoutineService/CancelWake",
        json!({ "id": dropped["id"] }),
    )
    .await;
    let listed = call(&api, "RoutineService/ListWakes", json!({})).await;

    let wakes = listed["wakes"].as_array().unwrap();
    assert_eq!(wakes.len(), 1, "{listed}");
    assert_eq!(wakes[0]["id"], kept["id"]);
    assert_eq!(wakes[0]["note"], "見積書が届いたか確かめる");
    assert_eq!(wakes[0]["why"], "主人が見積書を待っている");
    let at = chrono::DateTime::parse_from_rfc3339(wakes[0]["at"].as_str().unwrap()).unwrap();
    assert_eq!(
        wakes[0]["atUnix"].as_str().unwrap(),
        at.timestamp().to_string()
    );
}

#[tokio::test]
async fn cancelling_a_wake_that_is_not_waiting_is_an_error() {
    let dir = temp();
    let api = open(&dir);
    let answer = call(&api, "RoutineService/CancelWake", json!({ "id": 42 })).await;
    assert!(
        answer["message"].as_str().unwrap().contains("42"),
        "{answer}"
    );
}

fn priced(id: &str) -> ModelSpec {
    ModelSpec {
        id: id.into(),
        name: id.into(),
        kind: ModelKind::Api,
        endpoint: None,
        model: id.into(),
        cli: Default::default(),
        price: Price {
            input_per_mtok_yen: Some(1_000_000.0),
            output_per_mtok_yen: Some(2_000_000.0),
            per_call_yen: None,
        },
    }
}

fn used(input: u64, output: u64) -> Usage {
    Usage {
        input_tokens: Some(input),
        output_tokens: Some(output),
        calls: 1,
    }
}

#[tokio::test]
async fn usage_shows_each_role_and_model_by_day_and_in_total() {
    let dir = temp();
    let api = open(&dir);
    for (role, model, input, output) in [
        (Role::Secretary, "a", 10, 1),
        (Role::Secretary, "b", 5, 2),
        (Role::Dream, "a", 3, 0),
    ] {
        api.app()
            .models
            .record(role, &priced(model), &used(input, output))
            .unwrap();
    }

    let usage = call(
        &api,
        "ModelService/GetUsage",
        json!({ "span": "USAGE_SPAN_WEEK" }),
    )
    .await;

    let buckets = usage["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 7);
    let today = buckets
        .iter()
        .find(|b| b["from"] == usage["today"])
        .unwrap();
    assert_eq!(
        today["rows"],
        json!([
            { "role": "ROLE_SECRETARY", "model": "a", "calls": "1", "inputTokens": "10",
              "outputTokens": "1", "yen": 12.0, "metered": true },
            { "role": "ROLE_SECRETARY", "model": "b", "calls": "1", "inputTokens": "5",
              "outputTokens": "2", "yen": 9.0, "metered": true },
            { "role": "ROLE_DREAM", "model": "a", "calls": "1", "inputTokens": "3",
              "yen": 3.0, "metered": true },
        ])
    );
    assert_eq!(usage["rows"], today["rows"]);
    assert_eq!(usage["models"], json!(["a", "b"]));
    assert_eq!(usage["meteredYen"], json!(24.0));
}

#[tokio::test]
async fn usage_is_this_month_unless_asked_and_refuses_a_day_that_is_none() {
    let dir = temp();
    let api = open(&dir);
    let method = "ModelService/GetUsage";
    let usage = call(&api, method, json!({})).await;
    let today = usage["today"].as_str().unwrap();
    assert_eq!(usage["from"], json!(format!("{}-01", &today[..7])));
    let february = call(&api, method, json!({ "day": "2026-02-10" })).await;
    assert_eq!(february["to"], json!("2026-02-28"));
    let refused = call(&api, method, json!({ "day": "2026-02" })).await;
    assert!(
        refused["message"].as_str().unwrap().contains("YYYY-MM-DD"),
        "{refused}"
    );
}

#[tokio::test]
async fn usage_tells_which_delegated_task_used_what() {
    let dir = temp();
    let api = open(&dir);
    let order = famulen_core::workers::Order {
        code: "task-abc123".into(),
        title: "直す".into(),
        ..Default::default()
    };
    let run = api.app().workers.runs.begin("t1", &order).unwrap();
    let models = &api.app().models;
    models
        .record_run(run.id, &priced("a"), &used(10, 1))
        .unwrap();
    models
        .record(Role::Worker, &priced("a"), &used(5, 1))
        .unwrap();

    let rows = json!([
        { "role": "ROLE_WORKER", "model": "a", "calls": "1", "inputTokens": "10",
          "outputTokens": "1", "yen": 12.0, "metered": true },
    ]);
    let usage = call(&api, "ModelService/GetUsage", json!({})).await;
    assert_eq!(
        usage["runs"],
        json!([
            { "run": run.id.to_string(), "task": "t1", "code": "task-abc123", "title": "直す",
              "rows": rows },
        ])
    );
    let output = call(
        &api,
        "WorkerService/ReadRun",
        json!({ "id": run.id.to_string() }),
    )
    .await;
    assert_eq!(output["usage"], rows);
}
