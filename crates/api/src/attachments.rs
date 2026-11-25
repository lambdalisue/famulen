//! The files the owner attaches on the console: taken as they are, and shown
//! again in the conversation.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use famulen_core::attachments::MAX_BYTES;
use serde::Deserialize;

use crate::Api;

/// Where the files are, below the console's and the API's address.
pub(crate) const PATH: &str = "/attachments";

pub(crate) fn router() -> Router<Api> {
    Router::new()
        .route(PATH, post(keep))
        // One byte more is let in, so that it is told as too large.
        .layer(DefaultBodyLimit::max(MAX_BYTES + 1))
        .route(&format!("{PATH}/{{id}}"), get(show))
}

#[derive(Deserialize)]
struct Given {
    #[serde(default)]
    name: String,
}

async fn keep(State(api): State<Api>, Query(given): Query<Given>, body: Bytes) -> Response {
    let now = chrono::Utc::now().timestamp();
    match api.app.attachments.keep(&given.name, &body, now) {
        Ok(a) => {
            Json(serde_json::json!({ "id": a.id, "name": a.name, "mime": a.mime, "size": a.size }))
                .into_response()
        }
        Err(error) => (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()).into_response(),
    }
}

async fn show(State(api): State<Api>, Path(id): Path<String>) -> Response {
    let found = match api.app.attachments.get(&id) {
        Ok(Some(found)) => found,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let Ok(bytes) = tokio::fs::read(&found.path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let shown = if found.mime.starts_with("image/") {
        "inline"
    } else {
        "attachment"
    };
    (
        [
            (header::CONTENT_TYPE, found.mime.as_str()),
            (header::CONTENT_DISPOSITION, shown),
            (header::CACHE_CONTROL, "private, max-age=86400, immutable"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; sandbox",
            ),
            (
                header::HeaderName::from_static("cross-origin-resource-policy"),
                "same-origin",
            ),
        ],
        bytes,
    )
        .into_response()
}
