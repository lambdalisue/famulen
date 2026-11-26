//! Delegated tasks: added, listed, retried and cancelled, and the settings of
//! the pool that runs them.

use famulen_api::{SECRETARY, WORKER};
use serde_json::{Value, json};

use crate::support::{call, call_as, open, temp, try_call, watch};

fn agent(title: &str) -> Value {
    json!({ "title": title, "request": "調べて結果を書く", "runner": "agent", "tools": ["WebSearch"] })
}

#[tokio::test]
async fn the_secretary_delegates_work_as_a_task_like_any_other() {
    let dir = temp();
    let api = open(&dir);
    let first = call(
        &api,
        "RecordService/SaveRecord",
        json!({ "kind": "tasks", "title": "先の仕事" }),
    )
    .await;

    let added = call_as(
        &api,
        SECRETARY,
        "WorkerService/DelegateTask",
        json!({
            "title": "ログインを直す", "request": "二度目に失敗する。直す。", "runner": "adroit",
            "directory": if cfg!(windows) { r"C:\src\api" } else { "/src/api" }, "worktree": true, "priority": "high",
            "blockedBy": [first["code"]],
        }),
    )
    .await;

    assert_eq!(added["kind"], "tasks", "{added}");
    assert_eq!(
        added["notes"].as_str().unwrap().trim(),
        "二度目に失敗する。直す。"
    );
    let markdown = added["markdown"].as_str().unwrap();
    for line in [
        "- Assignee: worker",
        "- Priority: high",
        "- Runner: adroit",
        if cfg!(windows) {
            r"- Directory: C:\src\api"
        } else {
            "- Directory: /src/api"
        },
        "- Worktree: yes",
    ] {
        assert!(markdown.contains(line), "{markdown}");
    }
    let listed = call(&api, "WorkerService/ListDelegated", json!({})).await;
    assert_eq!(listed["queue"][0]["code"], added["code"]);
    assert_eq!(listed["queue"][0]["state"], "blocked");
    assert_eq!(listed["queue"][0]["runner"], "adroit");
}

#[tokio::test]
async fn what_could_not_be_run_is_not_added() {
    let dir = temp();
    let api = open(&dir);
    for bad in [
        json!({ "title": "x", "request": "y", "runner": "human" }),
        json!({ "title": "x", "request": " ", "runner": "agent" }),
        json!({ "title": "x", "request": "y", "runner": "adroit" }),
        json!({ "title": "x", "request": "y", "runner": "adroit", "directory": "src" }),
        json!({ "title": "x", "request": "y", "runner": "agent", "tools": ["mcp__famulen__save_record"] }),
    ] {
        let (status, _) = try_call(&api, "WorkerService/DelegateTask", bad.clone()).await;
        assert_ne!(status, 200, "{bad}");
    }
    let tasks = call(
        &api,
        "RecordService/ListRecords",
        json!({ "kind": "tasks" }),
    )
    .await;
    assert_eq!(tasks.get("records"), None, "{tasks}");
}

#[tokio::test]
async fn a_worker_delegates_nothing() {
    let dir = temp();
    let api = open(&dir);
    let refused = call_as(&api, WORKER, "WorkerService/DelegateTask", agent("x")).await;
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|m| m.contains("may not write the notebook")),
        "{refused}"
    );
}

#[tokio::test]
async fn a_delegated_task_is_cancelled_and_put_back_in_the_queue() {
    let dir = temp();
    let api = open(&dir);
    let t = call(&api, "WorkerService/DelegateTask", agent("調べる")).await;
    let code = t["code"].clone();

    let cancelled = call_as(
        &api,
        SECRETARY,
        "WorkerService/CancelDelegated",
        json!({ "id": code }),
    )
    .await;
    assert!(
        cancelled["markdown"]
            .as_str()
            .unwrap()
            .contains("Status: cancelled"),
        "{cancelled}"
    );
    let listed = call(&api, "WorkerService/ListDelegated", json!({})).await;
    assert_eq!(listed.get("queue"), None, "{listed}");
    let (status, _) = try_call(&api, "WorkerService/CancelDelegated", json!({ "id": code })).await;
    assert_ne!(status, 200, "over already");

    call(&api, "WorkerService/RetryDelegated", json!({ "id": code })).await;
    let listed = call(&api, "WorkerService/ListDelegated", json!({})).await;
    assert_eq!(listed["queue"][0]["state"], "starting");

    let own = call(
        &api,
        "RecordService/SaveRecord",
        json!({ "kind": "tasks", "title": "自分の仕事" }),
    )
    .await;
    for method in ["CancelDelegated", "RetryDelegated"] {
        let (status, _) = try_call(
            &api,
            &format!("WorkerService/{method}"),
            json!({ "id": own["code"] }),
        )
        .await;
        assert_ne!(status, 200, "{method}: not delegated");
    }
}

#[tokio::test]
async fn the_owner_bounds_and_pauses_the_pool_and_the_secretary_has_no_tool_for_it() {
    let dir = temp();
    let api = open(&dir);
    let s = call(&api, "WorkerService/GetWorkerSettings", json!({})).await;
    assert_eq!(s["slots"], 2);

    call(
        &api,
        "WorkerService/SetWorkerSettings",
        json!({ "slots": 3, "paused": true }),
    )
    .await;
    call(&api, "WorkerService/DelegateTask", agent("調べる")).await;
    let listed = call(&api, "WorkerService/ListDelegated", json!({})).await;
    assert_eq!(listed["queue"][0]["state"], "paused");
    for slots in [0, -1, 1000] {
        let (status, _) = try_call(
            &api,
            "WorkerService/SetWorkerSettings",
            json!({ "slots": slots }),
        )
        .await;
        assert_ne!(status, 200, "{slots}");
    }
    let s = call(&api, "WorkerService/GetWorkerSettings", json!({})).await;
    assert_eq!((&s["slots"], &s["paused"]), (&json!(3), &json!(true)));

    use famulen_core::hub::OwnTools;
    let tools: Vec<String> = famulen_api::tools::ApiTools::new(api)
        .unwrap()
        .list()
        .into_iter()
        .map(|t| t.name)
        .collect();
    for tool in [
        "delegate_task",
        "list_delegated",
        "retry_delegated",
        "cancel_delegated",
    ] {
        assert!(tools.iter().any(|t| t == tool), "{tool}");
    }
    for tool in ["set_worker_settings", "preview_tidying", "tidy_up"] {
        assert!(!tools.iter().any(|t| t == tool), "{tool}");
    }
}

#[tokio::test]
async fn nothing_is_tidied_where_nothing_was_delegated() {
    let dir = temp();
    let api = open(&dir);
    for method in ["PreviewTidying", "TidyUp"] {
        let left = call(&api, &format!("WorkerService/{method}"), json!({})).await;
        assert_eq!(left.get("left"), None, "{left}");
    }
}

#[tokio::test]
async fn the_owner_says_how_work_on_code_is_finished() {
    let dir = temp();
    let api = open(&dir);
    let finishes = json!([{ "steps": "1. レビューする" }]);
    call(
        &api,
        "WorkerService/SetWorkerSettings",
        json!({ "slots": 2, "finishes": finishes }),
    )
    .await;
    let s = call(&api, "WorkerService/GetWorkerSettings", json!({})).await;
    assert_eq!(s["finishes"], finishes);
    assert_eq!(
        s["bundledFinishes"][0]["repository"],
        "github.com/lambdalisue/famulen"
    );
    for bad in [
        json!({ "steps": " " }),
        json!({ "repository": "src", "steps": "x" }),
    ] {
        let (status, _) = try_call(
            &api,
            "WorkerService/SetWorkerSettings",
            json!({ "slots": 2, "finishes": [bad] }),
        )
        .await;
        assert_ne!(status, 200, "{bad}");
    }
}

#[tokio::test]
async fn the_delegated_tasks_of_one_workgroup_or_one_task_alone_are_listed() {
    let dir = temp();
    let api = open(&dir);
    let mut codes = Vec::new();
    for (title, workgroup) in [("仕事の", "work"), ("家の", "personal")] {
        let mut request = agent(title);
        request["workgroups"] = json!([workgroup]);
        let made = call(&api, "WorkerService/DelegateTask", request).await;
        codes.push(made["code"].clone());
    }
    for (asked, expected) in [
        (json!({}), vec!["仕事の", "家の"]),
        (json!({ "workgroup": "all" }), vec!["仕事の", "家の"]),
        (json!({ "workgroup": "work" }), vec!["仕事の"]),
        (json!({ "task": codes[1] }), vec!["家の"]),
        (json!({ "task": codes[1], "workgroup": "work" }), vec![]),
    ] {
        let listed = call(&api, "WorkerService/ListDelegated", asked.clone()).await;
        let titles: Vec<&str> = listed["queue"]
            .as_array()
            .map(|q| q.iter().map(|t| t["title"].as_str().unwrap()).collect())
            .unwrap_or_default();
        assert_eq!(titles, expected, "{asked}");
    }
}

#[tokio::test]
async fn only_a_task_no_slot_is_free_for_is_queued() {
    let dir = temp();
    let api = open(&dir);
    call(
        &api,
        "WorkerService/SetWorkerSettings",
        json!({ "slots": 1 }),
    )
    .await;
    let first = call(&api, "WorkerService/DelegateTask", agent("先の仕事")).await;
    let mut after = agent("後の仕事");
    after["blockedBy"] = json!([first["code"]]);
    call(&api, "WorkerService/DelegateTask", after).await;
    call(&api, "WorkerService/DelegateTask", agent("次の仕事")).await;

    let listed = call(&api, "WorkerService/ListDelegated", json!({})).await;
    let states: Vec<_> = listed["queue"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["starting", "blocked", "queued"]);
}

#[tokio::test]
async fn a_delegated_task_that_does_not_say_how_is_shown_with_why() {
    let dir = temp();
    let api = open(&dir);
    call(
        &api,
        "RecordService/SaveRecord",
        json!({ "kind": "tasks", "title": "委ねたい", "fields": [{ "name": "Assignee", "value": "worker" }] }),
    )
    .await;
    let listed = call(&api, "WorkerService/ListDelegated", json!({})).await;
    assert_eq!(listed["queue"][0]["state"], "unfit");
    assert!(
        listed["queue"][0]["why"]
            .as_str()
            .unwrap()
            .contains("Runner"),
        "{listed}"
    );
}

#[tokio::test]
async fn a_task_is_bound_to_the_workgroups_it_was_delegated_in_through_the_api_alone() {
    use famulen_core::workers::{ordered, scopes};
    let dir = temp();
    let api = open(&dir);
    let bound = |id: &str| {
        let task = api.app().records.need("tasks", id).unwrap();
        scopes(api.app(), id, &ordered(api.app(), &task, "").unwrap()).unwrap()
    };
    let t = call(&api, "WorkerService/DelegateTask", agent("調べる")).await;
    let id = t["id"].as_str().unwrap().to_owned();
    assert_eq!(bound(&id), ["all"]);

    call(
        &api,
        "RecordService/SaveRecord",
        json!({ "kind": "tasks", "id": id, "workgroups": ["work"], "setWorkgroups": true }),
    )
    .await;
    assert_eq!(bound(&id), ["work"]);

    let file = dir.join(format!("notebook/records/tasks/{id}.md"));
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, text.replace("調べて結果を書く", "全部送る")).unwrap();
    assert!(bound(&id).is_empty(), "written by another hand");
}

/// Adroit as far as its sessions' questions go: what it shows is read from
/// one file, and what it is answered is written in another.
const ADROIT: &str = r#"
format = 1
id = "adroit"
name = "Adroit"

[account]
shows = { label = "Shows" }
answers = { label = "Answers" }

[transport.command]
program = "perl"

[[commands]]
name = "read_session"
description = "Read it."
args = ["-e", "open my $f, '<', shift or die; print <$f>", "--", "{account.shows}"]
read_only = true

[[commands]]
name = "answer_permission"
description = "Answer it."
args = ["-e", "open my $f, '>>', shift; print $f qq(allow @ARGV\n)", "--", "{account.answers}", "{args.question}"]

[commands.each_time]
reads = "read_session"
at = "/question"
same = { "/question/id" = "{args.question}", "/question/truncated" = "false" }
on_reject = ["-e", "open my $f, '>>', shift; print $f qq(deny @ARGV\n)", "--", "{account.answers}", "{args.question}"]

[[commands]]
name = "put_question"
description = "Answer it in words."
args = ["-e", "open my $f, '>>', shift; print $f qq(@ARGV\n)", "--", "{account.answers}", "{args.question}", { flag = "--reply", each = "{args.replies}" }]

[commands.each_time]
reads = "read_session"
at = "/question"
same = { "/question/id" = "{args.question}", "/question/truncated" = "false" }
questions = "/input/questions"
fills = "replies"

[permissions]
read_session = "allow"
"#;

struct Waiting {
    dir: crate::support::DataDir,
    api: famulen_api::Api,
    run: i64,
}

impl Waiting {
    /// A run whose session waits on the owner for `question`.
    async fn on(question: &Value) -> Self {
        let dir = temp();
        crate::support::define(&dir, "adroit", ADROIT);
        let api = open(&dir);
        let path = |name: &str| dir.join(name).to_str().unwrap().to_owned();
        std::fs::write(
            dir.join("shows"),
            json!({ "state": "waiting", "question": question }).to_string(),
        )
        .unwrap();
        let values = [("shows", path("shows")), ("answers", path("answers"))]
            .map(|(k, v)| (k.to_owned(), v));
        let app = api.app().clone();
        app.gateway
            .add_defined("adroit", values.into(), None)
            .unwrap();
        app.gateway.read_tools("adroit").await.unwrap();
        call(
            &api,
            "WorkerService/SetWorkerSettings",
            json!({ "slots": 2, "paused": true }),
        )
        .await;
        let t = call(
            &api,
            "WorkerService/DelegateTask",
            json!({ "title": "直す", "request": "x", "runner": "adroit", "directory": if cfg!(windows) { r"C:\src" } else { "/src" } }),
        )
        .await;
        let id = t["id"].as_str().unwrap();
        let task = app
            .records
            .get(famulen_core::records::TASKS, id)
            .unwrap()
            .unwrap();
        let order = famulen_core::workers::Order::of(&task, t["code"].as_str().unwrap()).unwrap();
        let run = app.workers.runs.begin(id, &order).unwrap().id;
        app.workers.runs.through(run, "adroit").unwrap();
        app.workers.runs.running(run, "s1").unwrap();
        app.workers
            .runs
            .stand(run, famulen_core::workers::State::Waiting)
            .unwrap();
        Self { dir, api, run }
    }

    /// What the owner is shown of it.
    async fn shown(&self) -> Value {
        call(
            &self.api,
            "WorkerService/ReadRun",
            json!({ "id": self.run }),
        )
        .await["run"]["question"]
            .clone()
    }

    async fn answer(&self, question: &str, answer: &str, replies: &[&str]) -> bool {
        let (status, _) = try_call(
            &self.api,
            "WorkerService/AnswerRun",
            json!({ "run": self.run, "question": question, "answer": format!("APPROVAL_ANSWER_{answer}"), "replies": replies }),
        )
        .await;
        status == 200
    }

    async fn ask_again(&self) -> bool {
        let (status, _) = try_call(
            &self.api,
            "WorkerService/AskRunAgain",
            json!({ "run": self.run, "question": "q1" }),
        )
        .await;
        status == 200
    }

    fn answered(&self) -> String {
        std::fs::read_to_string(self.dir.join("answers")).unwrap_or_default()
    }
}

fn question() -> Value {
    json!({
        "id": "q1", "kind": "question", "tool": "AskUserQuestion", "truncated": false,
        "input": { "questions": [{ "question": "閉じますか？", "options": [{ "label": "閉じる" }, { "label": "残す" }] }] },
    })
}

#[tokio::test]
async fn the_owner_answers_what_a_session_asks_where_the_run_is_shown() {
    let w = Waiting::on(&question()).await;
    let shown = w.shown().await;
    assert_eq!(shown["id"], "q1", "{shown}");
    let asked = &shown["approval"]["questions"][0];
    assert_eq!(asked["question"], "閉じますか？", "{shown}");
    assert_eq!(asked["choices"][1]["label"], "残す");

    assert!(!w.answer("q0", "ALLOW", &["閉じる"]).await, "another");
    assert!(!w.answer("q1", "DENY", &[]).await, "a question takes words");
    assert!(!w.answer("q1", "ALLOW", &[]).await);
    assert!(w.answer("q1", "ALLOW", &["まだ 残す"]).await);
    assert!(!w.answer("q1", "ALLOW", &["閉じる"]).await, "answered once");

    assert_eq!(w.answered(), "q1 --reply まだ 残す\n");
    assert_eq!(w.shown().await, Value::Null);
    assert!(w.api.app().gateway.held_tickets().unwrap().is_empty());
}

#[tokio::test]
async fn two_answers_that_arrive_together_reach_the_session_once() {
    let w = Waiting::on(&question()).await;
    w.shown().await;
    let answer = |reply: &str| {
        try_call(
            &w.api,
            "WorkerService/AnswerRun",
            json!({ "run": w.run, "question": "q1", "answer": "APPROVAL_ANSWER_ALLOW", "replies": [reply] }),
        )
    };
    let ((a, _), (b, _)) = tokio::join!(answer("閉じる"), answer("残す"));
    let mut statuses = [a.as_u16(), b.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 400], "one answered, the other refused");
    assert_eq!(w.answered().lines().count(), 1, "{}", w.answered());
}

#[tokio::test]
async fn the_owner_allows_or_denies_what_a_session_may_do_where_the_run_is_shown() {
    for (answer, expected) in [("ALLOW", "allow q1\n"), ("DENY", "deny q1\n")] {
        let w = Waiting::on(
            &json!({ "id": "q1", "kind": "permission", "tool": "Bash", "truncated": false, "input": { "command": "rm -rf x" } }),
        )
        .await;
        let shown = w.shown().await;
        assert!(shown.to_string().contains("rm -rf x"), "{shown}");
        assert!(!w.answer("q1", "ALWAYS", &[]).await);
        assert!(w.answer("q1", answer, &[]).await);
        assert_eq!(w.answered(), expected);
    }
}

#[tokio::test]
async fn a_question_that_is_not_told_whole_is_not_answered_from_here() {
    let mut cut = question();
    cut["truncated"] = json!(true);
    let w = Waiting::on(&cut).await;
    assert_eq!(w.shown().await["truncated"], true);
    assert!(!w.answer("q1", "ALLOW", &["閉じる"]).await);
    assert!(!w.ask_again().await);
    assert_eq!(w.answered(), "");
}

#[tokio::test]
async fn a_question_is_asked_again_in_place_of_the_request_before() {
    let w = Waiting::on(&question()).await;
    w.shown().await;
    let held = || w.api.app().gateway.held_tickets().unwrap();

    assert!(w.ask_again().await);
    let first = held();
    assert!(w.ask_again().await);
    let second = held();

    assert_eq!((first.len(), second.len()), (1, 1));
    assert_ne!(first[0].id, second[0].id);
    assert_eq!(
        w.shown().await["approval"]["id"],
        second[0].id.to_string(),
        "the card knows the request"
    );
}

#[tokio::test]
async fn a_request_that_puts_the_question_to_the_owner_is_taken_back_once_it_is_answered_here() {
    let w = Waiting::on(&question()).await;
    w.shown().await;
    assert!(w.ask_again().await);
    let gateway = w.api.app().gateway.clone();
    let ticket = gateway.held_tickets().unwrap()[0].id;

    assert!(!w.answer("q1", "ALLOW", &[" "]).await);
    assert_eq!(gateway.held_tickets().unwrap().len(), 1, "not answered yet");
    assert!(w.answer("q1", "ALLOW", &["閉じる"]).await);

    let late = gateway
        .fill_ticket(ticket, vec!["残す".into()])
        .await
        .unwrap();
    assert!(late.is_none(), "no second answer");
    assert_eq!(w.answered(), "q1 --reply 閉じる\n");
}

#[tokio::test]
async fn the_secretary_neither_reads_the_question_there_nor_answers_for_the_owner() {
    let w = Waiting::on(&question()).await;
    w.shown().await;
    let listed = call_as(&w.api, SECRETARY, "WorkerService/ListDelegated", json!({})).await;
    assert_eq!(listed["queue"][0]["state"], "waiting", "{listed}");
    assert_eq!(listed["queue"][0]["run"]["question"], Value::Null);

    use famulen_core::hub::OwnTools;
    let dir = temp();
    let tools = famulen_api::tools::ApiTools::new(open(&dir)).unwrap();
    for tool in ["answer_run", "ask_run_again"] {
        assert!(!tools.list().iter().any(|t| t.name == tool), "{tool}");
    }
}

/// A run of an agent that has done something.
async fn agent_run(api: &famulen_api::Api) -> i64 {
    let app = api.app().clone();
    let t = call(api, "WorkerService/DelegateTask", agent("読む")).await;
    let id = t["id"].as_str().unwrap();
    let task = app
        .records
        .get(famulen_core::records::TASKS, id)
        .unwrap()
        .unwrap();
    let order = famulen_core::workers::Order::of(&task, t["code"].as_str().unwrap()).unwrap();
    let run = app.workers.runs.begin(id, &order).unwrap().id;
    let ledger = app.workers.sights.ledger();
    ledger
        .put(
            run,
            "a",
            1,
            famulen_core::workers::Item::Asked {
                text: "読んで".into(),
            },
        )
        .unwrap();
    let steps = vec![famulen_core::workers::Step {
        id: "g".into(),
        title: "読む".into(),
        status: "in_progress".into(),
        weight: 1.0,
        ..Default::default()
    }];
    ledger
        .put(
            run,
            "p",
            2,
            famulen_core::workers::Item::Plan {
                from: famulen_core::workers::PlanFrom::Timeline,
                steps,
            },
        )
        .unwrap();
    run
}

#[tokio::test]
async fn what_a_run_did_and_how_its_plan_changed_are_read_with_it() {
    let dir = temp();
    let api = open(&dir);
    let run = agent_run(&api).await;
    let output = call(&api, "WorkerService/ReadRun", json!({ "id": run })).await;
    assert_eq!(output["entries"][0]["asked"]["text"], "読んで", "{output}");
    assert_eq!(output["entries"][1]["plan"]["from"], "timeline");
    assert_eq!(output["run"]["planFrom"], "timeline");
    assert_eq!(output["run"]["doing"], "読む");
    let changes: Vec<_> = output["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["change"].as_str().unwrap(),
                c["to"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(changes, [("added", ""), ("status", "in_progress")]);
}

#[tokio::test]
async fn a_run_is_watched_from_what_it_did_to_what_it_is_writing() {
    let dir = temp();
    let api = open(&dir);
    let run = agent_run(&api).await;
    let ledger = api.app().workers.sights.ledger().clone();
    let later = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        ledger.part(
            run,
            3,
            famulen_core::workers::PartOf::Said("m".into()),
            "書".into(),
        );
        ledger
            .put(
                run,
                "s",
                4,
                famulen_core::workers::Item::Said {
                    message: "m".into(),
                    text: "書いた".into(),
                },
            )
            .unwrap();
    });
    let told = watch(
        &api,
        "WorkerService/WatchRun",
        json!({ "id": run, "after": 1 }),
        4,
    )
    .await;
    later.await.unwrap();
    assert_eq!(told[0]["state"]["run"]["id"], run.to_string(), "{told:?}");
    assert!(
        told[0]["state"]["entries"].is_null(),
        "how it stands comes alone"
    );
    assert_eq!(
        told[1]["entry"]["seq"], "2",
        "what came after the one asked for"
    );
    assert_eq!(told[2]["part"]["text"], "書");
    assert_eq!(told[3]["entry"]["said"]["text"], "書いた");
}
