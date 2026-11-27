use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::notebook::testing::TempDir;
use crate::workers::direct::tests::stand_in;

/// A jj that says what the checkout's change holds, and keeps what else it
/// was asked.
fn jj(dir: &Path, empty: bool) -> String {
    let path = dir.join("jj");
    let script = format!(
        "case \"$*\" in\n*' log '*) echo {empty} ;;\n*) echo \"$*\" >>\"{}\" ;;\nesac\n",
        dir.join("jj.log").display()
    );
    stand_in(&path, script);
    path.display().to_string()
}

struct World {
    dir: TempDir,
    checkout: PathBuf,
    kept: PathBuf,
}

impl World {
    fn new(name: &str) -> Self {
        let dir = TempDir::new(name);
        let checkout = dir.path().join("checkout");
        std::fs::create_dir(&checkout).unwrap();
        let kept = dir.path().join("kept");
        Self {
            dir,
            checkout,
            kept,
        }
    }

    async fn start(&self, jj: &str, command: &str, kept: &Path) -> Started {
        let lock = self.dir.path().join("deploy.lock");
        start_deploy(jj, &self.checkout, "main", command, kept, &lock)
            .await
            .unwrap()
    }

    /// One that told how it ended lets go only a moment later.
    async fn start_after(&self, jj: &str, command: &str) {
        for _ in 0..100 {
            if self.start(jj, command, &self.kept).await == Started::Yes {
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
}

#[tokio::test]
async fn what_was_merged_is_taken_up_and_the_command_tells_how_it_went() {
    let world = World::new("finishing-deploy");
    let jj = jj(world.dir.path(), true);
    assert_eq!(
        world.start(&jj, "echo done >made", &world.kept).await,
        Started::Yes
    );
    assert_eq!(world.outcome().await, Ok(()));
    assert!(world.checkout.join("made").exists());
    let asked = std::fs::read_to_string(world.dir.path().join("jj.log")).unwrap();
    assert!(
        asked.contains("git fetch") && asked.contains("new main@origin"),
        "{asked}"
    );

    world.start_after(&jj, "echo broke; exit 1").await;
    assert_eq!(world.outcome().await, Err("broke\n".into()));
}

#[tokio::test]
async fn a_checkout_with_work_in_it_is_not_touched() {
    let world = World::new("finishing-deploy-dirty");
    let jj = jj(world.dir.path(), false);
    let lock = world.dir.path().join("deploy.lock");
    let refused = start_deploy(
        &jj,
        &world.checkout,
        "main",
        "echo >made",
        &world.kept,
        &lock,
    )
    .await
    .unwrap_err();
    assert!(refused.to_string().contains("work in it"), "{refused}");
    assert!(!world.dir.path().join("jj.log").exists());
    assert!(!world.checkout.join("made").exists());
    assert_eq!(deployed(&world.kept).unwrap(), None);
}

#[tokio::test]
async fn the_source_is_not_moved_under_a_deployment_still_running() {
    let world = World::new("finishing-deploy-busy");
    let jj = jj(world.dir.path(), true);
    let held = "while [ ! -e go ]; do sleep 0.02; done";
    assert_eq!(world.start(&jj, held, &world.kept).await, Started::Yes);
    let asked = || std::fs::read_to_string(world.dir.path().join("jj.log")).unwrap();
    let before = asked();

    let other = world.dir.path().join("other");
    assert_eq!(
        world.start(&jj, "echo >second", &other).await,
        Started::Busy
    );
    assert_eq!(asked(), before);
    assert_eq!(deployed(&other).unwrap(), None);

    std::fs::write(world.checkout.join("go"), "").unwrap();
    assert_eq!(world.outcome().await, Ok(()));
    world.start_after(&jj, "echo >second").await;
    assert_eq!(world.outcome().await, Ok(()));
    assert!(world.checkout.join("second").exists());
}

#[tokio::test]
async fn what_the_command_leaves_running_does_not_keep_the_next_one_waiting() {
    let world = World::new("finishing-deploy-daemon");
    let jj = jj(world.dir.path(), true);
    let daemon = "sleep 30 >/dev/null 2>&1 &";
    assert_eq!(world.start(&jj, daemon, &world.kept).await, Started::Yes);
    assert_eq!(world.outcome().await, Ok(()));
    world.start_after(&jj, "true").await;
}
