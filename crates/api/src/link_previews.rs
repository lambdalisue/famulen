//! The pictures of the pages that links in the conversation lead to.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use famulen_core::link_previews::Preview;

use crate::Api;
use crate::pb::LinkPreview;

pub(crate) fn router() -> Router<Api> {
    Router::new().route("/link-images/{id}", get(show))
}

pub(crate) fn link_preview(preview: Preview) -> LinkPreview {
    LinkPreview {
        url: preview.url,
        site: preview.site,
        title: preview.title,
        description: preview.description,
        image: preview.image.unwrap_or_default(),
        ..Default::default()
    }
}

async fn show(State(api): State<Api>, Path(id): Path<String>) -> Response {
    let Some(image) = api.app.link_previews.image(&id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (header::CONTENT_TYPE, image.media_type().as_str()),
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
        image.body,
    )
        .into_response()
}
