//! What the Adroit sessions of delegated tasks used. Adroit tells neither
//! that nor which conversation a session holds, so it is read from what the
//! agents keep of their conversations: Claude Code its projects, Codex its
//! rollouts. Of those only the counts of tokens are kept.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use rusqlite::{OptionalExtension, params};
use serde::Deserialize;

use super::adroit::{bears, token};
use crate::app::App;
use crate::models::{Models, Price, Source, Usage, estimate_cost_yen, found_id};
use crate::store::Store;

const EVERY: Duration = Duration::from_secs(60);
/// How far into a conversation its title is looked for.
const TITLE_LINES: usize = 256;

/// Count what the sessions use until Famulen stops.
pub async fn keep_counting(app: App) {
    let mut sessions = Sessions::new(
        app.store.clone(),
        app.models.clone(),
        Logs::from_env(|name| std::env::var(name).ok()),
    );
    let mut tick = tokio::time::interval(EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let counted = tokio::task::spawn_blocking(move || {
            if let Err(error) = sessions.count() {
                tracing::warn!(%error, "could not count what delegated sessions used");
            }
            sessions
        })
        .await;
        sessions = match counted {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::error!(%error, "what delegated sessions use is counted no more");
                return;
            }
        };
    }
}

/// Where the agents keep their conversations.
#[derive(Debug, Clone, Default)]
pub struct Logs {
    pub claude: Option<PathBuf>,
    pub codex: Option<PathBuf>,
}

impl Logs {
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Self {
        let given = |name| env(name).filter(|d| !d.is_empty()).map(PathBuf::from);
        Self {
            claude: crate::claude::skills::claude_dir(&env).map(|d| d.join("projects")),
            codex: given("CODEX_HOME")
                .or_else(|| Some(given("HOME")?.join(".codex")))
                .map(|d| d.join("sessions")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Agent {
    Claude,
    Codex,
}

struct Ordered {
    run: i64,
    token: String,
    /// The directory its session was started for, as Claude Code names it.
    project: String,
}

/// One answer of a model.
struct Answer {
    id: String,
    at: i64,
    model: String,
    input: u64,
    output: u64,
    /// The part of the input read from a cache.
    cached: u64,
}

pub struct Sessions {
    store: Store,
    models: Models,
    logs: Logs,
    /// Whose each conversation is, where it was told by what it holds.
    owners: HashMap<PathBuf, Option<i64>>,
}

impl Sessions {
    pub fn new(store: Store, models: Models, logs: Logs) -> Self {
        Self {
            store,
            models,
            logs,
            owners: HashMap::new(),
        }
    }

    /// Count what was used since it was last counted, in every session there
    /// ever was: one goes on after its run ended, and is no less the run's.
    pub fn count(&mut self) -> anyhow::Result<()> {
        let runs = self.runs()?;
        if runs.is_empty() {
            return Ok(());
        }
        for (path, agent, run) in self.conversations(&runs) {
            if let Err(error) = self.take_in(&path, agent, run) {
                tracing::debug!(path = %path.display(), %error, "could not read what a session used");
            }
        }
        Ok(())
    }

    fn runs(&self) -> anyhow::Result<Vec<Ordered>> {
        let conn = self.store.conn();
        let mut statement = conn.prepare(
            "SELECT id, spec ->> '$.code', COALESCE(spec ->> '$.directory', '')
             FROM worker_runs WHERE runner = 'adroit'",
        )?;
        let runs = statement.query_map([], |r| {
            let run = r.get(0)?;
            let directory = PathBuf::from(r.get::<_, String>(2)?);
            Ok(Ordered {
                run,
                token: token(&r.get::<_, String>(1)?, run),
                project: project(&directory.canonicalize().unwrap_or(directory)),
            })
        })?;
        Ok(runs.collect::<Result<_, _>>()?)
    }

    fn conversations(&mut self, runs: &[Ordered]) -> Vec<(PathBuf, Agent, i64)> {
        let mut found = Vec::new();
        for dir in self.logs.claude.as_deref().map(entries).unwrap_or_default() {
            let Some(name) = dir.file_name().and_then(OsStr::to_str) else {
                continue;
            };
            // Whatever worked in the worktree made for a run worked for it.
            if let Some(run) = runs
                .iter()
                .filter_map(|r| Some((named_at(name, &r.token)?, r.run)))
                .max()
                .map(|(_, run)| run)
            {
                let files = files_in(&dir);
                found.extend(files.into_iter().map(|f| (f, Agent::Claude, run)));
            } else if runs.iter().any(|r| is_project(name, &r.project)) {
                for file in entries(&dir).into_iter().filter(|f| is_log(f)) {
                    if let Some(run) = self.owner(&file, || titled(&file, runs)) {
                        // Its subagents are beside it.
                        let files = files_in(&file.with_extension(""));
                        found.extend(files.into_iter().map(|f| (f, Agent::Claude, run)));
                        found.push((file, Agent::Claude, run));
                    }
                }
            }
        }
        for file in self.logs.codex.as_deref().map(files_in).unwrap_or_default() {
            if let Some(run) = self.owner(&file, || worked_in(&file, runs)) {
                found.push((file, Agent::Codex, run));
            }
        }
        found
    }

    /// Whose a conversation is; asked again while `told` cannot tell yet.
    fn owner(&mut self, file: &Path, told: impl Fn() -> Option<Option<i64>>) -> Option<i64> {
        if let Some(owner) = self.owners.get(file) {
            return *owner;
        }
        let owner = told()?;
        self.owners.insert(file.to_owned(), owner);
        owner
    }

    fn take_in(&self, path: &Path, agent: Agent, run: i64) -> anyhow::Result<()> {
        let key = path.to_string_lossy();
        let before: Option<(u64, String, bool)> = self
            .store
            .conn()
            .query_row(
                "SELECT read_bytes, model, counted FROM session_files WHERE path = ?1",
                [&key],
                |r| Ok((r.get::<_, i64>(0)?.cast_unsigned(), r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (from, mut model, mut counted) = before.unwrap_or_default();
        let size = fs::metadata(path)?.len();
        if size == from {
            return Ok(());
        }
        // A shorter one was written anew; what it repeats is known by its id.
        let from = if size < from { 0 } else { from };
        let (answers, read) = answers(path, agent, from, &mut model)?;
        let prices: HashMap<String, Price> = self
            .models
            .registered()?
            .into_iter()
            .map(|m| (m.spec.id, m.spec.price))
            .collect();
        let conn = self.store.conn();
        let tx = conn.unchecked_transaction()?;
        for a in answers {
            // A conversation counts as one call, in what answered it first.
            let calls = u64::from(!counted);
            let usage = Usage {
                input_tokens: Some(a.input),
                output_tokens: Some(a.output),
                calls,
            };
            let cost = prices
                .get(&a.model)
                .map_or(0.0, |price| estimate_cost_yen(price, &usage));
            let new = tx.execute(
                "INSERT INTO session_usage
                     (id, run, at, model, input_tokens, output_tokens, cached_tokens, calls, cost_yen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT (id) DO NOTHING",
                params![
                    a.id,
                    run,
                    a.at,
                    a.model,
                    a.input.cast_signed(),
                    a.output.cast_signed(),
                    a.cached.cast_signed(),
                    calls.cast_signed(),
                    cost
                ],
            )?;
            if new == 1 {
                counted = true;
            } else {
                // Told of again as it grows.
                tx.execute(
                    "UPDATE session_usage SET input_tokens = MAX(input_tokens, ?2),
                         output_tokens = MAX(output_tokens, ?3), cached_tokens = MAX(cached_tokens, ?4),
                         cost_yen = MAX(cost_yen, ?5) WHERE id = ?1",
                    params![
                        a.id,
                        a.input.cast_signed(),
                        a.output.cast_signed(),
                        a.cached.cast_signed(),
                        cost
                    ],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO session_files (path, read_bytes, model, counted) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (path) DO UPDATE SET read_bytes = ?2, model = ?3, counted = ?4",
            params![key, read.cast_signed(), model, counted],
        )?;
        tx.commit()?;
        Ok(())
    }
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries.flatten().map(|e| e.path()).collect()
}

fn is_log(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "jsonl")
}

/// Every log in a directory or below it. A link is not followed.
fn files_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .flat_map(|e| {
            let path = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => files_in(&path),
                Ok(t) if t.is_file() && is_log(&path) => vec![path],
                _ => Vec::new(),
            }
        })
        .collect()
}

/// The name Claude Code keeps a directory's conversations under.
fn project(dir: &Path) -> String {
    dir.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Whether `name` is kept for `project`: Claude Code cuts a long one short and
/// tells it from others by a hash.
fn is_project(name: &str, project: &str) -> bool {
    const LONGEST: usize = 200;
    match project.get(..LONGEST) {
        Some(head) if project.len() > LONGEST => name.starts_with(head),
        _ => name == project,
    }
}

/// Where a run's token stands in the name of a project that is kept for its
/// worktree or for a directory in it. Of worktrees in one another, the inner
/// one's stands last.
fn named_at(project: &str, token: &str) -> Option<usize> {
    project
        .rmatch_indices(&format!("-{token}"))
        .map(|(at, _)| at + 1)
        .find(|at| bears(&project[*at..], token))
}

/// The run whose token a Claude Code conversation is titled by.
fn titled(file: &Path, runs: &[Ordered]) -> Option<Option<i64>> {
    #[derive(Deserialize)]
    struct Title {
        #[serde(rename = "customTitle")]
        title: String,
    }
    let mut lines = BufReader::new(File::open(file).ok()?).lines();
    for _ in 0..TITLE_LINES {
        let line = lines.next()?.ok()?;
        if !line.contains(r#""type":"custom-title""#) {
            continue;
        }
        if let Ok(Title { title }) = serde_json::from_str(&line) {
            return Some(runs.iter().find(|r| bears(&title, &r.token)).map(|r| r.run));
        }
    }
    Some(None)
}

/// The run in whose worktree a Codex conversation was begun.
fn worked_in(file: &Path, runs: &[Ordered]) -> Option<Option<i64>> {
    #[derive(Deserialize)]
    struct Begun {
        payload: Meta,
    }
    #[derive(Deserialize)]
    struct Meta {
        cwd: PathBuf,
    }
    let mut line = String::new();
    BufReader::new(File::open(file).ok()?)
        .read_line(&mut line)
        .ok()?;
    if !line.ends_with('\n') {
        return None;
    }
    let Ok(Begun { payload }) = serde_json::from_str(&line) else {
        return Some(None);
    };
    // Of worktrees in one another, the inner one is where it works.
    let owner = payload
        .cwd
        .components()
        .rev()
        .find_map(|c| match c {
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                runs.iter().find(|r| bears(&name, &r.token))
            }
            _ => None,
        })
        .map(|r| r.run);
    Some(owner)
}

/// What a conversation tells beyond its first `from` bytes, and how far it was
/// read: a line being written is left for the next time.
fn answers(
    path: &Path,
    agent: Agent,
    from: u64,
    model: &mut String,
) -> anyhow::Result<(Vec<Answer>, u64)> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(from))?;
    let mut file = BufReader::new(file);
    let mut read = from;
    let mut found = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        let bytes = file.by_ref().read_until(b'\n', &mut line)?;
        if !line.ends_with(b"\n") {
            return Ok((found, read));
        }
        read += bytes as u64;
        let text = String::from_utf8_lossy(&line);
        found.extend(match agent {
            Agent::Claude => claude_answer(&text),
            Agent::Codex => codex_answer(&text, model),
        });
    }
}

fn unix(timestamp: &str) -> Option<i64> {
    Some(
        chrono::DateTime::parse_from_rfc3339(timestamp)
            .ok()?
            .timestamp(),
    )
}

fn claude_answer(line: &str) -> Option<Answer> {
    #[derive(Deserialize)]
    struct Line {
        timestamp: String,
        message: Message,
    }
    #[derive(Deserialize)]
    struct Message {
        id: String,
        model: String,
        usage: Tokens,
    }
    #[derive(Deserialize)]
    #[serde(default)]
    #[derive(Default)]
    struct Tokens {
        input_tokens: u64,
        output_tokens: u64,
        cache_read_input_tokens: u64,
        cache_creation_input_tokens: u64,
    }
    if !line.contains(r#""type":"assistant""#) {
        return None;
    }
    let Line { timestamp, message } = serde_json::from_str(line).ok()?;
    let u = message.usage;
    let input = u.input_tokens + u.cache_read_input_tokens + u.cache_creation_input_tokens;
    // Claude Code writes messages of its own, which no model answered.
    if input + u.output_tokens == 0 {
        return None;
    }
    Some(Answer {
        id: message.id,
        at: unix(&timestamp)?,
        model: found_id(Source::Claude, &message.model),
        input,
        output: u.output_tokens,
        cached: u.cache_read_input_tokens,
    })
}

fn codex_answer(line: &str, model: &mut String) -> Option<Answer> {
    #[derive(Deserialize)]
    struct Line {
        timestamp: String,
        #[serde(rename = "type")]
        kind: String,
        payload: Payload,
    }
    #[derive(Deserialize)]
    #[serde(default)]
    #[derive(Default)]
    struct Payload {
        model: Option<String>,
        response_id: Option<String>,
        usage: Tokens,
    }
    #[derive(Deserialize)]
    #[serde(default)]
    #[derive(Default)]
    struct Tokens {
        input_tokens: u64,
        output_tokens: u64,
        cached_input_tokens: u64,
    }
    if !line.contains(r#""type":"turn_context""#)
        && !line.contains(r#""type":"token_usage_record""#)
    {
        return None;
    }
    let line: Line = serde_json::from_str(line).ok()?;
    let payload = line.payload;
    match line.kind.as_str() {
        "turn_context" => {
            *model = payload.model.unwrap_or_default();
            None
        }
        "token_usage_record" if payload.usage.input_tokens + payload.usage.output_tokens > 0 => {
            Some(Answer {
                id: payload.response_id?,
                at: unix(&line.timestamp)?,
                model: found_id(Source::Codex, model),
                // As Codex counts, what was read from a cache is part of it.
                input: payload.usage.input_tokens,
                output: payload.usage.output_tokens,
                cached: payload.usage.cached_input_tokens,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
