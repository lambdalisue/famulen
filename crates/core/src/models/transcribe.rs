//! Turning what the owner said aloud into text, through an OpenAI-compatible
//! transcription endpoint.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::{ModelKind, ModelSpec, Usage, Via};

const TIMEOUT: Duration = Duration::from_secs(120);

/// Sound as it was heard: 16-bit signed little-endian samples of one channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub samples: Vec<u8>,
    pub rate: u32,
}

impl Pcm {
    /// The sound as a WAV file, which every transcription endpoint reads.
    pub fn wav(&self) -> Vec<u8> {
        let data = self.samples.len() as u32;
        let mut out = Vec::with_capacity(44 + self.samples.len());
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // one channel
        out.extend_from_slice(&self.rate.to_le_bytes());
        out.extend_from_slice(&(self.rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data.to_le_bytes());
        out.extend_from_slice(&self.samples);
        out
    }
}

/// What a transcription model heard.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    pub text: String,
    pub usage: Usage,
}

/// Calls `POST {endpoint}/audio/transcriptions` with the sound as a WAV file.
/// `language` is the ISO 639-1 code the owner speaks, when it is known.
pub async fn transcribe(
    spec: &ModelSpec,
    sound: &Pcm,
    language: Option<&str>,
    via: Option<Via<'_>>,
) -> Result<Transcript> {
    if spec.kind != ModelKind::Transcribe {
        bail!(
            "{} is a {} model, which cannot transcribe",
            spec.name,
            spec.kind.label()
        );
    }
    let endpoint = spec
        .endpoint
        .as_deref()
        .with_context(|| format!("{} has no endpoint", spec.name))?;
    let url = format!("{}/audio/transcriptions", endpoint.trim_end_matches('/'));
    let file = reqwest::multipart::Part::bytes(sound.wav())
        .file_name("said.wav")
        .mime_str("audio/wav")?;
    let mut form = reqwest::multipart::Form::new()
        .text("model", spec.model.clone())
        .text("response_format", "json")
        .part("file", file);
    if let Some(language) = language {
        form = form.text("language", language.to_owned());
    }
    // A proxy named in the environment would be handed what was said, even
    // to a model on this machine.
    let client = if on_this_machine(&url) {
        reqwest::Client::builder().no_proxy().build()?
    } else {
        reqwest::Client::new()
    };
    let request = client.post(&url).timeout(TIMEOUT).multipart(form);
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
        .get("text")
        .and_then(Value::as_str)
        .with_context(|| format!("{url} answered without text"))?
        .trim()
        .to_owned();
    Ok(Transcript {
        text,
        usage: Usage {
            input_tokens: None,
            output_tokens: None,
            calls: 1,
        },
    })
}

fn on_this_machine(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{Json, Router, body::Bytes, extract::State, http::StatusCode, routing::post};
    use serde_json::json;

    use super::*;
    use crate::models::Price;

    /// Serves `/v1/audio/transcriptions` answering `status` and `answer`, and
    /// records the bodies it was sent.
    async fn serve(status: StatusCode, answer: Value) -> (String, Arc<Mutex<Vec<Vec<u8>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route(
                "/v1/audio/transcriptions",
                post(
                    move |State(seen): State<Arc<Mutex<Vec<Vec<u8>>>>>, body: Bytes| async move {
                        seen.lock().unwrap().push(body.to_vec());
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

    fn spec(endpoint: &str) -> ModelSpec {
        ModelSpec {
            id: "w1".into(),
            name: "whisper".into(),
            kind: ModelKind::Transcribe,
            endpoint: Some(endpoint.into()),
            model: "whisper-1".into(),
            cli: Default::default(),
            price: Price::default(),
        }
    }

    fn sound() -> Pcm {
        Pcm {
            samples: vec![1, 0, 2, 0, 3, 0],
            rate: 16_000,
        }
    }

    fn has(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn the_wav_tells_its_rate_and_length_and_holds_the_samples() {
        let wav = sound().wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(wav[28..32].try_into().unwrap()), 32_000);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 6);
        assert_eq!(&wav[44..], &[1, 0, 2, 0, 3, 0]);
    }

    #[tokio::test]
    async fn sends_the_sound_as_a_wav_with_the_model_and_language_and_reads_the_text() {
        let (endpoint, seen) = serve(StatusCode::OK, json!({ "text": " 送って \n" })).await;

        let heard = transcribe(&spec(&endpoint), &sound(), Some("ja"), None)
            .await
            .unwrap();

        assert_eq!(heard.text, "送って");
        assert_eq!(heard.usage.calls, 1);
        let body = &seen.lock().unwrap()[0];
        assert!(has(body, b"name=\"model\"\r\n\r\nwhisper-1\r\n"));
        assert!(has(body, b"name=\"language\"\r\n\r\nja\r\n"));
        assert!(has(body, b"filename=\"said.wav\""));
        assert!(has(body, &sound().wav()));
    }

    #[tokio::test]
    async fn leaves_the_language_to_the_model_when_it_is_not_known() {
        let (endpoint, seen) = serve(StatusCode::OK, json!({ "text": "hi" })).await;

        transcribe(&spec(&endpoint), &sound(), None, None)
            .await
            .unwrap();

        assert!(!has(&seen.lock().unwrap()[0], b"name=\"language\""));
    }

    #[tokio::test]
    async fn fails_with_what_the_endpoint_said_when_it_refuses() {
        let (endpoint, _) = serve(StatusCode::UNAUTHORIZED, json!({ "error": "bad key" })).await;

        let error = transcribe(&spec(&endpoint), &sound(), None, None)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("401"), "{error}");
        assert!(error.to_string().contains("bad key"), "{error}");
    }

    #[tokio::test]
    async fn sound_for_a_model_on_this_machine_does_not_go_through_a_proxy() {
        // The proxy is read from the environment, which a test can only have
        // to itself in a process of its own.
        const BEHIND: &str = "FAMULEN_TEST_BEHIND_PROXY";
        if std::env::var_os(BEHIND).is_none() {
            let name = "models::transcribe::tests::sound_for_a_model_on_this_machine_does_not_go_through_a_proxy";
            let ran = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name])
                .env(BEHIND, "1")
                .env("HTTP_PROXY", "http://127.0.0.1:9")
                .env_remove("NO_PROXY")
                .env_remove("no_proxy")
                .output()
                .unwrap();
            let said = String::from_utf8_lossy(&ran.stdout);
            assert!(ran.status.success() && said.contains("1 passed"), "{said}");
            return;
        }
        let (endpoint, _) = serve(StatusCode::OK, json!({ "text": "hi" })).await;
        for endpoint in [endpoint.clone(), endpoint.replace("127.0.0.1", "localhost")] {
            let heard = transcribe(&spec(&endpoint), &sound(), None, None)
                .await
                .unwrap();
            assert_eq!(heard.text, "hi");
        }
    }

    #[test]
    fn only_loopback_endpoints_are_on_this_machine() {
        for url in [
            "http://127.0.0.1:8/v1",
            "http://[::1]/v1",
            "http://localhost/v1",
        ] {
            assert!(on_this_machine(url), "{url}");
        }
        for url in [
            "https://api.openai.com/v1",
            "http://192.168.1.2/v1",
            "nonsense",
        ] {
            assert!(!on_this_machine(url), "{url}");
        }
    }

    #[tokio::test]
    async fn a_model_of_another_kind_cannot_transcribe() {
        let mut local = spec("http://127.0.0.1:9");
        local.kind = ModelKind::Local;

        let error = transcribe(&local, &sound(), None, None).await.unwrap_err();

        assert!(error.to_string().contains("cannot transcribe"), "{error}");
    }
}
