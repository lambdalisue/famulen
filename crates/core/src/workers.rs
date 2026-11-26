//! The worker pool: delegated tasks are picked up in order of priority, a few
//! at a time, and each is run by an agent or by an Adroit session. The pool
//! is a plain program; no model decides what runs next.

pub mod adroit;
mod agent;
mod bounds;
mod direct;
mod dispatch;
mod finish;
mod ledger;
mod notebook;
mod order;
mod pool;
mod queue;
mod runners;
mod runs;
mod sight;
mod spent;
mod tidy;
mod timeline;

pub(crate) use agent::executor_program;
pub use direct::*;
pub use dispatch::*;
pub use finish::*;
pub use ledger::*;
pub use order::*;
pub use pool::{Pool, run};
pub use queue::*;
pub use runners::{Began, Looked, Runners};
pub use runs::*;
pub use sight::*;
pub use spent::*;
pub use tidy::{Fate, How, Kind, Left, keep_tidy, tidy};

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use famulen_executor::Executors;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::sealed::forms;
use crate::store::Store;

/// `Assignee` of a delegated task, and who the pool is to the gateway.
pub const WORKER: &str = crate::permissions::WORKER;

const SETTINGS_KEY: &str = "setting.workers";
/// The most runs the owner may let go on at once.
pub const MOST_SLOTS: u32 = 20;

/// How the owner lets the pool work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// How many runs go on at once.
    pub slots: u32,
    /// No run is started while set; those going on go on.
    pub paused: bool,
    /// How the owner wants work on code finished.
    // `finishes` as stored held directories, which name no repository.
    #[serde(default, rename = "repository_finishes")]
    pub finishes: Vec<Finish>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            slots: 2,
            paused: false,
            finishes: Vec::new(),
        }
    }
}

impl Settings {
    pub fn load(store: &Store) -> anyhow::Result<Self> {
        Ok(match store.state(SETTINGS_KEY)? {
            Some(json) => serde_json::from_str(&json)?,
            None => Self::default(),
        })
    }

    pub fn save(&self, store: &Store) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=MOST_SLOTS).contains(&self.slots),
            "1 to {MOST_SLOTS} tasks run at once"
        );
        check_finishes(&self.finishes)?;
        store.set_state(SETTINGS_KEY, serde_json::to_string(self)?)
    }
}

/// What no agent doing a delegated task may reach.
#[derive(Debug, Clone, Default)]
pub struct Kept {
    /// On top of what is sealed for the secretary: the notebook and Famulen's
    /// own data.
    pub places: Vec<PathBuf>,
    pub hosts: Vec<String>,
}

/// What the rest of Famulen shares with the pool.
#[derive(Clone)]
pub struct Workers {
    pub runs: Runs,
    pub sights: Sights,
    /// Where an agent works, one directory for each run.
    pub(crate) dir: PathBuf,
    /// The processes agents run under, each named by its run.
    pub(crate) executors: Executors,
    pub(crate) kept: Kept,
    /// Where work may run directly, when the owner set it.
    pub(crate) direct: Option<Direct>,
    poke: Arc<Notify>,
    /// Adroit sessions something was told of, to be looked at at once.
    heard: Arc<Mutex<HashSet<String>>>,
    /// Told when a session may have been closed.
    closed: Arc<Notify>,
    /// Held while what delegated work left is cleared away.
    tidying: Arc<tokio::sync::Mutex<()>>,
    /// Runs whose question is being answered now.
    answering: Arc<Mutex<HashSet<i64>>>,
}

impl Workers {
    pub fn new(store: Store, dir: PathBuf, executors: Executors, kept: Kept) -> Self {
        Self {
            runs: Runs::new(store.clone()),
            sights: Sights::new(store),
            dir,
            executors,
            kept,
            direct: None,
            poke: Arc::default(),
            heard: Arc::default(),
            closed: Arc::default(),
            tidying: Arc::default(),
            answering: Arc::default(),
        }
    }

    /// Let work run directly where `direct` says.
    pub fn with_direct(self, direct: Option<Direct>) -> Self {
        Self { direct, ..self }
    }

    /// Have the pool look at its tasks now.
    pub fn poke(&self) {
        self.poke.notify_one();
    }

    async fn poked(&self) {
        self.poke.notified().await;
    }

    /// Something was told of an Adroit session. Whether a run is or was on it:
    /// one that is over is not the lookout's to tell of either.
    pub fn heard_of(&self, session: &str) -> anyhow::Result<bool> {
        let run = self.runs.of_session(session)?;
        if run.is_some() {
            self.heard
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(session.to_owned());
            self.poke();
        }
        if run == Some(false) {
            self.closed.notify_one();
        }
        Ok(run.is_some())
    }

    /// Refuse a directory around or inside what is kept from agents, or where
    /// runs keep theirs.
    pub fn may_work_in(&self, directory: &Path) -> anyhow::Result<()> {
        let of = forms(directory);
        let overlaps = |place: &&PathBuf| {
            forms(place)
                .iter()
                .any(|p| of.iter().any(|d| p.starts_with(d) || d.starts_with(p)))
        };
        if let Some(place) = self.kept.places.iter().chain([&self.dir]).find(overlaps) {
            anyhow::bail!(
                "no agent works in {}: {} is kept from it",
                directory.display(),
                place.display()
            );
        }
        Ok(())
    }

    /// What was told of a session was not attended to: it is looked at again
    /// when the pool next looks.
    fn hear_again(&self, session: &str) {
        self.heard
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session.to_owned());
    }

    /// Take up answering or asking again what `run` asks, unless one of them
    /// is on its way already: two that arrive together would both pass every
    /// look at whether it still waits, as it waits until the session has the
    /// answer. A run asks one thing at a time, so it is held run by run.
    pub(crate) fn answering(&self, run: i64) -> Option<Answering> {
        let taken = self
            .answering
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(run);
        taken.then(|| Answering {
            runs: self.answering.clone(),
            run,
        })
    }

    /// Whether `session` was told of since the last time this was asked.
    fn take_heard(&self, session: &str) -> bool {
        self.heard
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(session)
    }
}

/// An answer on its way to a run; the run is free to answer again once it is dropped.
pub(crate) struct Answering {
    runs: Arc<Mutex<HashSet<i64>>>,
    run: i64,
}

impl Drop for Answering {
    fn drop(&mut self) {
        self.runs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_is_not_given_a_directory_around_or_inside_what_is_kept_from_it() {
        let workers = Workers::new(
            Store::open_in_memory().unwrap(),
            "/data/workers".into(),
            Executors::new("/data/executors", "key", executor_program().unwrap()),
            Kept {
                places: vec!["/data/notebook".into()],
                ..Kept::default()
            },
        );
        for directory in [
            "/data",
            "/data/notebook/memory",
            "/data/workers",
            "/data/workers/3",
            "/",
        ] {
            assert!(
                workers.may_work_in(Path::new(directory)).is_err(),
                "{directory}"
            );
        }
        for directory in ["/src", "/data/notebook-of-others"] {
            assert!(
                workers.may_work_in(Path::new(directory)).is_ok(),
                "{directory}"
            );
        }
    }

    #[test]
    fn a_few_tasks_run_at_once_until_the_owner_says_otherwise() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(Settings::load(&store).unwrap(), Settings::default());
        for slots in [0, MOST_SLOTS + 1] {
            assert!(
                Settings {
                    slots,
                    ..Settings::default()
                }
                .save(&store)
                .is_err()
            );
        }
        let chosen = Settings {
            slots: 5,
            paused: true,
            finishes: vec![Finish {
                repository: String::new(),
                steps: "Review it.".into(),
            }],
        };
        chosen.save(&store).unwrap();
        assert_eq!(Settings::load(&store).unwrap(), chosen);
    }
}
