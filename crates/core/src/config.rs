//! What famulen runs with: the environment (and `.env`), an optional
//! configuration file, and defaults.
//!
//! The environment wins over the file, and the file over the defaults, so a
//! file can be shared while each machine keeps its own values. Secrets never
//! go in the file: the vault key is read from the environment only, and the
//! apps the service definitions name are read by the gateway itself.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context as _;
use chrono_tz::Tz;
use serde::Deserialize;

pub use famulen_gateway::process::hide_secrets;

/// Where to keep data, instead of the default.
pub const DATA_DIR_ENV: &str = "FAMULEN_DATA_DIR";
/// Where to keep the notebook until the owner chooses another place on the
/// console, instead of inside the data directory.
pub const NOTEBOOK_DIR_ENV: &str = "FAMULEN_NOTEBOOK_DIR";
/// The owner's time zone, instead of this machine's.
pub const TIMEZONE_ENV: &str = "FAMULEN_TIMEZONE";
/// The model Claude Code runs, as `claude --model` takes it.
pub const MODEL_ENV: &str = "FAMULEN_CLAUDE_MODEL";
/// How much the model thinks, as `claude --effort` takes it.
pub const EFFORT_ENV: &str = "FAMULEN_CLAUDE_EFFORT";
/// Where the owner opens the console, for logins to come back to. Slack
/// wants it on https.
pub const CONSOLE_URL_ENV: &str = "FAMULEN_CONSOLE_URL";
/// Where the API listens, over the file's `[api] listen`.
pub const LISTEN_ENV: &str = "FAMULEN_LISTEN";
/// The other host names the API answers to, comma separated, over the file's
/// `[api] hosts`.
pub const API_HOSTS_ENV: &str = "FAMULEN_API_HOSTS";

/// Where the gateway's HTTP receivers listen.
pub const RECEIVE_LISTEN_ENV: &str = "FAMULEN_RECEIVE_LISTEN";
/// Where the receivers are reached from outside.
pub const RECEIVE_URL_ENV: &str = "FAMULEN_RECEIVE_URL";

/// The efforts `claude --effort` takes.
pub const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Where famulen keeps its database and the agent's working directory.
    pub data_dir: PathBuf,
    /// Where the notebook is kept unless the console says otherwise, when
    /// that is not inside the data directory.
    notebook_dir: Option<PathBuf>,
    /// Where the notebook turned out to be.
    notebook_at: Option<PathBuf>,
    /// The owner's time zone, for routines and the times the agent reads.
    pub timezone: Tz,
    pub claude: Claude,
    pub api: ApiConfig,
    /// Where the owner opens the console, if set.
    pub console_url: Option<String>,
    /// The key the vault seals secrets with, from the environment only.
    pub vault_key: Option<crate::gateway::VaultKey>,
    /// Where the gateway's HTTP receivers listen; none opens none.
    pub receive_listen: Option<SocketAddr>,
    /// Where the receivers are reached from outside, if the owner made
    /// them reachable: calendar changes are asked to be sent there.
    pub receive_url: Option<String>,
}

/// The configuration file, where every setting may be left out.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    data_dir: Option<PathBuf>,
    notebook_dir: Option<PathBuf>,
    timezone: Option<Tz>,
    #[serde(default)]
    claude: Claude,
    #[serde(default)]
    api: ApiConfig,
    console_url: Option<String>,
}

/// What applies when neither the environment nor the file says.
#[derive(Debug, Clone, PartialEq)]
pub struct Defaults {
    pub data_dir: PathBuf,
    pub timezone: Tz,
}

impl Defaults {
    /// `$XDG_DATA_HOME/famulen` (or `~/.local/share/famulen`) and this
    /// machine's time zone, falling back to UTC when it cannot be read.
    pub fn from_system(env: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let data_home = match env("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => {
                let home = env("HOME")
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("HOME is not set; set {DATA_DIR_ENV}"))?;
                PathBuf::from(home).join(".local/share")
            }
        };
        let timezone = iana_time_zone::get_timezone()
            .ok()
            .and_then(|name| name.parse().ok())
            .unwrap_or(Tz::UTC);
        Ok(Self {
            data_dir: data_home.join("famulen"),
            timezone,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ApiConfig {
    /// Where the API listens. Only the console and the owner's own tools
    /// should reach it: it has no login of its own.
    pub listen: SocketAddr,
    /// The host names it answers to besides this machine's own.
    pub hosts: Vec<String>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 7790)),
            hosts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Claude {
    /// The Claude Code executable.
    pub bin: String,
    /// The model, as `claude --model` takes it. Claude Code's default when unset.
    pub model: Option<String>,
    /// How much the model thinks (`low` to `max`), as `claude --effort` takes
    /// it. Lower answers sooner. Claude Code's default when unset.
    pub effort: Option<String>,
}

impl Default for Claude {
    fn default() -> Self {
        Self {
            bin: "claude".into(),
            model: None,
            effort: None,
        }
    }
}

impl Config {
    /// Put together the configuration from the file's text (if any), the
    /// environment and the defaults.
    pub fn load(
        file: Option<&str>,
        env: impl Fn(&str) -> Option<String>,
        defaults: Defaults,
    ) -> anyhow::Result<Self> {
        let file: File = match file {
            Some(text) => toml::from_str(text)?,
            None => File::default(),
        };
        let env = |name: &str| env(name).filter(|v| !v.trim().is_empty());

        let timezone = match env(TIMEZONE_ENV) {
            Some(name) => name
                .trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("{TIMEZONE_ENV}: {e}"))?,
            None => file.timezone.unwrap_or(defaults.timezone),
        };
        let data_dir = env(DATA_DIR_ENV)
            .map(PathBuf::from)
            .or(file.data_dir)
            .unwrap_or(defaults.data_dir);
        let notebook_dir = env(NOTEBOOK_DIR_ENV)
            .map(|dir| PathBuf::from(dir.trim()))
            .or(file.notebook_dir)
            .map(|dir| match dir.strip_prefix("~") {
                Ok(rest) => env("HOME")
                    .map(|home| PathBuf::from(home).join(rest))
                    .ok_or_else(|| {
                        anyhow::anyhow!("HOME is not set, so {} is nowhere", dir.display())
                    }),
                Err(_) => Ok(dir),
            })
            .transpose()?;
        let mut claude = file.claude;
        if let Some(model) = env(MODEL_ENV) {
            claude.model = Some(model.trim().to_owned());
        }
        if let Some(effort) = env(EFFORT_ENV) {
            claude.effort = Some(effort.trim().to_owned());
        }
        if let Some(effort) = &claude.effort {
            anyhow::ensure!(
                EFFORTS.contains(&effort.as_str()),
                "effort must be one of {}: {effort}",
                EFFORTS.join(", ")
            );
        }

        let mut api = file.api;
        if let Some(listen) = env(LISTEN_ENV) {
            api.listen = listen
                .trim()
                .parse()
                .map_err(|e| anyhow::anyhow!("{LISTEN_ENV} must be an address and port: {e}"))?;
        }
        if let Some(hosts) = env(API_HOSTS_ENV) {
            let names = hosts.split(',').map(str::trim);
            api.hosts = names.filter(|h| !h.is_empty()).map(Into::into).collect();
        }
        Ok(Self {
            data_dir,
            notebook_dir,
            notebook_at: None,
            timezone,
            claude,
            api,
            console_url: env(CONSOLE_URL_ENV)
                .or(file.console_url)
                .map(|u| u.trim().trim_end_matches('/').to_owned()),
            vault_key: env(crate::gateway::KEY_VAR)
                .map(crate::gateway::VaultKey::parse)
                .transpose()?,
            receive_listen: env(RECEIVE_LISTEN_ENV)
                .map(|l| {
                    l.trim().parse().map_err(|e| {
                        anyhow::anyhow!("{RECEIVE_LISTEN_ENV} must be an address and port: {e}")
                    })
                })
                .transpose()?,
            receive_url: env(RECEIVE_URL_ENV).map(|u| u.trim().trim_end_matches('/').to_owned()),
        })
    }

    /// Make the data directory, and name it by its real path: what the
    /// secretary may write is judged by path, and a link on the way would make
    /// the same place look like another.
    pub fn prepare(self) -> anyhow::Result<Self> {
        let data_dir = std::fs::create_dir_all(&self.data_dir)
            .and_then(|()| self.data_dir.canonicalize())
            .with_context(|| format!("preparing {}", self.data_dir.display()))?;
        Ok(Self {
            data_dir: crate::paths::plain(data_dir),
            ..self
        })
    }

    /// The starting values for the settings the console can change.
    pub fn settings_base(&self) -> crate::settings::Base {
        crate::settings::Base {
            claude: self.claude.clone(),
            timezone: self.timezone,
        }
    }

    /// The notebook: everything Famulen knows about the owner.
    pub fn notebook_dir(&self) -> PathBuf {
        self.notebook_at
            .clone()
            .unwrap_or_else(|| self.default_notebook_dir())
    }

    /// Where the notebook is kept unless the console says otherwise.
    pub fn default_notebook_dir(&self) -> PathBuf {
        self.notebook_dir
            .clone()
            .unwrap_or_else(|| self.data_dir.join("notebook"))
    }

    pub(crate) fn with_notebook_at(self, place: PathBuf) -> Self {
        Self {
            notebook_at: Some(place),
            ..self
        }
    }

    /// Where the agent keeps what it should remember, inside the notebook.
    pub fn memory_dir(&self) -> PathBuf {
        self.notebook_dir().join(crate::notebook::MEMORY)
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("famulen.sqlite3")
    }

    /// Where the gateway keeps its database and its vault.
    pub fn gateway_dir(&self) -> PathBuf {
        self.data_dir.join("gateway")
    }

    /// The directory Claude Code runs in.
    pub fn workdir(&self) -> PathBuf {
        self.data_dir.join("workspace")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn defaults() -> Defaults {
        Defaults {
            data_dir: "/default".into(),
            timezone: chrono_tz::Europe::Paris,
        }
    }

    fn load(file: Option<&str>, env: &[(&str, &str)]) -> anyhow::Result<Config> {
        let env: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::load(file, |name| env.get(name).cloned(), defaults())
    }

    #[test]
    fn the_console_url_is_kept_without_its_last_slash() {
        let config = load(
            Some("console_url = \"https://famulen.example.ts.net/\"\n"),
            &[],
        )
        .unwrap();
        assert_eq!(
            config.console_url.as_deref(),
            Some("https://famulen.example.ts.net")
        );
    }

    #[test]
    fn the_environment_names_where_the_api_listens() {
        let config = load(
            Some("[api]\nlisten = \"127.0.0.1:7000\"\n"),
            &[(LISTEN_ENV, "0.0.0.0:7790")],
        )
        .unwrap();
        assert_eq!(config.api.listen.to_string(), "0.0.0.0:7790");
        assert!(load(None, &[(LISTEN_ENV, "nowhere")]).is_err());
    }

    #[test]
    fn the_environment_names_the_hosts_the_api_answers_to() {
        let file = "[api]\nhosts = [\"a\"]\n";
        assert_eq!(load(Some(file), &[]).unwrap().api.hosts, ["a"]);
        let env = [(API_HOSTS_ENV, "backend, b.example,")];
        let hosts = load(Some(file), &env).unwrap().api.hosts;
        assert_eq!(hosts, ["backend", "b.example"]);
    }

    fn parse(text: &str) -> anyhow::Result<Config> {
        load(Some(text), &[])
    }

    #[test]
    fn the_environment_alone_is_enough() {
        let config = load(None, &[]).unwrap();
        assert_eq!(config.data_dir, PathBuf::from("/default"));
        assert_eq!(config.timezone, chrono_tz::Europe::Paris);
        assert_eq!(config.claude, Claude::default());
        assert_eq!(config.api.listen.to_string(), "127.0.0.1:7790");
    }

    #[test]
    fn the_receivers_listen_only_where_the_environment_says() {
        assert_eq!(load(None, &[]).unwrap().receive_listen, None);
        assert_eq!(
            load(None, &[(RECEIVE_LISTEN_ENV, "127.0.0.1:7800")])
                .unwrap()
                .receive_listen,
            Some("127.0.0.1:7800".parse().unwrap())
        );
        assert!(load(None, &[(RECEIVE_LISTEN_ENV, "nowhere")]).is_err());
    }

    #[test]
    fn the_receivers_outside_address_comes_from_the_environment() {
        assert_eq!(load(None, &[]).unwrap().receive_url, None);
        assert_eq!(
            load(None, &[(RECEIVE_URL_ENV, "https://me.example.ts.net/")])
                .unwrap()
                .receive_url
                .as_deref(),
            Some("https://me.example.ts.net")
        );
    }

    #[test]
    fn the_vault_key_comes_from_the_environment_and_must_be_one() {
        let key = crate::gateway::VaultKey::generate();
        assert!(
            load(None, &[(crate::gateway::KEY_VAR, &key)])
                .unwrap()
                .vault_key
                .is_some()
        );
        assert!(load(None, &[]).unwrap().vault_key.is_none());
        assert!(load(None, &[(crate::gateway::KEY_VAR, "short")]).is_err());
    }

    #[test]
    fn an_empty_variable_counts_as_unset() {
        assert_eq!(
            load(None, &[(CONSOLE_URL_ENV, "  ")]).unwrap().console_url,
            None
        );
    }

    #[test]
    fn the_environment_wins_over_the_file() {
        let file = "data_dir = \"/file\"\ntimezone = \"Asia/Tokyo\"\n";
        let config = load(
            Some(file),
            &[(DATA_DIR_ENV, "/env"), (TIMEZONE_ENV, "America/New_York")],
        )
        .unwrap();
        assert_eq!(config.data_dir, PathBuf::from("/env"));
        assert_eq!(config.timezone, chrono_tz::America::New_York);
    }

    #[test]
    fn the_file_wins_over_the_defaults() {
        let config = parse("data_dir = \"/file\"\ntimezone = \"Asia/Tokyo\"\n").unwrap();
        assert_eq!(config.data_dir, PathBuf::from("/file"));
        assert_eq!(config.timezone, chrono_tz::Asia::Tokyo);
    }

    #[test]
    fn an_unknown_time_zone_in_the_environment_names_the_variable() {
        let err = load(None, &[(TIMEZONE_ENV, "Mars/Base")]).unwrap_err();
        assert!(err.to_string().contains(TIMEZONE_ENV));
    }

    #[test]
    fn data_lives_under_xdg_data_home_or_the_home_directory() {
        let from_xdg =
            Defaults::from_system(|name| (name == "XDG_DATA_HOME").then(|| "/xdg".into())).unwrap();
        assert_eq!(from_xdg.data_dir, PathBuf::from("/xdg/famulen"));
        let from_home =
            Defaults::from_system(|name| (name == "HOME").then(|| "/home/me".into())).unwrap();
        assert_eq!(
            from_home.data_dir,
            PathBuf::from("/home/me/.local/share/famulen")
        );
        assert!(Defaults::from_system(|_| None).is_err());
    }

    #[test]
    fn claude_runs_from_path_with_its_default_model_unless_told_otherwise() {
        assert_eq!(Claude::default().bin, "claude");
        assert_eq!(Claude::default().model, None);
    }

    #[test]
    fn the_model_can_be_chosen() {
        let config = load(Some("[claude]\nmodel = \"opus\"\n"), &[]).unwrap();
        assert_eq!(config.claude.model.as_deref(), Some("opus"));
        assert_eq!(config.claude.bin, "claude");
    }

    #[test]
    fn model_and_effort_can_come_from_the_environment() {
        let file = "[claude]\nmodel = \"opus\"\neffort = \"high\"\n";
        let config = load(Some(file), &[(MODEL_ENV, "sonnet"), (EFFORT_ENV, "low")]).unwrap();
        assert_eq!(config.claude.model.as_deref(), Some("sonnet"));
        assert_eq!(config.claude.effort.as_deref(), Some("low"));
    }

    #[test]
    fn an_unknown_effort_is_an_error() {
        assert!(load(None, &[(EFFORT_ENV, "fast")]).is_err());
        assert!(load(Some("[claude]\neffort = \"fast\"\n"), &[]).is_err());
    }

    #[test]
    fn an_unknown_key_is_an_error() {
        assert!(load(Some("slack_token = \"xoxb\"\n"), &[]).is_err());
    }

    #[test]
    fn the_database_workspace_and_memory_live_in_the_data_directory() {
        let config = load(None, &[(DATA_DIR_ENV, "/d")]).unwrap();
        assert_eq!(config.database_path(), PathBuf::from("/d/famulen.sqlite3"));
        assert_eq!(config.workdir(), PathBuf::from("/d/workspace"));
        assert_eq!(config.notebook_dir(), PathBuf::from("/d/notebook"));
        assert_eq!(config.memory_dir(), PathBuf::from("/d/notebook/memory"));
    }

    #[test]
    fn the_notebook_can_live_outside_the_data_directory() {
        let file = "data_dir = \"/d\"\nnotebook_dir = \"/notes/file\"\n";
        let config = load(Some(file), &[]).unwrap();
        assert_eq!(config.notebook_dir(), PathBuf::from("/notes/file"));
        assert_eq!(config.memory_dir(), PathBuf::from("/notes/file/memory"));
        assert_eq!(config.database_path(), PathBuf::from("/d/famulen.sqlite3"));

        let config = load(Some(file), &[(NOTEBOOK_DIR_ENV, "/notes/env")]).unwrap();
        assert_eq!(config.notebook_dir(), PathBuf::from("/notes/env"));
    }

    #[test]
    fn the_notebook_can_be_named_from_the_home_directory() {
        let config = load(
            None,
            &[("HOME", "/home/me"), (NOTEBOOK_DIR_ENV, "~/notes/famulen")],
        )
        .unwrap();
        assert_eq!(
            config.notebook_dir(),
            PathBuf::from("/home/me/notes/famulen")
        );
        assert!(load(None, &[(NOTEBOOK_DIR_ENV, "~/notes")]).is_err());
    }

    #[test]
    fn an_unknown_time_zone_in_the_file_is_an_error() {
        assert!(load(Some("timezone = \"Mars/Base\"\n"), &[]).is_err());
    }
}
