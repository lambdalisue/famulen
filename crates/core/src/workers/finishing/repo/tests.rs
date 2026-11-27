use std::path::{Path, PathBuf};

use super::*;
use crate::notebook::testing::TempDir;
#[cfg(unix)]
use crate::workers::direct::tests::stand_in;
use crate::workers::finishing::{Ci, Forge};

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

/// A jj that tells where the Git repository of `-R` is and the head of the
/// workspace, and writes down that it was asked to import.
const JJ: &str = r#"case "$*" in
*" git root") if [ -d "$2/.jj/repo/store/git" ]; then echo "$2/.jj/repo/store/git"; else echo "$2/.git"; fi ;;
*" git import") echo "$*" >> "$2/../imported" ;;
*) if [ -e "$2/../conflicted" ]; then c=true; else c=false; fi
   git -C "$2" log -1 --format="change%n%H%n$c%n%B" ;;
esac
"#;

/// A bare repository, its checkout, a workspace the work is done in, and a
/// jj standing in.
struct World {
    dir: TempDir,
    repo: Repo,
    bare: PathBuf,
    forge: Vec<String>,
}

impl World {
    fn new(name: &str) -> Self {
        let dir = TempDir::new(name);
        let root = dir.path().canonicalize().unwrap();
        sh(&root, "git init -q --bare -b main bare.git");
        sh(&root, "git clone -q bare.git checkout 2>/dev/null");
        let checkout = root.join("checkout");
        sh(
            &checkout,
            "git config user.name owner && git config user.email owner@example.com \
             && git config commit.gpgsign false \
             && echo one >a && git add a && git commit -qm base && git push -q origin HEAD:main \
             && git worktree add -q ../workspace -b work",
        );
        let workspace = root.join("workspace");
        let jj = root.join("jj");
        #[cfg(unix)]
        stand_in(&jj, JJ);
        let bare = root.join("bare.git");
        let script = root.join("fake_gh.sh");
        std::fs::write(&script, include_str!("../fake_gh.sh")).unwrap();
        std::fs::create_dir(root.join("forge")).unwrap();
        let forge = [
            "sh".to_owned(),
            script.display().to_string(),
            root.join("forge").display().to_string(),
            bare.display().to_string(),
        ]
        .into();
        Self {
            repo: Repo {
                jj: jj.display().to_string(),
                checkout,
                workspace,
            },
            bare,
            forge,
            dir,
        }
    }

    /// The same, but with the Git repository kept where jj keeps it when it
    /// is not colocated.
    fn apart(name: &str) -> Self {
        let mut world = Self::new(name);
        let root = world.dir.path().canonicalize().unwrap();
        sh(
            &root,
            "git clone -q --bare bare.git apart/.jj/repo/store/git \
             && cd apart/.jj/repo/store/git \
             && git config user.name owner && git config user.email owner@example.com \
             && git config commit.gpgsign false \
             && git worktree add -q ../../../../../apart-workspace -b work main",
        );
        world.repo.checkout = root.join("apart");
        world.repo.workspace = root.join("apart-workspace");
        world
    }

    /// Move the target on the remote, from elsewhere.
    fn moved(&self) -> String {
        sh(
            self.dir.path(),
            "git clone -q bare.git other && cd other \
             && git config user.name other && git config user.email other@example.com \
             && git config commit.gpgsign false \
             && echo other >c && git add c && git commit -qm moved && git push -q origin HEAD:main",
        );
        self.bare("git rev-parse main")
    }

    fn tracked(&self) -> String {
        let git = self.repo.checkout.join(".git");
        sh(
            &git,
            "git --git-dir=. rev-parse --verify refs/remotes/origin/main",
        )
    }

    fn work(&self, command: &str) {
        sh(&self.repo.workspace, command);
    }

    fn bare(&self, command: &str) -> String {
        sh(&self.bare, command)
    }

    async fn snapshot(&self) -> Snapshot {
        let url = self.bare.display().to_string();
        self.repo.snapshot(&url, "main", "The task").await.unwrap()
    }

    async fn content(&self) -> Content {
        match self.snapshot().await {
            Snapshot::Changed(content) => content,
            other => panic!("{other:?}"),
        }
    }

    async fn push(&self, content: &Content, before: Option<&str>) -> anyhow::Result<String> {
        let url = self.bare.display().to_string();
        self.repo.push(&url, "famulen/x", content, before).await
    }

    fn checks(&self, bucket: &str) {
        std::fs::write(self.dir.path().join("forge/checks"), bucket).unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn the_remote_and_its_default_branch_are_asked_of_the_remote() {
    let world = World::new("finishing-remote");
    let (url, target) = Repo::remote(&world.repo.jj, &world.repo.checkout)
        .await
        .unwrap();
    assert!(url.ends_with("bare.git"), "{url}");
    assert_eq!(target, "main");
}

#[cfg(unix)]
#[tokio::test]
async fn the_target_jj_knows_is_brought_up_to_the_remotes_even_under_conflicts() {
    let world = World::new("finishing-tracking");
    world.work("echo two >b && git add b && git commit -qm 'Add b'");
    let tip = world.moved();
    std::fs::write(world.dir.path().join("conflicted"), "").unwrap();
    assert_eq!(world.snapshot().await, Snapshot::Conflicted);
    assert_eq!(world.tracked(), tip);
    assert!(world.dir.path().join("imported").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn a_tracking_ref_that_points_elsewhere_is_not_written_through() {
    let world = World::new("finishing-symref");
    let before = sh(&world.repo.checkout, "git rev-parse main");
    sh(
        &world.repo.checkout,
        "git symbolic-ref refs/remotes/origin/main refs/heads/main",
    );
    let tip = world.moved();
    world.snapshot().await;
    assert_eq!(world.tracked(), tip);
    assert_eq!(sh(&world.repo.checkout, "git rev-parse main"), before);
}

#[cfg(unix)]
#[tokio::test]
async fn work_of_a_repository_git_does_not_find_by_itself_is_pushed() {
    let world = World::apart("finishing-apart");
    let (url, target) = Repo::remote(&world.repo.jj, &world.repo.checkout)
        .await
        .unwrap();
    assert!(url.ends_with("bare.git"), "{url}");
    assert_eq!(target, "main");
    world.work("echo two >b && git add b && git commit -qm one");
    let content = world.content().await;
    world.push(&content, None).await.unwrap();
    assert_eq!(world.bare("git rev-parse 'famulen/x^{tree}'"), content.tree);
}

#[cfg(unix)]
#[tokio::test]
async fn work_is_told_from_the_base_it_stands_on_though_the_target_moved() {
    let world = World::new("finishing-snapshot");
    assert_eq!(world.snapshot().await, Snapshot::Unchanged);
    world.work("echo two >b && git add b && git commit -qm 'Add b' -m 'Because.'");
    let base = world.bare("git rev-parse main");
    sh(
        &world.repo.checkout,
        "echo other >c && git add c && git commit -qm moved && git push -q origin HEAD:main",
    );
    let content = world.content().await;
    assert_eq!(content.base, base);
    let diff = world.repo.diff(&content).await.unwrap();
    assert!(diff.contains("+two") && !diff.contains("other"), "{diff}");
    assert_eq!(
        (content.title.as_str(), content.body.as_str()),
        ("Add b", "Because.")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn the_work_is_pushed_as_one_commit_over_what_famulen_pushed_alone() {
    let world = World::new("finishing-push");
    world.work("echo two >b && git add b && git commit -qm one");
    world.work("echo three >d && git add d && git commit -qm two");
    let content = world.content().await;
    let first = world.push(&content, None).await.unwrap();
    assert_eq!(world.bare("git rev-parse famulen/x^"), content.base);
    assert_eq!(world.bare("git rev-parse 'famulen/x^{tree}'"), content.tree);
    // A step cut off before it told pushes nothing again.
    assert_eq!(world.push(&content, None).await.unwrap(), first);

    world.work("echo four >e && git add e && git commit -qm three");
    let content = world.content().await;
    // Someone else's commit on the branch is not written over.
    world.bare("git update-ref refs/heads/famulen/x main");
    assert!(world.push(&content, Some(&first)).await.is_err());
    assert_eq!(
        world.bare("git rev-parse famulen/x"),
        world.bare("git rev-parse main")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn nothing_is_pushed_from_a_repository_that_takes_objects_from_elsewhere() {
    let world = World::new("finishing-alternates");
    world.work("echo two >b && git add b && git commit -qm one");
    let content = world.content().await;
    std::fs::write(
        world.repo.checkout.join(".git/objects/info/alternates"),
        "/nowhere\n",
    )
    .unwrap();
    assert!(world.push(&content, None).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn a_pull_request_is_made_once_checked_and_merged_only_at_the_commit_pushed() {
    let world = World::new("finishing-forge");
    world.work("echo two >b && git add b && git commit -qm one");
    let content = world.content().await;
    let head = world.push(&content, None).await.unwrap();
    let url = world.bare.display().to_string();
    let forge = Forge::new(&world.forge, &world.repo.checkout, &url);
    let pull = forge
        .open("famulen/x", "main", &head, "one", "")
        .await
        .unwrap();
    assert_eq!(
        forge
            .open("famulen/x", "main", &head, "one", "left by the last review")
            .await
            .unwrap(),
        pull
    );
    assert_eq!(
        std::fs::read_to_string(world.dir.path().join("forge/body")).unwrap(),
        "left by the last review\n"
    );
    world.checks("pending");
    assert_eq!(forge.ci(&pull).await.unwrap(), Ci::Pending);
    world.checks("fail");
    assert_eq!(
        forge.ci(&pull).await.unwrap(),
        Ci::Failed {
            name: "ci".into(),
            log: "the log of the failed check\n".into()
        }
    );
    world.checks("pass");
    assert_eq!(forge.ci(&pull).await.unwrap(), Ci::Passed);

    // The branch changed: CI says nothing of it, and it is not merged.
    world.bare("git update-ref refs/heads/famulen/x main");
    assert_eq!(forge.ci(&pull).await.unwrap(), Ci::Pending);
    assert!(forge.merge(&pull).await.is_err());
    assert!(!forge.merged(&pull).await.unwrap());

    world.bare(&format!("git update-ref refs/heads/famulen/x {head}"));
    forge.merge(&pull).await.unwrap();
    assert!(forge.merged(&pull).await.unwrap());
    assert_eq!(world.bare("git show main:b"), "two");

    world
        .repo
        .delete_branch(&url, "famulen/x", &head)
        .await
        .unwrap();
    assert_eq!(world.bare("git branch --list famulen/x"), "");
}
