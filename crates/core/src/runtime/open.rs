//! Opening what Famulen keeps.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Notify;

use super::dream::DREAMING_KEY;
use crate::app::App;
use crate::calendars::Calendars;
use crate::config::Config;
use crate::connections::Connections;
use crate::console_log::ConsoleLog;
use crate::journal::Journal;
use crate::lookout::Lookout;
use crate::models::Models;
use crate::permissions::Permissions;
use crate::records::Records;
use crate::settings::Settings;
use crate::store::Store;
use crate::workgroups::Workgroups;

/// Open everything Famulen keeps where `config` says, without starting
/// anything.
pub fn open_app(config: &Config) -> anyhow::Result<App> {
    Ok(open(config.clone())?.1)
}

/// Where no tool of the secretary's may reach: the gateway's own data and
/// the file the secrets are started from.
pub(super) fn sealed_places(config: &Config) -> anyhow::Result<Vec<PathBuf>> {
    Ok(vec![
        config.gateway_dir(),
        std::env::current_dir()?.join(".env"),
    ])
}

pub(super) fn open(config: Config) -> anyhow::Result<(Config, App)> {
    let config = config.prepare()?;
    let store = Store::open(config.database_path())?;
    let config = crate::notebook_place::settle(config, &store)?;
    let app = open_app_on(&config, store)?;
    Ok((config, app))
}

fn open_app_on(config: &Config, store: Store) -> anyhow::Result<App> {
    let notebook = crate::notebook::open(config.notebook_dir());
    crate::notebook::ensure(&notebook)?;
    let gateway_store =
        crate::gateway::store::Store::open(config.gateway_dir().join("gateway.sqlite3"))?;
    let settings = Settings::new(store.clone(), config.settings_base());
    let records = Records::new(notebook.clone());
    let workgroups = Workgroups::new(store.clone(), notebook.clone())?;
    let calendars = Calendars::new(store.clone(), records.clone());
    let connections = Connections::new(gateway_store.clone());
    let console = ConsoleLog::new(store.clone());
    let lookout = Lookout::new(store.clone());
    let http = reqwest::Client::new();
    let vault = match &config.vault_key {
        Some(key) => Some(crate::gateway::Vault::open(
            config.gateway_dir().join("vault"),
            key,
        )?),
        None => {
            tracing::warn!(
                "no {} is set, so secrets cannot be kept in the vault; set it to a key such as {}",
                crate::gateway::KEY_VAR,
                crate::gateway::VaultKey::generate()
            );
            None
        }
    };
    let gateway = crate::gateway::Gateway::new(
        gateway_store.clone(),
        connections.clone(),
        http.clone(),
        vault,
    )
    .with_reserved([crate::hub::SERVER_NAME])
    .with_definitions(Some(&config.gateway_dir().join("services")), &[]);
    gateway.grant(
        crate::permissions::SECRETARY,
        crate::gateway::ANY,
        crate::gateway::ANY,
    )?;
    let app = App {
        store: store.clone(),
        records: records.clone(),
        workgroups: workgroups.clone(),
        calendars: calendars.clone(),
        connections: connections.clone(),
        console: console.clone(),
        permissions: Permissions::new(store.clone(), gateway.clone())
            .with_memory_dir(config.memory_dir())
            .with_notebook_dir(config.notebook_dir())
            .with_sealed(sealed_places(config)?, config.workdir()),
        models: Models::new(store.clone()).with_gateway(gateway.clone()),
        lookout: lookout.clone(),
        triage: crate::triage::Triage::new(store.clone()),
        journal: Journal::new(store.clone()),
        settings: settings.clone(),
        memory_dir: config.memory_dir(),
        notebook: notebook.clone(),
        index: famulen_notebook::index::Index::open(&notebook)?,
        notebook_place: crate::notebook_place::NotebookPlace::new(store.clone(), config),
        changed: Arc::new(Notify::new()),
        inputs: Arc::new(Notify::new()),
        http: http.clone(),
        gateway,
        answerer: Default::default(),
        slack: Default::default(),
        console_url: config.console_url.clone(),
        receive_url: config.receive_url.clone(),
    };
    let definitions = app.gateway.definitions();
    let loaded: Vec<&str> = definitions.list().map(|d| d.id.as_str()).collect();
    tracing::info!(definitions = ?loaded, "service definitions read");
    for (id, why) in definitions.broken() {
        tracing::warn!(%id, %why, "a service definition did not read");
    }
    let sealed = app.gateway.seal_existing()?;
    if sealed > 0 {
        tracing::info!(sealed, "moved connection secrets into the vault");
    }
    app.models.seed_defaults()?;
    app.gateway
        .set_assessor(Arc::new(crate::assessing::ModelAssessor::new(
            app.models.clone(),
            store.clone(),
            settings.clone(),
            config.memory_dir(),
        )));
    // A dream cut off by the last stop is not running any more.
    store.clear_state(DREAMING_KEY)?;
    Ok(app)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::Path;

    use super::*;

    /// Famulen opened on an empty data directory, with nothing started.
    pub(crate) fn app(dir: &Path) -> App {
        let config = Config::load(
            None,
            |_| None,
            crate::config::Defaults {
                data_dir: dir.to_owned(),
                timezone: chrono_tz::Asia::Tokyo,
            },
        )
        .unwrap();
        open_app(&config).unwrap()
    }
}
