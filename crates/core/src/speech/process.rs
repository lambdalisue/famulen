//! Running a program so that it, and what it started, ends when Famulen does,
//! however Famulen ends: restarted in place, stopped or killed.

use std::ffi::OsStr;
use std::fs::File;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::process::{Child, ChildStdin, Command};

/// Runs the program in a process group of its own and ends the whole group
/// once its input closes, which the system does when Famulen is gone. A
/// parent-death signal is only to be had on Linux. The watcher is left running
/// when the program ends first: what that leaves behind has to end even if
/// Famulen is gone before [`Kept::wait`] sees the end.
const KEEPER: &str = r#"exec 3<&0
"$@" </dev/null 3<&- &
child=$!
{
  trap '' TERM
  cat <&3 >/dev/null
  kill -TERM 0
  sleep 5
  kill -KILL 0
} >/dev/null 2>&1 &
wait "$child"
"#;

/// What a program needs of Famulen's environment, which also holds secrets.
const ENVIRONMENT: &[&str] = &["PATH", "HOME", "TMPDIR", "DEVELOPER_DIR", "SDKROOT", "LANG"];

/// A running program. Dropping it ends it.
pub(super) struct Kept {
    child: Child,
    input: Option<ChildStdin>,
    group: Option<Pid>,
}

impl Kept {
    /// Start `program`, writing what it says to `log`.
    pub fn start<S: AsRef<OsStr>>(
        program: impl AsRef<OsStr>,
        args: &[S],
        env: &[(&str, &OsStr)],
        log: &Path,
    ) -> Result<Self> {
        let out = File::create(log).with_context(|| format!("writing {}", log.display()))?;
        let err = out.try_clone()?;
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(KEEPER)
            .arg("sh")
            .arg(program.as_ref())
            .args(args)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(out)
            .stderr(err)
            .process_group(0);
        for name in ENVIRONMENT {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.envs(env.iter().copied());
        let mut child = command.spawn().context("starting /bin/sh")?;
        let input = child.stdin.take();
        let group = child.id().and_then(|id| Pid::from_raw(id.try_into().ok()?));
        Ok(Self {
            child,
            input,
            group,
        })
    }

    /// Wait until the program ends on its own: whether it succeeded.
    pub async fn wait(&mut self) -> Result<bool> {
        let status = self.child.wait().await?;
        self.end_group();
        Ok(status.success())
    }

    /// End the program and wait until it is gone.
    pub async fn stop(mut self) {
        drop(self.input.take());
        tokio::time::timeout(Duration::from_secs(10), self.child.wait())
            .await
            .ok();
        self.end_group();
    }

    /// Kill whatever is left in the program's group. Only once: an empty
    /// group's number is free for the system to hand out again.
    fn end_group(&mut self) {
        if let Some(group) = self.group.take() {
            kill_process_group(group, Signal::KILL).ok();
        }
    }
}

/// The end of what a program said, to tell why it failed.
pub(super) fn last_said(log: &Path) -> String {
    let said = std::fs::read(log).unwrap_or_default();
    let said = String::from_utf8_lossy(&said);
    let lines: Vec<&str> = said.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(5)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speech::tests::Scratch;

    async fn until(what: impl Fn() -> bool) {
        for _ in 0..500 {
            if what() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("never happened");
    }

    /// A program that beats into a file for as long as it, or what it
    /// started, lives.
    fn beating(dir: &Path) -> Kept {
        let beat = dir.join("beat");
        let script = format!(
            "while true; do echo . >> '{}'; sleep 0.05; done & wait",
            beat.display()
        );
        Kept::start("/bin/sh", &["-c", &script], &[], &dir.join("log")).unwrap()
    }

    async fn has_stopped_beating(dir: &Path) {
        let beat = dir.join("beat");
        let size = || std::fs::metadata(&beat).map(|m| m.len()).unwrap_or(0);
        for _ in 0..50 {
            let before = size();
            tokio::time::sleep(Duration::from_millis(300)).await;
            if size() == before {
                return;
            }
        }
        panic!("still beating");
    }

    #[tokio::test]
    async fn a_dropped_program_ends_with_what_it_started() {
        let dir = Scratch::new("dropped");
        let kept = beating(&dir);
        until(|| dir.join("beat").exists()).await;

        drop(kept);

        has_stopped_beating(&dir).await;
    }

    #[tokio::test]
    async fn a_stopped_program_ends_with_what_it_started() {
        let dir = Scratch::new("stopped");
        let kept = beating(&dir);
        until(|| dir.join("beat").exists()).await;

        kept.stop().await;

        has_stopped_beating(&dir).await;
    }

    #[tokio::test]
    async fn a_program_that_ends_tells_whether_it_succeeded_and_what_it_said() {
        let dir = Scratch::new("ended");
        let log = dir.join("log");
        for (script, succeeded) in [("echo fine", true), ("echo broken >&2; exit 3", false)] {
            let mut kept = Kept::start("/bin/sh", &["-c", script], &[], &log).unwrap();
            assert_eq!(kept.wait().await.unwrap(), succeeded, "{script}");
        }
        assert_eq!(last_said(&log), "broken");
    }

    #[tokio::test]
    async fn what_a_program_leaves_running_ends_with_it() {
        let dir = Scratch::new("left-behind");
        let script = format!(
            "while true; do echo . >> '{}'; sleep 0.05; done & sleep 0.2",
            dir.join("beat").display()
        );
        let mut kept = Kept::start("/bin/sh", &["-c", &script], &[], &dir.join("log")).unwrap();
        assert!(kept.wait().await.unwrap());

        has_stopped_beating(&dir).await;
    }

    #[tokio::test]
    async fn what_a_program_leaves_running_ends_when_nobody_saw_it_end() {
        let dir = Scratch::new("left-unseen");
        let script = format!(
            "while true; do echo . >> '{}'; sleep 0.05; done & sleep 0.2",
            dir.join("beat").display()
        );
        let mut kept = Kept::start("/bin/sh", &["-c", &script], &[], &dir.join("log")).unwrap();
        // The program has ended, and Famulen goes away before `Kept::wait`
        // can act on it.
        assert!(kept.child.wait().await.unwrap().success());

        drop(kept);

        has_stopped_beating(&dir).await;
    }

    #[tokio::test]
    async fn famulens_secrets_are_not_handed_to_a_program() {
        let dir = Scratch::new("environment");
        let log = dir.join("log");
        let mut kept = Kept::start("/usr/bin/env", &[] as &[&str], &[], &log).unwrap();
        assert!(kept.wait().await.unwrap());
        let said = std::fs::read_to_string(&log).unwrap();
        let handed = said.lines().filter_map(|l| l.split_once('=')).map(|v| v.0);
        let shells_own = ["PWD", "SHLVL", "_", "OLDPWD"];
        for name in handed {
            assert!(
                ENVIRONMENT.contains(&name) || shells_own.contains(&name),
                "{name}"
            );
        }
    }
}
