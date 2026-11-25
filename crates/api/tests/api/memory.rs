//! What a memory stands on, and what the owner decides of pending judgments.

use serde_json::{Value, json};

use crate::support::{DataDir, call, call_as, open, temp, try_call};
use famulen_api::Api;

async fn write(api: &Api, path: &str, text: &str) {
    let body = json!({ "path": path, "text": text });
    let (status, said) = try_call(api, "MemoryService/WriteFile", body).await;
    assert_eq!(status, 200, "{said}");
}

async fn detail(api: &Api, id: &str) -> Value {
    call(api, "MemoryService/GetMemory", json!({ "id": id })).await
}

async fn pending() -> (DataDir, Api) {
    let dir = temp();
    let api = open(&dir);
    let text = "- 朝が好き {id: p1, src: inferred, ev: 1, need: 2}\n- 猫派 {id: p2, src: inferred, ev: 1}\n";
    write(&api, "pending.md", text).await;
    (dir, api)
}

async fn layer(api: &Api, layer: &str) -> Vec<Value> {
    let at = call(api, "MemoryService/GetLayer", json!({ "layer": layer })).await;
    at["items"].as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn a_long_term_memory_tells_its_prose_records_relations_and_history() {
    let dir = temp();
    let api = open(&dir);
    let party = call(
        &api,
        "RecordService/SaveRecord",
        json!({ "kind": "parties", "title": "田中さん" }),
    )
    .await;
    let link = format!(
        "[[records/parties/{}|田中さん]]",
        party["id"].as_str().unwrap()
    );
    write(&api, "core.md", "- 昔の好み {id: c1, src: said}\n").await;
    let path = "long/semantic/tea.md";
    for body in ["紅茶", "緑茶"] {
        let text = format!(
            "---\nid: l1\nsummary: お茶が好き\nsrc: said\nsupersedes: c1\n---\n\n{link} は{body}が好き。\n"
        );
        write(&api, path, &text).await;
    }

    let shown = detail(&api, "l1").await;

    assert_eq!(shown["layer"], "long");
    assert_eq!(shown["group"], "semantic");
    assert_eq!(shown["item"]["text"], "お茶が好き");
    assert_eq!(shown["item"]["file"], path);
    assert!(shown["prose"].as_str().unwrap().ends_with("は緑茶が好き。"));
    assert_eq!(shown["links"][0]["title"], "田中さん");
    assert_eq!(shown["related"][0]["how"], "supersedes");
    assert_eq!(shown["related"][0]["item"]["text"], "昔の好み");
    let history = shown["history"].as_array().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["author"], "Owner");
    assert_eq!(detail(&api, "c1").await["related"][0]["how"], "superseded");
    let by_path = call(&api, "MemoryService/GetMemory", json!({ "path": path })).await;
    assert_eq!(by_path["item"]["id"], "l1");
}

#[tokio::test]
async fn a_memory_without_an_id_is_told_by_its_file_and_what_is_not_there_is_refused() {
    let dir = temp();
    let api = open(&dir);
    write(&api, "long/semantic/x.md", "---\nsummary: 名無し\n---\n").await;
    let path = json!({ "path": "long/semantic/x.md" });
    let shown = call(&api, "MemoryService/GetMemory", path).await;
    assert_eq!(shown["item"]["text"], "名無し");
    for gone in [
        json!({ "id": "m0" }),
        json!({ "path": "long/semantic/y.md" }),
        json!({ "path": "../x.md" }),
        json!({ "path": "core.md" }),
    ] {
        let (status, _) = try_call(&api, "MemoryService/GetMemory", gone.clone()).await;
        assert_ne!(status, 200, "{gone}");
    }
}

#[tokio::test]
async fn an_answer_waits_for_the_dream_as_the_owners_commit_until_it_is_taken_back() {
    let (_dir, api) = pending().await;

    let body = json!({ "id": "p1", "adopt": true, "words": "休みの日だけ" });
    let said = call(&api, "MemoryService/AnswerPending", body).await;

    assert_eq!(said["answer"]["adopted"], true);
    assert_eq!(said["answer"]["words"], "休みの日だけ");
    let notes = layer(&api, "episodic").await;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["src"], "said");
    let items = layer(&api, "pending").await;
    assert_eq!(items.len(), 2, "the judgment itself waits");
    assert_eq!(items[0]["answer"]["adopted"], true);
    assert!(items[1].get("answer").is_none());
    let file = json!({ "path": notes[0]["file"] });
    let file = call(&api, "MemoryService/ReadFile", file).await;
    assert_eq!(file["history"][0]["author"], "Owner");

    call(&api, "MemoryService/WithdrawAnswer", json!({ "id": "p1" })).await;
    assert!(layer(&api, "episodic").await.is_empty());
    let (status, _) = try_call(&api, "MemoryService/WithdrawAnswer", json!({ "id": "p1" })).await;
    assert_ne!(status, 200);
}

#[tokio::test]
async fn an_answer_a_running_dream_may_have_read_is_not_taken_back() {
    let (_dir, api) = pending().await;
    let body = json!({ "id": "p1", "adopt": true });
    call(&api, "MemoryService/AnswerPending", body.clone()).await;
    let store = &api.app().store;
    store
        .set_state(famulen_core::runtime::DREAMING_KEY, "1")
        .unwrap();

    for (method, body) in [
        ("WithdrawAnswer", json!({ "id": "p1" })),
        ("AnswerPending", body),
        ("RejectPending", json!({ "id": "p1" })),
    ] {
        let (status, _) = try_call(&api, &format!("MemoryService/{method}"), body).await;
        assert_ne!(status, 200, "{method}");
    }
    assert_eq!(layer(&api, "episodic").await.len(), 1);
    call(&api, "MemoryService/RejectPending", json!({ "id": "p2" })).await;
    assert_eq!(
        layer(&api, "pending").await.len(),
        1,
        "no answer, nothing read"
    );
}

#[tokio::test]
async fn a_rejected_judgment_is_in_forgotten_memory_at_once_and_comes_back() {
    let (_dir, api) = pending().await;
    let body = json!({ "id": "p1", "reason": "夜型" });

    call(&api, "MemoryService/RejectPending", body).await;

    assert_eq!(layer(&api, "pending").await.len(), 1);
    let shown = detail(&api, "p1").await;
    assert_eq!(shown["layer"], "forgotten");
    assert_eq!(shown["item"]["attrs"]["reason"], "夜型");
    assert_eq!(
        shown["history"][0]["message"],
        "あなたが保留中の判断を却下した"
    );

    call(&api, "MemoryService/RestoreMemory", json!({ "id": "p1" })).await;
    assert_eq!(layer(&api, "pending").await.len(), 2);
    let back = detail(&api, "p1").await;
    assert_eq!(back["layer"], "pending");
    assert!(back["item"]["attrs"].get("judged").is_none());
}

#[tokio::test]
async fn only_the_owner_decides_on_a_pending_judgment() {
    let (_dir, api) = pending().await;
    for caller in ["secretary", "lookout", "worker"] {
        for method in ["AnswerPending", "RejectPending"] {
            let method = format!("MemoryService/{method}");
            let body = json!({ "id": "p1", "adopt": true });
            let said = call_as(&api, caller, &method, body.clone()).await;
            assert!(said.get("code").is_some(), "{caller} {method}: {said}");
        }
    }
    assert_eq!(layer(&api, "pending").await.len(), 2);
    assert!(layer(&api, "episodic").await.is_empty());
}
