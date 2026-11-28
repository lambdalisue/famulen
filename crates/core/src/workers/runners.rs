//! What starts, watches and stops the runs.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use famulen_executor::Standing;
use futures_util::future::BoxFuture;
use tokio::task::AbortHandle;

use super::adroit;
use super::agent::{self, Finish, Job, ask_start};
use super::order::Runner;
use super::runs::{Run, State};
use super::{Driving, Finishing, Next as FinishingNext, Seen, Stage};
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
    Idle(Left),
    Done(String),
    Failed(String),
    /// It was closed before its plan was finished, saying this.
    Unsure(String),
    /// A passing fault of the model's service cut its turn off.
    Stumbled(String),
    /// It is gone with the process that held it.
    Lost,
}

/// What a session that ended its turn left of its plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Left {
    /// How many steps of its plan are completed.
    pub done: u32,
    /// What its blocked steps say they wait on.
    pub blocked: Vec<String>,
    /// Whether steps remain that are not blocked, or it kept no plan.
    pub open: bool,
    /// The owner stopped its last turn.
    pub interrupted: bool,
    /// A message sent to it has not been taken in.
    pub undelivered: bool,
    /// Tells its last turn from the others.
    pub turn: i64,
}

/// Why a session is asked to go on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum After {
    /// A passing fault cut its turn off.
    Fault,
    /// It ended its turn with its plan unfinished, and nothing happened since.
    Idle,
    /// The owner answered that it goes on, in these words when any.
    Owner(Option<String>),
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
    /// Ask a run's session to go on.
    fn go_on<'a>(&'a self, run: &'a Run, after: After) -> BoxFuture<'a, anyhow::Result<()>>;
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
    /// Runs whose agent's turn is over and which Famulen is finishing.
    finishings: Mutex<HashMap<i64, Driving>>,
    /// What an ended run's session was like when it was last read.
    read: Mutex<HashMap<i64, adroit::Trace>>,
}

impl Real {
    pub(super) fn new(app: App) -> Self {
        Self {
            app,
            agents: Mutex::default(),
            finishings: Mutex::default(),
            read: Mutex::default(),
        }
    }

    fn agents(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Working>> {
        self.agents.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn finishings(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Driving>> {
        self.finishings.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A run run directly in a repository the owner lets be finished: its
    /// agent's word that it is done begins the finishing, not the end.
    async fn look_at_work(&self, run: &Run) -> anyhow::Result<Looked> {
        let finishing = match run.order.runner {
            Runner::Direct => Finishing::load(&self.app.store, run.id)?,
            _ => None,
        };
        let Some(finishing) = finishing else {
            return self.look_at_agent(run).await;
        };
        if !finishing.with_agent() {
            return self.look_at_finishing(run, finishing).await;
        }
        match self.look_at_agent(run).await? {
            Looked::Done(report) => {
                let mut finishing = finishing;
                finishing.report = report;
                finishing.turned();
                finishing.save(&self.app.store, run.id)?;
                self.agents().remove(&run.id);
                self.look_at_finishing(run, finishing).await
            }
            looked => Ok(looked),
        }
    }

    async fn look_at_finishing(&self, run: &Run, finishing: Finishing) -> anyhow::Result<Looked> {
        match finishing.stage {
            Stage::Done => return Ok(Looked::Done(finishing.result())),
            Stage::Stopped => return Ok(Looked::Failed(finishing.result())),
            _ => {}
        }
        let seen = self.finishings().get(&run.id).map(Driving::seen);
        let Some(seen) = seen else {
            // A request to merge is the run's own, and taking back what the
            // agent was let call would withdraw it. The agent was ended
            // before it was made.
            if finishing.ticket.is_none() {
                agent::end_turn(&self.app, run.id).await?;
            }
            let hands = agent::own_hands(&self.app, run)?;
            let workers = self.app.workers.clone();
            let driving = Driving::start(finishing, hands, move || workers.poke());
            self.finishings().insert(run.id, driving);
            return Ok(Looked::Working);
        };
        match seen {
            Seen::Working => Ok(Looked::Working),
            Seen::Waiting => Ok(Looked::Waiting),
            Seen::Trouble(trouble) => anyhow::bail!(trouble),
            Seen::Over(next) => {
                self.finishings().remove(&run.id);
                if let FinishingNext::Turn(message) = next {
                    match Job::turn(&self.app, run, &message).await {
                        Ok(job) => self.attend(run.id, job),
                        Err(error) => {
                            return Ok(Looked::Failed(format!(
                                "It could not be handed back to the agent: {error:#}\n\n{message}"
                            )));
                        }
                    }
                }
                // How it ended is read from what is kept, at the next look.
                self.app.workers.poke();
                Ok(Looked::Working)
            }
        }
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
        // While it is finished no agent runs, and none is to be started.
        if run.order.runner == Runner::Direct
            && Finishing::load(&self.app.store, run.id)?.is_some_and(|f| !f.with_agent())
        {
            if let Some(Seen::Trouble(trouble)) = self.finishings().get(&run.id).map(Driving::seen)
            {
                anyhow::bail!(trouble);
            }
            return Ok(Some(Began::Running(String::new())));
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
                Standing::None | Standing::Gone => return self.lost(run).await,
                Standing::Working | Standing::Foreign | Standing::Over => {
                    self.attend(run.id, Job::again(&self.app, run));
                }
            }
        }
        let finish = {
            let agents = self.agents();
            let Some(working) = agents.get(&run.id) else {
                return Ok(Looked::Lost);
            };
            // Asked before the finish is read: an agent that ends in between
            // has its finish read below, and is not taken for lost.
            let over = working.abort.is_finished();
            let finish = working
                .finish
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            match finish {
                Some(Finish::Done(text)) => return Ok(Looked::Done(text)),
                Some(Finish::Failed(text)) => return Ok(Looked::Failed(text)),
                Some(Finish::Lost) => Finish::Lost,
                None if over => Finish::Lost,
                None => {
                    if let Some(trouble) = working.job.trouble() {
                        anyhow::bail!(trouble);
                    }
                    return Ok(if working.job.owes() {
                        Looked::Waiting
                    } else {
                        Looked::Working
                    });
                }
            }
        };
        debug_assert_eq!(finish, Finish::Lost);
        self.agents().remove(&run.id);
        self.lost(run).await
    }

    /// A run whose process went away with what it ran under. Work run
    /// directly goes on in its conversation, when it can.
    async fn lost(&self, run: &Run) -> anyhow::Result<Looked> {
        if run.order.runner != Runner::Direct {
            return Ok(Looked::Lost);
        }
        // What still runs is attended to again, not gone on with beside it.
        if let Standing::Working | Standing::Foreign =
            agent::executor(&self.app, run.id).standing().await?
        {
            self.attend(run.id, Job::again(&self.app, run));
            return Ok(Looked::Working);
        }
        match Job::resume(&self.app, run).await {
            Ok(job) => {
                self.attend(run.id, job);
                Ok(Looked::Working)
            }
            Err(error) => {
                tracing::warn!(run = run.id, %error, "could not go on with work run directly");
                Ok(Looked::Lost)
            }
        }
    }

    /// End what is left of runs that are over, or of which nothing is known.
    pub(super) async fn sweep(&self) -> anyhow::Result<()> {
        let workers = &self.app.workers;
        let mut open = BTreeSet::new();
        for run in workers.runs.open()? {
            if !run.order.runner.runs_here() || run.state == State::Ending {
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
                Runner::Agent | Runner::Direct => {
                    if let Some(ticket) = ask_start(&self.app, run).await? {
                        return Ok(Began::Asking(ticket));
                    }
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
                Runner::Agent | Runner::Direct => self.find_agent(run).await,
            }
        })
    }

    fn look<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Looked>> {
        Box::pin(async move {
            match run.order.runner {
                Runner::Adroit => adroit::look(&self.app, run).await,
                Runner::Agent | Runner::Direct => self.look_at_work(run).await,
            }
        })
    }

    fn went_on<'a>(&'a self, ended: &'a [(Run, bool)]) -> BoxFuture<'a, Vec<i64>> {
        Box::pin(adroit::went_on(&self.app, ended, &self.read))
    }

    fn go_on<'a>(&'a self, run: &'a Run, after: After) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            match run.order.runner {
                Runner::Adroit => adroit::go_on(&self.app, run, after).await,
                Runner::Agent | Runner::Direct => {
                    anyhow::bail!("an agent has no session to go on")
                }
            }
        })
    }

    fn stop(&self, run: &Run) {
        if !run.order.runner.runs_here() {
            return;
        }
        // Dropped, so that nothing more is done to the outside.
        self.finishings().remove(&run.id);
        let working = self.agents().remove(&run.id);
        // Its turn is over when its finish was read. One nobody attends to is
        // not known to be in a turn, and is not waited for.
        let ended = working
            .as_ref()
            .is_none_or(|w| w.finish.lock().unwrap_or_else(|p| p.into_inner()).is_some());
        if let Some(working) = working {
            working.abort.abort();
        }
        // Taken back here, not when it is cleared: ending it may take a while.
        agent::revoke(&self.app, run.id);
        if run.order.runner == Runner::Direct {
            let (app, run) = (self.app.clone(), run.id);
            tokio::spawn(async move {
                if let Err(error) = agent::stop_direct(&app, run, ended).await {
                    tracing::warn!(run, %error, "could not end work run directly");
                } else {
                    return;
                }
                // What cannot be ended now is ended at the next start.
                for _ in 0..CLEAR_TRIES {
                    tokio::time::sleep(CLEAR_AGAIN).await;
                    match agent::clear(&app, run).await {
                        Ok(()) => return,
                        Err(error) => tracing::warn!(run, %error, "could not end a worker"),
                    }
                }
            });
            return;
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
