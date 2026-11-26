use std::ops::Deref;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::*;
use crate::store::Store;

/// A directory of a test's own, removed when it ends.
pub(super) struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "famulen-speech-{name}-{}-{:x}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

const SMALL: &str = "https://models.example/small";
const BASE: &str = "https://models.example/base";
const ENGINE: &str = "https://builds.example/linux.tar.gz";
const SOURCE: &str = "https://builds.example/source.tar.gz";

/// Serves files from memory; a URL it does not have never answers.
#[derive(Default)]
struct Shelf {
    files: HashMap<String, Vec<u8>>,
    asked: Mutex<Vec<String>>,
}

impl Fetch for Shelf {
    fn fetch<'a>(
        &'a self,
        url: &'a str,
        to: &'a Path,
        most: u64,
        progress: &'a (dyn Fn(u64) + Send + Sync),
    ) -> BoxFuture<'a, Result<Downloaded>> {
        Box::pin(async move {
            self.asked.lock().unwrap().push(url.to_owned());
            let Some(bytes) = self.files.get(url) else {
                std::fs::write(to, b"half")?;
                progress(4);
                std::future::pending().await
            };
            ensure!(bytes.len() as u64 <= most, "too large");
            std::fs::write(to, bytes)?;
            progress(bytes.len() as u64);
            Ok(Downloaded {
                bytes: bytes.len() as u64,
                sha256: digest(bytes),
            })
        })
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn executable(path: &Path, script: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A `.tar.gz` holding one directory with `files` in it.
fn archive(scratch: &Path, files: &[(&str, &str)]) -> Vec<u8> {
    let made = scratch.join(format!("archive-{:x}", rand::random::<u32>()));
    for (name, content) in files {
        executable(&made.join("top").join(name), content);
    }
    let at = made.join("it.tar.gz");
    let status = std::process::Command::new("tar")
        .arg("-czf")
        .arg(&at)
        .arg("-C")
        .arg(&made)
        .arg("top")
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::read(at).unwrap()
}

fn pin(url: &str, bytes: &[u8]) -> Pinned {
    Pinned {
        url: url.into(),
        size: bytes.len() as u64,
        sha256: digest(bytes),
    }
}

struct Setup {
    scratch: Scratch,
    shelf: Shelf,
    catalog: Catalog,
    host: Host,
}

fn setup(name: &str) -> Setup {
    let scratch = Scratch::new(name);
    let engine = archive(
        &scratch,
        &[("whisper-server", "#!/bin/sh\nexec sleep 60\n")],
    );
    let source = archive(&scratch, &[("LICENSE", "MIT")]);
    let (small, base) = (b"small model".to_vec(), b"base model".to_vec());
    let model = |name: &str, url: &str, bytes: &[u8]| SpeechModel {
        name: name.into(),
        speed: "速い".into(),
        accuracy: "低め".into(),
        file: pin(url, bytes),
    };
    let catalog = Catalog {
        models: vec![model("small", SMALL, &small), model("base", BASE, &base)],
        version: "1".into(),
        linux_x64: pin(ENGINE, &engine),
        linux_arm64: pin(ENGINE, &engine),
        source: pin(SOURCE, &source),
        cmake_macos: pin("https://builds.example/cmake.tar.gz", b"cmake"),
    };
    let shelf = Shelf {
        files: [
            (SMALL, small),
            (BASE, base),
            (ENGINE, engine),
            (SOURCE, source),
        ]
        .map(|(url, bytes)| (url.to_owned(), bytes))
        .into(),
        ..Shelf::default()
    };
    let host = Host {
        platform: Platform::LinuxX64,
        cmake: None,
        xcode_select: "true".into(),
    };
    Setup {
        scratch,
        shelf,
        catalog,
        host,
    }
}

struct Opened {
    scratch: Scratch,
    shelf: Arc<Shelf>,
    speech: Speech,
    models: Models,
}

impl Setup {
    fn open(self) -> Opened {
        let store = Store::open(self.scratch.join("famulen.sqlite3")).unwrap();
        let models = Models::new(store);
        let shelf = Arc::new(self.shelf);
        let speech = Speech::new(
            self.scratch.join("whisper"),
            self.catalog,
            self.host,
            shelf.clone(),
            models.clone(),
        );
        Opened {
            scratch: self.scratch,
            shelf,
            speech,
            models,
        }
    }
}

async fn settled(speech: &Speech, name: &str) -> Held {
    for _ in 0..1000 {
        match speech.held(name) {
            Held::Downloading(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            held => return held,
        }
    }
    panic!("still downloading");
}

fn voice(models: &Models) -> Option<String> {
    models.assignment(Role::Voice).unwrap().map(|a| a.model)
}

#[tokio::test]
async fn nothing_is_downloaded_until_it_is_asked_for() {
    let o = setup("untouched").open();

    assert_eq!(o.speech.held("small"), Held::Absent);
    assert!(o.speech.engine_bytes() > 0);
    assert!(o.shelf.asked.lock().unwrap().is_empty());
    assert!(!o.scratch.join("whisper").exists());
}

#[tokio::test]
async fn a_download_brings_whisper_with_the_first_model_and_registers_without_assigning() {
    let o = setup("download").open();

    o.speech.download("small").unwrap();
    assert_eq!(settled(&o.speech, "small").await, Held::Downloaded);

    assert_eq!(*o.shelf.asked.lock().unwrap(), [ENGINE, SMALL]);
    assert_eq!(o.speech.engine_bytes(), 0);
    let registered = o.models.model(model_id("small")).unwrap().unwrap();
    assert_eq!(registered.spec.kind, ModelKind::Transcribe);
    assert_eq!(voice(&o.models), None);

    o.speech.download("base").unwrap();
    assert_eq!(settled(&o.speech, "base").await, Held::Downloaded);
    assert_eq!(*o.shelf.asked.lock().unwrap(), [ENGINE, SMALL, BASE]);
}

#[tokio::test]
async fn what_is_not_what_was_pinned_is_not_kept() {
    let mut s = setup("tampered");
    s.shelf.files.insert(SMALL.into(), b"tampered!!!".to_vec());
    let o = s.open();

    o.speech.download("small").unwrap();
    let Held::Failed(why) = settled(&o.speech, "small").await else {
        panic!("kept");
    };

    assert!(why.contains("not the file"), "{why}");
    let models = o.scratch.join("whisper/models");
    assert_eq!(std::fs::read_dir(models).unwrap().count(), 0);
    assert!(o.models.model(model_id("small")).unwrap().is_none());
}

#[tokio::test]
async fn only_a_downloaded_model_can_be_used_and_the_cap_is_kept() {
    let o = setup("use").open();
    assert!(o.speech.use_model("small").is_err());
    for name in ["small", "base"] {
        o.speech.download(name).unwrap();
        settled(&o.speech, name).await;
    }

    o.speech.use_model("small").unwrap();
    assert_eq!(voice(&o.models), Some(model_id("small")));
    o.models
        .assign(Role::Voice, model_id("small"), Some(20.0), None)
        .unwrap();
    o.speech.use_model("base").unwrap();

    let assigned = o.models.assignment(Role::Voice).unwrap().unwrap();
    assert_eq!(assigned.model, model_id("base"));
    assert_eq!(assigned.cap, Some(20.0));
}

#[tokio::test]
async fn deleting_a_model_gives_its_space_back_and_the_last_takes_whisper_with_it() {
    let o = setup("delete").open();
    for name in ["small", "base"] {
        o.speech.download(name).unwrap();
        settled(&o.speech, name).await;
    }
    o.speech.use_model("small").unwrap();

    o.speech.delete("small").await.unwrap();

    assert_eq!(voice(&o.models), None);
    assert!(o.models.model(model_id("small")).unwrap().is_none());
    assert!(!o.scratch.join("whisper/models/ggml-small.bin").exists());
    assert_eq!(o.speech.held("base"), Held::Downloaded);

    o.speech.delete("base").await.unwrap();

    assert!(!o.scratch.join("whisper").exists());
}

#[tokio::test]
async fn deleting_ends_a_download_and_keeps_nothing_of_it() {
    let mut s = setup("cancel");
    s.shelf.files.remove(SMALL);
    let o = s.open();
    o.speech.download("small").unwrap();
    let part = o.scratch.join("whisper/models/ggml-small.part");
    while !part.exists() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        o.speech.held("small"),
        Held::Downloading(Progress {
            step: Step::Model,
            done: 4,
            total: 11
        })
    );

    o.speech.delete("small").await.unwrap();

    assert_eq!(o.speech.held("small"), Held::Absent);
    assert!(!o.scratch.join("whisper").exists());
}

#[tokio::test]
async fn macos_builds_the_pinned_source_with_the_cmake_it_has() {
    let mut s = setup("build");
    let cmake = s.scratch.join("bin/cmake");
    // Configures nothing, and makes the server where a build would.
    executable(
        &cmake,
        "#!/bin/sh\n[ \"$1\" = --build ] || exit 0\nmkdir -p \"$2/bin\" && echo built > \"$2/bin/whisper-server\"\n",
    );
    s.host.platform = Platform::MacOs;
    s.host.cmake = Some(cmake);
    let o = s.open();
    assert!(o.speech.builds());

    o.speech.download("small").unwrap();
    assert_eq!(settled(&o.speech, "small").await, Held::Downloaded);

    assert_eq!(*o.shelf.asked.lock().unwrap(), [SOURCE, SMALL]);
    let engine = o.scratch.join("whisper/engine");
    assert_eq!(
        std::fs::read_to_string(engine.join("whisper-server")).unwrap(),
        "built\n"
    );
    assert!(!o.scratch.join("whisper/engine.new").exists());
}

#[tokio::test]
async fn macos_without_the_compiler_says_so_and_downloads_nothing() {
    let mut s = setup("no-compiler");
    s.host.platform = Platform::MacOs;
    s.host.xcode_select = "false".into();
    let o = s.open();

    o.speech.download("small").unwrap();
    let Held::Failed(why) = settled(&o.speech, "small").await else {
        panic!("downloaded");
    };

    assert!(why.contains("xcode-select --install"), "{why}");
    assert!(o.shelf.asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_os_whisper_is_not_brought_in_on_is_refused() {
    let mut s = setup("other-os");
    s.host.platform = Platform::Other;
    let o = s.open();

    assert!(!o.speech.supported());
    assert!(o.speech.download("small").is_err());
}

mod serving {
    use axum::routing::{get, post};
    use axum::{Json, Router};

    use super::*;
    use crate::models::Pcm;

    /// A server that notes how it was started and stays; the test answers on
    /// the port it was given.
    fn noting(scratch: &Path) -> Vec<u8> {
        let script = format!(
            "#!/bin/sh\necho \"$@\" > '{}'\nexec sleep 60\n",
            scratch.join("started").display()
        );
        archive(scratch, &[("whisper-server", &script)])
    }

    async fn started_with(scratch: &Path) -> Vec<String> {
        let at = scratch.join("started");
        for _ in 0..500 {
            if let Ok(said) = std::fs::read_to_string(&at)
                && said.ends_with('\n')
            {
                std::fs::remove_file(at).unwrap();
                return said.split_whitespace().map(str::to_owned).collect();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("never started");
    }

    fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
        let at = args.iter().position(|a| a == flag).unwrap();
        &args[at + 1]
    }

    async fn answer_on(port: &str) {
        let app = Router::new()
            .route("/health", get(|| async { "ok" }))
            .route(
                "/v1/audio/transcriptions",
                post(|| async { Json(serde_json::json!({ "text": "聞こえた" })) }),
            );
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
    }

    #[tokio::test]
    async fn the_voice_is_heard_by_a_server_started_for_its_model_on_this_machine_only() {
        let mut s = setup("serving");
        let engine = noting(&s.scratch);
        s.catalog.linux_x64 = pin(ENGINE, &engine);
        s.shelf.files.insert(ENGINE.into(), engine);
        let o = s.open();
        for name in ["small", "base"] {
            o.speech.download(name).unwrap();
            settled(&o.speech, name).await;
        }
        let running = tokio::spawn({
            let speech = o.speech.clone();
            async move { speech.run().await }
        });
        assert_eq!(o.speech.server(), Server::Stopped);

        o.speech.use_model("small").unwrap();
        let args = started_with(&o.scratch).await;
        assert_eq!(after(&args, "--host"), "127.0.0.1");
        assert_eq!(after(&args, "-l"), "auto");
        assert!(after(&args, "-m").ends_with("ggml-small.bin"));
        answer_on(after(&args, "--port")).await;
        let endpoint = o.speech.endpoint("small").await.unwrap();
        assert_eq!(
            endpoint,
            format!("http://127.0.0.1:{}/v1", after(&args, "--port"))
        );

        let models = o.models.clone().with_speech(o.speech.clone());
        let sound = Pcm {
            samples: vec![0; 32],
            rate: 16000,
        };
        let heard = models
            .transcribe(chrono_tz::Asia::Tokyo, &sound, Some("ja"), false)
            .await
            .unwrap();
        assert_eq!(heard, "聞こえた");

        o.speech.use_model("base").unwrap();
        let args = started_with(&o.scratch).await;
        assert!(after(&args, "-m").ends_with("ggml-base.bin"));
        answer_on(after(&args, "--port")).await;
        o.speech.endpoint("base").await.unwrap();

        o.speech.delete("base").await.unwrap();
        for _ in 0..500 {
            if o.speech.server() == Server::Stopped {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(o.speech.server(), Server::Stopped);
        running.abort();
    }

    #[tokio::test]
    async fn a_server_still_loading_is_given_up_once_another_model_is_wanted() {
        let mut s = setup("switch-while-loading");
        let engine = noting(&s.scratch);
        s.catalog.linux_x64 = pin(ENGINE, &engine);
        s.shelf.files.insert(ENGINE.into(), engine);
        let o = s.open();
        for name in ["small", "base"] {
            o.speech.download(name).unwrap();
            settled(&o.speech, name).await;
        }
        o.speech.use_model("small").unwrap();
        let running = tokio::spawn({
            let speech = o.speech.clone();
            async move { speech.run().await }
        });
        started_with(&o.scratch).await;

        o.speech.use_model("base").unwrap();

        let args = started_with(&o.scratch).await;
        assert!(after(&args, "-m").ends_with("ggml-base.bin"));
        running.abort();
    }

    #[tokio::test]
    async fn a_server_that_will_not_start_says_why() {
        let mut s = setup("broken-server");
        let engine = archive(
            &s.scratch,
            &[(
                "whisper-server",
                "#!/bin/sh\necho 'bad model' >&2\nexit 1\n",
            )],
        );
        s.catalog.linux_x64 = pin(ENGINE, &engine);
        s.shelf.files.insert(ENGINE.into(), engine);
        let o = s.open();
        o.speech.download("small").unwrap();
        settled(&o.speech, "small").await;
        o.speech.use_model("small").unwrap();
        let running = tokio::spawn({
            let speech = o.speech.clone();
            async move { speech.run().await }
        });

        let error = o.speech.endpoint("small").await.unwrap_err();

        assert!(error.to_string().contains("bad model"), "{error}");
        running.abort();
    }
}
