//! Records that may be out of date, as the console and the secretary see them.

use famulen_core::notebook::Writer;
use famulen_core::records::{MEMOS, Patch, TASKS};
use serde_json::json;

use crate::support::{call, open, temp};

#[tokio::test]
async fn a_marked_record_says_so_until_the_mark_is_taken_off() {
    let dir = temp();
    let api = open(&dir);
    let records = &api.app().records;
    let memo = records
        .add(Writer::Owner, MEMOS, Patch::titled("営業時間"))
        .unwrap();
    records
        .doubt(MEMOS, &memo.id, "三年前のもの", "2026-11-24")
        .unwrap();

    let listed = call(&api, "RecordService/ListRecords", json!({ "kind": MEMOS })).await;
    assert_eq!(listed["records"][0]["obsolete"], "2026-11-24");
    assert_eq!(listed["records"][0]["obsoleteReason"], "三年前のもの");
    let asked = json!({ "kind": TASKS, "openOnly": true });
    let tasks = call(&api, "RecordService/ListRecords", asked.clone()).await;
    assert_eq!(tasks["records"].as_array().unwrap().len(), 1);

    let id = json!({ "kind": MEMOS, "id": memo.id });
    let cleared = call(&api, "RecordService/ClearObsolete", id).await;
    assert!(cleared.get("obsolete").is_none_or(|o| o == ""), "{cleared}");
    let app = api.app();
    let later = chrono::Utc::now().date_naive() + chrono::Days::new(300);
    let tidied =
        famulen_core::tidying::tidy(&app.store, records, &app.memory_dir, later, |_, _| async {
            Ok(json!({ "records": [] }))
        })
        .await
        .unwrap();
    assert_eq!(tidied, None, "what was checked is not asked about again");
    let tasks = call(&api, "RecordService/ListRecords", asked).await;
    assert!(tasks["records"].as_array().is_none_or(|t| t.is_empty()));
}

#[tokio::test]
async fn a_memo_made_into_a_task_names_it_while_the_task_is_there() {
    let dir = temp();
    let api = open(&dir);
    let records = &api.app().records;
    let add = |kind, title| {
        records
            .add(Writer::Owner, kind, Patch::titled(title))
            .unwrap()
    };
    let (memo, task) = (add(MEMOS, "電話"), add(TASKS, "電話する"));
    records.moved_to_task(MEMOS, &memo.id, &task).unwrap();
    let get = json!({ "kind": MEMOS, "id": memo.id });

    let shown = call(&api, "RecordService/GetRecord", get.clone()).await;
    assert_eq!(shown["movedTo"]["id"], task.id.as_str());
    records.remove(Writer::Owner, TASKS, &task.id).unwrap();
    let shown = call(&api, "RecordService/GetRecord", get).await;
    assert!(shown.get("movedTo").is_none_or(|m| m.is_null()), "{shown}");
}
