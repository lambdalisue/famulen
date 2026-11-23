//! Assessing: the model that weighs, for the auto permission, a call that
//! writes but is neither beyond undoing nor reaching other people. It reads
//! the call, what the owner last asked and a short summary of what the memory
//! holds, and lets the call run only when it is what was asked. Whenever it
//! cannot answer — no model, over its daily cap, a model that fails, an
//! answer it cannot read — the owner is asked instead.

use std::path::PathBuf;

use chrono::Utc;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use famulen_gateway::{Assessment, Assessor, Call};

use crate::input::Sender;
use crate::models::{Models, Role};
use crate::settings::Settings;
use crate::store::Store;

/// How much of the owner's request the model reads.
const REQUEST_CHARS: usize = 2000;
/// How much of the call's input the model reads.
const INPUT_CHARS: usize = 4000;
/// How many memories, and how much of them, the model reads.
const MEMORY_ENTRIES: usize = 15;
const MEMORY_CHARS: usize = 1500;
/// How far back to look for the owner's request.
const RECENT_INPUTS: usize = 20;

/// The answer the model gives.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "allow": { "type": "boolean" },
            "reason": { "type": "string" }
        },
        "required": ["allow", "reason"],
        "additionalProperties": false
    })
}

/// What the model is told it does.
const INSTRUCTION: &str = "You decide, for an AI secretary's owner, whether one tool call may run \
without asking the owner first. Allow it only when all of these hold: it does what the owner asked \
in the request below, or plainly follows from it; it can be undone; and it does not reach or involve \
anyone but the owner. When in doubt, do not allow it. Answer in JSON: allow, and a short reason in \
Japanese, one sentence, that the owner reads.";

/// The first `limit` characters of `text`, marked when cut.
fn cut(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// What the model reads of one call.
pub fn prompt(call: &Call, request: Option<&str>, memory: &str) -> String {
    let connection = if call.connection.is_empty() {
        "(the secretary's own built-in tool)"
    } else {
        &call.connection
    };
    let request = request.map_or_else(|| "(none)".to_owned(), |r| cut(r, REQUEST_CHARS));
    let memory = if memory.is_empty() {
        "(nothing)"
    } else {
        memory
    };
    format!(
        "Caller: {}\nConnection: {connection}\nTool: {}\nInput: {}\n\n\
         The owner's latest request:\n{request}\n\n\
         What the memory holds about the owner, in short:\n{memory}",
        call.actor,
        call.tool,
        cut(&call.input.to_string(), INPUT_CHARS),
    )
}

/// The assessment the model's answer gives; an answer that cannot be read
/// asks the owner.
pub fn read_answer(answer: Option<&Value>) -> Assessment {
    let allow = answer.and_then(|a| a.get("allow")).and_then(Value::as_bool);
    let reason = answer
        .and_then(|a| a.get("reason"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .unwrap_or("理由が書かれていない")
        .to_owned();
    match allow {
        Some(true) => Assessment::Allow { reason },
        Some(false) => Assessment::Ask { reason },
        None => Assessment::Ask {
            reason: "見極めの答えが読めなかった".into(),
        },
    }
}

/// The assessor Famulen gives the gateway: the model of [`Role::Assess`].
#[derive(Clone)]
pub struct ModelAssessor {
    models: Models,
    store: Store,
    settings: Settings,
    memory_dir: PathBuf,
}

impl ModelAssessor {
    pub fn new(models: Models, store: Store, settings: Settings, memory_dir: PathBuf) -> Self {
        Self {
            models,
            store,
            settings,
            memory_dir,
        }
    }

    /// The text of the owner's newest input.
    fn request(&self) -> anyhow::Result<Option<String>> {
        Ok(self
            .store
            .recent_inputs(0, RECENT_INPUTS)?
            .into_iter()
            .find(|(stored, _)| stored.input.sender == Sender::Owner)
            .map(|(stored, _)| stored.input.text))
    }

    /// A short summary of what long-term memory holds.
    fn memory(&self) -> String {
        let entries: Vec<String> = crate::memory::long_term(&self.memory_dir)
            .unwrap_or_default()
            .into_iter()
            .flat_map(|g| g.files)
            .flat_map(|f| f.entries)
            .map(|e| e.body().trim().to_owned())
            .filter(|b| !b.is_empty())
            .take(MEMORY_ENTRIES)
            .collect();
        cut(&entries.join("; "), MEMORY_CHARS)
    }

    async fn weigh(&self, call: &Call) -> anyhow::Result<Assessment> {
        let tz = self.settings.timezone();
        if self.models.assignment(Role::Assess)?.is_none() {
            return Ok(Assessment::Ask {
                reason: "見極めにモデルが割り当てられていない".into(),
            });
        }
        if !self.models.within_cap(Role::Assess, tz, Utc::now())? {
            return Ok(Assessment::Ask {
                reason: "見極めが今日の上限に達した".into(),
            });
        }
        let prompt = prompt(call, self.request()?.as_deref(), &self.memory());
        let completion = match self
            .models
            .complete_brief(Role::Assess, tz, INSTRUCTION, &prompt, Some(&schema()))
            .await
        {
            Ok(completion) => completion,
            Err(error) => {
                return Ok(Assessment::Ask {
                    reason: format!("見極めのモデルが使えなかった（{error:#}）"),
                });
            }
        };
        Ok(read_answer(completion.json.as_ref()))
    }
}

impl Assessor for ModelAssessor {
    fn assess<'a>(&'a self, call: &'a Call) -> BoxFuture<'a, anyhow::Result<Assessment>> {
        Box::pin(self.weigh(call))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{Json, Router, routing::post};

    use super::*;
    use crate::input::{Input, Source};
    use crate::models::{ModelKind, ModelSpec};

    /// Serves `/v1/chat/completions` answering `content`, and keeps the
    /// prompts it was sent.
    async fn serve(content: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept = seen.clone();
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move |Json(body): Json<Value>| {
                let kept = kept.clone();
                async move {
                    kept.lock()
                        .unwrap()
                        .push(body["messages"][1]["content"].as_str().unwrap().to_owned());
                    Json(json!({
                        "choices": [{ "message": { "role": "assistant", "content": content } }],
                        "usage": { "prompt_tokens": 10, "completion_tokens": 5 },
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/v1"), seen)
    }

    fn assessor(endpoint: Option<&str>) -> ModelAssessor {
        let store = Store::open_in_memory().unwrap();
        let models = Models::new(store.clone());
        if let Some(endpoint) = endpoint {
            models
                .register(
                    ModelSpec {
                        id: "local".into(),
                        name: "Local".into(),
                        kind: ModelKind::Local,
                        endpoint: Some(endpoint.into()),
                        model: "m".into(),
                        cli: Default::default(),
                        price: Default::default(),
                    },
                    None,
                )
                .unwrap();
            models.assign(Role::Assess, "local", None, None).unwrap();
        }
        let settings = Settings::new(
            store.clone(),
            crate::settings::Base {
                claude: crate::config::Claude {
                    bin: "claude".into(),
                    model: None,
                    effort: None,
                },
                timezone: chrono_tz::UTC,
            },
        );
        ModelAssessor::new(
            models,
            store,
            settings,
            std::env::temp_dir().join("no-memory"),
        )
    }

    fn draft() -> Call {
        Call::new(
            "secretary",
            "gmail-work",
            "create_draft",
            json!({ "to": "me@example.com" }),
        )
    }

    fn said_by_owner(a: &ModelAssessor, text: &str) {
        a.store
            .enqueue(&Input {
                sender: Sender::Owner,
                source: Source::Schedule {
                    name: "n".into(),
                    at: "now".into(),
                },
                text: text.into(),
            })
            .unwrap();
    }

    #[tokio::test]
    async fn the_model_lets_a_call_run_with_its_reason_and_reads_the_owners_request() {
        let (endpoint, seen) = serve(r#"{"allow": true, "reason": "下書きを頼まれた"}"#).await;
        let a = assessor(Some(&endpoint));
        said_by_owner(&a, "来週の会議の返事を下書きして");

        let assessment = a.assess(&draft()).await.unwrap();

        assert_eq!(
            assessment,
            Assessment::Allow {
                reason: "下書きを頼まれた".into()
            }
        );
        let prompt = &seen.lock().unwrap()[0];
        assert!(prompt.contains("create_draft"), "{prompt}");
        assert!(prompt.contains("来週の会議の返事を下書きして"), "{prompt}");
    }

    #[tokio::test]
    async fn the_model_asks_the_owner_when_it_does_not_allow() {
        let (endpoint, _) = serve(r#"{"allow": false, "reason": "頼まれていない"}"#).await;
        let a = assessor(Some(&endpoint));

        assert_eq!(
            a.assess(&draft()).await.unwrap(),
            Assessment::Ask {
                reason: "頼まれていない".into()
            }
        );
    }

    #[tokio::test]
    async fn without_a_model_the_owner_is_asked() {
        let a = assessor(None);
        assert!(matches!(
            a.assess(&draft()).await.unwrap(),
            Assessment::Ask { .. }
        ));
    }

    #[tokio::test]
    async fn over_its_daily_cap_the_owner_is_asked_and_the_model_is_not_called() {
        let (endpoint, seen) = serve(r#"{"allow": true, "reason": "x"}"#).await;
        let a = assessor(Some(&endpoint));
        a.models
            .assign(Role::Assess, "local", Some(0.0), None)
            .unwrap();

        let Assessment::Ask { reason } = a.assess(&draft()).await.unwrap() else {
            panic!("should ask over the cap");
        };
        assert!(reason.contains("上限"), "{reason}");
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_model_that_cannot_be_reached_asks_the_owner() {
        let a = assessor(Some("http://127.0.0.1:9/v1"));
        let Assessment::Ask { reason } = a.assess(&draft()).await.unwrap() else {
            panic!("should ask when the model fails");
        };
        assert!(reason.contains("使えなかった"), "{reason}");
    }

    #[test]
    fn an_answer_that_cannot_be_read_asks_the_owner() {
        assert!(matches!(read_answer(None), Assessment::Ask { .. }));
        assert!(matches!(
            read_answer(Some(&json!({ "reason": "x" }))),
            Assessment::Ask { .. }
        ));
    }

    #[test]
    fn a_long_input_is_cut_by_characters() {
        let long = "あ".repeat(REQUEST_CHARS + 10);
        let shown = prompt(&draft(), Some(&long), "");
        assert!(shown.contains(&format!("{}…", "あ".repeat(REQUEST_CHARS))));
    }
}
