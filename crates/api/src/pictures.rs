//! The pictures of the parties.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::Api;

pub(crate) fn router() -> Router<Api> {
    Router::new().route("/party-pictures/{id}", get(show))
}

async fn show(State(api): State<Api>, Path(id): Path<String>) -> Response {
    match api.app.pictures.get(&id) {
        Ok(Some(p)) => picture(&p.mime, p.bytes),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(%error, "could not read a party's picture");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// A picture that came from outside, shown as nothing but a picture.
pub(crate) fn picture(mime: &str, bytes: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "private, max-age=86400"),
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
