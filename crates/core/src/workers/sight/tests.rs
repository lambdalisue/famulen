use serde_json::json;

use super::*;
use crate::agents::Adapter as _;
use crate::agents::claude::Claude;
use crate::workers::ledger::Live;

fn sights() -> Sights {
    Sights::new(Store::open_in_memory().unwrap())
}

fn session(reply: &str, steps: &[(&str, &str, &str, f64)]) -> Value {
    json!({
        "timeline": steps
            .iter()
            .map(|(id, parent, status, weight)| json!({
                "id": id, "title": format!("step {id}"), "status": status, "weight": weight,
                "parent": (!parent.is_empty()).then_some(parent), "note": null,
            }))
            .collect::<Vec<_>>(),
        "turns": [{ "turn": 1, "prompt": "直して", "reply": reply, "began_ms": 1, "last_ms": 2 }],
    })
}

fn kinds_and_texts(sights: &Sights, run: i64) -> Vec<(&'static str, String)> {
    sights
        .ledger()
        .since(run, 0, 100)
        .unwrap()
        .into_iter()
        .map(|e| {
            let text = match &e.item {
                Item::Asked { text } | Item::Said { text, .. } => text.clone(),
                Item::Called { tool, .. } => tool.clone(),
                Item::Answered { output, .. } => output.clone(),
                Item::Question { what, .. } => what.clone(),
                Item::Decided { allowed, .. } => allowed.to_string(),
                _ => String::new(),
            };
            (e.item.kind(), text)
        })
        .collect()
}

#[test]
fn a_plan_is_done_by_the_weight_of_its_steps() {
    for (steps, done) in [
        (&[][..], None),
        (&[("a", "", "pending", 1.0)][..], Some(0.0)),
        (
            &[
                ("g", "", "in_progress", 1.0),
                ("a", "g", "completed", 3.0),
                ("b", "g", "in_progress", 1.0),
            ][..],
            Some(0.75),
        ),
        (
            &[("g", "", "completed", 1.0), ("a", "g", "pending", 1.0)][..],
            Some(1.0),
        ),
        (
            &[("a", "", "completed", 1.0), ("b", "", "blocked", 1.0)][..],
            Some(0.5),
        ),
        (&[("a", "", "pending", 0.0)][..], Some(0.0)),
        // What goes in circles has no place in the plan.
        (
            &[("a", "b", "pending", 1.0), ("b", "a", "pending", 1.0)][..],
            None,
        ),
    ] {
        let sights = sights();
        sights.read(1, &session("", steps));
        assert_eq!(sights.of(1).done(), done, "{steps:?}");
    }
}

#[test]
fn what_a_run_does_is_its_innermost_step_or_what_it_wrote_last() {
    let sights = sights();
    sights.read(1, &session("\n調べる。\n続き", &[]));
    assert_eq!(sights.of(1).doing(), "調べる。");
    let steps = [
        ("g", "", "in_progress", 1.0),
        ("a", "g", "in_progress", 1.0),
        ("b", "g", "pending", 1.0),
    ];
    sights.read(1, &session("調べる。", &steps));
    let sight = sights.of(1);
    assert_eq!(sight.doing(), "step a");
    assert_eq!(sight.from, Some(PlanFrom::Adroit));
}

#[test]
fn what_a_session_wrote_is_kept_as_it_grows_and_beside_it_when_written_anew() {
    let sights = sights();
    for reply in ["", "調べ", "調べる。", "調べる。", "直す。"] {
        sights.read(1, &session(reply, &[]));
    }
    let mut next = session("戻した。", &[]);
    next["turns"][0]["turn"] = json!(2);
    sights.read(1, &next);
    assert_eq!(
        kinds_and_texts(&sights, 1),
        [
            ("asked", "直して".into()),
            ("said", "調べる。".into()),
            ("said", "直す。".into()),
            ("asked", "直して".into()),
            ("said", "戻した。".into()),
        ]
    );
}

#[test]
fn a_sessions_plan_is_kept_only_when_it_changes() {
    let sights = sights();
    let steps = [("a", "", "pending", 1.0)];
    sights.read(1, &session("", &steps));
    sights.read(1, &session("", &steps));
    sights.read(1, &session("", &[("a", "", "completed", 1.0)]));
    let plans = kinds_and_texts(&sights, 1)
        .into_iter()
        .filter(|(kind, _)| *kind == "plan")
        .count();
    assert_eq!(plans, 2);
    assert_eq!(sights.ledger().plan(1).unwrap().changes.len(), 2);
}

#[test]
fn what_a_session_asks_is_kept_once_and_how_it_was_answered() {
    let sights = sights();
    let mut waiting = session("", &[]);
    waiting["state"] = json!("waiting");
    waiting["question"] = json!({
        "id": "q1", "kind": "question", "tool": "AskUserQuestion",
        "input": { "questions": [{ "question": "閉じますか？" }] },
    });
    sights.read(1, &waiting);
    sights.read(1, &waiting);
    assert!(sights.of(1).question.is_some());
    sights.answered(1, "q1", true, "閉じる");
    sights.read(1, &waiting);
    assert_eq!(sights.of(1).question, None, "read before it was answered");
    let asked: Vec<_> = kinds_and_texts(&sights, 1)
        .into_iter()
        .filter(|(kind, _)| matches!(*kind, "question" | "decided"))
        .collect();
    assert_eq!(
        asked,
        [
            ("question", "閉じますか？".into()),
            ("decided", "true".into())
        ]
    );
}

#[test]
fn an_agents_stream_is_kept_as_its_plan_and_what_it_wrote_and_called() {
    let sights = sights();
    let (mut live, _) = sights.ledger().watch(1);
    for (seq, (kind, block)) in (1..).zip([
        ("assistant", json!({ "type": "text", "text": "読む。" })),
        (
            "assistant",
            json!({ "type": "tool_use", "id": "c1", "name": "Read", "input": { "file_path": "/a" } }),
        ),
        (
            "user",
            json!({ "type": "tool_result", "tool_use_id": "c1", "content": [{ "type": "text", "text": "中身" }] }),
        ),
        (
            "assistant",
            json!({ "type": "tool_use", "name": "TodoWrite", "input": { "todos": [
                { "content": "読む", "status": "completed" }, { "content": "書く", "status": "in_progress" },
            ] } }),
        ),
    ]) {
        let plans = block["name"] == "TodoWrite";
        let line = json!({ "type": kind, "message": { "content": [block] } }).to_string();
        let changed = (0..)
            .zip(Claude.codec().read(&line))
            .map(|(part, body)| {
                let event = Event {
                    seq,
                    part,
                    at: 1,
                    body,
                };
                sights.event(1, 0, &event, true)
            })
            .fold(false, |a, b| a | b);
        assert_eq!(changed, plans, "only a plan is told at once");
    }
    let sight = sights.of(1);
    assert_eq!(
        kinds_and_texts(&sights, 1),
        [
            ("said", "読む。".into()),
            ("called", "Read".into()),
            ("answered", "中身".into()),
            ("plan", String::new()),
        ]
    );
    assert_eq!(sight.from, Some(PlanFrom::Notes));
    assert_eq!((sight.done(), sight.doing()), (Some(0.5), "書く".into()));
    assert!(matches!(live.try_recv(), Ok(Live::Entry(_))));
}

#[test]
fn what_an_agent_is_writing_is_told_and_not_kept() {
    let sights = sights();
    let (mut live, _) = sights.ledger().watch(1);
    let event = |body| Event {
        seq: 1,
        part: 0,
        at: 1,
        body,
    };
    sights.event(
        1,
        0,
        &event(Body::SaidMore {
            message: "m".into(),
            text: "読".into(),
        }),
        true,
    );
    sights.event(
        1,
        0,
        &event(Body::ToolWorks {
            call: "c".into(),
            output: "1 passed".into(),
            seconds: None,
        }),
        true,
    );
    // Read again after a restart, it is told no more.
    sights.event(
        1,
        0,
        &event(Body::SaidMore {
            message: "m".into(),
            text: "前".into(),
        }),
        false,
    );
    assert!(matches!(live.try_recv(), Ok(Live::Part { of: PartOf::Said(m), .. }) if m == "m"));
    assert!(matches!(live.try_recv(), Ok(Live::Part { of: PartOf::Output(c), .. }) if c == "c"));
    assert!(live.try_recv().is_err());
    assert!(sights.ledger().since(1, 0, 10).unwrap().is_empty());
}

#[test]
fn what_an_agent_used_is_kept_once_for_each_message_as_it_grows() {
    let sights = sights();
    for (seq, output) in [(1, 5), (2, 9)] {
        sights.event(
            1,
            0,
            &Event {
                seq,
                part: 0,
                at: 1,
                body: Body::Used {
                    of: Counted::Message("m".into()),
                    model: None,
                    input: 10,
                    cached: 0,
                    output,
                },
            },
            true,
        );
    }
    let used: Vec<_> = sights
        .ledger()
        .since(1, 0, 10)
        .unwrap()
        .into_iter()
        .map(|e| e.item)
        .collect();
    assert!(
        matches!(used[..], [Item::Used { output: 9, .. }]),
        "{used:?}"
    );
}

#[test]
fn a_change_of_what_stands_in_a_list_is_told_once() {
    let sights = sights();
    sights.read(1, &session("a", &[]));
    assert!(sights.take_changed());
    assert!(!sights.take_changed());
    sights.read(1, &session("a", &[]));
    assert!(!sights.take_changed(), "nothing changed");
}

#[test]
fn a_run_is_read_no_more_often_than_asked() {
    let sights = sights();
    let long = Duration::from_secs(600);
    assert!(sights.due(1, long));
    assert!(!sights.due(1, long));
    assert!(sights.due(1, Duration::ZERO));
    assert!(sights.due(2, long));
}

#[test]
fn why_a_run_could_not_be_read_stays_until_it_is() {
    let sights = sights();
    sights.troubled(1, "Adroit is away".into());
    assert_eq!(sights.trouble(1), "Adroit is away");
    assert_eq!(sights.trouble(2), "");
    sights.troubled(1, String::new());
    assert_eq!(sights.trouble(1), "");
}

#[test]
fn what_an_agent_was_writing_when_its_process_ended_is_written_no_more() {
    let sights = sights();
    sights
        .ledger()
        .part(1, 1, PartOf::Said("m".into()), "途中".into());
    sights.ended(1, 0, 2, "stopped".into());
    assert!(sights.ledger().watch(1).1.is_empty());
    let kept = sights.ledger().since(1, 0, 10).unwrap();
    assert!(matches!(&kept[..], [e] if e.item == Item::Ended { reason: "stopped".into() }));
}

#[test]
fn a_question_waited_for_no_more_without_an_answer_here_is_kept_as_such() {
    let sights = sights();
    let mut waiting = session("", &[]);
    waiting["state"] = json!("waiting");
    waiting["question"] =
        json!({ "id": "q1", "kind": "question", "tool": "AskUserQuestion", "input": {} });
    sights.read(1, &waiting);
    sights.read(1, &session("", &[]));
    let told: Vec<_> = kinds_and_texts(&sights, 1)
        .into_iter()
        .map(|(kind, _)| kind)
        .filter(|kind| matches!(*kind, "question" | "withdrawn" | "decided"))
        .collect();
    assert_eq!(told, ["question", "withdrawn"]);
}

#[test]
fn a_question_asked_again_is_kept_again() {
    let sights = sights();
    let mut waiting = session("", &[]);
    waiting["state"] = json!("waiting");
    waiting["question"] =
        json!({ "id": "q1", "kind": "question", "tool": "AskUserQuestion", "input": {} });
    sights.read(1, &waiting);
    sights.read(1, &session("", &[]));
    sights.read(1, &waiting);
    let kinds: Vec<_> = kinds_and_texts(&sights, 1)
        .into_iter()
        .map(|(kind, _)| kind)
        .filter(|kind| matches!(*kind, "question" | "withdrawn"))
        .collect();
    assert_eq!(kinds, ["question", "withdrawn", "question"]);
}

#[test]
fn what_was_forgotten_for_its_age_is_not_kept_again_from_adroit() {
    let store = Store::open_in_memory().unwrap();
    let sights = Sights::new(store.clone());
    store
        .conn()
        .execute(
            "INSERT INTO worker_runs (id, task, runner, state, attempt, spec, created_at, ended_at)
             VALUES (1, 't', 'adroit', 'ended', 1, '{}', 0, 0)",
            [],
        )
        .unwrap();
    sights.read(1, &session("書いた", &[("a", "", "completed", 1.0)]));
    sights.ledger().forget_old(100 * 24 * 60 * 60).unwrap();
    sights.read(1, &session("書いた", &[("a", "", "completed", 1.0)]));
    let kinds: Vec<_> = kinds_and_texts(&sights, 1)
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(kinds, ["plan", "forgotten"]);
}

#[test]
fn what_an_agent_did_in_a_process_started_anew_is_kept_beside_the_first() {
    let sights = sights();
    let said = |text: &str| Event {
        seq: 1,
        part: 0,
        at: 1,
        body: Body::Said {
            message: "m".into(),
            text: text.into(),
        },
    };
    sights.event(1, 10, &said("一度目"), true);
    sights.event(1, 20, &said("一度目"), true);
    assert_eq!(sights.said(1), ["一度目", "一度目"]);
}
