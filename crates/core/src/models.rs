//! Calling registered models for roles other than the secretary.

mod agent_cli;
mod codex;
mod discovery;
#[cfg(test)]
mod fake_cli;
mod openai;
mod registry;
mod spec;

pub use discovery::*;
pub use registry::*;
pub use spec::*;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::gateway::Gateway;

/// The gateway a model's call goes through, and who it is made for, as the
/// gateway's log names them.
#[derive(Clone, Copy)]
pub struct Via<'a> {
    pub gateway: &'a Gateway,
    pub actor: &'a str,
}

/// What a model answered to one call.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub text: String,
    /// The answer as JSON, present when a schema was given.
    pub json: Option<Value>,
    pub usage: Usage,
}

/// Asks `spec` once. With `schema`, the answer must be JSON and is returned
/// in [`Completion::json`]. `effort` reaches only an agent CLI, the one kind
/// that takes it.
pub async fn complete(
    spec: &ModelSpec,
    effort: Option<&str>,
    system: &str,
    prompt: &str,
    schema: Option<&Value>,
    via: Option<Via<'_>>,
) -> Result<Completion> {
    match spec.kind {
        ModelKind::AgentCli => agent_cli::complete(spec, effort, system, prompt, schema).await,
        ModelKind::Local | ModelKind::Api | ModelKind::Judge => {
            openai::complete(spec, system, prompt, schema, via).await
        }
        ModelKind::Transcribe => bail!(
            "{} is a {} model, which cannot complete text",
            spec.name,
            spec.kind.label()
        ),
    }
}

/// [`complete`] with `system` in place of an agent CLI's own prompt, which is
/// most of a short call; other kinds are asked as usual.
pub async fn complete_alone(
    spec: &ModelSpec,
    effort: Option<&str>,
    system: &str,
    prompt: &str,
    schema: Option<&Value>,
    via: Option<Via<'_>>,
) -> Result<Completion> {
    match spec.kind {
        ModelKind::AgentCli => {
            agent_cli::complete_alone(spec, effort, system, prompt, schema).await
        }
        _ => complete(spec, effort, system, prompt, schema, via).await,
    }
}

/// [`complete`] for a short judgement. An agent CLI is asked with `system`
/// in place of its own prompt and without thinking, at low effort unless
/// `effort` says otherwise; other kinds are asked as usual.
pub async fn complete_brief(
    spec: &ModelSpec,
    effort: Option<&str>,
    system: &str,
    prompt: &str,
    schema: Option<&Value>,
    via: Option<Via<'_>>,
) -> Result<Completion> {
    match spec.kind {
        ModelKind::AgentCli => {
            agent_cli::complete_brief(spec, effort, system, prompt, schema).await
        }
        _ => complete(spec, effort, system, prompt, schema, via).await,
    }
}

/// Has `spec` pick one of `labels` for `item`, following `instruction`.
///
/// Every kind is asked through [`complete`] with the labels as a JSON schema
/// enum; a judge API is called as an OpenAI-compatible chat completion.
pub async fn classify(
    spec: &ModelSpec,
    effort: Option<&str>,
    labels: &[&str],
    instruction: &str,
    item: &str,
    via: Option<Via<'_>>,
) -> Result<(String, Usage)> {
    let schema = json!({
        "type": "object",
        "properties": { "label": { "type": "string", "enum": labels } },
        "required": ["label"],
        "additionalProperties": false,
    });
    let completion = complete(spec, effort, instruction, item, Some(&schema), via).await?;
    let label = completion
        .json
        .as_ref()
        .and_then(|json| json.get("label"))
        .and_then(Value::as_str)
        .with_context(|| {
            format!(
                "{} answered without a label: {}",
                spec.name, completion.text
            )
        })?;
    if !labels.contains(&label) {
        bail!(
            "{} answered {label:?}, which is not one of {labels:?}",
            spec.name
        );
    }
    Ok((label.to_string(), completion.usage))
}

/// Finds a JSON object or array in a model's prose answer.
fn json_in_text(text: &str) -> Option<Value> {
    let text = text.trim();
    if let Ok(value) = serde_json::from_str(text) {
        return Some(value);
    }
    [('{', '}'), ('[', ']')]
        .into_iter()
        .find_map(|(open, close)| {
            let start = text.find(open)?;
            let end = text.rfind(close)?;
            (start < end)
                .then(|| serde_json::from_str(&text[start..=end]).ok())
                .flatten()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_found_in_prose_and_fences() {
        let table = [
            (r#"{"a":1}"#, Some(json!({ "a": 1 }))),
            ("Here:\n```json\n{\"a\": 1}\n```", Some(json!({ "a": 1 }))),
            ("the list is [1, 2] ok", Some(json!([1, 2]))),
            ("no json here", None),
            ("} backwards {", None),
        ];
        for (text, expected) in table {
            assert_eq!(json_in_text(text), expected, "{text}");
        }
    }

    #[tokio::test]
    async fn a_transcription_model_cannot_complete_text() {
        let spec = ModelSpec {
            id: "w".into(),
            name: "whisper".into(),
            kind: ModelKind::Transcribe,
            endpoint: Some("http://127.0.0.1:1/v1".into()),
            model: "whisper-1".into(),
            cli: Default::default(),
            price: Price::default(),
        };

        let error = complete(&spec, None, "", "hi", None, None)
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "whisper is a 文字起こし model, which cannot complete text"
        );
    }
}
