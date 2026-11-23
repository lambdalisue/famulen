//! The engine: the Slack connections, the schedules and the secretary's
//! session, built again whenever a setting changes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::FutureExt;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinSet;

use super::keep;
use super::open::sealed_places;
use super::prompt::system_prompt;
use super::session::{SessionLinks, run_session};
use super::slack::read_channel;
use crate::app::{Answerer, App};
use crate::approvals::Answer;
use crate::claude::session::{Options, Services};
use crate::claude::skills;
use crate::config::Config;
use crate::dream_settings::DreamSettings;
use crate::gateway::Arrival;
use crate::hub::OwnTools;
use crate::models::{ModelKind, ModelSpec, Role};
use crate::schedule::Scheduler;
use crate::settings::Effective;
use crate::slack::web::GatewaySlack;

const SCHEDULE_TICK: Duration = Duration::from_secs(20);

/// What the gateway hands the channel, waiting for the engine that reads it.
/// The engine is built again whenever a setting changes; what arrives
/// meanwhile waits here.
pub(super) type ChannelInbox = Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Arrival>>>;

pub(super) fn start_engine(
    config: &Config,
    app: &App,
    tools: Arc<dyn OwnTools>,
    links: &SessionLinks,
    inbox: ChannelInbox,
) -> anyhow::Result<JoinSet<()>> {
    let store = &app.store;
    let effective = app.settings.effective()?;
    let tz = effective.timezone;
    let workspaces = app.workspaces()?;
    let main = workspaces.iter().find(|w| w.main);
    let clients = app.slack_clients()?;
    let console = app.console.clone();
    let (bin, model, effort, spec) = secretary(app, &effective);
    let mut services = Services::new(
        clients.as_ref(),
        store.clone(),
        app.permissions.clone(),
        main.map(|m| m.owner.clone()).unwrap_or_default(),
        config.memory_dir(),
        tools,
        Some(console.clone()),
    );
    let clients = clients.map(Arc::new);
    *app.slack.write().unwrap_or_else(|p| p.into_inner()) = clients.clone();
    services.usage = Some((app.models.clone(), spec));
    services.turn_ended = Some(app.inputs.clone());
    services.in_flight = links.in_flight.clone();
    services.gateway = Some(app.gateway.clone());
    let services = Arc::new(services);
    install_answerer(app, &services);
    let scheduler = Scheduler::new(store.clone(), app.records.clone(), tz);
    scheduler.list(Utc::now())?;
    let arrived = app.inputs.clone();
    let mut engine = JoinSet::new();

    engine.spawn({
        let services = services.clone();
        let mut held = app.gateway.subscribe_held();
        async move {
            // Tickets held while no engine ran are put to the owner first.
            if let Err(error) = services.approvals.present_held().await {
                tracing::warn!(%error, "could not put the held calls to the owner");
            }
            loop {
                let result = match held.recv().await {
                    Ok(ticket) => services.approvals.present(&ticket).await,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        services.approvals.present_held().await
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if let Err(error) = result {
                    tracing::warn!(%error, "could not put a held call to the owner");
                }
            }
        }
    });

    engine.spawn({
        let (app, services, clients) = (app.clone(), services.clone(), clients.clone());
        async move {
            let mut inbox = inbox.lock().await;
            while let Some(arrival) = inbox.recv().await {
                if let Err(error) = read_channel(&app, &services, clients.as_deref(), &arrival).await {
                    tracing::warn!(%error, connection = %arrival.connection, "could not read what arrived for the channel");
                }
            }
        }
    });

    engine.spawn({
        let (store, arrived) = (store.clone(), arrived.clone());
        async move {
            let mut tick = tokio::time::interval(SCHEDULE_TICK);
            loop {
                tick.tick().await;
                match scheduler.due(Utc::now()) {
                    Ok(inputs) => {
                        for input in inputs {
                            if let Err(error) = keep(&store, &arrived, &input) {
                                tracing::warn!(%error, "could not keep a scheduled input");
                            }
                        }
                    }
                    Err(error) => tracing::warn!(%error, "could not check the schedules"),
                }
            }
        }
    });

    let today = Utc::now().with_timezone(&tz).date_naive();
    let options = Options {
        bin,
        model,
        effort,
        workdir: config.workdir(),
        system_prompt: system_prompt(
            &app.aide_name(),
            &store.state("persona.tone")?.unwrap_or_default(),
            &config.memory_dir(),
            today,
            DreamSettings::load(store)?.core_lines as usize,
        ),
        add_dirs: vec![config.memory_dir()],
        resume: None,
        connections: app.gateway.served()?,
        sealed: sealed_places(config)?,
        writable: crate::writable::places(store)?,
        plugin_dir: owners_skills(&config.workdir()),
    };
    engine.spawn({
        let (store, workgroups, links) = (store.clone(), app.workgroups.clone(), links.clone());
        async move {
            if let Err(error) =
                run_session(options, services, store, workgroups, arrived, links, tz).await
            {
                tracing::error!(%error, "the session loop stopped");
            }
        }
    });
    Ok(engine)
}

/// The secretary's executable, model and effort: the agent CLI assigned to
/// the role and the effort chosen with it, else the settings.
fn secretary(
    app: &App,
    effective: &Effective,
) -> (String, Option<String>, Option<String>, ModelSpec) {
    let assigned = app
        .models
        .assignment(Role::Secretary)
        .ok()
        .flatten()
        .and_then(|a| Some((app.models.model(&a.model).ok().flatten()?, a.effort)))
        .filter(|(m, _)| Role::Secretary.accepts(&m.spec));
    match assigned {
        Some((m, effort)) => (
            m.spec
                .endpoint
                .clone()
                .filter(|e| !e.is_empty())
                .unwrap_or_else(|| effective.claude.bin.clone()),
            Some(m.spec.model.clone())
                .filter(|m| !m.is_empty())
                .or(effective.claude.model.clone()),
            effort.or(effective.claude.effort.clone()),
            m.spec.clone(),
        ),
        None => (
            effective.claude.bin.clone(),
            effective.claude.model.clone(),
            effective.claude.effort.clone(),
            ModelSpec {
                id: "claude-code".into(),
                name: "Claude Code".into(),
                kind: ModelKind::AgentCli,
                endpoint: Some(effective.claude.bin.clone()),
                model: effective.claude.model.clone().unwrap_or_default(),
                cli: Default::default(),
                price: Default::default(),
            },
        ),
    }
}

/// Put the owner's skills into `workdir` for the secretary, and give the
/// folder of plugins Claude Code should load, when there are any. A failure
/// leaves the secretary without skills rather than without a session.
fn owners_skills(workdir: &Path) -> Option<PathBuf> {
    let sources = skills::Sources::from_env(|name| std::env::var(name).ok())?;
    match skills::install(&sources, workdir) {
        Ok(installed) => {
            tracing::info!(skills = installed.skills.len(), plugins = ?installed.plugins, "gave the secretary the owner's skills");
            (!installed.plugins.is_empty()).then(|| workdir.join(skills::PLUGINS_DIR))
        }
        Err(error) => {
            tracing::warn!(%error, "could not give the secretary the owner's skills");
            None
        }
    }
}

/// Let the console's approval buttons reach this engine's approvals.
fn install_answerer(app: &App, services: &Arc<Services<GatewaySlack>>) {
    let (services, store, arrived) = (services.clone(), app.store.clone(), app.inputs.clone());
    let answerer: Answerer = Arc::new(move |id: i64, answer: Answer, replies: Vec<String>| {
        let (services, store, arrived) = (services.clone(), store.clone(), arrived.clone());
        async move {
            let told = if replies.is_empty() {
                services.approvals.answer_by_id(id, answer).await?
            } else {
                services.approvals.fill_by_id(id, replies).await?
            };
            match told {
                Some(input) => {
                    keep(&store, &arrived, &input)?;
                    Ok(true)
                }
                None => Ok(false),
            }
        }
        .boxed()
    });
    *app.answerer.lock().unwrap_or_else(|p| p.into_inner()) = Some(answerer);
}
