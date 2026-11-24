//! The notebook's Git, as a copy on another machine reaches it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use famulen_api::Api;
use famulen_notebook::Notebook;
use famulen_notebook::sync::{self, Sync, Trouble};
use famulen_notebook::{Author, Signature};
use serde_json::json;

use crate::support::{DataDir, call, open, temp};

/// A Famulen whose API is served, and where copies of its notebook go.
struct Served {
    dir: DataDir,
    api: Api,
    url: String,
}

async fn serve() -> Served {
    let dir = temp();
    let api = open(&dir);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/notebook.git", listener.local_addr().unwrap());
    let router = api.clone().axum();
    tokio::spawn(async move { axum::serve(listener, router).await });
    Served { dir, api, url }
}

impl Served {
    async fn share(&self, on: bool) {
        call(
            &self.api,
            "SettingsService/SetNotebookSharing",
            json!({ "on": on }),
        )
        .await;
    }

    fn copy(&self, name: &str) -> PathBuf {
        self.dir.join("copies").join(name)
    }

    async fn clone(&self, name: &str) -> anyhow::Result<Notebook> {
        let (url, to) = (self.url.clone(), self.copy(name));
        tokio::task::spawn_blocking(move || sync::clone(url, to))
            .await
            .unwrap()
    }

    async fn sync(&self, copy: &Notebook) -> anyhow::Result<()> {
        let copy = copy.clone();
        tokio::task::spawn_blocking(move || Sync::of(&copy).unwrap().sync())
            .await
            .unwrap()
    }

    fn here(&self, rel: &str) -> String {
        std::fs::read_to_string(self.api.app().notebook.root().join(rel)).unwrap()
    }
}

fn write(copy: &Notebook, rel: &str, text: &str) {
    std::fs::write(copy.root().join(rel), text).unwrap();
    let bob = Signature::new(Author::new("Bob", "bob@example.com"));
    copy.commit(&bob, "x", &[rel]).unwrap();
}

async fn git(dir: &Path, args: &[&str]) -> Output {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0");
    tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap()
}

const CORE: &str = "memory/core.md";

#[tokio::test]
async fn the_notebook_is_reached_only_while_the_owner_shares_it() {
    let served = serve().await;
    let sharing = call(&served.api, "SettingsService/GetNotebookSharing", json!({})).await;
    assert_eq!(sharing.get("on"), None);
    assert!(served.clone("off").await.is_err());

    served.share(true).await;
    let copy = served.clone("on").await.unwrap();

    served.share(false).await;
    write(&copy, CORE, "# 心得\n\n- 一 {id: a}\n");
    let trouble: Trouble = served.sync(&copy).await.unwrap_err().downcast().unwrap();
    assert!(trouble.passes(), "{trouble:?}");
    assert!(!served.here(CORE).contains("一"));
}

#[tokio::test]
async fn a_copy_and_the_owner_read_what_the_other_writes() {
    let served = serve().await;
    served.share(true).await;
    let copy = served.clone("one").await.unwrap();

    write(&copy, CORE, "# 心得\n\n- 一 {id: a}\n");
    served.sync(&copy).await.unwrap();
    assert!(served.here(CORE).contains("一"));
    let read = call(
        &served.api,
        "MemoryService/ReadFile",
        json!({ "path": "core.md" }),
    )
    .await;
    assert_eq!(read["history"][0]["author"], "Bob", "{read}");

    call(
        &served.api,
        "MemoryService/WriteFile",
        json!({ "path": "core.md", "text": "# 心得\n\n- 一 {id: a}\n- 二 {id: b}\n" }),
    )
    .await;
    served.sync(&copy).await.unwrap();
    let core = std::fs::read_to_string(copy.root().join(CORE)).unwrap();
    assert!(core.contains("二"), "{core}");

    let log = call(
        &served.api,
        "LogService/ListLog",
        json!({ "show": "other" }),
    )
    .await;
    assert!(log.to_string().contains(CORE), "{log}");
}

#[tokio::test]
async fn a_push_that_breaks_the_rules_is_refused_with_why() {
    let served = serve().await;
    served.share(true).await;
    let copy = served.clone("one").await.unwrap();
    let before = served.here(CORE);

    write(&copy, CORE, "# 心得\n\n- [[どこにもない]] {id: a}\n");
    let trouble: Trouble = served.sync(&copy).await.unwrap_err().downcast().unwrap();

    assert!(
        matches!(&trouble, Trouble::Refused(why) if why.contains("broken-link")),
        "{trouble:?}"
    );
    assert_eq!(served.here(CORE), before);
}

#[tokio::test]
async fn history_cannot_be_rewritten_nor_the_branch_deleted_or_another_made() {
    let served = serve().await;
    served.share(true).await;
    let copy = served.clone("one").await.unwrap();
    let root = copy.root();
    write(&copy, CORE, "# 心得\n\n- 一 {id: a}\n");
    served.sync(&copy).await.unwrap();
    let reset = git(root, &["update-ref", "refs/heads/main", "HEAD~1"]).await;
    assert!(reset.status.success());

    for args in [
        &["push", "--force", "origin", "main"][..],
        &["push", "origin", ":main"],
        &["push", "origin", "main:other"],
    ] {
        let pushed = git(root, args).await;
        assert!(!pushed.status.success(), "{args:?}");
    }
    assert!(served.here(CORE).contains("一"));
    let listed = git(root, &["ls-remote", "origin"]).await;
    let listed = String::from_utf8_lossy(&listed.stdout).into_owned();
    assert_eq!(listed.lines().count(), 2, "HEAD and main: {listed}");
}

#[tokio::test]
async fn a_large_push_is_taken() {
    let served = serve().await;
    served.share(true).await;
    let copy = served.clone("one").await.unwrap();
    // More than Git keeps in one piece, and not compressible.
    let mut seed = 1u64;
    let text: String = (0..3_000_000)
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            char::from(b'a' + (seed >> 59) as u8 % 26)
        })
        .collect();
    write(&copy, "notes.md", &format!("# x\n\n{text}\n"));

    served.sync(&copy).await.unwrap();

    assert!(served.here("notes.md").contains(&text));
}

#[tokio::test]
async fn nothing_but_what_git_asks_is_served() {
    use tower::ServiceExt as _;
    let served = serve().await;
    served.share(true).await;
    let ask = async |method: &str, path: &str, kind: &str| {
        let request = http::Request::builder()
            .method(method)
            .uri(format!("/notebook.git/{path}"))
            .header("content-type", kind)
            .body(axum::body::Body::empty())
            .unwrap();
        let router = served.api.clone().axum();
        router.oneshot(request).await.unwrap().status().as_u16()
    };
    for path in ["config", "HEAD", "objects/info/packs", "hooks/update", ""] {
        assert_eq!(ask("GET", path, "").await, 404, "{path}");
    }
    assert_eq!(ask("GET", "info/refs", "").await, 403);
    assert_eq!(ask("GET", "info/refs?service=git-x", "").await, 403);
    for path in ["git-receive-pack", "git-upload-pack"] {
        assert_eq!(ask("POST", path, "text/plain").await, 415);
    }
}
