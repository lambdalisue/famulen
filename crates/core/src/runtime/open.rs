//! Opening what Famulen keeps.

use std::path::PathBuf;
use std::sync::Arc;

use famulen_executor::Executors;
use tokio::sync::Notify;

use super::dream::DREAMING_KEY;
use crate::app::App;
use crate::calendars::Calendars;
use crate::claude::skills::claude_dir;
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
use crate::workers::Kept;
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
        executors_dir(config),
        std::env::current_dir()?.join(".env"),
    ])
}

/// The hosts no agent doing a delegated task may reach: this machine, with
/// the API, the console and the receivers on it, by every name it is known
/// to go by.
fn hosts_kept_from_workers(config: &Config) -> Vec<String> {
    let mut hosts: Vec<String> = [
        "localhost",
        "*.localhost",
        "127.0.0.1",
        "0.0.0.0",
        "::1",
        "[::1]",
        "::",
        "[::]",
        "host.docker.internal",
    ]
    .map(String::from)
    .into();
    let urls = [&config.console_url, &config.receive_url];
    hosts.extend(
        urls.into_iter()
            .flatten()
            .filter_map(|u| url::Url::parse(u).ok()?.host_str().map(str::to_owned)),
    );
    hosts.extend(config.api.hosts.iter().cloned());
    for listen in std::iter::once(config.api.listen).chain(config.receive_listen) {
        hosts.push(listen.ip().to_string());
        if listen.is_ipv6() {
            hosts.push(format!("[{}]", listen.ip()));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    hosts.retain(|h| seen.insert(h.clone()));
    hosts
}

/// Where the processes agents run under keep what they keep and are asked.
fn executors_dir(config: &Config) -> PathBuf {
    config.data_dir.join("executors")
}

fn executors(config: &Config, store: &Store) -> anyhow::Result<Executors> {
    const KEY: &str = "executors.key";
    let key = match store.state(KEY)? {
        Some(key) => key,
        None => {
            let key = format!("{:032x}", rand::random::<u128>());
            store.set_state(KEY, &key)?;
            key
        }
    };
    Ok(Executors::new(
        executors_dir(config),
        key,
        crate::workers::executor_program()?,
    ))
}

/// Claude Code's own directory on this system.
fn claudes() -> Option<PathBuf> {
    claude_dir(|name| match name {
        "HOME" => Some(crate::sealed::home()?.display().to_string()),
        _ => std::env::var(name).ok(),
    })
}

/// Where Claude Code keeps what was said, in its own directory.
const SAID: &[&str] = &["projects", "history.jsonl", "file-history", "todos"];

/// Where no tool of an agent doing a delegated task may reach, on top of
/// that: the notebook, of which it reads only what it was handed, and what
/// Famulen keeps of the secretary.
fn sealed_from_workers(config: &Config) -> anyhow::Result<Vec<PathBuf>> {
    let mut places = sealed_places(config)?;
    places.push(config.notebook_dir());
    places.push(config.workdir());
    places.extend(config.workers_sealed.iter().cloned());
    let database = config.database_path();
    for suffix in ["", "-wal", "-shm"] {
        let mut path = database.clone().into_os_string();
        path.push(suffix);
        places.push(path.into());
    }
    // What was said to the secretary and to other workers. The rest of the
    // directory is read by the model CLI's own shell.
    if let Some(claude) = claudes() {
        places.extend(SAID.iter().map(|name| claude.join(name)));
    }
    Ok(places)
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
    // Which tools a run is handed is kept by the run; its calls are judged
    // like any other's.
    gateway.grant(
        crate::permissions::WORKER,
        crate::gateway::ANY,
        crate::gateway::ANY,
    )?;
    let permissions = Permissions::new(store.clone(), gateway.clone())
        .with_memory_dir(config.memory_dir())
        .with_notebook_dir(config.notebook_dir())
        .with_sealed(sealed_places(config)?, config.workdir());
    let app = App {
        store: store.clone(),
        records: records.clone(),
        workgroups: workgroups.clone(),
        calendars: calendars.clone(),
        connections: connections.clone(),
        working: crate::working::Working::new(store.clone(), console.clone()),
        console: console.clone(),
        conversations: {
            let conversations = crate::conversations::Conversations::new(store.clone());
            conversations.adopt_the_one_before()?;
            conversations
        },
        attachments: crate::attachments::Attachments::new(store.clone(), config.workdir()),
        link_previews: crate::link_previews::LinkPreviews::new(
            gateway.clone(),
            permissions.clone(),
        ),
        pictures: crate::pictures::Pictures::new(store.clone(), gateway.clone()),
        holding_pictures: crate::purchases::pictures::HoldingPictures::new(
            store.clone(),
            gateway.clone(),
        ),
        notices: crate::notices::Notices::new(store.clone(), permissions.clone()),
        permissions,
        models: Models::new(store.clone()).with_gateway(gateway.clone()),
        workers: crate::workers::Workers::new(
            store.clone(),
            config.data_dir.join("workers"),
            executors(config, &store)?,
            Kept {
                places: sealed_from_workers(config)?,
                hosts: hosts_kept_from_workers(config),
            },
        ),
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
        slack_app_icons: Default::default(),
        slack_looks: crate::slack::looks::Looks::new(store.clone()),
        slack_names: Default::default(),
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
        crate::notebook::testing::lay(&config.notebook_dir());
        open_app(&config).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use crate::notebook::testing::TempDir;

    #[test]
    fn what_was_said_in_other_sessions_is_sealed_from_workers() {
        let dir = TempDir::new("sealed-said");
        let app = crate::runtime::testing::app(dir.path());
        let claude = super::claudes().unwrap();
        for said in ["projects", "history.jsonl"] {
            assert!(
                app.workers.kept.places.contains(&claude.join(said)),
                "{said}"
            );
        }
    }

    #[test]
    fn famulen_and_the_notebook_wherever_it_is_cloned_are_kept_from_workers() {
        let dir = TempDir::new("kept-from-workers");
        let config = crate::config::Config::load(
            Some(
                "console_url = \"https://famulen.example.ts.net\"\n[api]\nhosts = [\"backend\"]\n",
            ),
            |name| (name == "HOME").then(|| crate::paths::at("/home/me")),
            crate::config::Defaults {
                data_dir: dir.path().to_owned(),
                timezone: chrono_tz::Asia::Tokyo,
            },
        )
        .unwrap();
        let places = super::sealed_from_workers(&config).unwrap();
        let clone = std::path::PathBuf::from(crate::paths::at("/home/me")).join("notebook");
        for place in [
            config.gateway_dir(),
            config.notebook_dir(),
            config.database_path(),
            super::executors_dir(&config),
            clone,
        ] {
            assert!(places.contains(&place), "{}", place.display());
        }
        let hosts = super::hosts_kept_from_workers(&config);
        for host in [
            "localhost",
            "127.0.0.1",
            "[::1]",
            "famulen.example.ts.net",
            "backend",
        ] {
            assert!(hosts.iter().any(|h| h == host), "{host}");
        }
    }
}
