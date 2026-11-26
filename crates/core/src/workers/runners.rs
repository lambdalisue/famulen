//! What starts, watches and stops the runs.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use famulen_executor::Standing;
use futures_util::future::BoxFuture;
use tokio::task::AbortHandle;

use super::adroit;
use super::agent::{self, Finish, Job};
use super::order::Runner;
use super::runs::{Run, State};
use crate::app::App;

/// How often an end is tried, and how long after one that failed.
const CLEAR_TRIES: u32 = 5;
const CLEAR_AGAIN: Duration = Duration::from_secs(5);

/// What came of starting a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Began {
    /// Its start is held as this ticket for the owner's approval.
    Asking(i64),
    /// It runs; the Adroit session's id, or nothing for an agent.
    Running(String),
}

/// How a run stands when looked at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Looked {
    Working,
    /// It waits for the owner.
    Waiting,
    /// Its turn ended with tasks of its plan unfinished.
    Idle,
    Done(String),
    Failed(String),
    /// It was closed before its plan was finished, saying this.
    Unsure(String),
    /// A passing fault of the model's service cut its turn off.
    Stumbled(String),
    /// It is gone with the process that held it.
    Lost,
}

pub trait Runners: Send + Sync + 'static {
    fn begin<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Began>>;
    /// Whether a start whose answer was lost went through after all. An error
    /// says that it cannot be told.
    fn find<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Option<Began>>>;
    fn look<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Looked>>;
    /// Those of these ended runs whose sessions went on since, by their ids;
    /// with each, whether something was told of its session.
    fn went_on<'a>(&'a self, ended: &'a [(Run, bool)]) -> BoxFuture<'a, Vec<i64>>;
    /// Ask a run's session to go on after a passing fault.
    fn go_on<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<()>>;
    /// Stop what Famulen itself runs. An Adroit session is the owner's to end.
    fn stop(&self, run: &Run);
}

struct Working {
    job: Arc<Job>,
    abort: AbortHandle,
    finish: Arc<Mutex<Option<Finish>>>,
}

/// The agents Famulen starts itself, and the sessions of Adroit.
pub(super) struct Real {
    app: App,
    agents: Mutex<HashMap<i64, Working>>,
    /// What an ended run's session was like when it was last read.
    read: Mutex<HashMap<i64, adroit::Trace>>,
}

impl Real {
    pub(super) fn new(app: App) -> Self {
        Self {
            app,
            agents: Mutex::default(),
            read: Mutex::default(),
        }
    }

    fn agents(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Working>> {
        self.agents.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// See to a run that was started, until it ends.
    fn attend(&self, run: i64, job: Job) {
        let job = Arc::new(job);
        let finish = Arc::new(Mutex::new(None));
        let (kept, workers) = (finish.clone(), self.app.workers.clone());
        let task = tokio::spawn({
            let job = job.clone();
            async move {
                let done = job.work().await;
                *kept.lock().unwrap_or_else(|p| p.into_inner()) = Some(done);
                workers.poke();
            }
        });
        self.agents().insert(
            run,
            Working {
                job,
                abort: task.abort_handle(),
                finish,
            },
        );
    }

    async fn find_agent(&self, run: &Run) -> anyhow::Result<Option<Began>> {
        if let Some(trouble) = self.agents().get(&run.id).and_then(|w| w.job.trouble()) {
            anyhow::bail!(trouble);
        }
        Ok(match agent::executor(&self.app, run.id).standing().await? {
            Standing::None => None,
            Standing::Foreign => {
                anyhow::bail!("it runs under an executor of a version that is not spoken with here")
            }
            Standing::Working | Standing::Over | Standing::Gone => {
                Some(Began::Running(String::new()))
            }
        })
    }

    async fn look_at_agent(&self, run: &Run) -> anyhow::Result<Looked> {
        if !self.agents().contains_key(&run.id) {
            match agent::executor(&self.app, run.id).standing().await? {
                Standing::None | Standing::Gone => return Ok(Looked::Lost),
                Standing::Working | Standing::Foreign | Standing::Over => {
                    self.attend(run.id, Job::again(&self.app, run));
                }
            }
        }
        let agents = self.agents();
        let Some(working) = agents.get(&run.id) else {
            return Ok(Looked::Lost);
        };
        // Asked before the finish is read: an agent that ends in between has
        // its finish read below, and is not taken for lost.
        let over = working.abort.is_finished();
        let finish = working
            .finish
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        Ok(match finish {
            Some(Finish::Done(text)) => Looked::Done(text),
            Some(Finish::Failed(text)) => Looked::Failed(text),
            Some(Finish::Lost) => Looked::Lost,
            None if over => Looked::Lost,
            None => match working.job.trouble() {
                Some(trouble) => anyhow::bail!(trouble),
                None => Looked::Working,
            },
        })
    }

    /// End what is left of runs that are over, or of which nothing is known.
    pub(super) async fn sweep(&self) -> anyhow::Result<()> {
        let workers = &self.app.workers;
        let mut open = BTreeSet::new();
        for run in workers.runs.open()? {
            if run.order.runner != Runner::Agent || run.state == State::Ending {
                continue;
            }
            open.insert(run.id);
        }
        let mut left: BTreeSet<i64> = BTreeSet::new();
        left.extend(
            workers
                .executors
                .names()?
                .iter()
                .filter_map(|n| n.parse::<i64>().ok()),
        );
        if let Ok(entries) = std::fs::read_dir(&workers.dir) {
            left.extend(
                entries
                    .flatten()
                    .filter_map(|e| e.file_name().to_str()?.parse::<i64>().ok()),
            );
        }
        for run in left.difference(&open) {
            if let Err(error) = agent::clear(&self.app, *run).await {
                tracing::warn!(run, %error, "could not clear what a run left");
            }
        }
        Ok(())
    }
}

impl Runners for Real {
    fn begin<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Began>> {
        Box::pin(async move {
            match run.order.runner {
                Runner::Adroit => adroit::begin(&self.app, run).await,
                Runner::Agent => {
                    let job = Job::begin(&self.app, run).await?;
                    let attended = self
                        .agents()
                        .get(&run.id)
                        .is_some_and(|w| !w.abort.is_finished());
                    if !attended {
                        self.attend(run.id, job);
                    }
                    Ok(Began::Running(String::new()))
                }
            }
        })
    }

    fn find<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Option<Began>>> {
        Box::pin(async move {
            match run.order.runner {
                Runner::Adroit => adroit::find(&self.app, run).await,
                Runner::Agent => self.find_agent(run).await,
            }
        })
    }

    fn look<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Looked>> {
        Box::pin(async move {
            match run.order.runner {
                Runner::Adroit => adroit::look(&self.app, run).await,
                Runner::Agent => self.look_at_agent(run).await,
            }
        })
    }

    fn went_on<'a>(&'a self, ended: &'a [(Run, bool)]) -> BoxFuture<'a, Vec<i64>> {
        Box::pin(adroit::went_on(&self.app, ended, &self.read))
    }

    fn go_on<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            match run.order.runner {
                Runner::Adroit => adroit::go_on(&self.app, run).await,
                Runner::Agent => anyhow::bail!("an agent has no session to go on"),
            }
        })
    }

    fn stop(&self, run: &Run) {
        if run.order.runner != Runner::Agent {
            return;
        }
        if let Some(working) = self.agents().remove(&run.id) {
            working.abort.abort();
        }
        // What it was handed is gone at once, whatever it takes to end it.
        std::fs::remove_dir_all(agent::workdir(&self.app, run.id)).ok();
        let (app, run) = (self.app.clone(), run.id);
        tokio::spawn(async move {
            // What cannot be ended now is ended at the next start.
            for _ in 0..CLEAR_TRIES {
                match agent::clear(&app, run).await {
                    Ok(()) => return,
                    Err(error) => tracing::warn!(run, %error, "could not end a worker"),
                }
                tokio::time::sleep(CLEAR_AGAIN).await;
            }
        });
    }
}
