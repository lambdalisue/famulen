//! The secretary's name and the way it speaks, as the console sets them.

use serde_json::json;

use crate::support::{call, open, temp};

#[tokio::test]
async fn the_secretary_is_called_fam_until_named() {
    let dir = temp();
    let api = open(&dir);

    let persona = call(&api, "SettingsService/GetPersona", json!({})).await;

    assert_eq!(persona["name"], "ファム");
}

#[tokio::test]
async fn a_way_of_speaking_written_by_hand_is_kept_while_a_preset_is_used() {
    let dir = temp();
    let api = open(&dir);

    call(
        &api,
        "SettingsService/UpdatePersona",
        json!({ "tone": "関西弁で。", "ownTone": "関西弁で。" }),
    )
    .await;
    let preset = call(
        &api,
        "SettingsService/UpdatePersona",
        json!({ "tone": "丁寧語で話す。" }),
    )
    .await;

    assert_eq!(preset["tone"], "丁寧語で話す。");
    assert_eq!(preset["ownTone"], "関西弁で。");
}
