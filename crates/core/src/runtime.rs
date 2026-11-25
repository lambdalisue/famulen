//! The resident process. The console always runs, and so do the chores
//! (calendars, logins, the lookout, the dream). The engine (the Slack
//! connections, the schedules and the secretary's session) is built again
//! whenever a setting changes; it runs with or without Slack.

mod chores;
mod dream;
mod engine;
mod files;
mod open;
mod prompt;
mod receive;
mod session;
mod slack;
mod watch;

pub use dream::*;
pub use open::*;
pub use prompt::*;
pub use receive::*;
pub use watch::*;

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use tokio::sync::{Notify, mpsc};

use crate::app::App;
use crate::claude::session::InFlight;
use crate::config::{ApiConfig, Config};
use crate::hub::OwnTools;
use crate::input::Input;
use crate::notebook_place;
use crate::store::Store;
use session::{RELOAD_KEY, SETTLE_LIMIT, SessionLinks};

const RESTART_AFTER: Duration = Duration::from_secs(5);

/// What serves Famulen to the outside: the API, and the secretary's own
/// tools that stand for it.
pub trait Surface: Send + Sync + 'static {
    /// The secretary's own tools, answering through the API.
    fn tools(&self, app: &App) -> Arc<dyn OwnTools>;
    /// Serve the API where `at` says until it stops.
    fn serve(&self, app: App, at: ApiConfig) -> BoxFuture<'static, anyhow::Result<()>>;
}

/// Why [`run`] came to an end without an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The notebook waits to be moved, which the next start does. The whole
    /// process is to be started again, so that nothing is left holding it.
    StartAgain,
}

pub async fn run(config: Config, surface: Arc<dyn Surface>) -> anyhow::Result<Ended> {
    let (config, app) = open::open(config)?;

    let listen = config.api.listen;
    tokio::spawn({
        let serving = surface.serve(app.clone(), config.api.clone());
        async move {
            if let Err(error) = serving.await {
                tracing::error!(%error, "the API stopped");
            }
        }
    });
    tracing::info!(api = %format!("http://{listen}/"), "serving the API");
    let tools = surface.tools(&app);
    app.permissions.record_own_tools(&tools.list())?;
    tokio::spawn(chores::chores(app.clone()));
    tokio::spawn(chores::read_purchases(app.clone()));
    tokio::spawn(chores::keep_pictures(app.clone()));
    tokio::spawn(chores::keep_holding_pictures(app.clone()));
    tokio::spawn(chores::keep_finding_models(app.clone()));
    tokio::spawn(crate::workers::run(app.clone()));
    tokio::spawn(chores::rebuild_when_redefined(app.clone()));
    let (inbox_tx, inbox_rx) = mpsc::unbounded_channel();
    let inbox: engine::ChannelInbox = Arc::new(tokio::sync::Mutex::new(inbox_rx));
    receive::receive(&app, config.receive_listen, inbox_tx)?;
    let links = SessionLinks {
        in_flight: InFlight::default(),
        reload_now: Arc::new(Notify::new()),
        app: app.clone(),
    };

    loop {
        let mut between_turns = false;
        let mut engine = engine::start_engine(&config, &app, tools.clone(), &links, inbox.clone())?;
        loop {
            tokio::select! {
                // A changed setting is applied between turns: the session loop
                // answers on `reload_now` once no turn runs, so a turn is never
                // cut off by a setting saved while it works.
                () = app.changed.notified() => {
                    app.store.set_state(RELOAD_KEY, "1")?;
                    app.inputs.notify_one();
                }
                () = links.reload_now.notified() => {
                    tracing::info!("settings changed; building the engine again between turns");
                    between_turns = true;
                    break;
                }
                Some(ended) = engine.join_next() => {
                    tracing::warn!(?ended, "part of the engine stopped; starting it again");
                    tokio::time::sleep(RESTART_AFTER).await;
                    break;
                }
            }
        }
        engine.shutdown().await;
        if between_turns && notebook_place::moving(&app.store)?.is_some() {
            tracing::info!("starting again to move the notebook");
            while app.store.state(DREAMING_KEY)?.is_some() {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            let _ = tokio::time::timeout(SETTLE_LIMIT, links.in_flight.settled()).await;
            return Ok(Ended::StartAgain);
        }
    }
}

/// Keep an input for the session. Returns whether it was new.
fn keep(store: &Store, arrived: &Notify, input: &Input) -> anyhow::Result<bool> {
    let Some(id) = store.enqueue(input)? else {
        return Ok(false);
    };
    tracing::info!(id, "input kept");
    arrived.notify_one();
    Ok(true)
}
