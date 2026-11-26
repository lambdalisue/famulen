//! Work on code run directly, in a workspace of its own, asking the owner.

use super::*;
use crate::workers::direct::fenced;
use crate::workers::{Direct, answer_direct, direct_question};

/// Tells how it was started and where, asks to use the shell and then asks
/// its user, and writes down each answer.
const ASKS_THE_OWNER: &str = r#"#!/bin/sh
echo "$@" > DIR/args; pwd > DIR/cwd
read -r init; read -r ask
echo '{"type":"system","subtype":"init","session_id":"c1"}'
echo '{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"make"}}}'
read -r first; echo "$first" > DIR/first
echo '{"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"どれ？","options":[{"label":"A"},{"label":"B"}],"multiSelect":false}]}}}'
read -r second; echo "$second" > DIR/second
echo '{"type":"result","subtype":"success","is_error":false,"result":"done"}'
read -r never
"#;

/// Makes the workspace as `jj workspace add` would, and tells it was asked.
const JJ: &str = "#!/bin/sh\necho \"$@\" >> DIR/jj.log\ncase \"$*\" in *\"workspace add\"*) ;; *) exit 0 ;; esac\nfor last; do :; done\nmkdir -p \"$last/.jj\"\ntouch \"$last/README.md\"\n";

fn install(dir: &TempDir, name: &str, script: &str) -> String {
    let (source, bin) = (dir.path().join(format!("{name}.sh")), dir.path().join(name));
    std::fs::write(
        &source,
        script.replace("DIR", &dir.path().display().to_string()),
    )
    .unwrap();
    assert!(
        std::process::Command::new("install")
            .args(["-m", "755"])
            .arg(&source)
            .arg(&bin)
            .status()
            .unwrap()
            .success()
    );
    bin.display().to_string()
}

async fn until(pool: &Pool, what: &str, done: impl Fn() -> bool) {
    let runs = pool.app.workers.runs.clone();
    for _ in 0..400 {
        pool.tick().await.unwrap();
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never: {what}; {:?}", runs.recent(3).unwrap());
}

fn read(dir: &TempDir, name: &str) -> String {
    std::fs::read_to_string(dir.path().join(name)).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn work_run_directly_waits_for_the_owner_in_a_workspace_of_its_own_and_ends() {
    let dir = TempDir::new("pool-direct");
    let checkout = dir.path().join("src");
    std::fs::create_dir_all(checkout.join(".jj/repo")).unwrap();
    let checkout = checkout.canonicalize().unwrap();
    let (mut app, _, task) = agent_app(
        &dir,
        &ASKS_THE_OWNER.replace("DIR", &dir.path().display().to_string()),
        &[
            ("Runner", "direct"),
            ("Directory", &checkout.display().to_string()),
        ],
    );
    app.workers = app.workers.clone().with_direct(Some(Direct {
        repository: checkout.clone(),
        jj: install(&dir, "jj", JJ),
        fenced: true,
    }));
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));

    until(&pool, "asked to use the shell", || {
        app.workers.runs.recent(1).unwrap()[0].state == State::Waiting
    })
    .await;
    let run = app.workers.runs.recent(1).unwrap().remove(0);
    let workspace = checkout.join(format!(
        ".claude/worktrees/{}-run{}",
        run.order.code, run.id
    ));
    assert!(workspace.join(".jj").is_dir(), "{}", read(&dir, "jj.log"));
    assert_eq!(read(&dir, "cwd").trim(), workspace.display().to_string());
    let conversation = app
        .store
        .state(crate::workers::direct::conversation_key(run.id))
        .unwrap()
        .expect("kept before it was started");
    assert!(
        read(&dir, "args").contains(&format!("--session-id {conversation}")),
        "{}",
        read(&dir, "args")
    );
    // Asked as any call is, and nothing is told it meanwhile.
    let asked = direct_question(&app, &run).expect("a question");
    assert!(asked.ticket > 0, "{asked:?}");
    let ticket = app.gateway.ticket(asked.ticket).unwrap().unwrap();
    assert_eq!(
        (ticket.actor, ticket.tool.as_str()),
        (crate::permissions::direct_actor(run.id), "Bash")
    );
    assert!(read(&dir, "first").is_empty());
    assert!(
        answer_direct(
            &app,
            run.id,
            "other",
            crate::approvals::Answer::Approve,
            Vec::new()
        )
        .await
        .is_err(),
        "only what it asks is answered"
    );

    app.gateway
        .answer_ticket(asked.ticket, Answer::Approve)
        .await
        .unwrap();
    until(&pool, "allowed", || !read(&dir, "first").is_empty()).await;
    assert!(
        read(&dir, "first").contains("\"allow\""),
        "{} {:?}",
        read(&dir, "first"),
        app.workers.runs.recent(1).unwrap()
    );

    until(&pool, "asked its user", || {
        direct_question(&app, &app.workers.runs.get(run.id).unwrap().unwrap())
            .is_some_and(|q| q.id == "r2")
    })
    .await;
    let asked = direct_question(&app, &run).unwrap();
    assert_eq!(
        asked.choices,
        [(
            "どれ？".to_owned(),
            vec!["A".to_owned(), "B".to_owned()],
            false
        )]
    );
    answer_direct(
        &app,
        run.id,
        "r2",
        crate::approvals::Answer::Approve,
        vec!["B".into()],
    )
    .await
    .unwrap();
    until(&pool, "over", || {
        app.workers.runs.open().unwrap().is_empty()
    })
    .await;
    assert!(
        read(&dir, "second").contains("どれ？"),
        "{}",
        read(&dir, "second")
    );
    assert!(
        read(&dir, "second").contains("\"B\""),
        "{}",
        read(&dir, "second")
    );
    let run = app.workers.runs.get(run.id).unwrap().unwrap();
    assert_eq!(run.outcome, Some(Outcome::Done));
    let status = app.records.need(TASKS, &task).unwrap().body.value("Status");
    assert_eq!(status.as_deref(), Some("done"));
    assert!(
        workspace.join(".jj").is_dir(),
        "the work is left where it is"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn work_run_directly_is_refused_where_the_owner_did_not_let_it_run() {
    let dir = TempDir::new("pool-direct-refused");
    let checkout = dir.path().join("src");
    std::fs::create_dir_all(checkout.join(".jj/repo")).unwrap();
    let (app, pool, _) = agent_app(
        &dir,
        &ASKS_THE_OWNER.replace("DIR", &dir.path().display().to_string()),
        &[
            ("Runner", "direct"),
            ("Directory", &checkout.display().to_string()),
        ],
    );
    pool.tick().await.unwrap();
    assert!(
        app.workers.runs.recent(1).unwrap().is_empty() || {
            let run = app.workers.runs.recent(1).unwrap().remove(0);
            run.state == State::Starting && run.note.contains("no repository is set")
        }
    );
    assert!(!dir.path().join("args").exists(), "nothing is started");
}

/// A real Claude Code with the owner's login does a small piece of work on
/// code, run directly in a jj workspace of a repository made for it, through
/// the pool and an executor as Famulen runs it. What it asks the owner is
/// answered here as the owner would, and printed. Nothing is opened: run it
/// alone, where `jj` and `claude` are on the PATH:
///
/// ```sh
/// cargo test -p famulen-core --lib live_direct -- --ignored --nocapture
/// ```
#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs a real Claude Code with the owner's login"]
async fn live_direct_work_on_code_runs_to_its_end() {
    let home = crate::sealed::home().unwrap();
    let root = home.join(format!(".famulen-live-direct-{}", std::process::id()));
    let checkout = root.join("calc");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::write(
        checkout.join("calc.sh"),
        "#!/bin/sh\ncase \"$1\" in\n  add) echo $(($2 + $3)) ;;\n  *) echo \"unknown: $1\" >&2; exit 2 ;;\nesac\n",
    )
    .unwrap();
    std::fs::write(
        checkout.join("test.sh"),
        "#!/bin/sh\nset -e\n[ \"$(sh calc.sh add 2 3)\" = 5 ]\necho ok\n",
    )
    .unwrap();
    std::fs::write(
        checkout.join("AGENTS.md"),
        "Check every change with `sh test.sh`.\n",
    )
    .unwrap();
    let jj = |args: &[&str]| {
        let done = std::process::Command::new("jj")
            .args(args)
            .current_dir(&checkout)
            .output()
            .unwrap();
        assert!(
            done.status.success(),
            "jj {args:?}: {}",
            String::from_utf8_lossy(&done.stderr)
        );
    };
    jj(&["git", "init", "--colocate"]);
    jj(&["describe", "-m", "calc: add"]);
    jj(&["bookmark", "create", "main", "-r", "@"]);
    jj(&["new"]);
    let checkout = checkout.canonicalize().unwrap();

    let dir = TempDir::new("live-direct");
    let mut app = crate::runtime::testing::app(dir.path());
    app.workers = app
        .workers
        .clone()
        .with_direct(Some(fenced(checkout.clone())));
    let task = app
        .records
        .add(
            Writer::Owner,
            TASKS,
            Patch {
                notes: Some(
                    "calc.sh に引き算 `sub` を足し、`sh calc.sh sub 5 3` が 2 を出すようにする。test.sh にその確かめを足し、`sh test.sh` が通ることを確かめる。変更の説明を jj describe で書く。".into(),
                ),
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
        .unwrap()
        .id;
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    let mut answered = Vec::new();
    let mut ended = None;
    for _ in 0..(20 * 60 * 2) {
        pool.tick().await.unwrap();
        if let Some(run) = app.workers.runs.recent(1).unwrap().pop() {
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
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let run = ended.expect("an end within 20 minutes");
    println!("outcome: {:?}\nresult:\n{}", run.outcome, run.result);
    let workspace = checkout.join(format!(
        ".claude/worktrees/{}-run{}",
        run.order.code, run.id
    ));
    let calc = std::fs::read_to_string(workspace.join("calc.sh")).unwrap_or_default();
    let tested = std::process::Command::new("sh")
        .arg("test.sh")
        .current_dir(&workspace)
        .output()
        .unwrap();
    let sub = std::process::Command::new("sh")
        .args(["calc.sh", "sub", "5", "3"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    let described = std::process::Command::new("jj")
        .args(["log", "--no-graph", "-r", "@", "-T", "description"])
        .current_dir(&workspace)
        .output()
        .unwrap();
    println!(
        "calc.sh:\n{calc}\ndescription: {}",
        String::from_utf8_lossy(&described.stdout)
    );
    let status = app.records.need(TASKS, &task).unwrap().body.value("Status");
    let left: Vec<String> = app.workers.sights.said(run.id);
    println!("said: {left:?}");
    assert_eq!(run.outcome, Some(Outcome::Done), "{}", run.result);
    assert_eq!(status.as_deref(), Some("done"));
    assert!(
        tested.status.success(),
        "{}",
        String::from_utf8_lossy(&tested.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&sub.stdout).trim(), "2");
    assert!(
        !std::fs::read_to_string(checkout.join("calc.sh"))
            .unwrap()
            .contains("sub"),
        "the checkout itself is not touched"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn work_run_directly_that_is_cancelled_is_interrupted_then_ended_and_its_request_taken_back()
{
    let script = r#"#!/bin/sh
read -r init; read -r ask
echo '{"type":"system","subtype":"init","session_id":"c1"}'
echo '{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"make"}}}'
read -r first; echo "$first" > DIR/first
exec sleep 300
"#;
    let dir = TempDir::new("pool-direct-cancel");
    let checkout = dir.path().join("src");
    std::fs::create_dir_all(checkout.join(".jj/repo")).unwrap();
    let checkout = checkout.canonicalize().unwrap();
    let (mut app, _, task) = agent_app(
        &dir,
        &script.replace("DIR", &dir.path().display().to_string()),
        &[
            ("Runner", "direct"),
            ("Directory", &checkout.display().to_string()),
        ],
    );
    app.workers = app.workers.clone().with_direct(Some(Direct {
        repository: checkout.clone(),
        jj: install(&dir, "jj", JJ),
        fenced: true,
    }));
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    until(&pool, "waiting", || {
        app.workers.runs.recent(1).unwrap()[0].state == State::Waiting
    })
    .await;
    let run = app.workers.runs.recent(1).unwrap().remove(0);
    let ticket = direct_question(&app, &run).unwrap().ticket;

    app.records
        .change(
            Writer::Owner,
            TASKS,
            &task,
            Patch::default().with("Status", "cancelled"),
        )
        .unwrap();
    until(&pool, "over", || {
        app.workers.runs.open().unwrap().is_empty()
    })
    .await;
    until(&pool, "ended", || {
        app.workers.executors.names().unwrap().is_empty()
    })
    .await;
    assert!(
        read(&dir, "first").contains("interrupt"),
        "{}",
        read(&dir, "first")
    );
    let run = app.workers.runs.get(run.id).unwrap().unwrap();
    assert_eq!(run.outcome, Some(Outcome::Stopped));
    assert_eq!(
        app.gateway.ticket(ticket).unwrap().unwrap().status,
        "lapsed"
    );
    assert_eq!(direct_question(&app, &run), None);
    let workspace = checkout.join(format!(
        ".claude/worktrees/{}-run{}",
        run.order.code, run.id
    ));
    assert!(
        workspace.join(".jj").is_dir(),
        "the work is left where it is"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_the_owner_declines_is_refused_to_the_agent_and_it_goes_on() {
    let dir = TempDir::new("pool-direct-decline");
    let checkout = dir.path().join("src");
    std::fs::create_dir_all(checkout.join(".jj/repo")).unwrap();
    let checkout = checkout.canonicalize().unwrap();
    let (mut app, _, _) = agent_app(
        &dir,
        &ASKS_THE_OWNER.replace("DIR", &dir.path().display().to_string()),
        &[
            ("Runner", "direct"),
            ("Directory", &checkout.display().to_string()),
        ],
    );
    app.workers = app.workers.clone().with_direct(Some(Direct {
        repository: checkout.clone(),
        jj: install(&dir, "jj", JJ),
        fenced: true,
    }));
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    until(&pool, "asked to use the shell", || {
        app.workers.runs.recent(1).unwrap()[0].state == State::Waiting
    })
    .await;
    let run = app.workers.runs.recent(1).unwrap().remove(0);
    let ticket = direct_question(&app, &run).unwrap().ticket;
    app.gateway
        .answer_ticket(ticket, Answer::Approve)
        .await
        .unwrap();
    until(&pool, "asked its user", || {
        direct_question(&app, &run).is_some_and(|q| q.id == "r2")
    })
    .await;
    answer_direct(
        &app,
        run.id,
        "r2",
        crate::approvals::Answer::Reject,
        Vec::new(),
    )
    .await
    .unwrap();
    until(&pool, "over", || {
        app.workers.runs.open().unwrap().is_empty()
    })
    .await;
    let second = read(&dir, "second");
    assert!(
        second.contains("\"deny\"") && second.contains("decide by yourself"),
        "{second}"
    );
    let run = app.workers.runs.get(run.id).unwrap().unwrap();
    assert_eq!(run.outcome, Some(Outcome::Done));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_of_its_name_famulen_did_not_make_for_it_is_neither_used_nor_removed() {
    let dir = TempDir::new("pool-direct-there");
    let checkout = dir.path().join("src");
    std::fs::create_dir_all(checkout.join(".jj/repo")).unwrap();
    let checkout = checkout.canonicalize().unwrap();
    let (mut app, _, task) = agent_app(
        &dir,
        &ASKS_THE_OWNER.replace("DIR", &dir.path().display().to_string()),
        &[
            ("Runner", "direct"),
            ("Directory", &checkout.display().to_string()),
        ],
    );
    // Where its first run would work.
    let code = app.records.codes(TASKS).unwrap()[&task].clone();
    let there = checkout.join(format!(".claude/worktrees/{code}-run1"));
    std::fs::create_dir_all(there.join(".jj")).unwrap();
    std::fs::write(there.join("left.md"), "of other work").unwrap();
    app.workers = app.workers.clone().with_direct(Some(Direct {
        repository: checkout.clone(),
        jj: install(&dir, "jj", JJ),
        fenced: true,
    }));
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    pool.tick().await.unwrap();
    let run = app.workers.runs.recent(1).unwrap().remove(0);
    assert_eq!(run.id, 1);
    assert_eq!(run.state, State::Starting);
    assert!(run.note.contains("there already"), "{}", run.note);
    assert_eq!(
        std::fs::read_to_string(there.join("left.md")).unwrap(),
        "of other work"
    );
    assert!(!dir.path().join("args").exists(), "nothing is started");
}
