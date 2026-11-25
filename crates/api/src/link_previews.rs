//! The pictures of the pages that links in the conversation lead to.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use famulen_core::link_previews::Preview;

use crate::Api;
use crate::pb::LinkPreview;
use crate::pictures::picture;

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
    picture(&image.media_type(), image.body)
}
