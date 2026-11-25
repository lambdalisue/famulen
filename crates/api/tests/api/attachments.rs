//! Files the owner attaches on the console.

use axum::body::Body;
use famulen_api::Api;
use famulen_core::attachments::{Attached, MAX_BYTES};
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::support::{call, open, temp, try_call};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

async fn answer(api: &Api, request: Request<Body>) -> (StatusCode, http::HeaderMap, Vec<u8>) {
    let response = api.clone().axum().oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, MAX_BYTES + 1).await.unwrap();
    (parts.status, parts.headers, bytes.to_vec())
}

async fn upload(api: &Api, name: &str, bytes: &[u8]) -> (StatusCode, Vec<u8>) {
    let query: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    let request = Request::post(format!("/attachments?name={query}"))
        .body(Body::from(bytes.to_vec()))
        .unwrap();
    let (status, _, body) = answer(api, request).await;
    (status, body)
}

async fn kept(api: &Api, name: &str, bytes: &[u8]) -> String {
    let (status, body) = upload(api, name, bytes).await;
    assert_eq!(status, StatusCode::OK);
    let file: Value = serde_json::from_slice(&body).unwrap();
    file["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn an_attached_file_reaches_the_secretary_and_the_conversation() {
    let dir = temp();
    let api = open(&dir);
    let id = kept(&api, "画面 1.png", PNG).await;

    let said = call(
        &api,
        "ConsoleService/Send",
        json!({ "text": "これは何？", "workgroup": "all", "attachments": [id] }),
    )
    .await;

    assert_eq!(said["attachments"][0]["name"], "画面 1.png");
    assert_eq!(said["attachments"][0]["mime"], "image/png");
    let listed = call(&api, "ConsoleService/ListMessages", json!({})).await;
    assert_eq!(listed["messages"][0]["attachments"][0]["id"], id);
    let input = api.app().store.next_pending().unwrap().unwrap().input;
    let [Attached::Kept(file)] = input.attached() else {
        panic!("{:?}", input.attached())
    };
    assert!(
        file.path
            .parent()
            .unwrap()
            .ends_with("workspace/attachments")
    );
    assert_eq!(std::fs::read(&file.path).unwrap(), PNG);
    assert!(input.render().contains(&file.path.display().to_string()));
}

#[tokio::test]
async fn a_file_alone_is_a_message_and_one_never_kept_is_refused() {
    let dir = temp();
    let api = open(&dir);
    let id = kept(&api, "a.png", PNG).await;
    let send = async |ids: Vec<String>| {
        let body = json!({ "text": "", "workgroup": "all", "attachments": ids });
        try_call(&api, "ConsoleService/Send", body).await.0
    };

    assert_eq!(send(vec![id.clone()]).await, StatusCode::OK);
    for refused in [vec![], vec!["../famulen.sqlite3".into()], vec![id; 11]] {
        assert_eq!(send(refused).await, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn a_file_that_cannot_be_taken_says_why() {
    let dir = temp();
    let api = open(&dir);
    for refused in [vec![], b"PK\x03\x04\0".to_vec(), vec![b'a'; MAX_BYTES + 1]] {
        let (status, why) = upload(&api, "x", &refused).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!why.is_empty());
    }
    assert_eq!(
        upload(&api, "x", &vec![b'a'; MAX_BYTES + 2]).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn a_file_is_shown_without_running_anything_in_it() {
    let dir = temp();
    let api = open(&dir);
    let image = kept(&api, "a.png", PNG).await;
    let page = kept(&api, "a.html", b"<script>alert(1)</script>").await;
    let get = async |id: &str| {
        let request = Request::get(format!("/attachments/{id}"));
        answer(&api, request.body(Body::empty()).unwrap()).await
    };

    let (status, headers, bytes) = get(&image).await;
    assert_eq!((status, &bytes[..]), (StatusCode::OK, PNG));
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    let (_, headers, _) = get(&page).await;
    assert_eq!(headers[header::CONTENT_TYPE], "text/plain");
    assert_eq!(headers[header::CONTENT_DISPOSITION], "attachment");
    assert_eq!(get("nope").await.0, StatusCode::NOT_FOUND);
}
