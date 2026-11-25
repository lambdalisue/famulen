//! The secretary's name and the way it speaks, as the console sets them.

use serde_json::json;

use crate::support::{call, define, open, temp, try_call};

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

/// A Slack that takes a trade of the app's token and refuses its icon.
async fn slack_refusing_icons() -> String {
    let router = axum::Router::new().fallback(|uri: http::Uri| async move {
        if uri.path().ends_with("/tooling.tokens.rotate") {
            json!({ "ok": true, "token": "t-1", "refresh_token": "r-1" }).to_string()
        } else {
            json!({ "ok": false, "error": "app_edit_disabled" }).to_string()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

#[tokio::test]
async fn a_gender_slack_would_not_follow_is_kept_and_the_owner_is_told() {
    let dir = temp();
    let base = slack_refusing_icons().await;
    define(
        &dir,
        "slack",
        &include_str!("../../../gateway/services/slack/slack.toml")
            .replace("https://slack.com/api", &format!("{base}/api")),
    );
    let api = open(&dir);
    api.app()
        .gateway
        .add_defined(
            "slack",
            [("app", "A1"), ("config_refresh", "r-0")]
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .into(),
            None,
        )
        .unwrap();

    let renamed = call(
        &api,
        "SettingsService/UpdatePersona",
        json!({ "name": "ミナ" }),
    )
    .await;
    assert_eq!(
        renamed["troubles"],
        json!(null),
        "only a gender is followed"
    );
    let chosen = call(
        &api,
        "SettingsService/UpdatePersona",
        json!({ "gender": "PERSONA_GENDER_FEMALE" }),
    )
    .await;

    assert_eq!(chosen["gender"], "PERSONA_GENDER_FEMALE");
    let trouble = chosen["troubles"][0].as_str().unwrap();
    assert!(trouble.contains("app_edit_disabled"), "{trouble}");
    let later = call(&api, "SettingsService/GetPersona", json!({})).await;
    assert_eq!(later["gender"], "PERSONA_GENDER_FEMALE");
    assert_eq!(later["troubles"], json!(null));
}

#[tokio::test]
async fn a_token_pasted_for_a_slack_app_goes_into_the_vault_and_is_not_read_back() {
    let dir = temp();
    let base = slack_refusing_icons().await;
    define(
        &dir,
        "slack",
        &include_str!("../../../gateway/services/slack/slack.toml")
            .replace("https://slack.com/api", &format!("{base}/api")),
    );
    let api = open(&dir);
    api.app()
        .gateway
        .add_defined("slack", [("app".into(), "A1".into())].into(), None)
        .unwrap();

    let answer = call(
        &api,
        "SettingsService/LetSlackAppFollow",
        json!({ "app": "A1", "refreshToken": "xoxe-1-secret" }),
    )
    .await;

    let trouble = answer["troubles"][0].as_str().unwrap();
    assert!(trouble.contains("app_edit_disabled"), "{trouble}");
    let apps = call(&api, "SettingsService/ListSlackApps", json!({})).await;
    assert_eq!(apps["apps"][0]["id"], "A1");
    assert_eq!(apps["apps"][0]["follows"], true);
    let row = api.app().connections.get("slack").unwrap().unwrap();
    assert!(row.secret.account.is_empty());
    assert!(!format!("{answer}{apps}").contains("xoxe-1-secret"));
}
