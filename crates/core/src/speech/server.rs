//! Keeping the one server that hears with the model the voice is assigned
//! to, and none when the voice hears elsewhere.

use std::ffi::OsStr;
use std::net::{Ipv4Addr, TcpListener};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::process::{Kept, last_said};
use super::{Speech, engine};
use crate::models::Role;

/// How often it is looked at which model the voice hears with, which can be
/// changed without [`Speech`] being told.
const LOOK: Duration = Duration::from_secs(5);
/// How long one request waits for the server.
const WAIT: Duration = Duration::from_secs(120);
/// How long a server may take to answer. The first start on a machine can take
/// far longer than later ones, and giving up on it only starts it over.
const START: Duration = Duration::from_secs(600);
const LONGEST_REST: Duration = Duration::from_secs(60);

/// What the server is doing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Server {
    #[default]
    Stopped,
    Starting {
        name: String,
    },
    Ready {
        name: String,
        endpoint: String,
    },
    Failed {
        name: String,
        why: String,
    },
}

impl Speech {
    /// The model the server is to hear with now.
    fn wanted(&self) -> Option<String> {
        let assignment = self.models().assignment(Role::Voice);
        let assignment = assignment
            .inspect_err(|error| tracing::warn!(%error, "the voice's model was not read"))
            .ok()??;
        let name = super::local_name(&assignment.model)?;
        self.downloaded(name).then(|| name.to_owned())
    }

    pub fn server(&self) -> Server {
        self.0.server.borrow().clone()
    }

    /// Where the server hearing with `name` is asked, if it is ready now. It
    /// is started when it is not.
    pub fn ready(&self, name: impl AsRef<str>) -> Option<String> {
        match self.server() {
            Server::Ready { name: n, endpoint } if n == name.as_ref() => Some(endpoint),
            _ => {
                self.0.wake.notify_one();
                None
            }
        }
    }

    /// [`Self::ready`], waiting while the model loads.
    pub async fn endpoint(&self, name: impl AsRef<str>) -> Result<String> {
        let name = name.as_ref();
        let mut server = self.0.server.subscribe();
        if let Some(endpoint) = self.ready(name) {
            return Ok(endpoint);
        }
        anyhow::ensure!(self.downloaded(name), "{name} is not downloaded");
        let waited = tokio::time::timeout(WAIT, async {
            loop {
                server.changed().await?;
                match &*server.borrow_and_update() {
                    Server::Ready { name: n, endpoint } if n == name => {
                        return Ok(endpoint.clone());
                    }
                    Server::Failed { name: n, why } if n == name => {
                        bail!("whisper.cpp did not start: {why}")
                    }
                    _ => {}
                }
            }
        });
        waited.await.context("whisper.cpp is not ready yet")?
    }

    /// Keep the server as the voice's assignment asks, until dropped.
    pub async fn run(&self) {
        let mut failures = 0u32;
        loop {
            let Some(name) = self.wanted() else {
                self.tell(Server::Stopped);
                self.rest(LOOK).await;
                continue;
            };
            self.tell(Server::Starting { name: name.clone() });
            let started = tokio::select! {
                started = self.start(&name) => started,
                () = self.unwanted(&name) => continue,
            };
            let why = match started {
                Ok((kept, endpoint)) => {
                    tracing::info!(model = %name, %endpoint, "whisper.cpp is ready");
                    self.tell(Server::Ready {
                        name: name.clone(),
                        endpoint,
                    });
                    match self.serve_until_unwanted(kept, &name).await {
                        Ok(()) => {
                            failures = 0;
                            continue;
                        }
                        Err(error) => error.to_string(),
                    }
                }
                Err(error) => format!("{error:#}"),
            };
            failures += 1;
            tracing::warn!(model = %name, %why, failures, "whisper.cpp is not running");
            self.tell(Server::Failed { name, why });
            let rest = Duration::from_secs(1 << failures.min(6)).min(LONGEST_REST);
            self.rest(rest).await;
        }
    }

    fn tell(&self, now: Server) {
        self.0.server.send_if_modified(|server| {
            let changed = *server != now;
            *server = now;
            changed
        });
    }

    /// Wait for `long`, or until something changed.
    async fn rest(&self, long: Duration) {
        tokio::select! {
            () = tokio::time::sleep(long) => {}
            () = self.0.wake.notified() => {}
        }
    }

    async fn serve_until_unwanted(&self, mut kept: Kept, name: &str) -> Result<()> {
        tokio::select! {
            ended = kept.wait() => {
                ended?;
                bail!("it stopped: {}", last_said(&self.log()));
            }
            () = self.unwanted(name) => {
                self.tell(Server::Stopped);
                kept.stop().await;
                Ok(())
            }
        }
    }

    async fn unwanted(&self, name: &str) {
        while self.wanted().as_deref() == Some(name) {
            self.rest(LOOK).await;
        }
    }

    fn log(&self) -> std::path::PathBuf {
        self.0.dir.join("server.log")
    }

    async fn start(&self, name: &str) -> Result<(Kept, String)> {
        let model = self.path(name);
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
            .local_addr()?
            .port();
        // The server serves the files of a directory, by default one under
        // wherever it was started.
        let public = self.0.dir.join("public");
        std::fs::create_dir_all(&public)?;
        let port_said = port.to_string();
        let args: [&OsStr; 12] = [
            "-m".as_ref(),
            model.as_ref(),
            "-l".as_ref(),
            "auto".as_ref(),
            "--host".as_ref(),
            "127.0.0.1".as_ref(),
            "--port".as_ref(),
            port_said.as_ref(),
            "--inference-path".as_ref(),
            "/v1/audio/transcriptions".as_ref(),
            "--public".as_ref(),
            public.as_ref(),
        ];
        let libraries = engine::libraries(&self.0.dir);
        let mut kept = Kept::start(
            engine::server(&self.0.dir),
            &args,
            &[("LD_LIBRARY_PATH", libraries.as_os_str())],
            &self.log(),
        )?;
        let origin = format!("http://127.0.0.1:{port}");
        let health = format!("{origin}/health");
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()?;
        let healthy = async {
            loop {
                let answered = client.get(&health).send().await;
                if answered.is_ok_and(|r| r.status().is_success()) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        };
        tokio::select! {
            ready = tokio::time::timeout(START, healthy) => {
                ready.context("it did not become ready")?;
            }
            ended = kept.wait() => {
                ended?;
                bail!("it stopped: {}", last_said(&self.log()));
            }
        }
        Ok((kept, format!("{origin}/v1")))
    }
}
