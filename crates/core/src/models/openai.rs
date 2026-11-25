use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::{Completion, ModelSpec, Usage, Via, json_in_text};

const TIMEOUT: Duration = Duration::from_secs(300);

/// Calls `POST {endpoint}/chat/completions` of an OpenAI-compatible API.
pub(super) async fn complete(
    spec: &ModelSpec,
    system: &str,
    prompt: &str,
    schema: Option<&Value>,
    via: Option<Via<'_>>,
) -> Result<Completion> {
    let endpoint = spec
        .endpoint
        .as_deref()
        .with_context(|| format!("{} has no endpoint", spec.name))?;
    let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));

    let mut body = json!({
        "model": spec.model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": prompt },
        ],
    });
    if let Some(schema) = schema {
        body["response_format"] = json!({
            "type": "json_schema",
            "json_schema": { "name": "output", "strict": true, "schema": strict(schema) },
        });
    }

    let request = reqwest::Client::new()
        .post(&url)
        .timeout(TIMEOUT)
        .json(&body);
    let response = match via {
        Some(via) => {
            via.gateway
                .send_for_model(via.actor, &spec.id, request)
                .await
        }
        None => request.send().await.map_err(Into::into),
    }
    .with_context(|| format!("failed to call {url}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let body: String = body.chars().take(500).collect();
        bail!("{url} answered {status}: {body}");
    }
    let response: Value = response
        .json()
        .await
        .with_context(|| format!("{url} answered with a body that is not JSON"))?;

    let text = response
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .with_context(|| format!("{url} answered without choices[0].message.content"))?
        .to_string();
    let json = match schema {
        Some(_) => Some(
            json_in_text(&text)
                .with_context(|| format!("{} answered without JSON: {text}", spec.name))?,
        ),
        None => None,
    };
    let tokens = |name: &str| {
        response
            .get("usage")
            .and_then(|usage| usage.get(name))
            .and_then(Value::as_u64)
    };
    Ok(Completion {
        usage: Usage {
            input_tokens: tokens("prompt_tokens"),
            output_tokens: tokens("completion_tokens"),
            calls: 1,
        },
        text,
        json,
    })
}

/// A schema as strict servers take it, which leave no property out.
fn strict(schema: &Value) -> Value {
    match schema {
        Value::Array(list) => list.iter().map(strict).collect(),
        Value::Object(map) => {
            let mut map: serde_json::Map<String, Value> = map
                .iter()
                .map(|(key, value)| (key.clone(), strict(value)))
                .collect();
            if map.get("type").is_some_and(|t| t == "object")
                && let Some(Value::Object(properties)) = map.get("properties")
            {
                let names = properties.keys().cloned().collect::<Vec<_>>();
                map.insert("required".into(), names.into());
            }
            Value::Object(map)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{Json, Router, extract::State, http::HeaderMap, http::StatusCode, routing::post};

    use crate::gateway::Gateway;

    use super::*;
    use crate::models::{ModelKind, Price};

    #[derive(Default)]
    struct Seen {
        bodies: Vec<Value>,
        authorizations: Vec<Option<String>>,
    }

    /// Serves `/v1/chat/completions` answering `status` and `answer`, and
    /// records what it was sent.
    async fn serve(status: StatusCode, answer: Value) -> (String, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let app = Router::new()
            .route(
                "/v1/chat/completions",
                post(
                    move |State(seen): State<Arc<Mutex<Seen>>>,
                          headers: HeaderMap,
                          Json(body): Json<Value>| async move {
                        let mut seen = seen.lock().unwrap();
                        seen.bodies.push(body);
                        seen.authorizations.push(
                            headers
                                .get("authorization")
                                .map(|value| value.to_str().unwrap().to_string()),
                        );
                        (status, Json(answer))
                    },
                ),
            )
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/v1"), seen)
    }

    /// A gateway whose vault keeps `key` for model `id`, and the vault's
    /// directory, removed when dropped.
    fn keyed_gateway(id: &str, key: &str) -> (Gateway, TempVault) {
        let dir = std::env::temp_dir().join(format!(
            "famulen-model-key-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let vault_key =
            crate::gateway::VaultKey::parse(crate::gateway::VaultKey::generate()).unwrap();
        let vault = crate::gateway::Vault::open(&dir, &vault_key).unwrap();
        let store = famulen_gateway::store::Store::open_in_memory().unwrap();
        let gateway = Gateway::new(
            store.clone(),
            crate::connections::Connections::new(store),
            reqwest::Client::new(),
            Some(vault),
        );
        gateway.put_model_key("owner", id, key).unwrap();
        (gateway, TempVault(dir))
    }

    struct TempVault(std::path::PathBuf);

    impl Drop for TempVault {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn answer(content: &str) -> Value {
        json!({
            "choices": [{ "message": { "role": "assistant", "content": content } }],
            "usage": { "prompt_tokens": 12, "completion_tokens": 3 },
        })
    }

    fn spec(endpoint: &str) -> ModelSpec {
        ModelSpec {
            id: "m1".into(),
            name: "local".into(),
            kind: ModelKind::Local,
            endpoint: Some(endpoint.into()),
            model: "llama".into(),
            cli: Default::default(),
            price: Price::default(),
        }
    }

    #[tokio::test]
    async fn sends_the_model_and_the_system_and_user_messages() {
        let (endpoint, seen) = serve(StatusCode::OK, answer("hello")).await;

        let completion = complete(&spec(&endpoint), "be brief", "hi", None, None)
            .await
            .unwrap();

        assert_eq!(completion.text, "hello");
        assert_eq!(completion.json, None);
        let body = &seen.lock().unwrap().bodies[0];
        assert_eq!(body["model"], "llama");
        assert_eq!(
            body["messages"],
            json!([
                { "role": "system", "content": "be brief" },
                { "role": "user", "content": "hi" },
            ])
        );
        assert!(body.get("response_format").is_none());
    }

    #[tokio::test]
    async fn accepts_an_endpoint_with_a_trailing_slash() {
        let (endpoint, _) = serve(StatusCode::OK, answer("ok")).await;

        let completion = complete(&spec(&format!("{endpoint}/")), "", "hi", None, None)
            .await
            .unwrap();

        assert_eq!(completion.text, "ok");
    }

    #[tokio::test]
    async fn sends_the_api_key_as_a_bearer_token_logged_as_the_role_that_asked() {
        let (endpoint, seen) = serve(StatusCode::OK, answer("ok")).await;
        let (gateway, _vault) = keyed_gateway("m1", "sk-1");
        let via = Via {
            gateway: &gateway,
            actor: "dream",
        };

        complete(&spec(&endpoint), "", "hi", None, Some(via))
            .await
            .unwrap();

        assert_eq!(
            seen.lock().unwrap().authorizations,
            [Some("Bearer sk-1".to_string())]
        );
        let entry = &gateway.log_entries(1).unwrap()[0];
        assert_eq!(
            (entry.tool.as_str(), entry.actor.as_str()),
            ("model.call", "dream")
        );
    }

    #[tokio::test]
    async fn sends_no_authorization_without_an_api_key() {
        let (endpoint, seen) = serve(StatusCode::OK, answer("ok")).await;

        complete(&spec(&endpoint), "", "hi", None, None)
            .await
            .unwrap();

        assert_eq!(seen.lock().unwrap().authorizations, [None]);
    }

    #[tokio::test]
    async fn asks_for_the_schema_and_reads_the_json_answer() {
        let (endpoint, seen) = serve(StatusCode::OK, answer(r#"{"label":"a"}"#)).await;
        let schema = json!({ "type": "object" });

        let completion = complete(&spec(&endpoint), "", "hi", Some(&schema), None)
            .await
            .unwrap();

        assert_eq!(completion.json, Some(json!({ "label": "a" })));
        assert_eq!(
            seen.lock().unwrap().bodies[0]["response_format"],
            json!({
                "type": "json_schema",
                "json_schema": { "name": "output", "strict": true, "schema": schema },
            })
        );
    }

    #[test]
    fn every_property_is_required_of_a_strict_schema() {
        let item = json!({ "type": "object", "properties": { "a": {}, "required": {} }, "required": ["a"] });
        let schema = json!({ "type": "object", "properties": { "items": { "type": "array", "items": item } } });
        let told = strict(&schema);
        assert_eq!(told["required"], json!(["items"]));
        let item = &told["properties"]["items"]["items"];
        assert_eq!(item["required"], json!(["a", "required"]));
        assert_eq!(item["properties"], json!({ "a": {}, "required": {} }));
    }

    #[tokio::test]
    async fn finds_the_json_in_prose_when_the_server_ignores_the_schema() {
        let content = "Sure.\n```json\n{\"label\": \"b\"}\n```\nDone.";
        let (endpoint, _) = serve(StatusCode::OK, answer(content)).await;

        let completion = complete(&spec(&endpoint), "", "hi", Some(&json!({})), None)
            .await
            .unwrap();

        assert_eq!(completion.json, Some(json!({ "label": "b" })));
        assert_eq!(completion.text, content);
    }

    #[tokio::test]
    async fn fails_when_a_schema_was_asked_but_no_json_came_back() {
        let (endpoint, _) = serve(StatusCode::OK, answer("no idea")).await;

        let error = complete(&spec(&endpoint), "", "hi", Some(&json!({})), None)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("without JSON"), "{error}");
    }

    #[tokio::test]
    async fn reads_the_reported_tokens_as_one_call() {
        let (endpoint, _) = serve(StatusCode::OK, answer("ok")).await;

        let completion = complete(&spec(&endpoint), "", "hi", None, None)
            .await
            .unwrap();

        assert_eq!(
            completion.usage,
            Usage {
                input_tokens: Some(12),
                output_tokens: Some(3),
                calls: 1,
            }
        );
    }

    #[tokio::test]
    async fn leaves_tokens_unknown_when_usage_is_not_reported() {
        let answer = json!({ "choices": [{ "message": { "content": "ok" } }] });
        let (endpoint, _) = serve(StatusCode::OK, answer).await;

        let completion = complete(&spec(&endpoint), "", "hi", None, None)
            .await
            .unwrap();

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
    async fn reports_an_error_status_with_its_body() {
        let (endpoint, _) = serve(
            StatusCode::UNAUTHORIZED,
            json!({ "error": { "message": "bad key" } }),
        )
        .await;

        let error = complete(&spec(&endpoint), "", "hi", None, None)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("401"), "{error}");
        assert!(error.contains("bad key"), "{error}");
    }

    #[tokio::test]
    async fn a_judge_picks_a_label_constrained_by_an_enum_schema() {
        let (endpoint, seen) = serve(StatusCode::OK, answer(r#"{"label":"drop"}"#)).await;
        let spec = ModelSpec {
            kind: ModelKind::Judge,
            ..spec(&endpoint)
        };

        let (label, usage) = crate::models::classify(
            &spec,
            None,
            &["keep", "drop"],
            "is it relevant?",
            "an ad",
            None,
        )
        .await
        .unwrap();

        assert_eq!(label, "drop");
        assert_eq!(usage.calls, 1);
        let body = &seen.lock().unwrap().bodies[0];
        assert_eq!(
            body.pointer("/response_format/json_schema/schema/properties/label/enum"),
            Some(&json!(["keep", "drop"]))
        );
        assert_eq!(body["messages"][0]["content"], "is it relevant?");
        assert_eq!(body["messages"][1]["content"], "an ad");
    }

    #[tokio::test]
    async fn a_label_outside_the_choices_is_refused() {
        let (endpoint, _) = serve(StatusCode::OK, answer(r#"{"label":"maybe"}"#)).await;

        let error =
            crate::models::classify(&spec(&endpoint), None, &["keep", "drop"], "", "x", None)
                .await
                .unwrap_err();

        assert!(error.to_string().contains("\"maybe\""), "{error}");
    }

    #[tokio::test]
    async fn fails_without_an_endpoint() {
        let spec = ModelSpec {
            endpoint: None,
            ..spec("unused")
        };

        let error = complete(&spec, "", "hi", None, None).await.unwrap_err();

        assert_eq!(error.to_string(), "local has no endpoint");
    }
}
