//! The pages that links in the conversation lead to.

use axum::body::Body;
use http::{Request, StatusCode};
use serde_json::json;
use tower::ServiceExt;

use crate::support::{call, open, temp};

#[tokio::test]
async fn links_to_this_machine_and_messages_that_are_not_there_show_nothing() {
    let dir = temp();
    let api = open(&dir);
    let said = call(
        &api,
        "ConsoleService/PostMessage",
        json!({ "text": "http://127.0.0.1/ と http://localhost/ と http://169.254.169.254/" }),
    )
    .await;
    for message in [said["id"].clone(), json!("99")] {
        let shown = call(
            &api,
            "ConsoleService/ListLinkPreviews",
            json!({ "message": message }),
        )
        .await;
        assert!(shown.get("previews").is_none(), "{shown}");
    }
}

#[tokio::test]
async fn a_picture_no_page_named_is_not_served() {
    let dir = temp();
    let request = Request::get("/link-images/0000")
        .body(Body::empty())
        .unwrap();
    let response = open(&dir).axum().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
