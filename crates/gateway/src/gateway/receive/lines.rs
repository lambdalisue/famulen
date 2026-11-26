//! A command kept running, each JSON line it prints handed over as one thing
//! that arrived.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;
use tokio::time::Instant;

use super::super::Gateway;
use super::Arrival;
use crate::definitions::seconds;
use crate::template::Context;

/// How often to look whether the owner has gone while something settles.
const LOOK_FOR_OWNER: Duration = Duration::from_secs(5);

/// The lines waiting to settle, by what each is about, with when it does.
type Waiting = HashMap<String, (Instant, Value)>;

impl Gateway {
    pub(super) async fn lines_forever(&self, id: &str, receiver: &str) {
        loop {
            if let Err(error) = self.lines_once(id, receiver).await {
                tracing::debug!(connection = %id, %receiver, %error, "the command stopped; running it again");
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

    /// Run the command of `receiver` until it ends, handing over each JSON
    /// line it prints.
    pub(super) async fn lines_once(&self, id: &str, receiver: &str) -> anyhow::Result<()> {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let (c, def, connect) = self.connect_of(id, receiver)?;
        let key = crate::connections::resume_key(id, receiver);
        let resumed: Vec<String> = match self.store.state(&key)? {
            Some(kept) => serde_json::from_str(&kept).unwrap_or_default(),
            None => Vec::new(),
        };
        let mut child = self
            .command(&c, &def, connect.args.iter().chain(&resumed))?
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let out = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no output"))?;
        let mut lines = BufReader::new(out).lines();
        let settle = duration(connect.settle.as_deref());
        let away = duration(connect.away.as_deref());
        // What waits to settle, by what it is about. Until it is empty the
        // place to go on from stays before it, so a restart loses nothing.
        let mut waiting: Waiting = HashMap::new();
        let mut next: Option<Vec<String>> = None;
        let mut look = Instant::now();
        loop {
            let due = waiting
                .values()
                .map(|(at, _)| if away.is_some() { (*at).min(look) } else { *at })
                .min();
            // What is already due is looked at before more is read, so that a
            // command that has just ended does not take the look with it.
            let read = if due.is_some_and(|at| at <= Instant::now()) {
                None
            } else {
                tokio::select! {
                    line = lines.next_line() => Some(line?),
                    () = async {
                        match due {
                            Some(at) => tokio::time::sleep_until(at).await,
                            None => std::future::pending().await,
                        }
                    } => None,
                }
            };
            let Some(line) = read else {
                let now = Instant::now();
                let gone = match away {
                    Some(away) => self.owner_elsewhere(away, connect.front.as_deref()).await,
                    None => false,
                };
                look = now + LOOK_FOR_OWNER;
                let settled: Vec<String> = waiting
                    .iter()
                    .filter(|(_, (at, _))| gone || *at <= now)
                    .map(|(about, _)| about.clone())
                    .collect();
                for about in settled {
                    if let Some((_, event)) = waiting.remove(&about) {
                        self.hand_over(id, receiver, &about, event, false)?;
                    }
                }
                self.go_on_from(&key, &waiting, &next)?;
                continue;
            };
            let Some(line) = line else {
                break;
            };
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let found = Context::of(event.clone());
            let wanted = connect.when.iter().all(|(at, values)| {
                found
                    .render_text(at)
                    .is_some_and(|value| values.contains(&value))
            });
            let resume: Option<Vec<String>> = connect
                .resume
                .iter()
                .map(|arg| found.render_text(arg))
                .collect();
            if let Some(resume) = resume.filter(|n| !n.is_empty()) {
                next = Some(resume);
            }
            match settle {
                None if wanted => self.hand_over(id, receiver, "", event, false)?,
                None => {}
                Some(settle) => {
                    let about = connect
                        .per
                        .as_deref()
                        .and_then(|per| found.render_text(per))
                        .unwrap_or_default();
                    waiting.remove(&about);
                    if wanted {
                        waiting.insert(about, (Instant::now() + settle, event));
                    } else {
                        self.hand_over(id, receiver, &about, event, true)?;
                    }
                }
            }
            self.go_on_from(&key, &waiting, &next)?;
        }
        let status = child.wait().await?;
        anyhow::ensure!(status.success(), "it ended with {status}");
        Ok(())
    }

    /// Whether the owner would not see it here: they have not touched the
    /// machine for `away`, or do not look at `front`. A machine that does
    /// not say has no owner at it.
    async fn owner_elsewhere(&self, away: Duration, front: Option<&str>) -> bool {
        let presence = self.presence.clone();
        let Ok(now) = tokio::task::spawn_blocking(move || presence()).await else {
            return true;
        };
        now.idle.is_none_or(|idle| idle >= away)
            || front.is_some_and(|front| now.front.as_deref() != Some(front))
    }

    fn hand_over(
        &self,
        id: &str,
        receiver: &str,
        about: &str,
        payload: Value,
        over: bool,
    ) -> anyhow::Result<()> {
        self.deliver(Arrival {
            about: about.to_owned(),
            over,
            ..Arrival::new(id, receiver, payload)
        })?;
        Ok(())
    }

    /// Remember where to go on from, unless something still waits to settle.
    fn go_on_from(
        &self,
        key: &str,
        waiting: &Waiting,
        next: &Option<Vec<String>>,
    ) -> anyhow::Result<()> {
        if let Some(next) = next
            && waiting.is_empty()
        {
            self.store.set_state(key, serde_json::to_string(next)?)?;
        }
        Ok(())
    }
}

fn duration(text: Option<&str>) -> Option<Duration> {
    text.and_then(seconds).map(Duration::from_secs)
}
