//! A Famulen of its own for a test, and calls to its API.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use famulen_api::{Api, CALLER_HEADER};
use famulen_core::config::{Config, Defaults};
use famulen_core::gateway::{KEY_VAR, VaultKey};
use http::StatusCode;
use serde_json::Value;

/// A data directory removed with the test.
pub struct DataDir(std::path::PathBuf);

impl std::ops::Deref for DataDir {
    type Target = std::path::Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for DataDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

pub fn temp() -> DataDir {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("famulen-api-{}-{n}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    DataDir(dir)
}

/// The API over `dir`, with a vault.
pub fn open(dir: &DataDir) -> Api {
    open_with(dir, Some(VaultKey::generate()))
}

/// The API over `dir` with the vault's key, or with no vault.
pub fn open_with(dir: &DataDir, key: Option<String>) -> Api {
    let config = Config::load(
        None,
        move |name| key.clone().filter(|_| name == KEY_VAR),
        Defaults {
            data_dir: dir.to_path_buf(),
            timezone: chrono_tz::Asia::Tokyo,
        },
    )
    .unwrap();
    lay_notebook(&config.notebook_dir());
    Api::new(famulen_core::runtime::open_app(&config).unwrap())
}

/// Lay a notebook as Famulen makes it, copied from one made once: making one
/// starts Git six times. One that is there already is left as it is.
fn lay_notebook(root: &Path) {
    static MADE: OnceLock<Vec<(PathBuf, Option<Vec<u8>>)>> = OnceLock::new();
    if root.exists() {
        return;
    }
    let made = MADE.get_or_init(|| {
        let tmp = temp();
        famulen_core::notebook::ensure(&famulen_core::notebook::open(&*tmp)).unwrap();
        let mut found = Vec::new();
        gather(&tmp, Path::new(""), &mut found);
        found
    });
    for (rel, bytes) in made {
        let path = root.join(rel);
        match bytes {
            Some(bytes) => std::fs::write(path, bytes).unwrap(),
            None => std::fs::create_dir_all(path).unwrap(),
        }
    }
}

/// What lies in `root`, each directory before what is in it and with no content.
fn gather(root: &Path, at: &Path, found: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
    found.push((at.to_path_buf(), None));
    for entry in std::fs::read_dir(root.join(at)).unwrap() {
        let rel = at.join(entry.unwrap().file_name());
        if root.join(&rel).is_dir() {
            gather(root, &rel, found);
        } else {
            let bytes = std::fs::read(root.join(&rel)).unwrap();
            found.push((rel, Some(bytes)));
        }
    }
}

/// Add a service definition of the owner's own.
pub fn define(dir: &DataDir, id: &str, toml: &str) {
    let services = dir.join("gateway/services");
    std::fs::create_dir_all(&services).unwrap();
    std::fs::write(services.join(format!("{id}.toml")), toml).unwrap();
}

/// What the owner's call of `<Service>/<Method>` answers; an error's body
/// when it failed.
pub async fn call(api: &Api, method: &str, body: Value) -> Value {
    request(api, None, method, body).await.1
}

/// [`call`] with the status.
pub async fn try_call(api: &Api, method: &str, body: Value) -> (StatusCode, Value) {
    request(api, None, method, body).await
}

/// [`call`] made by someone who names themselves.
pub async fn call_as(api: &Api, caller: &str, method: &str, body: Value) -> Value {
    request(api, Some(caller), method, body).await.1
}

async fn request(
    api: &Api,
    caller: Option<&str>,
    method: &str,
    body: Value,
) -> (StatusCode, Value) {
    use tower::ServiceExt;
    let mut request = http::Request::post(format!("/famulen.v1.{method}"))
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1");
    if let Some(caller) = caller {
        request = request.header(CALLER_HEADER, caller);
    }
    let request = request
        .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let response = api.clone().axum().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
