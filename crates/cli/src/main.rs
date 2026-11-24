use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use famulen_api::guard::{Hosts, guarded};
use famulen_api::{Api, tools::ApiTools};
use famulen_core::app::App;
use famulen_core::config::{
    ApiConfig, Config, DATA_DIR_ENV, Defaults, LISTEN_ENV, NOTEBOOK_DIR_ENV,
};
use famulen_core::hub::OwnTools;
use famulen_core::runtime::{self, Ended, Surface};
use futures::future::BoxFuture;

fn usage() -> String {
    format!(
        "usage: famulen [config.toml]

Start the Famulen backend: the API the console and the secretary use, and the
secretary's session. The console runs beside it (see compose.yaml). Nothing
has to be set beforehand.

The environment (and a .env file in the current directory) and the optional
config file give starting values; what is chosen on the console wins.
Where data is kept and where the API listens are set only here, and where the
notebook is kept at first:

  {DATA_DIR_ENV}  where to keep data (default: ~/.local/share/famulen)
  {NOTEBOOK_DIR_ENV}  where to keep the notebook until another place is
                    chosen on the console (default: notebook/ in the data directory)
  {LISTEN_ENV}    where the API listens (default: 127.0.0.1:7790)"
    )
}

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let ended = runtime.block_on(start())?;
    // Waits for what is still being written, so nothing is cut off halfway.
    runtime.shutdown_timeout(std::time::Duration::from_secs(30));
    match ended {
        Some(Ended::StartAgain) => start_again(),
        None => Ok(()),
    }
}

/// Become a fresh Famulen with the same arguments and environment.
fn start_again() -> anyhow::Result<()> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.args(std::env::args_os().skip(1));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        Err(command.exec()).context("starting Famulen again")
    }
    #[cfg(not(unix))]
    {
        let status = command.status().context("starting Famulen again")?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

async fn start() -> anyhow::Result<Option<Ended>> {
    let dotenv = std::env::current_dir()
        .context("finding the current directory")
        .and_then(|dir| load_dotenv(&dir))?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    if let Some(path) = dotenv {
        tracing::info!(path = %path.display(), "read .env");
    }

    let path = match std::env::args().nth(1).as_deref() {
        Some("-h" | "--help") => {
            println!("Famulen {}\n\n{}", env!("CARGO_PKG_VERSION"), usage());
            return Ok(None);
        }
        Some(path) => Some(PathBuf::from(path)),
        None => None,
    };
    let text = path
        .as_ref()
        .map(|path| {
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
        })
        .transpose()?;
    let lookup = |name: &str| std::env::var(name).ok();
    let config = Config::load(text.as_deref(), lookup, Defaults::from_system(lookup)?)
        .with_context(|| match &path {
            Some(path) => format!("loading {}", path.display()),
            None => "loading the configuration from the environment".into(),
        })?;
    tracing::info!(data_dir = %config.data_dir.display(), api = %format!("http://{}", config.api.listen), "configured");
    Ok(Some(runtime::run(config, Arc::new(ApiSurface)).await?))
}

/// Famulen served as its API: the console and the secretary's tools both
/// go through it.
struct ApiSurface;

impl Surface for ApiSurface {
    fn tools(&self, app: &App) -> Arc<dyn OwnTools> {
        ApiTools::shared(Api::new(app.clone()))
            .expect("the API's tools are built from its own definition")
    }

    fn serve(&self, app: App, at: ApiConfig) -> BoxFuture<'static, anyhow::Result<()>> {
        Box::pin(async move {
            let listener = tokio::net::TcpListener::bind(at.listen).await?;
            tracing::info!(listen = %at.listen, "API listening");
            let hosts = Hosts::new(at.listen, &at.hosts);
            axum::serve(listener, guarded(Api::new(app).axum(), hosts)).await?;
            Ok(())
        })
    }
}

/// Read `.env` from `dir`, if there is one. Variables already in the
/// environment are kept. Returns the file read. Parent directories are not
/// searched: a checkout nested in another (a workspace, a test run) must not
/// pick up the outer one's secrets.
fn load_dotenv(dir: &std::path::Path) -> anyhow::Result<Option<PathBuf>> {
    let path = dir.join(".env");
    match dotenvy::from_path(&path) {
        Ok(()) => Ok(Some(path)),
        Err(error) if error.not_found() => Ok(None),
        Err(error) => Err(error).context("reading .env"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("famulen-dotenv-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_the_env_file_in_the_starting_directory_is_read() {
        let parent = temp("parent");
        std::fs::write(parent.join(".env"), "FAMULEN_DOTENV_TEST_PARENT=1\n").unwrap();
        let child = parent.join("child");
        std::fs::create_dir_all(&child).unwrap();

        assert_eq!(load_dotenv(&child).unwrap(), None);
        assert!(std::env::var("FAMULEN_DOTENV_TEST_PARENT").is_err());

        std::fs::write(child.join(".env"), "FAMULEN_DOTENV_TEST_HERE=1\n").unwrap();
        assert_eq!(load_dotenv(&child).unwrap(), Some(child.join(".env")));
        assert_eq!(
            std::env::var("FAMULEN_DOTENV_TEST_HERE").as_deref(),
            Ok("1")
        );
        std::fs::remove_dir_all(parent).ok();
    }
}
