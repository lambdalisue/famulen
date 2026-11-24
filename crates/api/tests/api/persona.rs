//! The secretary's name and the way it speaks, as the console sets them.

use serde_json::json;

use crate::support::{call, open, temp, try_call};

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

#[tokio::test]
async fn the_secretary_is_a_man_until_the_owner_chooses() {
    let dir = temp();
    let api = open(&dir);

    let persona = call(&api, "SettingsService/GetPersona", json!({})).await;

    assert_eq!(persona["gender"], "PERSONA_GENDER_MALE");
}

#[tokio::test]
async fn the_gender_chosen_is_kept_and_leaves_the_rest_alone() {
    let dir = temp();
    let api = open(&dir);

    call(
        &api,
        "SettingsService/UpdatePersona",
        json!({ "name": "ミナ", "gender": "PERSONA_GENDER_FEMALE" }),
    )
    .await;
    call(
        &api,
        "SettingsService/UpdatePersona",
        json!({ "tone": "丁寧語で話す。" }),
    )
    .await;

    let persona = call(&api, "SettingsService/GetPersona", json!({})).await;
    assert_eq!(persona["gender"], "PERSONA_GENDER_FEMALE");
    assert_eq!(persona["name"], "ミナ");
}

#[tokio::test]
async fn a_gender_there_is_no_portrait_for_is_refused_and_nothing_is_changed() {
    let dir = temp();
    let api = open(&dir);

    for gender in [json!("PERSONA_GENDER_UNSPECIFIED"), json!(9)] {
        let (status, _) = try_call(
            &api,
            "SettingsService/UpdatePersona",
            json!({ "name": "ミナ", "gender": gender }),
        )
        .await;
        assert!(!status.is_success(), "{status}");
    }

    let persona = call(&api, "SettingsService/GetPersona", json!({})).await;
    assert_eq!(persona["gender"], "PERSONA_GENDER_MALE");
    assert_eq!(persona["name"], "ファム");
}

#[tokio::test]
async fn a_time_zone_there_is_not_is_refused_and_nothing_is_changed() {
    let dir = temp();
    let api = open(&dir);

    let (status, _) = try_call(
        &api,
        "SettingsService/UpdatePersona",
        json!({ "name": "ミナ", "gender": "PERSONA_GENDER_FEMALE", "timezone": "Mars/Base" }),
    )
    .await;
    assert!(!status.is_success(), "{status}");

    let persona = call(&api, "SettingsService/GetPersona", json!({})).await;
    assert_eq!(persona["gender"], "PERSONA_GENDER_MALE");
    assert_eq!(persona["name"], "ファム");
}
