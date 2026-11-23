//! Where the secretary's shell commands may write, as the owner sets it on
//! the secretary's page.

use serde_json::json;

use crate::support::{open, temp, try_call};

#[tokio::test]
async fn there_is_nowhere_to_write_at_first() {
    let dir = temp();
    let (_, places) = try_call(&open(&dir), "SettingsService/GetWritablePlaces", json!({})).await;
    assert_eq!(places.get("paths"), None);
}

#[tokio::test]
async fn places_the_owner_adds_are_listed_until_removed() {
    let dir = temp();
    let api = open(&dir);

    try_call(
        &api,
        "SettingsService/AddWritablePlace",
        json!({ "path": "/srv/notes" }),
    )
    .await;
    let (_, added) = try_call(
        &api,
        "SettingsService/AddWritablePlace",
        json!({ "path": "/srv/out" }),
    )
    .await;
    assert_eq!(added["paths"], json!(["/srv/notes", "/srv/out"]));

    let (_, removed) = try_call(
        &api,
        "SettingsService/RemoveWritablePlace",
        json!({ "path": "/srv/notes" }),
    )
    .await;
    assert_eq!(removed["paths"], json!(["/srv/out"]));
    let (_, listed) = try_call(&api, "SettingsService/GetWritablePlaces", json!({})).await;
    assert_eq!(listed["paths"], json!(["/srv/out"]));
}

#[tokio::test]
async fn a_path_that_is_not_whole_is_refused_with_why() {
    let dir = temp();
    let (status, error) = try_call(
        &open(&dir),
        "SettingsService/AddWritablePlace",
        json!({ "path": "notes" }),
    )
    .await;
    assert_ne!(status, 200);
    assert!(
        error["message"].as_str().unwrap().contains("whole path"),
        "{error}"
    );
}

#[tokio::test]
async fn the_owner_walks_the_folders_of_the_machine_famulen_runs_on() {
    let dir = temp();
    std::fs::create_dir_all(dir.join("notes")).unwrap();
    let api = open(&dir);

    let (status, folder) = try_call(
        &api,
        "SettingsService/GetFolder",
        json!({ "path": dir.display().to_string() }),
    )
    .await;
    assert_eq!(status, 200, "{folder}");
    assert_eq!(folder["path"], dir.display().to_string());
    assert_eq!(
        folder["parent"],
        dir.parent().unwrap().display().to_string()
    );
    assert!(
        folder["children"]
            .as_array()
            .unwrap()
            .contains(&json!("notes"))
    );

    let (status, _) = try_call(
        &api,
        "SettingsService/GetFolder",
        json!({ "path": "notes" }),
    )
    .await;
    assert_ne!(status, 200);
}

#[tokio::test]
async fn a_folder_is_looked_for_by_its_name_alone() {
    let dir = temp();
    let api = open(&dir);
    for name in ["", "a/b"] {
        let (status, _) =
            try_call(&api, "SettingsService/FindFolders", json!({ "name": name })).await;
        assert_ne!(status, 200, "{name:?}");
    }
}
