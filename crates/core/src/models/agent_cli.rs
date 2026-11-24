use std::process::{Output, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::{io::AsyncWriteExt, process::Command};

use super::{Cli, Completion, ModelSpec, codex, json_in_text};
use crate::claude::protocol::turn_usage;

/// The arguments for one turn of Claude Code in print mode.
///
/// No tools, no MCP servers and no setting sources: the call is a plain
/// completion, so nothing from the host user's configuration may let it act.
fn args(
    spec: &ModelSpec,
    effort: Option<&str>,
    system: &str,
    schema: Option<&Value>,
    brief: bool,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "--print",
        "--output-format",
        "json",
        "--tools",
        "",
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-session-persistence",
    ]
    .map(String::from)
    .into();
    if !spec.model.is_empty() {
        args.push("--model".into());
        args.push(spec.model.clone());
    }
    if let Some(effort) = effort.filter(|e| !e.is_empty()).or(brief.then_some("low")) {
        args.push("--effort".into());
        args.push(effort.into());
    }
    if brief {
        // Claude Code's own prompt is most of a call's tokens and time; a
        // one-question judgement needs none of it.
        args.extend(
            [
                "--system-prompt",
                system,
                "--exclude-dynamic-system-prompt-sections",
            ]
            .map(String::from),
        );
    } else if !system.is_empty() {
        args.push("--append-system-prompt".into());
        args.push(system.into());
    }
    if let Some(schema) = schema {
        args.push("--json-schema".into());
        args.push(schema.to_string());
    }
    args
}

/// Runs the agent CLI once, passing `prompt` on stdin so that it is never
/// read as a flag.
pub(super) async fn complete(
    spec: &ModelSpec,
    effort: Option<&str>,
    system: &str,
    prompt: &str,
    schema: Option<&Value>,
) -> Result<Completion> {
    run(spec, effort, system, prompt, schema, false).await
}

/// [`complete`] for a short judgement: `system` replaces Claude Code's own
/// prompt, and the model answers without thinking, at low effort unless the
/// role chose one.
pub(super) async fn complete_brief(
    spec: &ModelSpec,
    effort: Option<&str>,
    system: &str,
    prompt: &str,
    schema: Option<&Value>,
) -> Result<Completion> {
    run(spec, effort, system, prompt, schema, true).await
}

/// Runs `command` to its end with `input` on its stdin, without Famulen's own
/// secrets in its environment.
pub(super) async fn run_cli(mut command: Command, input: &str) -> Result<Output> {
    crate::config::hide_secrets(&mut command);
    let program = command.as_std().get_program().to_owned();
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("failed to start {}", program.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A program that ends without reading says why by itself.
        let _ = stdin.write_all(input.as_bytes()).await;
    }
    Ok(child.wait_with_output().await?)
}

async fn run(
    spec: &ModelSpec,
    effort: Option<&str>,
    system: &str,
    prompt: &str,
    schema: Option<&Value>,
    brief: bool,
) -> Result<Completion> {
    if spec.cli == Cli::Codex {
        return codex::complete(spec, effort, system, prompt, schema, brief).await;
    }
    let executable = spec.executable();
    let mut command = Command::new(executable);
    command.args(args(spec, effort, system, schema, brief));
    if brief {
        command.env("MAX_THINKING_TOKENS", "0");
    }
    let output = run_cli(command, prompt).await?;

    let Ok(result) = serde_json::from_slice::<Value>(&output.stdout) else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "{executable} exited with {}: {}",
            output.status,
            stderr.trim()
        );
    };
    let text = result
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if result.get("is_error").and_then(Value::as_bool) == Some(true) || !output.status.success() {
        bail!("{executable} failed ({}): {text}", output.status);
    }

    let json = match schema {
        Some(_) => Some(
            result
                .get("structured_output")
                .filter(|value| !value.is_null())
                .cloned()
                .or_else(|| json_in_text(&text))
                .with_context(|| format!("{} answered without JSON: {text}", spec.name))?,
        ),
        None => None,
    };
    Ok(Completion {
        usage: turn_usage(&result),
        text,
        json,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::models::fake_cli::FakeCli;
    use crate::models::{ModelKind, Price, Usage};

    fn spec(cli: &FakeCli) -> ModelSpec {
        ModelSpec {
            id: "m1".into(),
            name: "claude code".into(),
            kind: ModelKind::AgentCli,
            endpoint: Some(cli.program().into()),
            model: "haiku".into(),
            cli: Default::default(),
            price: Price::default(),
        }
    }

    fn answering(name: &str) -> FakeCli {
        FakeCli::new(name).prints(&[json!({
            "type": "result", "is_error": false, "result": "hi",
            "usage": { "input_tokens": 2, "cache_creation_input_tokens": 100,
                       "cache_read_input_tokens": 10, "output_tokens": 7 },
        })])
    }

    #[tokio::test]
    async fn runs_one_turn_without_tools_or_the_users_settings() {
        let cli = answering("flags");

        let completion = complete(&spec(&cli), None, "be brief", "hello", None)
            .await
            .unwrap();

        let args = cli.args();
        assert!(args.contains(&"--print".into()));
        assert!(args.contains(&"--strict-mcp-config".into()));
        let after = |flag| cli.value_after(flag);
        assert_eq!(after("--output-format").as_deref(), Some("json"));
        assert_eq!(after("--tools").as_deref(), Some(""));
        assert_eq!(after("--setting-sources").as_deref(), Some(""));
        assert_eq!(after("--model").as_deref(), Some("haiku"));
        assert_eq!(after("--append-system-prompt").as_deref(), Some("be brief"));
        assert_eq!(after("--json-schema"), None);
        assert_eq!(cli.stdin(), "hello");
        assert_eq!(completion.text, "hi");
        assert_eq!(completion.json, None);
    }

    #[tokio::test]
    async fn passes_the_roles_effort_only_when_one_is_chosen() {
        let cli = answering("effort");
        for (effort, expected) in [(Some("xhigh"), Some("xhigh")), (None, None)] {
            complete(&spec(&cli), effort, "", "hello", None)
                .await
                .unwrap();
            assert_eq!(cli.value_after("--effort").as_deref(), expected);
        }
        assert!(!cli.args().contains(&"--effort".into()));
    }

    #[tokio::test]
    async fn a_brief_call_takes_the_roles_effort_over_its_own() {
        let cli = answering("brief-effort");

        complete_brief(&spec(&cli), Some("high"), "judge it", "hello", None)
            .await
            .unwrap();

        assert_eq!(cli.value_after("--effort").as_deref(), Some("high"));
        assert_eq!(cli.args().iter().filter(|a| *a == "--effort").count(), 1);
    }

    #[tokio::test]
    async fn a_brief_call_replaces_the_system_prompt_and_does_not_think() {
        let cli = answering("brief");

        complete_brief(&spec(&cli), None, "judge it", "hello", None)
            .await
            .unwrap();

        let after = |flag| cli.value_after(flag);
        assert_eq!(after("--system-prompt").as_deref(), Some("judge it"));
        assert_eq!(after("--append-system-prompt"), None);
        assert_eq!(after("--effort").as_deref(), Some("low"));
        assert_eq!(after("--tools").as_deref(), Some(""));
        assert!(cli.args().contains(&"--strict-mcp-config".into()));
        assert_eq!(cli.thinking(), "0");
    }

    #[tokio::test]
    async fn counts_cached_input_as_input_tokens() {
        let cli = answering("usage");

        let completion = complete(&spec(&cli), None, "", "hello", None)
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
    async fn passes_the_schema_and_reads_the_structured_output() {
        let schema = json!({ "label": "a" });
        let cli = FakeCli::new("schema").prints(&[json!({
            "type": "result", "is_error": false, "result": "", "structured_output": schema,
        })]);

        let completion = complete(&spec(&cli), None, "", "hello", Some(&schema))
            .await
            .unwrap();

        assert_eq!(cli.value_after("--json-schema"), Some(schema.to_string()));
        assert_eq!(completion.json, Some(schema));
        assert_eq!(
            completion.usage,
            Usage {
                input_tokens: None,
                output_tokens: None,
                calls: 1,
            }
        );
    }

    #[tokio::test]
    async fn reads_json_from_the_result_without_structured_output() {
        let cli = FakeCli::new("text-json").prints(&[json!({
            "type": "result", "is_error": false, "result": r#"{"label":"b"}"#,
        })]);

        let completion = complete(&spec(&cli), None, "", "hello", Some(&json!({})))
            .await
            .unwrap();

        assert_eq!(completion.json, Some(json!({ "label": "b" })));
    }

    #[tokio::test]
    async fn reports_an_error_result() {
        let cli = FakeCli::new("is-error").prints(&[json!({
            "type": "result", "is_error": true, "result": "rate limited",
        })]);

        let error = complete(&spec(&cli), None, "", "hello", None)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("rate limited"), "{error}");
    }

    #[tokio::test]
    async fn reports_stderr_of_a_program_that_ends_without_reading_its_prompt() {
        let cli = FakeCli::new("claude-unread")
            .without_reading()
            .fails("not logged in");
        let prompt = "x".repeat(1 << 20);
        let error = complete(&spec(&cli), None, "", &prompt, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not logged in"), "{error:#}");
    }

    #[tokio::test]
    async fn reports_stderr_when_the_output_is_not_json() {
        let cli = FakeCli::new("crash").fails("not logged in");

        let error = complete(&spec(&cli), None, "", "hello", None)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("not logged in"), "{error}");
    }
}
