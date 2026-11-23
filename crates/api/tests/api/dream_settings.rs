//! When the dream runs and how far it tidies memory, as the console sets them.

use serde_json::json;

use crate::support::{open, temp, try_call};

#[tokio::test]
async fn without_a_choice_the_dream_runs_nightly_at_four() {
    let dir = temp();
    let api = open(&dir);

    let (_, s) = try_call(&api, "MemoryService/GetDreamSettings", json!({})).await;

    assert_eq!(s["at"], "04:00");
    assert_eq!(s["frequency"], "DREAM_FREQUENCY_NIGHTLY");
    assert_eq!(s["earlyAt"], 30);
    assert_eq!(s["coreLines"], 150);
    assert_eq!(s["tidyRecords"], true);
    assert!(s["next"].as_str().unwrap().ends_with(" 4:00"), "{s}");
}

#[tokio::test]
async fn settings_saved_are_read_back_and_say_the_next_dream() {
    let dir = temp();
    let api = open(&dir);

    let (status, saved) = try_call(
        &api,
        "MemoryService/SetDreamSettings",
        json!({
            "at": "23:30",
            "frequency": "DREAM_FREQUENCY_WEEKLY",
            "weekday": 3,
            "earlyAt": 0,
            "coreLines": 80
        }),
    )
    .await;
    assert_eq!(status, 200, "{saved}");
    let (_, read) = try_call(&api, "MemoryService/GetDreamSettings", json!({})).await;

    assert_eq!(read, saved);
    assert_eq!(read["at"], "23:30");
    assert_eq!(read["frequency"], "DREAM_FREQUENCY_WEEKLY");
    assert_eq!(read["weekday"], 3);
    assert_eq!(read["earlyAt"].as_i64().unwrap_or(0), 0, "{read}");
    assert_eq!(read["coreLines"], 80);
    assert!(read["next"].as_str().unwrap().ends_with(" 23:30"), "{read}");
    let (_, dreams) = try_call(&api, "MemoryService/ListDreams", json!({})).await;
    assert_eq!(dreams["next"], read["next"]);
}

#[tokio::test]
async fn values_out_of_range_are_refused_and_nothing_changes() {
    let dir = temp();
    let api = open(&dir);

    for bad in [
        json!({ "at": "25:00", "frequency": "DREAM_FREQUENCY_NIGHTLY", "coreLines": 150 }),
        json!({ "at": "4時", "frequency": "DREAM_FREQUENCY_NIGHTLY", "coreLines": 150 }),
        json!({ "at": "04:00", "coreLines": 150 }),
        json!({ "at": "04:00", "frequency": "DREAM_FREQUENCY_WEEKLY", "weekday": 7, "coreLines": 150 }),
        json!({ "at": "04:00", "frequency": "DREAM_FREQUENCY_NIGHTLY", "coreLines": 0 }),
        json!({ "at": "04:00", "frequency": "DREAM_FREQUENCY_NIGHTLY", "earlyAt": -1, "coreLines": 150 }),
    ] {
        let (status, _) = try_call(&api, "MemoryService/SetDreamSettings", bad.clone()).await;
        assert_ne!(status, 200, "{bad}");
    }
    let (_, s) = try_call(&api, "MemoryService/GetDreamSettings", json!({})).await;
    assert_eq!(s["at"], "04:00");
    assert_eq!(s["coreLines"], 150);
}

#[tokio::test]
async fn the_core_layer_counts_its_lines_against_the_limit_the_owner_set() {
    let dir = temp();
    let api = open(&dir);
    try_call(
        &api,
        "MemoryService/SetDreamSettings",
        json!({ "at": "04:00", "frequency": "DREAM_FREQUENCY_NIGHTLY", "coreLines": 80 }),
    )
    .await;

    let (_, core) = try_call(&api, "MemoryService/GetLayer", json!({ "layer": "core" })).await;

    let count = core["layer"]["count"].as_str().unwrap();
    assert!(count.ends_with("/ 80 行"), "{core}");
}

#[tokio::test]
async fn tidying_records_is_turned_off_and_stays_so_when_left_out() {
    let dir = temp();
    let api = open(&dir);
    let mut settings = json!({
        "at": "04:00", "frequency": "DREAM_FREQUENCY_NIGHTLY", "coreLines": 150, "tidyRecords": false
    });
    for _ in 0..2 {
        try_call(&api, "MemoryService/SetDreamSettings", settings.clone()).await;
        let (_, s) = try_call(&api, "MemoryService/GetDreamSettings", json!({})).await;
        assert_eq!(s["tidyRecords"], false, "{s}");
        settings.as_object_mut().unwrap().remove("tidyRecords");
    }
}
