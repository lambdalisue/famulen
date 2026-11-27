use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::notebook::testing::TempDir;
use crate::workers::direct::tests::stand_in;

fn sh(dir: &Path, command: &str) -> String {
    let ran = std::process::Command::new("sh")
        .args(["-ec", command])
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "{command}: {}",
        String::from_utf8_lossy(&ran.stderr)
    );
    String::from_utf8_lossy(&ran.stdout).trim().to_owned()
}

/// A repository the work was merged into, the checkout it is deployed from,
/// and a jj standing in: `at` is what the checkout is on, `moved` what it
/// was moved to.
struct World {
    dir: TempDir,
    root: PathBuf,
    checkout: PathBuf,
    kept: PathBuf,
    jj: String,
    base: String,
    merged: String,
    /// Merged after it.
    later: String,
}

impl World {
    fn new(name: &str) -> Self {
        let dir = TempDir::new(name);
        let root = dir.path().canonicalize().unwrap();
        sh(&root, "git init -q --bare -b main bare.git");
        sh(&root, "git clone -q bare.git checkout 2>/dev/null");
        sh(&root, "git clone -q bare.git forge 2>/dev/null");
        let (checkout, forge) = (root.join("checkout"), root.join("forge"));
        let commit = |at: &Path, name: &str| {
            sh(
                at,
                &format!(
                    "git -c user.name=o -c user.email=o@example.com -c commit.gpgsign=false \
                     commit -qm {name} --allow-empty && git rev-parse HEAD"
                ),
            )
        };
        let base = commit(&forge, "base");
        sh(&forge, "git push -q origin HEAD:main");
        sh(&checkout, "git pull -q origin main");
        let merged = commit(&forge, "merged");
        let later = commit(&forge, "later");
        sh(&forge, "git push -q origin HEAD:main");
        std::fs::write(root.join("at"), &base).unwrap();
        let jj = root.join("jj");
        stand_in(
            &jj,
            r#"root=$(dirname "$2")
for last; do :; done
case "$*" in
*" git root") echo "$2/.git" ;;
*" git import") ;;
*" -T empty") cat "$root/empty" 2>/dev/null || echo true ;;
*" -T commit_id"*) cat "$root/at" ;;
*" new "*) echo "$last" >"$root/at"; echo "$last" >>"$root/moved" ;;
*) echo "$*" >>"$root/asked" ;;
esac
"#,
        );
        let kept = root.join("kept");
        Self {
            jj: jj.display().to_string(),
            dir,
            root,
            checkout,
            kept,
            base,
            merged,
            later,
        }
    }

    async fn deploy(&self, merged: &str, command: &str, kept: &Path) -> anyhow::Result<Started> {
        let remote = self.root.join("bare.git").display().to_string();
        let source = Source {
            jj: &self.jj,
            checkout: &self.checkout,
            remote: &remote,
            target: "main",
            merged,
        };
        start_deploy(&source, command, kept, &self.dir.path().join("deploy.lock")).await
    }

    async fn start(&self, command: &str, kept: &Path) -> Started {
        self.deploy(&self.merged, command, kept).await.unwrap()
    }

    /// One that told how it ended lets go only a moment later.
    async fn start_after(&self, command: &str) {
        for _ in 0..100 {
            if self.start(command, &self.kept).await == Started::Yes {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the one before did not let go");
    }

    async fn outcome(&self) -> Result<(), String> {
        for _ in 0..200 {
            if let Some(outcome) = deployed(&self.kept).unwrap() {
                return outcome;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("deploying told nothing");
    }

    fn at(&self) -> String {
        std::fs::read_to_string(self.root.join("at"))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn moved(&self) -> String {
        std::fs::read_to_string(self.root.join("moved")).unwrap_or_default()
    }
}

#[tokio::test]
async fn what_was_merged_is_taken_up_from_the_remote_pinned_and_the_command_tells_how_it_went() {
    let world = World::new("finishing-deploy");
    // What the checkout calls origin is not what the work was pinned to.
    sh(&world.checkout, "git remote set-url origin /nowhere");
    assert_eq!(
        world.start("echo done >made", &world.kept).await,
        Started::Yes
    );
    assert_eq!(world.outcome().await, Ok(()));
    assert!(world.checkout.join("made").exists());
    assert_eq!(world.at(), world.merged);
    sh(
        &world.checkout,
        &format!("git cat-file -e {}", world.merged),
    );

    world.start_after("echo broke; exit 1").await;
    assert_eq!(world.outcome().await, Err("broke\n".into()));
}

#[tokio::test]
async fn a_later_merge_already_deployed_is_not_gone_back_from() {
    let world = World::new("finishing-deploy-later");
    std::fs::write(world.root.join("at"), &world.later).unwrap();
    assert_eq!(world.start("true", &world.kept).await, Started::Yes);
    assert_eq!(world.outcome().await, Ok(()));
    assert_eq!(world.moved(), "");
}

#[tokio::test]
async fn what_the_checkout_holds_that_was_never_merged_is_not_deployed_with_it() {
    let world = World::new("finishing-deploy-local");
    sh(&world.checkout, "git pull -q origin main");
    let local = sh(
        &world.checkout,
        "git -c user.name=o -c user.email=o@example.com -c commit.gpgsign=false \
         commit -qm local --allow-empty && git rev-parse HEAD",
    );
    std::fs::write(world.root.join("at"), local).unwrap();
    assert_eq!(world.start("true", &world.kept).await, Started::Yes);
    assert_eq!(world.outcome().await, Ok(()));
    assert_eq!(world.at(), world.merged);
}

#[tokio::test]
async fn what_the_target_does_not_hold_is_not_deployed() {
    let world = World::new("finishing-deploy-elsewhere");
    sh(
        &world.root.join("forge"),
        &format!("git push -qf origin {}:main", world.base),
    );
    let refused = world
        .deploy(&world.merged, "echo >made", &world.kept)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("not on main"), "{refused}");
    assert_eq!(world.moved(), "");
    assert!(!world.checkout.join("made").exists());
}

#[tokio::test]
async fn a_checkout_with_work_in_it_is_not_touched() {
    let world = World::new("finishing-deploy-dirty");
    std::fs::write(world.root.join("empty"), "false").unwrap();
    let refused = world
        .deploy(&world.merged, "echo >made", &world.kept)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("work in it"), "{refused}");
    assert_eq!(world.moved(), "");
    assert!(!world.checkout.join("made").exists());
    assert_eq!(deployed(&world.kept).unwrap(), None);
}

#[tokio::test]
async fn the_source_is_not_moved_under_a_deployment_still_running() {
    let world = World::new("finishing-deploy-busy");
    let held = "while [ ! -e go ]; do sleep 0.02; done";
    assert_eq!(world.start(held, &world.kept).await, Started::Yes);
    let before = world.moved();

    let other = world.root.join("other");
    assert_eq!(
        world
            .deploy(&world.later, "echo >second", &other)
            .await
            .unwrap(),
        Started::Busy
    );
    assert_eq!(world.moved(), before);
    assert_eq!(deployed(&other).unwrap(), None);

    std::fs::write(world.checkout.join("go"), "").unwrap();
    assert_eq!(world.outcome().await, Ok(()));
    world.start_after("echo >second").await;
    assert_eq!(world.outcome().await, Ok(()));
    assert!(world.checkout.join("second").exists());
}

#[tokio::test]
async fn what_the_command_leaves_running_does_not_keep_the_next_one_waiting() {
    let world = World::new("finishing-deploy-daemon");
    let daemon = "sleep 30 >/dev/null 2>&1 &";
    assert_eq!(world.start(daemon, &world.kept).await, Started::Yes);
    assert_eq!(world.outcome().await, Ok(()));
    world.start_after("true").await;
}
