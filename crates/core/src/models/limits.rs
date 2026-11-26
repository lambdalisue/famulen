//! How much of an agent CLI's account limits is used, asked of the CLI
//! itself without running a turn, and kept for a while: asking starts a
//! process that asks the network, while the limits move slowly against
//! windows of hours and days.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::{Cli, ModelKind, ModelSpec};

/// How long a CLI is given to answer, as it asks the account's usage over
/// the network.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// How long limits are shown before they are asked for again.
pub const LIMITS_TTL: Duration = Duration::from_secs(5 * 60);

/// The span a limit counts usage over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Window {
    /// The few hours since usage began.
    Session,
    /// The week, every model together.
    Week,
    /// The week, for the one model or surface named.
    WeekOf(String),
    /// A limit Famulen has no name for, by the CLI's own.
    Other(String),
}

impl Window {
    /// The name the owner reads in the console.
    pub fn label(&self) -> String {
        match self {
            Window::Session => "5 時間".into(),
            Window::Week => "週".into(),
            Window::WeekOf(name) => format!("週・{name}"),
            Window::Other(kind) => kind.replace('_', " "),
        }
    }
}

/// How near the CLI says a limit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Normal,
    Warning,
    Critical,
}

/// One limit and how much of it is used.
#[derive(Debug, Clone, PartialEq)]
pub struct Limit {
    pub window: Window,
    /// Used, from 0 to 100; above 100 once the limit is passed.
    pub percent: f64,
    /// When the window starts again, in milliseconds since the Unix epoch.
    pub resets_ms: Option<i64>,
    pub severity: Severity,
}

/// The account a CLI is logged in to, known by the program that runs it:
/// every model run by one program shares its limits.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Account {
    pub cli: Cli,
    pub executable: String,
}

impl Account {
    /// The account `spec` is called on; none for a model that is not an
    /// agent CLI. A Claude Code model naming no program is on the one at
    /// `claude_bin`, which Famulen runs as Claude Code and found it in.
    pub fn of(spec: &ModelSpec, claude_bin: &str) -> Option<Self> {
        let executable = match (spec.cli, spec.endpoint.as_deref()) {
            (Cli::Claude, None | Some("")) => claude_bin,
            _ => spec.executable(),
        };
        (spec.kind == ModelKind::AgentCli).then(|| Self {
            cli: spec.cli,
            executable: executable.to_owned(),
        })
    }
}

/// The id every request Famulen writes carries, which its answer echoes.
const REQUEST_ID: &str = "famulen";

/// A `get_usage` control request, never a user message, so no turn runs and
/// no tokens are spent. It leaves out the account's habits, which Claude Code
/// would read from every transcript.
const CLAUDE_REQUEST: &str = "{\"request_id\":\"famulen\",\"type\":\"control_request\",\"request\":{\"subtype\":\"get_usage\",\"skip_behaviors\":true}}\n";

/// Claude Code taking control requests on its input and answering on its
/// output, keeping no session, and reading none of the owner's settings, so
/// no hook or MCP server of theirs starts for it.
const CLAUDE_ARGS: [&str; 10] = [
    "--output-format",
    "stream-json",
    "--verbose",
    "--input-format",
    "stream-json",
    "--no-session-persistence",
    "--setting-sources",
    "",
    "--strict-mcp-config",
    "--disable-slash-commands",
];

/// What one line Claude Code printed says about the usage asked for.
#[derive(Debug, Clone, PartialEq)]
enum Reply {
    /// The limits.
    Limits(Vec<Limit>),
    /// Claude Code answered, but not with limits.
    Refused,
    /// Not the answer: another message, or no JSON at all.
    Other,
}

fn claude_reply(line: &str) -> Reply {
    let Ok(msg) = serde_json::from_str::<Value>(line) else {
        return Reply::Other;
    };
    let response = &msg["response"];
    if msg["type"] != "control_response" || response["request_id"] != REQUEST_ID {
        return Reply::Other;
    }
    if response["subtype"] != "success" {
        return Reply::Refused;
    }
    let Some(list) = response["response"]["rate_limits"]["limits"].as_array() else {
        return Reply::Refused;
    };
    Reply::Limits(list.iter().filter_map(claude_limit).collect())
}

fn claude_limit(v: &Value) -> Option<Limit> {
    let kind = v["kind"].as_str()?;
    let window = match kind {
        "session" => Window::Session,
        "weekly_all" => Window::Week,
        "weekly_scoped" => match ["model", "surface"]
            .iter()
            .find_map(|k| v["scope"][k]["display_name"].as_str())
        {
            Some(name) => Window::WeekOf(name.into()),
            None => Window::Other(kind.into()),
        },
        other => Window::Other(other.into()),
    };
    let percent = v["percent"].as_f64()?;
    let severity = match v["severity"].as_str() {
        Some("normal") => Severity::Normal,
        Some("warning") => Severity::Warning,
        Some("critical" | "exceeded") => Severity::Critical,
        _ => severity_of(percent),
    };
    Some(Limit {
        window,
        percent,
        resets_ms: v["resets_at"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.timestamp_millis()),
        severity,
    })
}

/// How near a limit used `percent` is, for a CLI that does not say.
fn severity_of(percent: f64) -> Severity {
    if percent >= 90.0 {
        Severity::Critical
    } else if percent >= 80.0 {
        Severity::Warning
    } else {
        Severity::Normal
    }
}

/// The limits in Codex's answer to `account/rateLimits/read`; `None` when it
/// carries none at all. The account's own bucket, the one `rateLimits` also
/// holds, goes unnamed; the others are named after their limits.
fn codex_limits(result: &Value) -> Option<Vec<Limit>> {
    let own = result["rateLimits"]["limitId"].as_str().unwrap_or("codex");
    let buckets: Vec<(Option<&str>, &Value)> = match result["rateLimitsByLimitId"].as_object() {
        Some(map) if !map.is_empty() => map
            .iter()
            .map(|(id, bucket)| {
                let name = (id != own).then(|| bucket["limitName"].as_str().unwrap_or(id));
                (name, bucket)
            })
            .collect(),
        _ => vec![(None, result.get("rateLimits").filter(|v| v.is_object())?)],
    };
    let mut found = Vec::new();
    for (name, bucket) in buckets {
        for key in ["primary", "secondary"] {
            let v = &bucket[key];
            let Some(percent) = v["usedPercent"]
                .as_f64()
                .filter(|p| p.is_finite() && *p >= 0.0)
            else {
                continue;
            };
            let window = match (v["windowDurationMins"].as_u64(), name) {
                (Some(10080), None) => Window::Week,
                (Some(10080), Some(name)) => Window::WeekOf(name.into()),
                (duration, name) => {
                    let span = match duration {
                        Some(min) if min > 0 && min % 60 == 0 => format!("{} 時間", min / 60),
                        Some(min) => format!("{min} 分"),
                        None => key.into(),
                    };
                    Window::Other(match name {
                        Some(name) => format!("{span}・{name}"),
                        None => span,
                    })
                }
            };
            found.push(Limit {
                window,
                percent,
                resets_ms: v["resetsAt"].as_i64().and_then(|s| s.checked_mul(1000)),
                severity: severity_of(percent),
            });
        }
    }
    Some(found)
}

/// Ask the CLI of `account` how much of its limits is used. `None` for every
/// failure: the requests are unversioned, and a release that changes them
/// leaves the limits unknown rather than guessed.
pub async fn probe(account: &Account) -> Option<Vec<Limit>> {
    let command = Command::new(&account.executable);
    probe_with(account.cli, command, PROBE_TIMEOUT).await
}

async fn probe_with(cli: Cli, mut command: Command, timeout: Duration) -> Option<Vec<Limit>> {
    match cli {
        // The owner's claude.ai connectors start with every Claude Code but for this.
        Cli::Claude => command
            .args(CLAUDE_ARGS)
            .env("ENABLE_CLAUDEAI_MCP_SERVERS", "false"),
        Cli::Codex => command.arg("app-server"),
    };
    crate::config::hide_secrets(&mut command);
    let mut talk = Talk::start(command)?;
    let asked = async {
        match cli {
            Cli::Claude => {
                talk.write(CLAUDE_REQUEST).await?;
                talk.until(|line| match claude_reply(line) {
                    Reply::Limits(list) => Some(Some(list)),
                    Reply::Refused => Some(None),
                    Reply::Other => None,
                })
                .await?
            }
            Cli::Codex => {
                let initialize = json!({
                    "id": "famulen-init", "method": "initialize",
                    "params": {"clientInfo": {"name": "famulen", "version": env!("CARGO_PKG_VERSION")}},
                });
                talk.write(&format!("{initialize}\n")).await?;
                talk.until(|line| codex_result(line, "famulen-init"))
                    .await?;
                let query = json!({"id": "famulen-query", "method": "account/rateLimits/read", "params": null});
                talk.write(&format!("{{\"method\":\"initialized\"}}\n{query}\n"))
                    .await?;
                codex_limits(
                    &talk
                        .until(|line| codex_result(line, "famulen-query"))
                        .await??,
                )
            }
        }
    };
    tokio::time::timeout(timeout, asked).await.ok().flatten()
}

/// The result `line` carries when it answers the request `id`: `Some(None)`
/// for an error.
fn codex_result(line: &str, id: &str) -> Option<Option<Value>> {
    let reply = serde_json::from_str::<Value>(line).ok()?;
    (reply["id"] == id).then(|| reply.get("result").cloned())
}

/// A process started only to answer, killed when it is dropped.
struct Talk {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}

impl Talk {
    fn start(mut command: Command) -> Option<Self> {
        let mut child = command
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = BufReader::new(child.stdout.take()?).lines();
        Some(Self {
            _child: child,
            stdin,
            stdout,
        })
    }

    async fn write(&mut self, text: &str) -> Option<()> {
        self.stdin.write_all(text.as_bytes()).await.ok()?;
        self.stdin.flush().await.ok()
    }

    /// Read lines until `read` settles on one; `None` once the output ends.
    async fn until<T>(&mut self, mut read: impl FnMut(&str) -> Option<T>) -> Option<T> {
        while let Some(line) = self.stdout.next_line().await.ok()? {
            if let Some(got) = read(&line) {
                return Some(got);
            }
        }
        None
    }
}

/// The limits last heard of one account, with when they were asked for.
#[derive(Debug, Default)]
struct Fresh {
    limits: Option<Vec<Limit>>,
    asked: Option<Instant>,
}

impl Fresh {
    fn stale(&self, now: Instant) -> bool {
        self.asked
            .is_none_or(|at| now.saturating_duration_since(at) >= LIMITS_TTL)
    }

    /// A failure keeps the limits there were, and waits as long as an
    /// answer before asking again.
    fn settle(&mut self, now: Instant, got: Option<Vec<Limit>>) {
        self.asked = Some(now);
        if got.is_some() {
            self.limits = got;
        }
    }
}

/// The limits of every account asked about, each asked once at a time.
#[derive(Clone, Default)]
pub struct Limits {
    held: Arc<Mutex<HashMap<Account, Arc<tokio::sync::Mutex<Fresh>>>>>,
}

impl Limits {
    /// The limits of `account`, asked of its CLI when those held are older
    /// than [`LIMITS_TTL`]; `None` while it has never answered.
    pub async fn of(&self, account: &Account) -> Option<Vec<Limit>> {
        self.read(account, Instant::now(), probe).await
    }

    async fn read(
        &self,
        account: &Account,
        now: Instant,
        ask: impl AsyncFnOnce(&Account) -> Option<Vec<Limit>>,
    ) -> Option<Vec<Limit>> {
        let fresh = {
            let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
            held.entry(account.clone()).or_default().clone()
        };
        let mut fresh = fresh.lock().await;
        if fresh.stale(now) {
            let got = ask(account).await;
            fresh.settle(now, got);
        }
        fresh.limits.clone()
    }
}

#[cfg(test)]
mod tests;
