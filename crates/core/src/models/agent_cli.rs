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
pub(super) mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::json;

    use super::*;
    use crate::models::{ModelKind, Price, Usage};

    /// Writes a perl script standing in for `claude`: it prints what `body`
    /// leaves in `$out` as JSON, with `@ARGV` and `$stdin` at hand.
    pub(in crate::models) fn fake_cli(name: &str, body: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("famulen-models-cli-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("claude");
        let script = format!(
            "#!/usr/bin/env perl\nuse strict;\nuse JSON::PP;\nmy $stdin = do {{ local $/; <STDIN> }};\nmy $out;\n{body}\nprint encode_json($out);\n"
        );
        // Writing the script from this process and running it at once fails
        // now and then with ETXTBSY: a child forked by another test thread
        // inherits the open write handle until it execs. `install` writes the
        // executable in a process of its own, so no handle leaks here.
        let source = dir.join("claude.pl");
        std::fs::write(&source, script).unwrap();
        let installed = std::process::Command::new("install")
            .args(["-m", "755"])
            .arg(&source)
            .arg(&path)
            .status()
            .unwrap();
        assert!(installed.success(), "install {}", path.display());
        path
    }

    fn spec(executable: &Path) -> ModelSpec {
        ModelSpec {
            id: "m1".into(),
            name: "claude code".into(),
            kind: ModelKind::AgentCli,
            endpoint: Some(executable.display().to_string()),
            model: "haiku".into(),
            cli: Default::default(),
            price: Price::default(),
        }
    }

    const ECHO: &str = r#"$out = { type => "result", is_error => JSON::PP::false,
        result => encode_json({ args => \@ARGV, stdin => $stdin }),
        usage => { input_tokens => 2, cache_creation_input_tokens => 100,
                   cache_read_input_tokens => 10, output_tokens => 7 } };"#;

    fn value_after<'a>(args: &'a [Value], flag: &str) -> Option<&'a str> {
        let at = args.iter().position(|arg| arg == flag)?;
        args.get(at + 1).and_then(Value::as_str)
    }

    #[tokio::test]
    async fn runs_one_turn_without_tools_or_the_users_settings() {
        let cli = fake_cli("flags", ECHO);

        let completion = complete(&spec(&cli), None, "be brief", "hello", None)
            .await
            .unwrap();

        let echoed: Value = serde_json::from_str(&completion.text).unwrap();
        let args = echoed["args"].as_array().unwrap();
        assert!(args.contains(&json!("--print")));
        assert!(args.contains(&json!("--strict-mcp-config")));
        assert_eq!(value_after(args, "--output-format"), Some("json"));
        assert_eq!(value_after(args, "--tools"), Some(""));
        assert_eq!(value_after(args, "--setting-sources"), Some(""));
        assert_eq!(value_after(args, "--model"), Some("haiku"));
        assert_eq!(
            value_after(args, "--append-system-prompt"),
            Some("be brief")
        );
        assert_eq!(value_after(args, "--json-schema"), None);
        assert_eq!(echoed["stdin"], "hello");
        assert_eq!(completion.json, None);
    }

    #[tokio::test]
    async fn passes_the_roles_effort_only_when_one_is_chosen() {
        let cli = fake_cli("effort", ECHO);
        let args_of = async |effort| {
            let completion = complete(&spec(&cli), effort, "", "hello", None)
                .await
                .unwrap();
            let echoed: Value = serde_json::from_str(&completion.text).unwrap();
            echoed["args"].as_array().unwrap().clone()
        };

        let chosen = args_of(Some("xhigh")).await;
        let default = args_of(None).await;

        assert_eq!(value_after(&chosen, "--effort"), Some("xhigh"));
        assert!(!default.contains(&json!("--effort")), "{default:?}");
    }

    #[tokio::test]
    async fn a_brief_call_takes_the_roles_effort_over_its_own() {
        let cli = fake_cli("brief-effort", ECHO);

        let completion = complete_brief(&spec(&cli), Some("high"), "judge it", "hello", None)
            .await
            .unwrap();

        let echoed: Value = serde_json::from_str(&completion.text).unwrap();
        let args = echoed["args"].as_array().unwrap();
        assert_eq!(value_after(args, "--effort"), Some("high"));
        assert_eq!(args.iter().filter(|a| *a == "--effort").count(), 1);
    }

    #[tokio::test]
    async fn a_brief_call_replaces_the_system_prompt_and_does_not_think() {
        let cli = fake_cli(
            "brief",
            r#"$out = { type => "result", is_error => JSON::PP::false,
                result => encode_json({ args => \@ARGV, thinking => $ENV{MAX_THINKING_TOKENS} }) };"#,
        );

        let completion = complete_brief(&spec(&cli), None, "judge it", "hello", None)
            .await
            .unwrap();

        let echoed: Value = serde_json::from_str(&completion.text).unwrap();
        let args = echoed["args"].as_array().unwrap();
        assert_eq!(value_after(args, "--system-prompt"), Some("judge it"));
        assert_eq!(value_after(args, "--append-system-prompt"), None);
        assert_eq!(value_after(args, "--effort"), Some("low"));
        assert_eq!(value_after(args, "--tools"), Some(""));
        assert!(args.contains(&json!("--strict-mcp-config")));
        assert_eq!(echoed["thinking"], "0");
    }

    #[tokio::test]
    async fn counts_cached_input_as_input_tokens() {
        let cli = fake_cli("usage", ECHO);

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
        let cli = fake_cli(
            "schema",
            r#"my ($at) = grep { $ARGV[$_] eq "--json-schema" } 0..$#ARGV;
                $out = { type => "result", is_error => JSON::PP::false,
                result => "", structured_output => decode_json($ARGV[$at + 1]) };"#,
        );
        let schema = json!({ "label": "a" });

        let completion = complete(&spec(&cli), None, "", "hello", Some(&schema))
            .await
            .unwrap();

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
        let cli = fake_cli(
            "text-json",
            r#"$out = { type => "result", is_error => JSON::PP::false, result => "{\"label\":\"b\"}" };"#,
        );

        let completion = complete(&spec(&cli), None, "", "hello", Some(&json!({})))
            .await
            .unwrap();

        assert_eq!(completion.json, Some(json!({ "label": "b" })));
    }

    #[tokio::test]
    async fn reports_an_error_result() {
        let cli = fake_cli(
            "is-error",
            r#"$out = { type => "result", is_error => JSON::PP::true, result => "rate limited" };"#,
        );

        let error = complete(&spec(&cli), None, "", "hello", None)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("rate limited"), "{error}");
    }

    #[tokio::test]
    async fn reports_stderr_of_a_program_that_ends_without_reading_its_prompt() {
        let cli = fake_cli(
            "claude-unread",
            "BEGIN { print STDERR 'not logged in'; exit 1 }",
        );
        let prompt = "x".repeat(1 << 20);
        let error = complete(&spec(&cli), None, "", &prompt, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not logged in"), "{error:#}");
    }

    #[tokio::test]
    async fn reports_stderr_when_the_output_is_not_json() {
        let cli = fake_cli("crash", r#"print STDERR "not logged in"; exit 1;"#);

        let error = complete(&spec(&cli), None, "", "hello", None)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("not logged in"), "{error}");
    }
}
