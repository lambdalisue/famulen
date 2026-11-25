//! The conversation read through the API a page at a time, oldest first, with
//! `before` naming the oldest message already shown.

use famulen_core::console_log::Who;
use famulen_core::input::Source;
use http::StatusCode;
use serde_json::{Value, json};

use crate::support::{call, call_as, open, temp, try_call};

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

/// The turn of the oldest input that waits, in which the secretary `posts`.
async fn turn(api: &famulen_api::Api, posts: &str) -> Value {
    let app = api.app();
    let row = app.store.next_pending().unwrap().unwrap();
    let c = app.conversations.settle(&row, |_| None).unwrap();
    app.store.start_turn(row.id, "all").unwrap();
    app.conversations.began(c.id, &row.input, "all").unwrap();
    let posted = call(api, "ConsoleService/PostMessage", json!({ "text": posts })).await;
    app.conversations.ended(c.id, &row).unwrap();
    app.store.end_turn().unwrap();
    posted
}

#[tokio::test]
async fn what_comes_back_later_is_said_in_the_thread_it_was_asked_in() {
    let dir = temp();
    let api = open(&dir);
    let send = json!({ "text": "あとで知らせて", "workgroup": "all" });
    let asked = call(&api, "ConsoleService/Send", send).await;
    let row = api.app().store.next_pending().unwrap().unwrap();
    api.app().store.start_turn(row.id, "all").unwrap();
    let wake = call_as(
        &api,
        "secretary",
        "RoutineService/WakeAt",
        json!({ "note": "知らせる", "inMinutes": 1 }),
    )
    .await;
    let task = call_as(
        &api,
        "secretary",
        "WorkerService/DelegateTask",
        json!({ "title": "調べる", "runner": "agent", "request": "調べて" }),
    )
    .await;
    api.app().store.end_turn().unwrap();

    let conversations = &api.app().conversations;
    let of: i64 = asked["conversation"].as_str().unwrap().parse().unwrap();
    let key = famulen_core::conversations::task_key(task["id"].as_str().unwrap());
    assert_eq!(conversations.of(&key).unwrap(), Some(of), "{task}");

    let due = famulen_core::input::Input {
        sender: famulen_core::input::Sender::System,
        source: Source::Wake {
            id: wake["id"].as_str().unwrap().parse().unwrap(),
            at: "now".into(),
        },
        text: "知らせる".into(),
    };
    api.app().store.enqueue(&due).unwrap();
    let told = turn(&api, "時間です").await;
    assert_eq!(told["replyTo"]["id"], asked["id"]);
    assert_eq!(told["conversation"], asked["conversation"]);

    let body = json!({ "conversation": asked["conversation"] });
    let whole = call(&api, "ConsoleService/ListMessages", body).await;
    assert_eq!(texts(&whole), ["あとで知らせて", "時間です"]);
}

#[tokio::test]
async fn the_owner_goes_on_begins_anew_or_chooses_the_conversation() {
    let dir = temp();
    let api = open(&dir);
    let send = async |text: &str, more: Value| {
        let mut body = json!({ "text": text, "workgroup": "work" });
        body.as_object_mut()
            .unwrap()
            .extend(more.as_object().unwrap().clone());
        call(&api, "ConsoleService/Send", body).await["conversation"].clone()
    };
    let first = send("一つ目の話\n詳しく", json!({})).await;
    assert_eq!(send("続き", json!({})).await, first);
    let second = send("二つ目の話", json!({ "newConversation": true })).await;
    assert_ne!(second, first);
    assert_eq!(
        send("一つ目の続き", json!({ "conversation": first })).await,
        first
    );

    // A thread may run through several; what the owner chose holds.
    let reply = call(
        &api,
        "ConsoleService/Send",
        json!({ "text": "二つ目への返信", "workgroup": "work", "replyTo": "3", "conversation": first }),
    )
    .await;
    assert_eq!(reply["conversation"], first);

    let listed = call(&api, "ConsoleService/ListConversations", json!({})).await;
    let titles: Vec<_> = listed["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["title"].as_str().unwrap(),
                c["last"]["text"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        titles,
        [
            ("二つ目の話", "二つ目の話"),
            ("一つ目の話", "二つ目への返信")
        ]
    );
    assert_eq!(listed["conversations"][0]["waiting"], true);
    let also = json!({ "limit": 1, "also": first });
    let listed = call(&api, "ConsoleService/ListConversations", also).await;
    assert_eq!(listed["conversations"].as_array().unwrap().len(), 2);
    let elsewhere = json!({ "workgroup": "personal", "also": first });
    let listed = call(&api, "ConsoleService/ListConversations", elsewhere).await;
    assert_eq!(listed["conversations"], Value::Null);

    let body = json!({ "text": "a", "workgroup": "work", "conversation": "99" });
    let (status, _) = try_call(&api, "ConsoleService/Send", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_conversation_tells_how_much_of_it_is_unread_until_it_was_shown() {
    let dir = temp();
    let api = open(&dir);
    let body = json!({ "text": "頼み", "workgroup": "work" });
    let sent = call(&api, "ConsoleService/Send", body).await;
    let answer = call(
        &api,
        "ConsoleService/PostMessage",
        json!({ "text": "答え", "replyTo": sent["id"] }),
    )
    .await;
    let id = &sent["conversation"];
    assert_eq!(&answer["conversation"], id);
    let unread = async || {
        let listed = call(&api, "ConsoleService/ListConversations", json!({})).await;
        listed["conversations"][0]["unread"].clone()
    };
    assert_eq!(unread().await, 1);
    let shown = json!({ "conversation": id, "message": answer["id"] });
    call(&api, "ConsoleService/ReadConversation", shown).await;
    assert_eq!(unread().await, Value::Null);

    let (status, _) = try_call(
        &api,
        "ConsoleService/ReadConversation",
        json!({ "conversation": "99", "message": "1" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn what_conversations_used_in_a_period_is_told_with_the_total() {
    let dir = temp();
    let api = open(&dir);
    let send = json!({ "text": "話", "workgroup": "work" });
    let said = call(&api, "ConsoleService/Send", send).await;
    let id: i64 = said["conversation"].as_str().unwrap().parse().unwrap();
    let usage = famulen_core::models::Usage {
        input_tokens: Some(100),
        output_tokens: Some(5),
        calls: 1,
    };
    let cached = famulen_core::conversations::Cached {
        read: 80,
        written: 10,
    };
    for _ in 0..2 {
        api.app().conversations.spent(id, &usage, cached).unwrap();
    }
    let used = call(&api, "ModelService/GetConversationUsage", json!({})).await;
    let expected = json!({
        "turns": "2", "inputTokens": "200", "cachedTokens": "160",
        "cacheWriteTokens": "20", "outputTokens": "10",
    });
    assert_eq!(used["total"], expected);
    assert_eq!(used["conversations"][0]["title"], "話");
    assert_eq!(used["conversations"][0]["spent"], expected);
}
