//! The conversation read through the API a page at a time, oldest first, with
//! `before` naming the oldest message already shown.

use famulen_core::console_log::Who;
use serde_json::{Value, json};

use crate::support::{call, open, temp};

fn texts(page: &Value) -> Vec<&str> {
    let messages = page["messages"].as_array();
    messages
        .into_iter()
        .flatten()
        .map(|m| m["text"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn the_conversation_reads_back_to_its_very_first_message() {
    let dir = temp();
    let api = open(&dir);
    for n in 0..1100 {
        api.app()
            .console
            .add(Who::Aide, format!("{n}"), Value::Null)
            .unwrap();
    }
    let mut before = json!("0");
    let mut read: Vec<String> = vec![];
    loop {
        let page = call(
            &api,
            "ConsoleService/ListMessages",
            json!({ "before": before, "limit": 100 }),
        )
        .await;
        let mut older: Vec<String> = texts(&page).into_iter().map(str::to_owned).collect();
        older.append(&mut read);
        read = older;
        if page["more"] != true {
            break;
        }
        before = page["messages"][0]["id"].clone();
    }
    let all: Vec<String> = (0..1100).map(|n| n.to_string()).collect();
    assert_eq!(read, all);
}

#[tokio::test]
async fn a_page_is_filled_with_the_workgroups_own_however_far_back_they_are() {
    let dir = temp();
    let api = open(&dir);
    let add = |text: &str, workgroup: &str| {
        let meta = json!({ "workgroup": workgroup });
        api.app().console.add(Who::Aide, text, meta).unwrap();
    };
    add("a", "work");
    add("b", "work");
    for _ in 0..1100 {
        add("x", "personal");
    }
    let page = call(
        &api,
        "ConsoleService/ListMessages",
        json!({ "workgroup": "work", "limit": 2 }),
    )
    .await;
    assert_eq!(texts(&page), ["a", "b"]);
    assert_ne!(page["more"], true);
}
