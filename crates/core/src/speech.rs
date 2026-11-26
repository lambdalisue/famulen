//! Hearing on this machine: whisper.cpp and the models it hears with,
//! brought in only when the owner asks, kept in a directory of Famulen's own
//! and run as Famulen's child.

mod catalog;
mod engine;
mod process;
mod server;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::{Context, Result, ensure};
use futures_util::future::BoxFuture;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;

pub use catalog::*;
pub use engine::*;
pub use server::*;

use crate::gateway::{Downloaded, Gateway};
use crate::models::{CapUnit, ModelKind, ModelSpec, Models, Price, Role};

/// Who asks for what is brought in: the owner, from the console.
const OWNER: &str = "owner";
const ID_PREFIX: &str = "local-whisper-";

/// The id a model brought in is registered under.
pub fn model_id(name: impl AsRef<str>) -> String {
    format!("{ID_PREFIX}{}", name.as_ref())
}

/// The model brought in that is registered under `id`, if it is one.
pub fn local_name(id: &str) -> Option<&str> {
    id.strip_prefix(ID_PREFIX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Downloading what whisper.cpp is made of.
    Engine,
    /// Building it, which tells no amount.
    Building,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub step: Step,
    /// Bytes of the file being downloaded now.
    pub done: u64,
    pub total: u64,
}

/// Downloads files.
pub trait Fetch: Send + Sync {
    /// Write what `url` answers to `to`, taking no more than `most` bytes.
    fn fetch<'a>(
        &'a self,
        url: &'a str,
        to: &'a Path,
        most: u64,
        progress: &'a (dyn Fn(u64) + Send + Sync),
    ) -> BoxFuture<'a, Result<Downloaded>>;
}

impl Fetch for Gateway {
    fn fetch<'a>(
        &'a self,
        url: &'a str,
        to: &'a Path,
        most: u64,
        progress: &'a (dyn Fn(u64) + Send + Sync),
    ) -> BoxFuture<'a, Result<Downloaded>> {
        Box::pin(self.download_public(OWNER, url, to, most, progress))
    }
}

type Report = Box<dyn Fn(Progress) + Send + Sync>;

/// Downloading for one thing the owner asked for.
struct Fetching {
    fetch: Arc<dyn Fetch>,
    report: Report,
}

impl Fetching {
    fn report(&self, step: Step, done: u64, total: u64) {
        (self.report)(Progress { step, done, total });
    }

    /// Download `pinned` to `to`, keeping nothing that is not what was pinned.
    async fn fetch(&self, step: Step, pinned: &Pinned, to: &Path) -> Result<()> {
        self.report(step, 0, pinned.size);
        let progress = |done| self.report(step, done, pinned.size);
        let got = self
            .fetch
            .fetch(&pinned.url, to, pinned.size, &progress)
            .await
            .with_context(|| format!("failed to download {}", pinned.url))?;
        if got.bytes != pinned.size || got.sha256 != pinned.sha256 {
            std::fs::remove_file(to).ok();
            anyhow::bail!("{} is not the file that was expected", pinned.url);
        }
        Ok(())
    }
}

/// How a model stands on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Held {
    Absent,
    Downloading(Progress),
    /// Why the last download failed.
    Failed(String),
    Downloaded,
}

enum Task {
    Running {
        progress: Progress,
        handle: Option<JoinHandle<()>>,
    },
    Failed(String),
}

/// What is brought in to hear on this machine.
#[derive(Clone)]
pub struct Speech(Arc<Inner>);

struct Inner {
    dir: PathBuf,
    catalog: Catalog,
    host: Host,
    fetch: Arc<dyn Fetch>,
    models: Models,
    tasks: Mutex<HashMap<String, Task>>,
    /// Held while whisper.cpp itself is put in or taken out.
    engine: tokio::sync::Mutex<()>,
    /// Told when what the server is to do may have changed.
    wake: Notify,
    server: watch::Sender<Server>,
}

impl Speech {
    pub fn new(
        dir: impl Into<PathBuf>,
        catalog: Catalog,
        host: Host,
        fetch: Arc<dyn Fetch>,
        models: Models,
    ) -> Self {
        Self(Arc::new(Inner {
            dir: dir.into(),
            catalog,
            host,
            fetch,
            models,
            tasks: Mutex::default(),
            engine: Default::default(),
            wake: Notify::new(),
            server: watch::Sender::default(),
        }))
    }

    pub fn catalog(&self) -> &Catalog {
        &self.0.catalog
    }

    fn models(&self) -> &Models {
        &self.0.models
    }

    fn tasks(&self) -> std::sync::MutexGuard<'_, HashMap<String, Task>> {
        self.0.tasks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.dir.join("models").join(format!("ggml-{name}.bin"))
    }

    fn model(&self, name: &str) -> Result<&SpeechModel> {
        self.0
            .catalog
            .model(name)
            .with_context(|| format!("no model {name}"))
    }

    /// Whether the model and what hears with it are here.
    pub fn downloaded(&self, name: impl AsRef<str>) -> bool {
        self.path(name.as_ref()).is_file() && engine::server(&self.0.dir).is_file()
    }

    pub fn held(&self, name: impl AsRef<str>) -> Held {
        let name = name.as_ref();
        match self.tasks().get(name) {
            Some(Task::Running { progress, .. }) => Held::Downloading(*progress),
            _ if self.downloaded(name) => Held::Downloaded,
            Some(Task::Failed(why)) => Held::Failed(why.clone()),
            None => Held::Absent,
        }
    }

    /// How much is downloaded for whisper.cpp itself along with a first model.
    pub fn engine_bytes(&self) -> u64 {
        engine::bytes_to_fetch(&self.0.dir, &self.0.host, &self.0.catalog)
    }

    /// Whether whisper.cpp is built on this machine, for want of a build of it.
    pub fn builds(&self) -> bool {
        self.0.host.builds()
    }

    pub fn supported(&self) -> bool {
        self.0.host.platform != Platform::Other
    }

    /// Start downloading a model, and whisper.cpp if it is not here. It goes
    /// on after this returns; [`Self::held`] tells how it stands.
    pub fn download(&self, name: impl AsRef<str>) -> Result<()> {
        let model = self.model(name.as_ref())?.clone();
        ensure!(self.supported(), "whisper.cpp is not brought in on this OS");
        let mut tasks = self.tasks();
        if matches!(tasks.get(&model.name), Some(Task::Running { .. })) {
            return Ok(());
        }
        let name = model.name.clone();
        let handle = tokio::spawn({
            let speech = self.clone();
            async move {
                let brought = speech.bring_in(&model).await;
                let mut tasks = speech.tasks();
                match brought {
                    Ok(()) => {
                        tasks.remove(&model.name);
                    }
                    Err(error) => {
                        tracing::warn!(model = %model.name, error = %format!("{error:#}"), "not downloaded");
                        tasks.insert(model.name.clone(), Task::Failed(format!("{error:#}")));
                    }
                }
                drop(tasks);
                speech.0.wake.notify_one();
            }
        });
        tasks.insert(
            name,
            Task::Running {
                progress: Progress {
                    step: Step::Engine,
                    done: 0,
                    total: 0,
                },
                handle: Some(handle),
            },
        );
        Ok(())
    }

    async fn bring_in(&self, model: &SpeechModel) -> Result<()> {
        let fetching = Fetching {
            fetch: self.0.fetch.clone(),
            report: Box::new({
                let tasks = Arc::downgrade(&self.0);
                let name = model.name.clone();
                move |now| {
                    let Some(inner) = tasks.upgrade() else {
                        return;
                    };
                    let mut tasks = inner.tasks.lock().unwrap_or_else(PoisonError::into_inner);
                    if let Some(Task::Running { progress, .. }) = tasks.get_mut(&name) {
                        *progress = now;
                    }
                }
            }),
        };
        {
            // Also held while the whole directory is removed.
            let _one = self.0.engine.lock().await;
            engine::install(&self.0.dir, &self.0.host, &self.0.catalog, &fetching).await?;
            std::fs::create_dir_all(self.0.dir.join("models"))?;
        }
        let to = self.path(&model.name);
        if !to.is_file() {
            let part = to.with_extension("part");
            fetching.fetch(Step::Model, &model.file, &part).await?;
            std::fs::rename(part, to)?;
        }
        self.register(model)
    }

    fn register(&self, model: &SpeechModel) -> Result<()> {
        self.models().register(
            ModelSpec {
                id: model_id(&model.name),
                name: format!("whisper {}（このマシン）", model.name),
                kind: ModelKind::Transcribe,
                // The server's address is a new one each time it starts.
                endpoint: None,
                model: model.name.clone(),
                cli: Default::default(),
                price: Price::default(),
            },
            None,
        )
    }

    /// Have the voice heard with a model that is downloaded.
    pub fn use_model(&self, name: impl AsRef<str>) -> Result<()> {
        let model = self.model(name.as_ref())?;
        ensure!(
            self.downloaded(&model.name),
            "{} is not downloaded",
            model.name
        );
        self.register(model)?;
        let cap = self
            .models()
            .assignment(Role::Voice)?
            .filter(|a| a.cap_unit == CapUnit::Calls)
            .and_then(|a| a.cap);
        self.models()
            .assign(Role::Voice, model_id(&model.name), cap, None)?;
        self.0.wake.notify_one();
        Ok(())
    }

    /// Take a model off this machine, ending its download if it has one, and
    /// whisper.cpp with the last of them.
    pub async fn delete(&self, name: impl AsRef<str>) -> Result<()> {
        let name = self.model(name.as_ref())?.name.clone();
        let task = self.tasks().remove(&name);
        if let Some(Task::Running {
            handle: Some(handle),
            ..
        }) = task
        {
            handle.abort();
            // Ended by the abort, or just before it.
            handle.await.ok();
        }
        let id = model_id(&name);
        if self
            .models()
            .assignment(Role::Voice)?
            .is_some_and(|a| a.model == id)
        {
            self.models().unassign(Role::Voice)?;
        }
        if self.models().model(&id)?.is_some() {
            self.models().unregister(&id)?;
        }
        let path = self.path(&name);
        remove_file(&path)?;
        remove_file(&path.with_extension("part"))?;
        self.0.wake.notify_one();
        self.remove_unneeded_engine().await
    }

    async fn remove_unneeded_engine(&self) -> Result<()> {
        let _one = self.0.engine.lock().await;
        let downloading = self
            .tasks()
            .values()
            .any(|t| matches!(t, Task::Running { .. }));
        let held = self
            .0
            .catalog
            .models
            .iter()
            .any(|m| self.path(&m.name).is_file());
        if !downloading && !held {
            match std::fs::remove_dir_all(&self.0.dir) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error)
                        .with_context(|| format!("removing {}", self.0.dir.display()));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn remove_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
