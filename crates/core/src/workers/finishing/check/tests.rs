use std::path::Path;

use super::*;
use crate::notebook::testing::TempDir;

/// What `command` comes to in a workspace beside a sealed file.
async fn run(dir: &Path, command: &str) -> Result<(), String> {
    let workspace = dir.join("workspace");
    std::fs::create_dir_all(workspace.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join("sealed")).unwrap();
    std::fs::write(dir.join("sealed/key"), "secret-key").unwrap();
    let fence = Fence {
        sealed: vec![dir.join("sealed")],
        unwritable: vec![workspace.join("settings")],
        ..Fence::default()
    };
    check(command, &workspace, &fence, &dir.join("log/check.log"))
        .await
        .unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn a_check_works_in_its_workspace_and_tells_what_it_wrote_when_it_fails() {
    if !Path::new(SANDBOX).exists() {
        return;
    }
    let dir = TempDir::new("finishing-check");
    let ran = run(
        dir.path(),
        r#"echo made >made && cat made && echo t >"$TMPDIR/t" && git --version"#,
    )
    .await;
    assert_eq!(ran, Ok(()));
    let failed = run(dir.path(), "echo out; echo err >&2; exit 3").await;
    assert_eq!(failed, Err("out\nerr\n".into()));
}

#[cfg(unix)]
#[tokio::test]
async fn a_check_reaches_neither_the_keys_nor_the_network_nor_writes_elsewhere() {
    if !Path::new(SANDBOX).exists() {
        return;
    }
    let dir = TempDir::new("finishing-check-kept");
    for command in [
        "cat ../sealed/key",
        "echo x >../outside",
        "echo x >settings",
        "echo x >.git/config",
        "/bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/9'",
        "security list-keychains && security find-generic-password -s famulen-no-such-item",
    ] {
        let told = run(dir.path(), command).await.expect_err(command);
        assert!(!told.contains("secret-key"), "{command}: {told}");
    }
    let network = run(dir.path(), "/bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/9'")
        .await
        .unwrap_err();
    assert!(network.contains("Operation not permitted"), "{network}");
    assert!(!dir.path().join("outside").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn what_a_check_leaves_running_ends_with_it() {
    if !Path::new(SANDBOX).exists() {
        return;
    }
    let dir = TempDir::new("finishing-check-left");
    let ran = run(dir.path(), "(sleep 2; echo late >late) >/dev/null 2>&1 &").await;
    assert_eq!(ran, Ok(()));
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert!(!dir.path().join("workspace/late").exists());
}
