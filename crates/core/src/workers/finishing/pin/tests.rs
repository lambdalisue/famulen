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
fn a_key_kept_in_git_settings_is_named_and_never_told() {
    let named = |listing: &str| credential_key(listing);
    assert_eq!(
        named("user.name\nme\0remote.origin.url\nhttps://tok@github.com/a/b\0"),
        Some("remote.origin.url".into())
    );
    assert_eq!(
        named("url.https://u:tok@github.com/.insteadof\ngh:\0"),
        Some("url.<url>.insteadof".into())
    );
    assert_eq!(
        named("http.https://github.com/.extraheader\nAUTHORIZATION: basic x\0"),
        Some("http.<url>.extraheader".into())
    );
    assert_eq!(
        named("credential.helper\n!f() { echo password=tok; }; f\0"),
        Some("credential.helper".into())
    );
    assert_eq!(named("github.token\ntok\0"), Some("github.token".into()));
    assert_eq!(
        named(
            "remote.origin.url\ngit@github.com:a/b.git\0remote.up.url\nssh://git@host/a\0\
             credential.helper\nosxkeychain\0http.extraheader\n\0\
             remote.web.url\nhttps://github.com/a/b@c\0core.bare\0"
        ),
        None
    );
}

#[test]
fn famulens_own_repository_is_known_by_any_address() {
    assert!(deployable("git@github.com:lambdalisue/famulen.git"));
    assert!(deployable("https://github.com/lambdalisue/famulen"));
    assert!(!deployable("https://github.com/someone/famulen"));
    assert!(!deployable("/srv/lambdalisue/famulen.git"));
}
