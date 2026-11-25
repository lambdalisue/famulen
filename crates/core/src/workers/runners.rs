//! What starts, watches and stops the runs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use tokio::task::AbortHandle;

use super::adroit;
use super::agent::{Finish, Job};
use super::order::Runner;
use super::runs::Run;
use crate::app::App;

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
    Done(String),
    Failed(String),
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

    fn begin_agent(&self, run: &Run) -> anyhow::Result<Began> {
        let job = Job::ready(&self.app, run)?;
        let finish = Arc::new(Mutex::new(None));
        let (kept, workers) = (finish.clone(), self.app.workers.clone());
        let task = tokio::spawn(async move {
            let done = job.work().await;
            *kept.lock().unwrap_or_else(|p| p.into_inner()) = Some(done);
            workers.poke();
        });
        self.agents().insert(
            run.id,
            Working {
                abort: task.abort_handle(),
                finish,
            },
        );
        Ok(Began::Running(String::new()))
    }

    fn look_at_agent(&self, run: &Run) -> Looked {
        let agents = self.agents();
        let Some(working) = agents.get(&run.id) else {
            return Looked::Lost;
        };
        // Asked before the finish is read: an agent that ends in between has
        // its finish read below, and is not taken for lost.
        let over = working.abort.is_finished();
        let finish = working
            .finish
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        match finish {
            Some(finish) if finish.ok => Looked::Done(finish.text),
            Some(finish) => Looked::Failed(finish.text),
            None if over => Looked::Lost,
            None => Looked::Working,
        }
    }
}

impl Runners for Real {
    fn begin<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Began>> {
        Box::pin(async move {
            match run.order.runner {
                Runner::Adroit => adroit::begin(&self.app, run).await,
                Runner::Agent => self.begin_agent(run),
            }
        })
    }

    fn find<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Option<Began>>> {
        Box::pin(async move {
            match run.order.runner {
                Runner::Adroit => adroit::find(&self.app, run).await,
                Runner::Agent => Ok(None),
            }
        })
    }

    fn look<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Looked>> {
        Box::pin(async move {
            match run.order.runner {
                Runner::Adroit => adroit::look(&self.app, run).await,
                Runner::Agent => Ok(self.look_at_agent(run)),
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
        if let Some(working) = self.agents().remove(&run.id) {
            working.abort.abort();
        }
    }
}
