//! The conversation read through the API a page at a time, oldest first, with
//! `before` naming the oldest message already shown.

use famulen_core::console_log::Who;
use famulen_core::input::Source;
use http::StatusCode;
use serde_json::{Value, json};

use crate::support::{call, open, temp, try_call};

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

#[tokio::test]
async fn a_reply_is_kept_with_what_it_replies_to_and_hands_it_to_the_secretary() {
    let dir = temp();
    let api = open(&dir);
    let console = &api.app().console;
    let root = console.add(Who::Aide, "一つ目\n二つ目", json!({})).unwrap();

    let said = call(
        &api,
        "ConsoleService/Send",
        json!({ "text": "二つ目は？", "workgroup": "all", "replyTo": root.id.to_string() }),
    )
    .await;

    assert_eq!(said["replyTo"]["id"], root.id.to_string());
    assert_eq!(said["replyTo"]["text"], "一つ目\n二つ目");
    assert_eq!(said["thread"], root.id.to_string());
    let listed = call(&api, "ConsoleService/ListMessages", json!({})).await;
    assert_eq!(listed["messages"][1], said);
    let input = api.app().store.next_pending().unwrap().unwrap().input;
    assert!(input.render().contains("> 一つ目\n> 二つ目\n"));
}

#[tokio::test]
async fn a_reply_to_a_message_that_is_not_there_is_refused() {
    let dir = temp();
    let api = open(&dir);
    for (method, body) in [
        (
            "Send",
            json!({ "text": "a", "workgroup": "all", "replyTo": "9" }),
        ),
        ("PostMessage", json!({ "text": "a", "replyTo": "9" })),
    ] {
        let (status, _) = try_call(&api, &format!("ConsoleService/{method}"), body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    assert!(api.app().console.recent(1).unwrap().is_empty());
    assert!(api.app().store.next_pending().unwrap().is_none());
}

#[tokio::test]
async fn the_secretary_answers_a_reply_in_its_thread_and_anything_else_outside() {
    let dir = temp();
    let api = open(&dir);
    let app = api.app();
    let root = app.console.add(Who::Aide, "報告", json!({})).unwrap();
    let post = async |text: &str, to: i64| {
        let body = json!({ "text": text, "replyTo": to.to_string() });
        call(&api, "ConsoleService/PostMessage", body).await
    };
    let send = async |text: &str, to: i64| {
        let body = json!({ "text": text, "workgroup": "all", "replyTo": to.to_string() });
        call(&api, "ConsoleService/Send", body).await
    };

    let reply = send("どこ？", root.id).await;
    let turn = app.store.next_pending().unwrap().unwrap();
    app.store.start_turn(turn.id, "all").unwrap();
    let answer = post("ここです", 0).await;
    app.store.end_turn().unwrap();
    send("別の話", 0).await;
    let turn = app.store.next_pending().unwrap().unwrap();
    assert!(matches!(
        turn.input.source,
        Source::Console { reply_to: None, .. }
    ));
    app.store.start_turn(turn.id, "all").unwrap();
    let outside = post("はい", 0).await;
    let later = post("済みました", root.id).await;

    assert_eq!(answer["replyTo"]["id"], reply["id"]);
    assert_eq!(answer["thread"], root.id.to_string());
    assert_eq!(outside["thread"], Value::Null);
    assert_eq!(later["thread"], root.id.to_string());
    for of in [&root.id.to_string(), answer["id"].as_str().unwrap()] {
        let thread = call(&api, "ConsoleService/ListMessages", json!({ "thread": of })).await;
        assert_eq!(texts(&thread), ["報告", "どこ？", "ここです", "済みました"]);
    }
    let whole = call(&api, "ConsoleService/ListMessages", json!({})).await;
    assert_eq!(
        texts(&whole).len(),
        6,
        "a thread is in the conversation too"
    );
}

#[tokio::test]
async fn nothing_said_in_another_workgroup_is_told_by_its_thread() {
    let dir = temp();
    let api = open(&dir);
    let console = &api.app().console;
    let root = console
        .add(Who::Aide, "仕事の話", json!({ "workgroup": "work" }))
        .unwrap();
    console
        .reply(Who::Owner, "返信", json!({ "workgroup": "all" }), &root)
        .unwrap();
    console
        .reply(
            Who::Owner,
            "私事",
            json!({ "workgroup": "personal" }),
            &root,
        )
        .unwrap();
    let list = async |body: Value| call(&api, "ConsoleService/ListMessages", body).await;

    let personal = list(json!({ "workgroup": "personal" })).await;
    let thread = list(json!({ "workgroup": "work", "thread": root.id.to_string() })).await;

    assert_eq!(texts(&personal), ["返信", "私事"]);
    assert_eq!(personal["messages"][0]["replyTo"], Value::Null);
    assert_eq!(texts(&thread), ["仕事の話", "返信"]);
    let body = json!({ "text": "a", "workgroup": "personal", "replyTo": root.id.to_string() });
    let none = list(json!({ "workgroup": "personal", "thread": "99" })).await;
    assert!(texts(&none).is_empty());
    let (status, _) = try_call(&api, "ConsoleService/Send", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(api.app().store.next_pending().unwrap().is_none());
}
