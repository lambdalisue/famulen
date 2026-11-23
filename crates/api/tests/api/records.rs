//! Links between records, and from memory, as the console and the secretary
//! write and read them.

use serde_json::{Value, json};

use crate::support::{call, call_as, open, temp, try_call};

async fn save(api: &famulen_api::Api, body: Value) -> Value {
    let (status, saved) = try_call(api, "RecordService/SaveRecord", body).await;
    assert_eq!(status, 200, "{saved}");
    saved
}

fn link_to(record: &Value) -> String {
    let [kind, id, title] = ["kind", "id", "title"].map(|k| record[k].as_str().unwrap());
    format!("[[records/{kind}/{id}|{title}]]")
}

#[tokio::test]
async fn a_memo_links_a_party_and_the_party_shows_it_by_field_and_by_prose() {
    let dir = temp();
    let api = open(&dir);
    let party = save(&api, json!({ "kind": "parties", "title": "田中さん" })).await;

    let memo = save(
        &api,
        json!({
            "kind": "memos", "title": "手土産",
            "notes": format!("[[{}]] は和菓子", party["code"].as_str().unwrap()),
            "fields": [{ "name": "Related", "values": ["田中さん"] }],
        }),
    )
    .await;

    assert_eq!(memo["notes"], format!("{} は和菓子", link_to(&party)));
    assert_eq!(memo["fields"][0]["values"], json!([link_to(&party)]));
    assert_eq!(memo["links"][0]["title"], "田中さん");
    let got = call(
        &api,
        "RecordService/GetRecord",
        json!({ "kind": "parties", "id": party["id"] }),
    )
    .await;
    let from: Vec<&str> = got["backlinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["from"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(from, ["Related", ""]);
}

#[tokio::test]
async fn a_renamed_record_is_shown_by_its_new_title_and_a_removed_one_as_missing() {
    let dir = temp();
    let api = open(&dir);
    let party = save(&api, json!({ "kind": "parties", "title": "田中" })).await;
    let memo = save(
        &api,
        json!({ "kind": "memos", "title": "x", "notes": link_to(&party) }),
    )
    .await;
    let of = |r: &Value| json!({ "kind": r["kind"], "id": r["id"] });

    save(
        &api,
        json!({ "kind": "parties", "id": party["id"], "title": "田中太郎" }),
    )
    .await;
    let renamed = call(&api, "RecordService/GetRecord", of(&memo)).await;
    assert_eq!(renamed["links"][0]["title"], "田中太郎");
    assert!(renamed["notes"].as_str().unwrap().contains("|田中太郎]]"));

    call(&api, "RecordService/DeleteRecord", of(&party)).await;
    let left = call(&api, "RecordService/GetRecord", of(&memo)).await;
    assert_eq!(left["links"][0]["missing"], true);
    let found = call(&api, "NotebookService/CheckNotebook", json!({})).await;
    assert_eq!(found["findings"][0]["rule"], "broken-link");
}

#[tokio::test]
async fn a_name_that_fits_several_records_is_answered_with_them() {
    let dir = temp();
    let api = open(&dir);
    let a = save(&api, json!({ "kind": "memos", "title": "同じ" })).await;
    let b = save(&api, json!({ "kind": "tasks", "title": "同じ" })).await;

    let error = call_as(
        &api,
        "secretary",
        "RecordService/SaveRecord",
        json!({ "kind": "memos", "title": "x", "notes": "[[同じ]]" }),
    )
    .await;

    let why = error["message"].as_str().unwrap();
    for r in [a, b] {
        assert!(why.contains(r["code"].as_str().unwrap()), "{why}");
    }
}

#[tokio::test]
async fn a_memory_that_names_a_record_links_it_and_shows_on_it() {
    let dir = temp();
    let api = open(&dir);
    let task = save(&api, json!({ "kind": "tasks", "title": "見積もり" })).await;

    let item = call_as(
        &api,
        "secretary",
        "MemoryService/Remember",
        json!({ "text": "[[見積もり]] は来週まで待つ" }),
    )
    .await;

    assert_eq!(item["text"], format!("{} は来週まで待つ", link_to(&task)));
    let got = call(
        &api,
        "RecordService/GetRecord",
        json!({ "kind": "tasks", "id": task["id"] }),
    )
    .await;
    assert_eq!(got["memories"][0]["id"], item["id"]);
    assert!(
        got["memories"][0]["file"]
            .as_str()
            .unwrap()
            .starts_with("episodic/")
    );
}

#[tokio::test]
async fn a_working_memory_goes_on_though_a_link_in_it_came_to_fit_several() {
    let dir = temp();
    let api = open(&dir);
    let set = |next: &'static str| {
        call_as(
            &api,
            "secretary",
            "MemoryService/SetWorking",
            json!({ "name": "trip", "done": "[[同じ]] を見た", "next": next }),
        )
    };
    set("待つ").await;
    for kind in ["tasks", "memos"] {
        save(&api, json!({ "kind": kind, "title": "同じ" })).await;
    }

    let working = set("尋ねる").await;

    assert_eq!(working["done"], "[[同じ]] を見た");
    assert_eq!(working["next"], "尋ねる");
}

#[tokio::test]
async fn a_record_that_is_not_there_is_answered_as_not_found() {
    let dir = temp();
    let api = open(&dir);
    let memo = save(&api, json!({ "kind": "memos", "title": "x" })).await;
    let get = |id: &Value| json!({ "kind": "memos", "id": id });

    let (found, _) = try_call(&api, "RecordService/GetRecord", get(&memo["id"])).await;
    let (status, gone) = try_call(
        &api,
        "RecordService/GetRecord",
        get(&json!("01ARZ3NDEKTSV4RRFFQ69G5FAV")),
    )
    .await;

    assert_eq!(found, 200);
    assert_eq!(status, 404, "{gone}");
    assert_eq!(gone["code"], "not_found");
}
