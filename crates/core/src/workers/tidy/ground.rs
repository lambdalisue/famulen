//! What tidying asks of the machine: the sessions in Adroit, and the programs
//! that know the rest.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context as _;
use futures_util::future::BoxFuture;
use serde_json::Value;

use super::super::adroit;
use super::super::runs::Run;
use crate::app::App;

/// How long a program may take to answer.
const TAKES: Duration = Duration::from_secs(120);

/// A session Adroit knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Session {
    pub id: String,
    pub dir: PathBuf,
    pub ended: bool,
}

/// What a program left.
#[derive(Debug, Clone, Default)]
pub(super) struct Ran {
    pub ok: bool,
    pub out: String,
    pub err: String,
}

impl Ran {
    /// Why it failed, in its own words.
    pub fn why(&self) -> String {
        let said = if self.err.trim().is_empty() {
            &self.out
        } else {
            &self.err
        };
        let said: String = said.trim().chars().take(300).collect();
        if said.is_empty() {
            "no reason told".to_owned()
        } else {
            said
        }
    }
}

pub(super) trait Ground: Send + Sync {
    /// The session of a run, and every session that goes on in the Adroit it
    /// was started in.
    fn sessions<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Vec<Session>>>;
    /// Run a program in `dir`. An error says that it did not run to its end.
    fn run<'a>(
        &'a self,
        dir: &'a Path,
        program: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, anyhow::Result<Ran>>;
    /// Run a program that is not cut off however long it takes, nor when
    /// Famulen is interrupted or starts again.
    fn run_whole<'a>(
        &'a self,
        dir: &'a Path,
        program: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, anyhow::Result<Ran>>;
}

pub(super) struct Machine {
    app: App,
}

impl Machine {
    pub fn new(app: App) -> Self {
        Self { app }
    }
}

impl Ground for Machine {
    fn sessions<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Vec<Session>>> {
        Box::pin(async move {
            let (own, going_on) = adroit::sessions(&self.app, run).await?;
            let own = session(&own)?;
            anyhow::ensure!(own.id == run.session, "another session was read");
            let mut all = sessions(&going_on)?;
            // One that went on since it was read is listed, and that counts.
            all.push(own);
            Ok(all)
        })
    }

    fn run<'a>(
        &'a self,
        dir: &'a Path,
        program: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, anyhow::Result<Ran>> {
        Box::pin(async move {
            let output = command(dir, program, args).kill_on_drop(true).output();
            let output = tokio::time::timeout(TAKES, output)
                .await
                .with_context(|| format!("{program} took too long"))?
                .with_context(|| format!("{program} did not run"))?;
            ran(program, output)
        })
    }

    fn run_whole<'a>(
        &'a self,
        dir: &'a Path,
        program: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, anyhow::Result<Ran>> {
        Box::pin(async move {
            let mut command = command(dir, program, args);
            #[cfg(unix)]
            command.process_group(0);
            let output = command
                .output()
                .await
                .with_context(|| format!("{program} did not run"))?;
            ran(program, output)
        })
    }
}

fn command(dir: &Path, program: &str, args: &[&str]) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null());
    command
}

fn ran(program: &str, output: std::process::Output) -> anyhow::Result<Ran> {
    Ok(Ran {
        ok: output.status.success(),
        out: String::from_utf8(output.stdout)
            .with_context(|| format!("{program} answered no text"))?,
        err: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Every session must be read whole: one passed over could be the one that
/// works where something is removed.
fn sessions(listed: &Value) -> anyhow::Result<Vec<Session>> {
    listed
        .as_array()
        .context("the sessions could not be read")?
        .iter()
        .map(session)
        .collect()
}

fn session(told: &Value) -> anyhow::Result<Session> {
    let text = |name: &str| {
        told[name]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .with_context(|| format!("a session has no {name}"))
    };
    Ok(Session {
        id: text("id")?,
        dir: text("dir")?.into(),
        ended: text("state")? == "ended",
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn sessions_are_read_whole_or_not_at_all() {
        let one = json!({ "id": "a", "dir": "/src", "state": "ended" });
        assert_eq!(
            sessions(&json!([one])).unwrap(),
            [Session {
                id: "a".into(),
                dir: "/src".into(),
                ended: true
            }]
        );
        for missing in ["id", "dir", "state"] {
            let mut other = one.clone();
            other[missing] = Value::Null;
            assert!(sessions(&json!([one, other])).is_err(), "{missing}");
        }
        assert!(sessions(&json!({})).is_err());
    }
}
