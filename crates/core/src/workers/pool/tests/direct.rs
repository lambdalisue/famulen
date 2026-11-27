//! Work on code run directly, in a workspace of its own, asking the owner.

use super::*;
use crate::workers::direct::fenced;
use crate::workers::{Direct, answer_direct, direct_question, tell_direct};

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

pub(super) fn install(dir: &TempDir, name: &str, script: &str) -> String {
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

/// Waits to be told something more, and writes it down.
const WAITS_TO_BE_TOLD: &str = r#"#!/bin/sh
read -r init; read -r ask
echo '{"type":"system","subtype":"init","session_id":"c1"}'
read -r more; echo "$more" > DIR/more
echo '{"type":"result","subtype":"success","is_error":false,"result":"done"}'
read -r never
"#;

#[tokio::test(flavor = "multi_thread")]
async fn what_the_owner_says_more_reaches_work_run_directly_while_it_is_at_work() {
    let dir = TempDir::new("pool-direct-told");
    let checkout = dir.path().join("src");
    std::fs::create_dir_all(checkout.join(".jj/repo")).unwrap();
    let checkout = checkout.canonicalize().unwrap();
    let (mut app, _, _) = agent_app(
        &dir,
        &WAITS_TO_BE_TOLD.replace("DIR", &dir.path().display().to_string()),
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

    until(&pool, "at work", || {
        app.workers
            .runs
            .recent(1)
            .unwrap()
            .first()
            .is_some_and(|r| r.state == State::Running)
    })
    .await;
    let run = app.workers.runs.recent(1).unwrap().remove(0);
    assert!(tell_direct(&app, run.id, "  ").await.is_err());
    tell_direct(&app, run.id, "also add c").await.unwrap();
    until(&pool, "over", || {
        app.workers.runs.open().unwrap().is_empty()
    })
    .await;
    assert!(read(&dir, "more").contains("also add c"));
    let error = tell_direct(&app, run.id, "and d").await.unwrap_err();
    assert!(error.to_string().contains("over"), "{error}");
}

pub(super) async fn until(pool: &Pool, what: &str, done: impl Fn() -> bool) {
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

pub(super) fn read(dir: &TempDir, name: &str) -> String {
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
        (crate::permissions::job_actor(run.id), "Bash")
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

/// A jj repository of one small script at `root`, whose `main` is what it holds.
pub(super) fn made_repository(root: &std::path::Path) -> std::path::PathBuf {
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
    checkout.canonicalize().unwrap()
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
    let checkout = made_repository(&root);

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

/// Asks to use a connection's tool it was handed, one it was not, and reads
/// the notebook.
const USES_WHAT_IT_WAS_HANDED: &str = r#"#!/bin/sh
read -r init; read -r ask; echo "$ask" > DIR/message
may() { echo "{\"type\":\"control_request\",\"request_id\":\"$1\",\"request\":{\"subtype\":\"can_use_tool\",\"tool_name\":\"$1\",\"input\":{}}}"; read -r answer; echo "$answer" > "DIR/$1"; }
may mcp__adroit__send_to_session
may mcp__adroit__start_session
may mcp__famulen__notebook_read
may mcp__famulen__notebook_search
echo '{"type":"control_request","request_id":"r","request":{"subtype":"mcp_message","server_name":"famulen","message":{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"notebook_read","arguments":{"path":"records/memos/README.md"}}}}}'
read -r got; echo "$got" > DIR/got
echo '{"type":"result","subtype":"success","is_error":false,"result":"done"}'
read -r never
"#;

#[tokio::test(flavor = "multi_thread")]
async fn work_run_directly_uses_the_connections_and_the_notebook_it_was_handed() {
    let dir = TempDir::new("pool-direct-handed");
    let checkout = dir.path().join("src");
    std::fs::create_dir_all(checkout.join(".jj/repo")).unwrap();
    let checkout = checkout.canonicalize().unwrap();
    let (mut app, _, _) = agent_app(
        &dir,
        &USES_WHAT_IT_WAS_HANDED.replace("DIR", &dir.path().display().to_string()),
        &[
            ("Runner", "direct"),
            ("Directory", &checkout.display().to_string()),
            ("Tools", "mcp__adroit__send_to_session"),
            ("Files", "records/memos/README.md"),
        ],
    );
    app.workers = app.workers.clone().with_direct(Some(Direct {
        repository: checkout.clone(),
        jj: install(&dir, "jj", JJ),
        fenced: true,
    }));
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));

    // It is given the notebook along with a connection, so the owner says yes
    // before it starts.
    until(&pool, "asked to start", || {
        !pool.app.gateway.held_tickets().unwrap().is_empty()
    })
    .await;
    let start = pool.app.gateway.held_tickets().unwrap().remove(0);
    let approve = crate::approvals::Answer::Approve;
    pool.app
        .gateway
        .answer_ticket(start.id, approve.into())
        .await
        .unwrap();
    until(&pool, "read the notebook", || !read(&dir, "got").is_empty()).await;

    for (asked, answer) in [
        ("mcp__adroit__send_to_session", "\"allow\""),
        ("mcp__adroit__start_session", "\"deny\""),
        ("mcp__famulen__notebook_read", "\"allow\""),
        ("mcp__famulen__notebook_search", "\"deny\""),
    ] {
        let got = read(&dir, asked);
        assert!(got.contains(answer), "{asked}: {got}");
    }
    assert!(
        read(&dir, "got").contains("code: memo"),
        "{}",
        read(&dir, "got")
    );
    assert!(
        read(&dir, "message").contains("records/memos/README.md"),
        "{}",
        read(&dir, "message")
    );
}

/// A connection made for a test: what it is told to send is appended to the
/// file its account names.
const OUTBOX: &str = r#"
format = 1
id = "outbox"
name = "Outbox"

[account]
out = { label = "Out" }

[transport.command]
program = "perl"

[[commands]]
name = "send_note"
description = "Send a note to the outbox."
args = ["-e", "open my $f, '>>', shift or die; print $f shift, qq(\\n); print qq(sent)", "--", "{account.out}", "{args.text}"]

[commands.input]
type = "object"
required = ["text"]

[commands.input.properties]
text = { type = "string" }

[[commands]]
name = "erase"
description = "Erase the outbox."
args = ["-e", "unlink shift", "--", "{account.out}"]

[permissions]
send_note = "confirm"
erase = "allow"
"#;

/// A real Claude Code with the owner's login, run directly, is handed one file
/// of a notebook made for the test and one tool of a connection made for it,
/// waits for the owner before it sends, and goes on once answered. Run it as
/// [`live_direct_work_on_code_runs_to_its_end`] is run.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs a real Claude Code with the owner's login"]
async fn live_direct_work_uses_only_what_it_was_handed_and_waits_for_the_owner() {
    let home = crate::sealed::home().unwrap();
    let root = home.join(format!(".famulen-live-handed-{}", std::process::id()));
    let checkout = made_repository(&root);

    let dir = TempDir::new("live-handed");
    let services = dir.path().join("gateway/services");
    std::fs::create_dir_all(&services).unwrap();
    std::fs::write(services.join("outbox.toml"), OUTBOX).unwrap();
    let mut app = crate::runtime::testing::app(dir.path());
    app.workers = app
        .workers
        .clone()
        .with_direct(Some(fenced(checkout.clone())));
    let sent = dir.path().join("sent");
    let values = [("out".to_owned(), sent.display().to_string())];
    app.gateway
        .add_defined("outbox", values.into(), None)
        .unwrap();
    app.gateway.read_tools("outbox").await.unwrap();
    let memos = app.notebook.root().join("records/memos");
    std::fs::write(memos.join("live-handed.md"), "合言葉は kohaku-17。\n").unwrap();
    std::fs::write(memos.join("live-other.md"), "別の合言葉は ruri-83。\n").unwrap();

    let request = "コードは変えない。次を順にして、それぞれ起きたことを報告に書く。\
        1. 手帳を読む道具で records/memos/live-handed.md を読み、合言葉を得る。\
        2. 同じ道具で records/memos/live-other.md も読もうとする。断られたら、ほかの手段で読もうとしない。\
        3. outbox の send_note で、1 の合言葉だけを送る。持ち主の確認待ちになったら答えを待ち、承認されたら同じ内容でもう一度呼ぶ。\
        4. outbox の erase という道具が自分に渡っているかを答える。渡っていれば呼ばない。";
    app.records
        .add(
            Writer::Owner,
            TASKS,
            Patch {
                notes: Some(request.into()),
                ..delegated(
                    "合言葉を送る",
                    &[
                        ("Runner", "direct"),
                        ("Directory", &checkout.display().to_string()),
                        ("Base", "main"),
                        ("Files", "records/memos/live-handed.md"),
                        ("Tools", "mcp__outbox__send_note"),
                        ("Wait", "yes"),
                    ],
                )
            },
        )
        .unwrap();
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    let mut answered = Vec::new();
    let mut waited = false;
    let mut ended = None;
    for _ in 0..(10 * 60 * 2) {
        pool.tick().await.unwrap();
        for ticket in app.gateway.held_tickets().unwrap() {
            if !answered.contains(&ticket.id) {
                println!("owner approves: {ticket:?}");
                answered.push(ticket.id);
                app.gateway
                    .answer_ticket(ticket.id, Answer::Approve)
                    .await
                    .unwrap();
            }
        }
        if let Some(run) = app.workers.runs.recent(1).unwrap().pop() {
            waited |= run.state == State::Waiting;
            if run.state == State::Ended {
                ended = Some(run);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let run = ended.expect("an end within 10 minutes");
    println!("outcome: {:?}\nresult:\n{}", run.outcome, run.result);
    println!("said: {:?}", app.workers.sights.said(run.id));
    println!("approved: {}, waited: {waited}", answered.len());
    assert!(waited, "it never waited for the owner");
    assert_eq!(run.outcome, Some(Outcome::Done), "{}", run.result);
    assert_eq!(
        std::fs::read_to_string(&sent).unwrap_or_default(),
        "kohaku-17\n"
    );
    assert!(!run.result.contains("ruri-83"), "{}", run.result);
    std::fs::remove_dir_all(&root).ok();
}
