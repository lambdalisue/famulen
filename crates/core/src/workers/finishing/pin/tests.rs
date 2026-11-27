use super::*;
use crate::notebook::testing::TempDir;
use crate::workers::direct::tests::stand_in;
use crate::workers::finishing::tests::rule;

fn git(dir: &Path, args: &[&str]) {
    let done = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(
        done.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&done.stderr)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn only_a_repository_the_owner_named_is_finished_and_only_famulen_is_deployed() {
    let dir = TempDir::new("finishing-pin");
    let (remote, checkout) = (dir.path().join("remote.git"), dir.path().join("checkout"));
    git(dir.path(), &["init", "--bare", "-b", "main", "remote.git"]);
    git(dir.path(), &["clone", "-q", "remote.git", "checkout"]);
    git(&checkout, &["commit", "--allow-empty", "-m", "one"]);
    git(&checkout, &["push", "-q", "origin", "HEAD:main"]);
    let checkout = checkout.canonicalize().unwrap();
    let branch = || "famulen/t1".to_owned();
    let jj = dir.path().join("jj");
    stand_in(&jj, "echo \"$2/.git\"\n");
    let jj = jj.display().to_string();

    assert_eq!(pin(&[], &jj, &checkout, branch()).await.unwrap(), None);
    let mut named = rule();
    named.repository = remote.clone();
    assert_eq!(
        pin(&[named.clone()], &jj, &checkout, branch())
            .await
            .unwrap(),
        None
    );

    named.repository = checkout.clone();
    named.deploy = Some("just deploy".into());
    let pinned = pin(&[named], &jj, &checkout, branch())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pinned.target, "main");
    assert!(pinned.remote.ends_with("remote.git"), "{}", pinned.remote);
    assert_eq!(pinned.rule.deploy, None);
}

#[test]
fn famulens_own_repository_is_known_by_any_address() {
    assert!(deployable("git@github.com:lambdalisue/famulen.git"));
    assert!(deployable("https://github.com/lambdalisue/famulen"));
    assert!(!deployable("https://github.com/someone/famulen"));
    assert!(!deployable("/srv/lambdalisue/famulen.git"));
}
