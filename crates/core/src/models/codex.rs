//! Codex, run once per call in its non-interactive mode.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::process::Command;

use super::agent_cli::run_cli;
use super::{Completion, ModelSpec, Usage, json_in_text};

/// What Codex could act with. All of it is off: the call is a plain
/// completion over text that may come from outside.
const FEATURES_OFF: [&str; 12] = [
    "apps",
    "code_mode_host",
    "goals",
    "hooks",
    "image_generation",
    "memories",
    "multi_agent",
    "plugins",
    "shell_tool",
    "skill_search",
    "unified_exec",
    "view_image",
];

fn args(spec: &ModelSpec, effort: Option<&str>, instructions: &str, brief: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "exec",
        "--json",
        "--ephemeral",
        "--ignore-user-config",
        "--ignore-rules",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "--config",
        "web_search=\"disabled\"",
        "--config",
        "project_doc_max_bytes=0",
    ]
    .map(String::from)
    .into();
    let mut set = |key: &str, value: String| {
        args.push("--config".into());
        args.push(format!("{key}={value}"));
    };
    for feature in FEATURES_OFF {
        set(&format!("features.{feature}"), "false".into());
    }
    if let Some(effort) = effort.filter(|e| !e.is_empty()).or(brief.then_some("low")) {
        set("model_reasoning_effort", toml_string(effort));
    }
    if !instructions.is_empty() {
        set("developer_instructions", toml_string(instructions));
    }
    if !spec.model.is_empty() {
        args.push("--model".into());
        args.push(spec.model.clone());
    }
    args.push("-".into());
    args
}

fn toml_string(text: &str) -> String {
    toml::Value::String(text.into()).to_string()
}

/// Codex's own `--output-schema` is not used: it takes only strict schemas,
/// which leave no room for an optional property.
fn instructions(system: &str, schema: Option<&Value>) -> String {
    match schema {
        Some(schema) => format!(
            "{system}\n\nAnswer with one JSON value matching this JSON Schema and nothing else:\n{schema}"
        )
        .trim_start()
        .to_owned(),
        None => system.to_owned(),
    }
}

/// Runs Codex once, passing `prompt` on stdin so that it is never read as a
/// flag.
pub(super) async fn complete(
    spec: &ModelSpec,
    effort: Option<&str>,
    system: &str,
    prompt: &str,
    schema: Option<&Value>,
    brief: bool,
) -> Result<Completion> {
    let executable = spec.executable();
    let mut command = Command::new(executable);
    command.args(args(spec, effort, &instructions(system, schema), brief));
    let output = run_cli(command, prompt).await?;

    let mut answer = None;
    let mut usage = Usage {
        calls: 1,
        ..Default::default()
    };
    let mut failure = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match event["type"].as_str() {
            Some("item.completed") if event["item"]["type"] == "agent_message" => {
                answer = event["item"]["text"].as_str().map(str::to_owned);
            }
            Some("turn.completed") => {
                usage.input_tokens = event["usage"]["input_tokens"].as_u64();
                usage.output_tokens = event["usage"]["output_tokens"].as_u64();
            }
            Some("turn.failed") => {
                failure = event["error"]["message"].as_str().map(str::to_owned);
            }
            _ => {}
        }
    }
    let text = match answer {
        Some(text) if output.status.success() && failure.is_none() => text,
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "{executable} failed ({}): {}",
                output.status,
                failure.as_deref().unwrap_or(stderr.trim())
            );
        }
    };
    let json = match schema {
        Some(_) => Some(
            json_in_text(&text)
                .with_context(|| format!("{} answered without JSON: {text}", spec.name))?,
        ),
        None => None,
    };
    Ok(Completion { text, json, usage })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::models::fake_cli::FakeCli;
    use crate::models::{Cli, ModelKind};

    fn spec(cli: &FakeCli) -> ModelSpec {
        ModelSpec {
            id: "m1".into(),
            name: "codex".into(),
            kind: ModelKind::AgentCli,
            endpoint: Some(cli.program().into()),
            model: "gpt-6-luna".into(),
            cli: Cli::Codex,
            price: Default::default(),
        }
    }

    /// Streams a turn whose message is `text`, as Codex does.
    fn answering(name: &str, text: &str) -> FakeCli {
        FakeCli::new(name).prints(&[
            json!({ "type": "turn.started" }),
            json!({ "type": "item.completed", "item": { "type": "agent_message", "text": text } }),
            json!({ "type": "turn.completed", "usage": {
                "input_tokens": 112, "cached_input_tokens": 100, "output_tokens": 7 } }),
        ])
    }

    fn configs(cli: &FakeCli) -> toml::Table {
        let mut text = String::new();
        for pair in cli.args().windows(2).filter(|pair| pair[0] == "--config") {
            text += &pair[1];
            text += "\n";
        }
        text.parse().unwrap()
    }

    #[tokio::test]
    async fn runs_one_turn_without_tools_or_the_users_settings() {
        let cli = answering("codex-flags", "hi");

        let completion = complete(
            &spec(&cli),
            None,
            "be \"brief\"\nplease",
            "hello",
            None,
            false,
        )
        .await
        .unwrap();

        assert_eq!(completion.text, "hi");
        let args = cli.args();
        assert_eq!(args[0], "exec");
        for flag in ["--ephemeral", "--ignore-user-config", "--ignore-rules"] {
            assert!(args.contains(&flag.to_owned()), "{flag}");
        }
        assert_eq!(cli.value_after("--sandbox").as_deref(), Some("read-only"));
        assert_eq!(cli.value_after("--model").as_deref(), Some("gpt-6-luna"));
        assert_eq!(args.last().unwrap(), "-");
        assert_eq!(cli.stdin(), "hello");
        let configs = configs(&cli);
        for feature in FEATURES_OFF {
            assert_eq!(configs["features"][feature].as_bool(), Some(false));
        }
        assert_eq!(configs["web_search"].as_str(), Some("disabled"));
        assert_eq!(
            configs["developer_instructions"].as_str(),
            Some("be \"brief\"\nplease")
        );
        assert!(!configs.contains_key("model_reasoning_effort"));
    }

    #[tokio::test]
    async fn the_roles_effort_wins_and_a_brief_call_without_one_thinks_little() {
        let cli = answering("codex-effort", "hi");
        for (effort, brief, expected) in [
            (Some("xhigh"), false, Some("xhigh")),
            (Some("high"), true, Some("high")),
            (None, true, Some("low")),
            (None, false, None),
        ] {
            complete(&spec(&cli), effort, "", "hello", None, brief)
                .await
                .unwrap();
            let configs = configs(&cli);
            assert_eq!(
                configs
                    .get("model_reasoning_effort")
                    .and_then(|e| e.as_str()),
                expected
            );
        }
    }

    #[tokio::test]
    async fn counts_the_tokens_of_the_turn() {
        let cli = answering("codex-usage", "hi");

        let completion = complete(&spec(&cli), None, "", "hello", None, false)
            .await
            .unwrap();

        assert_eq!(
            completion.usage,
            Usage {
                input_tokens: Some(112),
                output_tokens: Some(7),
                calls: 1,
            }
        );
    }

    #[tokio::test]
    async fn tells_the_schema_and_reads_json_from_the_answer() {
        let cli = answering("codex-schema", r#"{"label":"a"}"#);
        let schema = json!({ "type": "object", "title": "asked for" });

        let completion = complete(&spec(&cli), None, "judge", "hello", Some(&schema), true)
            .await
            .unwrap();

        assert_eq!(completion.json, Some(json!({ "label": "a" })));
        let instructions = configs(&cli)["developer_instructions"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(instructions.starts_with("judge\n"), "{instructions}");
        assert!(instructions.contains("asked for"), "{instructions}");
    }

    #[tokio::test]
    async fn reports_why_the_turn_failed() {
        let cli = FakeCli::new("codex-failed").prints(&[
            json!({ "type": "turn.failed", "error": { "message": "usage limit reached" } }),
        ]);

        let error = complete(&spec(&cli), None, "", "hello", None, false)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("usage limit reached"), "{error}");
    }

    #[tokio::test]
    async fn reports_stderr_when_there_is_no_answer() {
        let cli = FakeCli::new("codex-crash").fails("not logged in");

        let error = complete(&spec(&cli), None, "", "hello", None, false)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("not logged in"), "{error}");
    }
}
