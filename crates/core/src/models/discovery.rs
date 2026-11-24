//! What models this machine can use: what Claude Code and Codex say they
//! offer, and what a local Ollama holds.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::Value;
use tokio::process::Command;

use super::agent_cli::run_cli;
use super::{Cli, ModelKind, ModelSpec};

pub const OLLAMA: &str = "http://localhost:11434";

const CLI_TIMEOUT: Duration = Duration::from_secs(60);

/// Where a model on this machine comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Claude,
    Codex,
    Ollama,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::Claude, Source::Codex, Source::Ollama];

    pub fn as_str(self) -> &'static str {
        match self {
            Source::Claude => "claude",
            Source::Codex => "codex",
            Source::Ollama => "ollama",
        }
    }

    /// The source `spec` would be found in, if it is called the way a found
    /// model is.
    pub fn of(spec: &ModelSpec) -> Option<Self> {
        let endpoint = spec.endpoint.as_deref().filter(|e| !e.is_empty());
        match (spec.kind, spec.cli) {
            (ModelKind::AgentCli, cli) if endpoint.is_none_or(|e| e == cli.executable()) => {
                Some(match cli {
                    Cli::Claude => Source::Claude,
                    Cli::Codex => Source::Codex,
                })
            }
            (ModelKind::Local, _)
                if endpoint.map(|e| e.trim_end_matches('/')) == Some(&ollama_endpoint()) =>
            {
                Some(Source::Ollama)
            }
            _ => None,
        }
    }

    fn spec(self, model: &str, name: &str) -> ModelSpec {
        let (kind, endpoint, cli, name) = match self {
            Source::Claude => (
                ModelKind::AgentCli,
                None,
                Cli::Claude,
                format!("Claude {name}"),
            ),
            Source::Codex => (
                ModelKind::AgentCli,
                None,
                Cli::Codex,
                format!("{name}（Codex）"),
            ),
            Source::Ollama => (
                ModelKind::Local,
                Some(ollama_endpoint()),
                Cli::default(),
                format!("{name}（このマシン）"),
            ),
        };
        ModelSpec {
            id: found_id(self, model),
            name,
            kind,
            endpoint,
            model: model.to_owned(),
            cli,
            price: Default::default(),
        }
    }
}

fn ollama_endpoint() -> String {
    format!("{OLLAMA}/v1")
}

/// The id a model found in `source` is registered under, the same each time.
pub(super) fn found_id(source: Source, model: &str) -> String {
    let source = source.as_str();
    if model.starts_with(&format!("{source}-")) {
        model.to_owned()
    } else {
        format!("{source}-{model}")
    }
}

/// The models the Claude Code at `bin` offers the account it is logged in as.
pub async fn claude_models(bin: &str) -> Result<Vec<ModelSpec>> {
    let request = crate::claude::protocol::initialize_request("famulen-models");
    let out = output(
        bin,
        &[
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--tools",
            "",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--no-session-persistence",
        ],
        &format!("{request}\n"),
    )
    .await?;
    let listed = out
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|line| line["type"] == "control_response")
        .with_context(|| format!("{bin} did not answer the initialize request"))?;
    let models = listed["response"]["response"]["models"]
        .as_array()
        .with_context(|| format!("{bin} listed no models"))?;
    let mut found = Vec::new();
    for m in models {
        // `default` stands for one of the others.
        if let Some(model) = m["value"].as_str().filter(|v| *v != "default") {
            let name = m["displayName"].as_str().unwrap_or(model);
            found.push(Source::Claude.spec(model, name));
        }
    }
    ensure!(!found.is_empty(), "{bin} listed no models");
    Ok(found)
}

/// The models the Codex at `bin` offers for picking.
pub async fn codex_models(bin: &str) -> Result<Vec<ModelSpec>> {
    let out = output(bin, &["debug", "models"], "").await?;
    let listed: Value = serde_json::from_str(&out)
        .with_context(|| format!("{bin} answered with a catalog that is not JSON"))?;
    let mut found = Vec::new();
    for m in listed["models"].as_array().into_iter().flatten() {
        if let Some(model) = m["slug"].as_str().filter(|_| m["visibility"] == "list") {
            let name = m["display_name"].as_str().unwrap_or(model);
            found.push(Source::Codex.spec(model, name));
        }
    }
    ensure!(!found.is_empty(), "{bin} listed no models");
    Ok(found)
}

/// The models the Ollama at `url` holds.
pub async fn ollama_models(http: &reqwest::Client, url: &str) -> Result<Vec<ModelSpec>> {
    let listed: Value = http
        .get(format!("{url}/api/tags"))
        .timeout(Duration::from_secs(2))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(listed["models"]
        .as_array()
        .context("the list of models is missing")?
        .iter()
        .filter_map(|m| m["name"].as_str())
        .map(|model| Source::Ollama.spec(model, model))
        .collect())
}

/// The models an OpenAI-compatible server without a key has.
pub async fn server_models(
    http: &reqwest::Client,
    endpoint: impl AsRef<str>,
) -> Result<Vec<String>> {
    let url = format!("{}/models", endpoint.as_ref().trim().trim_end_matches('/'));
    let listed: Value = http
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .with_context(|| format!("could not reach {url}"))?
        .error_for_status()?
        .json()
        .await
        .with_context(|| format!("{url} answered with a body that is not JSON"))?;
    Ok(listed["data"]
        .as_array()
        .with_context(|| format!("{url} listed no models"))?
        .iter()
        .filter_map(|m| m["id"].as_str())
        .map(str::to_owned)
        .collect())
}

async fn output(bin: &str, args: &[&str], input: &str) -> Result<String> {
    let mut command = Command::new(bin);
    command.args(args);
    let output = tokio::time::timeout(CLI_TIMEOUT, run_cli(command, input))
        .await
        .with_context(|| format!("{bin} did not list its models in time"))??;
    ensure!(
        output.status.success(),
        "{bin} exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::models::fake_cli::FakeCli;

    fn listed(found: &[ModelSpec]) -> Vec<(&str, &str, &str)> {
        found
            .iter()
            .map(|m| (m.id.as_str(), m.name.as_str(), m.model.as_str()))
            .collect()
    }

    #[tokio::test]
    async fn claudes_models_are_the_ones_it_answers_to_initialize_with() {
        let cli = FakeCli::new("models-claude").prints(&[
            json!({ "type": "system" }),
            json!({ "type": "control_response", "response": { "response": { "models": [
                { "value": "default", "displayName": "Default" },
                { "value": "opus", "displayName": "Opus 5.5" },
                { "value": "claude-opus-5", "displayName": "Opus 5" },
            ] } } }),
        ]);

        let found = claude_models(cli.program()).await.unwrap();

        let asked: Value = serde_json::from_str(&cli.stdin()).unwrap();
        assert_eq!(asked["request"]["subtype"], "initialize");

        assert_eq!(
            listed(&found),
            [
                ("claude-opus", "Claude Opus 5.5", "opus"),
                ("claude-opus-5", "Claude Opus 5", "claude-opus-5"),
            ]
        );
        assert!(found.iter().all(|m| Source::of(m) == Some(Source::Claude)));
    }

    #[tokio::test]
    async fn codexs_models_are_the_ones_its_catalog_lists() {
        let cli = FakeCli::new("models-codex").prints(&[json!({ "models": [
            { "slug": "gpt-6-sol", "display_name": "GPT-6-Sol", "visibility": "list" },
            { "slug": "codex-auto-review", "display_name": "Review", "visibility": "hide" },
        ] })]);

        let found = codex_models(cli.program()).await.unwrap();

        assert_eq!(cli.args(), ["debug", "models"]);

        assert_eq!(
            listed(&found),
            [("codex-gpt-6-sol", "GPT-6-Sol（Codex）", "gpt-6-sol")]
        );
        assert_eq!(found[0].cli, Cli::Codex);
        assert_eq!(Source::of(&found[0]), Some(Source::Codex));
    }

    #[tokio::test]
    async fn a_program_that_is_missing_or_lists_nothing_is_an_error() {
        assert!(claude_models("/nonexistent/claude").await.is_err());
        assert!(codex_models("/nonexistent/codex").await.is_err());
        let empty = FakeCli::new("models-empty").prints(&[json!({ "models": [] })]);
        assert!(claude_models(empty.program()).await.is_err());
        assert!(codex_models(empty.program()).await.is_err());
    }

    #[tokio::test]
    async fn a_servers_models_are_the_ones_it_lists() {
        let app = axum::Router::new().route(
            "/v1/models",
            axum::routing::get(|| async { r#"{"data":[{"id":"a"},{"id":"b"}]}"# }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = reqwest::Client::new();

        assert_eq!(server_models(&http, &url).await.unwrap(), ["a", "b"]);
        assert!(server_models(&http, "http://127.0.0.1:1/v1").await.is_err());
        assert!(server_models(&http, "not a url").await.is_err());
    }

    #[tokio::test]
    async fn ollamas_models_are_the_ones_it_holds() {
        let app = axum::Router::new().route(
            "/api/tags",
            axum::routing::get(|| async { r#"{"models":[{"name":"qwen3:8b"}]}"# }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let found = ollama_models(&reqwest::Client::new(), &url).await.unwrap();

        assert_eq!(
            listed(&found),
            [("ollama-qwen3:8b", "qwen3:8b（このマシン）", "qwen3:8b")]
        );
        assert_eq!(Source::of(&found[0]), Some(Source::Ollama));
    }
}
