//! The pictures of the parties.

use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

use crate::support::{open, temp};

#[tokio::test]
async fn a_picture_that_is_not_kept_is_not_served() {
    let dir = temp();
    let request = Request::get("/party-pictures/0000")
        .body(Body::empty())
        .unwrap();
    let response = open(&dir).axum().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
