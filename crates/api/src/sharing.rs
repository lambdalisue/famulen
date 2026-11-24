//! The notebook's Git, for its copies on the owner's other machines: the few
//! addresses Git's smart HTTP protocol asks, and nothing of `.git` itself.

use std::io::Read as _;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use famulen_core::notebook;
use famulen_notebook::share::{self, Outcome};
use serde::Deserialize;

use crate::{Api, changed};

/// Where the notebook's Git is, below the console's and the API's address.
pub(crate) const PATH: &str = "/notebook.git";

/// A push's pack, and a little of the protocol around it.
const REQUEST_LIMIT: usize = share::PUSH_LIMIT + 64 * 1024;

/// Who the proxy in front of the console says is there. Only told in the log:
/// any program on this machine can say it too.
const LOGIN: &str = "tailscale-user-login";

pub(crate) fn router() -> Router<Api> {
    Router::new()
        .route(&format!("{PATH}/info/refs"), get(refs))
        .route(&format!("{PATH}/git-upload-pack"), post(fetch))
        .route(&format!("{PATH}/git-receive-pack"), post(push))
}

#[derive(Deserialize)]
struct Asked {
    service: Option<String>,
}

async fn refs(State(api): State<Api>, Query(asked): Query<Asked>) -> Response {
    let service = asked.service.unwrap_or_default();
    let refs = match &*service {
        "git-upload-pack" => share::refs_to_fetch,
        "git-receive-pack" => share::refs_to_push,
        _ => return StatusCode::FORBIDDEN.into_response(),
    };
    let kind = format!("application/x-{service}-advertisement");
    serve(&api, &kind, move |api| refs(&api.app.notebook)).await
}

async fn fetch(State(api): State<Api>, request: Request) -> Response {
    let body = match read(request, "git-upload-pack").await {
        Ok(body) => body,
        Err(refused) => return refused.into_response(),
    };
    serve(&api, "application/x-git-upload-pack-result", move |api| {
        share::fetch(&api.app.notebook, &body)
    })
    .await
}

async fn push(State(api): State<Api>, request: Request) -> Response {
    let login = request
        .headers()
        .get(LOGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = match read(request, "git-receive-pack").await {
        Ok(body) => body,
        Err(refused) => return refused.into_response(),
    };
    serve(&api, "application/x-git-receive-pack-result", move |api| {
        let pushed = share::push(&api.app.notebook, &body, || {
            notebook::is_shared(&api.app.store)
        })?;
        let by = login.map(|l| format!(" by {l}")).unwrap_or_default();
        match pushed.outcome {
            Outcome::Nothing => {}
            Outcome::Refused(why) => api
                .app
                .journal
                .record("notebook", format!("a push{by} was refused: {why}")),
            Outcome::Taken(taken) => {
                let commit: String = taken.commit.chars().take(8).collect();
                let detail = format!("pushed{by}: {commit} {}", taken.files.join(", "));
                api.app.journal.record("notebook", detail);
                changed(api);
            }
        }
        Ok(pushed.answer)
    })
    .await
}

/// A request's body, when it is one Git sends. The content type is one a page
/// of another site cannot send without being asked about first.
async fn read(request: Request, service: &str) -> Result<Vec<u8>, StatusCode> {
    let is = |headers: &HeaderMap, name, value: &str| {
        headers
            .get(name)
            .is_some_and(|v| v.as_bytes() == value.as_bytes())
    };
    let kind = format!("application/x-{service}-request");
    if !is(request.headers(), header::CONTENT_TYPE, &kind) {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
    let zipped = is(request.headers(), header::CONTENT_ENCODING, "gzip");
    let too_large = StatusCode::PAYLOAD_TOO_LARGE;
    let body = to_bytes(request.into_body(), REQUEST_LIMIT)
        .await
        .map_err(|_| too_large)?;
    if !zipped {
        return Ok(body.into());
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(&body[..])
        .take(REQUEST_LIMIT as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if out.len() > REQUEST_LIMIT {
        return Err(too_large);
    }
    Ok(out)
}

/// What `answer` gives, unless the owner does not share the notebook.
async fn serve(
    api: &Api,
    kind: &str,
    answer: impl FnOnce(&Api) -> anyhow::Result<Vec<u8>> + Send + 'static,
) -> Response {
    let api = api.clone();
    let answered = tokio::task::spawn_blocking(move || {
        if !notebook::is_shared(&api.app.store)? {
            return Ok(None);
        }
        answer(&api).map(Some)
    })
    .await
    .unwrap_or_else(|error| Err(error.into()));
    match answered {
        Ok(Some(body)) => {
            let mut response = Response::new(Body::from(body));
            let headers = response.headers_mut();
            if let Ok(kind) = HeaderValue::from_str(kind) {
                headers.insert(header::CONTENT_TYPE, kind);
            }
            headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(
                error = format!("{error:#}"),
                "could not answer for the notebook's Git"
            );
            StatusCode::BAD_REQUEST.into_response()
        }
    }
}
