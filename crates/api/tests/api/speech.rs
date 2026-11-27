//! The models brought in onto this machine to hear with. Nothing here is
//! downloaded: what a download leaves is laid down beforehand.

use serde_json::{Value, json};

use crate::support::{DataDir, call, open, temp, try_call};

fn lay_downloaded(dir: &DataDir, name: &str) {
    let whisper = dir.join("whisper");
    std::fs::create_dir_all(whisper.join("engine")).unwrap();
    std::fs::create_dir_all(whisper.join("models")).unwrap();
    std::fs::write(whisper.join("engine/whisper-server"), "").unwrap();
    std::fs::write(whisper.join(format!("models/ggml-{name}.bin")), "model").unwrap();
}

fn listed<'a>(speech: &'a Value, id: &str) -> &'a Value {
    speech["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .unwrap_or_else(|| panic!("no {id} in {speech}"))
}

async fn voice(api: &famulen_api::Api) -> String {
    let models = call(api, "ModelService/GetModels", json!({})).await;
    let voice = models["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["role"] == "ROLE_VOICE")
        .unwrap();
    voice["model"].as_str().unwrap_or_default().to_owned()
}

#[tokio::test]
async fn the_models_to_pick_from_are_listed_with_their_size_and_none_here() {
    let dir = temp();
    let api = open(&dir);

    let speech = call(&api, "ModelService/GetSpeechModels", json!({})).await;

    let small = listed(&speech, "small");
    assert_eq!(small["bytes"], "487601967");
    assert_eq!(small["state"], "SPEECH_MODEL_STATE_ABSENT");
    assert_eq!(small["speed"], "速い");
    assert_eq!(small["accuracy"], "中くらい");
    assert!(speech["engineBytes"].as_str().is_some(), "{speech}");
    assert_eq!(speech["supported"], true);
    assert!(!dir.join("whisper").exists());
}

#[tokio::test]
async fn a_downloaded_model_is_used_switched_and_deleted() {
    let dir = temp();
    let api = open(&dir);
    lay_downloaded(&dir, "small");
    lay_downloaded(&dir, "base");

    let speech = call(
        &api,
        "ModelService/UseSpeechModel",
        json!({ "id": "small" }),
    )
    .await;
    assert_eq!(listed(&speech, "small")["used"], true);
    assert_eq!(voice(&api).await, "local-whisper-small");

    let speech = call(&api, "ModelService/UseSpeechModel", json!({ "id": "base" })).await;
    assert!(listed(&speech, "small").get("used").is_none(), "{speech}");
    assert_eq!(voice(&api).await, "local-whisper-base");

    let speech = call(
        &api,
        "ModelService/DeleteSpeechModel",
        json!({ "id": "base" }),
    )
    .await;
    assert_eq!(
        listed(&speech, "base")["state"],
        "SPEECH_MODEL_STATE_ABSENT"
    );
    assert_eq!(
        listed(&speech, "small")["state"],
        "SPEECH_MODEL_STATE_DOWNLOADED"
    );
    assert_eq!(voice(&api).await, "");
}

#[tokio::test]
async fn a_model_that_is_not_here_or_not_listed_is_refused() {
    let dir = temp();
    let api = open(&dir);

    for (method, id) in [
        ("ModelService/UseSpeechModel", "small"),
        ("ModelService/DownloadSpeechModel", "tiny.en"),
        ("ModelService/DeleteSpeechModel", "../secrets"),
    ] {
        let (status, body) = try_call(&api, method, json!({ "id": id })).await;
        assert!(!status.is_success(), "{method} {id}: {body}");
    }
}
