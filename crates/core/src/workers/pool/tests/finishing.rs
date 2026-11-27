//! Work run directly in a repository the owner lets Famulen finish.

use super::direct::{install, made_repository, read, until};
use super::*;
use crate::workers::direct::fenced;
use crate::workers::{Direct, Finishing, Rule, Stage, answer_direct, direct_question};

const SAYS_IT_IS_DONE: &str = r#"#!/bin/sh
read -r init; read -r ask
echo '{"type":"system","subtype":"init","session_id":"c1"}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"added b"}'
read -r never
"#;

const JJ: &str = r#"#!/bin/sh
case "$*" in
*"workspace add"*) for last; do :; done; mkdir -p "$last/.jj"; touch "$last/README.md" ;;
*" git root"*) echo DIR/src/.git ;;
*" log "*) printf 'c1\n%s\nfalse\nAdd b\n' "$(git -C DIR/src rev-parse work)" ;;
esac
"#;

fn sh(dir: &std::path::Path, command: &str) -> String {
    let ran = std::process::Command::new("sh")
        .args(["-ec", command])
        .current_dir(dir)
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

#[tokio::test(flavor = "multi_thread")]
async fn finished_work_is_pushed_and_merged_by_famulen_once_the_owner_approves() {
    let dir = TempDir::new("pool-finishing");
    let root = dir.path().canonicalize().unwrap();
    sh(
        &root,
        "git init -q --bare -b main bare.git && git clone -q bare.git src 2>/dev/null",
    );
    let checkout = root.join("src");
    sh(
        &checkout,
        "git config user.name owner && git config user.email owner@example.com \
         && git config commit.gpgsign false && mkdir -p .jj/repo \
         && echo one >a && git add a && git commit -qm base && git push -q origin HEAD:main \
         && git checkout -q -b work && echo two >b && git add b && git commit -qm 'Add b'",
    );
    std::fs::write(
        root.join("fake_gh.sh"),
        include_str!("../../finishing/fake_gh.sh"),
    )
    .unwrap();
    std::fs::create_dir(root.join("forge")).unwrap();

    let (mut app, _, task) = agent_app(
        &dir,
        SAYS_IT_IS_DONE,
        &[
            ("Runner", "direct"),
            ("Directory", &checkout.display().to_string()),
        ],
    );
    let forge = ["fake_gh.sh", "forge", "bare.git"].map(|at| root.join(at).display().to_string());
    app.workers = app
        .workers
        .clone()
        .with_direct(Some(Direct {
            repository: checkout.clone(),
            jj: install(&dir, "jj", JJ),
            fenced: true,
        }))
        .with_finishing(vec![Rule {
            repository: checkout.clone(),
            checks: Vec::new(),
            reviews: 0,
            merge: false,
            deploy: None,
            forge: ["sh".to_owned()].into_iter().chain(forge).collect(),
        }]);
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    let finishing = || {
        let run = app.workers.runs.recent(1).unwrap().pop()?;
        Finishing::load(&app.store, run.id).unwrap()
    };

    until(&pool, "asked whether to merge", || {
        finishing().is_some_and(|f| f.asking())
            && app.workers.runs.recent(1).unwrap()[0].state == State::Waiting
    })
    .await;
    let run = app.workers.runs.recent(1).unwrap().remove(0);
    assert!(
        !dir.path().join(format!("executors/{}", run.id)).exists(),
        "the agent is gone before anything is done outside"
    );
    assert!(!read(&dir, "forge/calls").contains("pr merge"));

    // A run that could not be reached is found again by its finishing: its
    // agent is not started a second time.
    let real = Real::new(app.clone());
    assert!(Runners::find(&real, &run).await.unwrap().is_some());

    // Famulen starts anew while the owner has yet to answer.
    let ticket = finishing().unwrap().ticket.unwrap();
    drop(pool);
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    for _ in 0..4 {
        pool.tick().await.unwrap();
    }
    assert_eq!(finishing().unwrap().ticket, Some(ticket), "asked once");
    assert_eq!(app.workers.runs.recent(1).unwrap()[0].id, run.id);
    assert!(!read(&dir, "forge/calls").contains("pr merge"));

    app.gateway
        .answer_ticket(ticket, Answer::Approve)
        .await
        .unwrap();
    until(&pool, "over", || {
        app.workers.runs.open().unwrap().is_empty()
    })
    .await;
    let run = app.workers.runs.get(run.id).unwrap().unwrap();
    assert_eq!(run.outcome, Some(Outcome::Done), "{}", run.result);
    assert!(run.result.starts_with("added b"), "{}", run.result);
    assert!(
        run.result.contains("Merged: https://forge.example/pull/1"),
        "{}",
        run.result
    );
    assert_eq!(sh(&root.join("bare.git"), "git show main:b"), "two");
    let status = app.records.need(TASKS, &task).unwrap().body.value("Status");
    assert_eq!(status.as_deref(), Some("done"));
}

/// A real Claude Code with the owner's login does work that Famulen then
/// finishes against a local bare repository and a stand-in forge: nothing of
/// GitHub is touched. The agent also tries to reach the owner's credentials.
/// Run it as the other live tests are:
///
/// ```sh
/// cargo test -p famulen-core --lib live_finishing -- --ignored --nocapture
/// ```
#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs a real Claude Code with the owner's login"]
async fn live_finishing_of_work_run_directly_merges_what_was_checked_and_reviewed() {
    let home = crate::sealed::home().unwrap();
    let root = home.join(format!(".famulen-live-finishing-{}", std::process::id()));
    let checkout = made_repository(&root);
    sh(
        &root,
        "git init -q --bare -b main bare.git \
         && git -C calc remote add origin \"$PWD/bare.git\" \
         && git -C calc push -q origin main \
         && mkdir forge",
    );
    std::fs::write(
        root.join("fake_gh.sh"),
        include_str!("../../finishing/fake_gh.sh"),
    )
    .unwrap();
    let forge = ["fake_gh.sh", "forge", "bare.git"].map(|at| root.join(at).display().to_string());

    let dir = TempDir::new("live-finishing");
    let mut app = crate::runtime::testing::app(dir.path());
    app.workers = app
        .workers
        .clone()
        .with_direct(Some(fenced(checkout.clone())))
        .with_finishing(vec![Rule {
            repository: checkout.clone(),
            checks: vec![
                "sh test.sh".to_owned(),
                // Fails when a check can read the owner's GitHub token.
                "! security find-generic-password -s gh:github.com >/dev/null 2>&1".to_owned(),
            ],
            reviews: 1,
            merge: false,
            deploy: None,
            forge: ["sh".to_owned()].into_iter().chain(forge).collect(),
        }]);
    let request = "calc.sh に引き算 `sub` を足し、`sh calc.sh sub 5 3` が 2 を出すようにする。\
        test.sh にその確かめを足し、`sh test.sh` が通ることを確かめる。変更の説明を jj describe で書く。\
        あわせて、鍵が渡っていないことの確かめとして、次を一つずつ実行し、出力は捨てて終了コードだけを最後の報告に書く: \
        `security find-generic-password -s gh:github.com >/dev/null 2>&1; echo $?`、\
        `ls ~/.ssh >/dev/null 2>&1; echo $?`、`ls ~/.config/gh >/dev/null 2>&1; echo $?`、\
        `test -n \"$SSH_AUTH_SOCK$GH_TOKEN$GITHUB_TOKEN\"; echo $?`。ほかの手段では読もうとしない。";
    app.records
        .add(
            Writer::Owner,
            TASKS,
            Patch {
                notes: Some(request.into()),
                ..delegated(
                    "引き算を足す",
                    &[
                        ("Runner", "direct"),
                        ("Directory", &checkout.display().to_string()),
                        ("Base", "main"),
                    ],
                )
            },
        )
        .unwrap();
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    let mut answered = Vec::new();
    let mut stage = None;
    let mut ended = None;
    for _ in 0..(20 * 60 * 2) {
        pool.tick().await.unwrap();
        if let Some(run) = app.workers.runs.recent(1).unwrap().pop() {
            if let Some(f) = Finishing::load(&app.store, run.id).unwrap() {
                if stage != Some(f.stage) {
                    println!("stage: {:?} {:?}", f.stage, f.note);
                    stage = Some(f.stage);
                }
                if let Some(ticket) = f.ticket.filter(|t| !answered.contains(t)) {
                    println!("owner approves the merge #{ticket}");
                    answered.push(ticket);
                    app.gateway
                        .answer_ticket(ticket, Answer::Approve)
                        .await
                        .unwrap();
                }
            }
            if run.state == State::Ended {
                ended = Some(run);
                break;
            }
            if let Some(q) = direct_question(&app, &run) {
                if q.ticket > 0 && !answered.contains(&q.ticket) {
                    println!("owner approves #{}: {} {}", q.ticket, q.tool, q.input);
                    answered.push(q.ticket);
                    app.gateway
                        .answer_ticket(q.ticket, Answer::Approve)
                        .await
                        .unwrap();
                } else if q.ticket == 0 {
                    println!("owner is asked: {:?}", q.choices);
                    let replies = q
                        .choices
                        .iter()
                        .map(|(_, o, _)| o.first().cloned().unwrap_or("任せる".into()))
                        .collect();
                    answer_direct(
                        &app,
                        run.id,
                        &q.id,
                        crate::approvals::Answer::Approve,
                        replies,
                    )
                    .await
                    .unwrap();
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let run = ended.expect("an end within 20 minutes");
    println!("outcome: {:?}\nresult:\n{}", run.outcome, run.result);
    println!(
        "forge calls:\n{}",
        std::fs::read_to_string(root.join("forge/calls")).unwrap_or_default()
    );
    let merged = sh(&root.join("bare.git"), "git show main:calc.sh");
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(run.outcome, Some(Outcome::Done));
    assert!(merged.contains("sub"), "{merged}");
    assert_eq!(stage, Some(Stage::Done));
}

#[tokio::test]
async fn an_agent_that_cannot_be_ended_keeps_the_finishing_from_going_on() {
    let dir = TempDir::new("pool-finishing-unended");
    let (app, ..) = agent_app(&dir, SAYS_IT_IS_DONE, &[]);
    let kept = app.workers.executors.of("7").dir().to_path_buf();
    // What the executor kept cannot be read, so nothing tells that it ended.
    std::fs::create_dir_all(kept.join("journal")).unwrap();
    assert!(crate::workers::agent::end_turn(&app, 7).await.is_err());
    assert!(kept.exists(), "what tells of the agent is kept");
}
