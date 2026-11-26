//! What the owner says aloud to the glasses, heard by the voice's model.

use std::sync::{Arc, Mutex};

use axum::{Json, Router, body::Bytes, extract::State, routing::post};
use base64::Engine;
use serde_json::{Value, json};

use crate::support::{call, open, temp, try_call};

/// A transcription endpoint that hears `text`, and the bodies it was sent.
async fn whisper(text: &'static str) -> (String, Arc<Mutex<Vec<Vec<u8>>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route(
            "/v1/audio/transcriptions",
            post(
                move |State(seen): State<Arc<Mutex<Vec<Vec<u8>>>>>, body: Bytes| async move {
                    seen.lock().unwrap().push(body.to_vec());
                    Json(json!({ "text": text }))
                },
            ),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/v1"), seen)
}

fn said(samples: usize) -> Value {
    let pcm = base64::engine::general_purpose::STANDARD.encode(vec![0u8; samples * 2]);
    json!({ "pcm": pcm, "rate": 16000 })
}

#[tokio::test]
async fn nothing_is_heard_until_the_voice_has_a_model() {
    let dir = temp();
    let api = open(&dir);
    let models = call(&api, "ModelService/GetModels", json!({})).await;
    let voice = models["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["role"] == "ROLE_VOICE")
        .unwrap();
    assert_eq!(voice["label"], "声の聞き取り");
    assert!(voice.get("model").is_none(), "{voice}");

    let (status, body) = try_call(&api, "ConsoleService/Transcribe", said(1600)).await;

    assert!(!status.is_success(), "{status}");
    assert!(
        body["message"].as_str().unwrap().contains("声の聞き取り"),
        "{body}"
    );
}

#[tokio::test]
async fn the_voice_hears_what_was_said_as_a_wav() {
    let dir = temp();
    let api = open(&dir);
    let (endpoint, seen) = whisper("15 分遅れるって伝えて").await;
    let model = call(
        &api,
        "ModelService/RegisterModel",
        json!({ "name": "whisper", "kind": "MODEL_KIND_TRANSCRIBE", "model": "whisper-1", "endpoint": endpoint }),
    )
    .await;
    call(
        &api,
        "ModelService/AssignRole",
        json!({ "role": "ROLE_VOICE", "model": model["id"] }),
    )
    .await;

    let heard = call(&api, "ConsoleService/Transcribe", said(1600)).await;

    assert_eq!(heard["text"], "15 分遅れるって伝えて");
    let body = seen.lock().unwrap()[0].clone();
    assert!(body.windows(4).any(|w| w == b"RIFF"), "a WAV is sent");
    let models = call(&api, "ModelService/GetModels", json!({})).await;
    let voice = models["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["role"] == "ROLE_VOICE")
        .unwrap();
    assert_eq!(voice["callsToday"], "1", "{voice}");
}

#[tokio::test]
async fn silence_is_heard_as_nothing_without_calling_the_model() {
    let dir = temp();
    let api = open(&dir);

    let heard = call(&api, "ConsoleService/Transcribe", said(0)).await;

    assert_eq!(heard.get("text"), None, "{heard}");
}

#[tokio::test]
async fn what_is_said_on_the_glasses_comes_from_the_even_g2() {
    let dir = temp();
    let api = open(&dir);

    let sent = call(
        &api,
        "ConsoleService/Send",
        json!({ "text": "送って", "workgroup": "all", "view": "Even G2", "device": "g2" }),
    )
    .await;

    assert_eq!(sent["origin"], "Even G2", "{sent}");
    let typed = call(
        &api,
        "ConsoleService/Send",
        json!({ "text": "送って", "workgroup": "all" }),
    )
    .await;
    assert_eq!(typed["origin"], "コンソール", "{typed}");
}

#[tokio::test]
async fn half_a_sample_or_too_long_a_sound_is_refused() {
    let dir = temp();
    let api = open(&dir);
    let half = base64::engine::general_purpose::STANDARD.encode(vec![0u8; 3201]);

    let (odd, _) = try_call(
        &api,
        "ConsoleService/Transcribe",
        json!({ "pcm": half, "rate": 16000 }),
    )
    .await;
    let (long, _) = try_call(&api, "ConsoleService/Transcribe", said(16000 * 91)).await;

    assert!(!odd.is_success(), "{odd}");
    assert!(!long.is_success(), "{long}");
}
