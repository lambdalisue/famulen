use serde_json::json;

use super::*;
use crate::connections::Connections;
use crate::permissions::BUILTINS;
use crate::store::{Store, now};

fn gateway() -> Gateway {
    let store = Store::open_in_memory().unwrap();
    let gateway = Gateway::new(
        store.clone(),
        Connections::new(store),
        reqwest::Client::new(),
        None,
    );
    gateway.grant("secretary", ANY, ANY).unwrap();
    gateway
}

fn call(tool: &str) -> Call {
    Call::new("secretary", "mail", tool, json!({ "to": "a@example.com" }))
}

fn is_allow(v: &Verdict) -> bool {
    matches!(v, Verdict::Allow { .. })
}

#[test]
fn a_reading_tool_runs_and_a_writing_tool_is_held() {
    let g = gateway();
    assert!(is_allow(&g.judge(&call("search_threads")).unwrap()));
    assert!(matches!(
        g.judge(&call("send_message")).unwrap(),
        Verdict::Hold(_)
    ));
}

#[test]
fn a_forbidden_tool_is_refused_even_for_the_owner() {
    let g = gateway();
    g.permissions()
        .set_scoped("conn:mail", "send_message", Some(Permission::Forbid))
        .unwrap();
    let mut owner = call("send_message");
    owner.actor = OWNER.into();
    assert!(matches!(
        g.judge(&call("send_message")).unwrap(),
        Verdict::Deny { .. }
    ));
    assert!(matches!(g.judge(&owner).unwrap(), Verdict::Deny { .. }));
}

#[test]
fn a_call_its_caller_refuses_is_denied_with_that_reason_and_logged() {
    let g = gateway();
    let refused = Call {
        refuse: Some("Read reaches the gateway's own data".into()),
        ..call("search_threads")
    };

    let verdict = g.judge(&refused).unwrap();

    assert_eq!(
        verdict,
        Verdict::Deny {
            reason: "Read reaches the gateway's own data".into()
        }
    );
    let entry = &g.log_entries(1).unwrap()[0];
    assert_eq!(
        (entry.decision.as_str(), entry.reason.as_str()),
        ("deny", "Read reaches the gateway's own data")
    );
}

#[test]
fn the_owner_is_not_asked() {
    let g = gateway();
    let mut owner = call("send_message");
    owner.actor = OWNER.into();
    assert!(is_allow(&g.judge(&owner).unwrap()));
}

#[test]
fn a_caller_without_grants_calls_nothing() {
    let g = gateway();
    let mut lookout = call("search_threads");
    lookout.actor = "lookout".into();
    assert!(matches!(g.judge(&lookout).unwrap(), Verdict::Deny { .. }));
    g.grant("lookout", "mail", "search_threads").unwrap();
    assert!(is_allow(&g.judge(&lookout).unwrap()));
    assert!(matches!(
        g.judge(&Call {
            tool: "get_thread".into(),
            ..lookout
        })
        .unwrap(),
        Verdict::Deny { .. }
    ));
}

#[test]
fn a_grant_is_not_wider_than_the_permissions() {
    let g = gateway();
    g.grant("worker", "mail", ANY).unwrap();
    let mut worker = call("send_message");
    worker.actor = "worker".into();
    assert!(matches!(g.judge(&worker).unwrap(), Verdict::Hold(_)));
}

#[test]
fn an_allowed_tool_outside_the_workgroup_is_asked() {
    let g = gateway();
    let asked = Call {
        ask: Some("outside the workgroup".into()),
        ..call("search_threads")
    };
    let Verdict::Hold(ticket) = g.judge(&asked).unwrap() else {
        panic!("should be held");
    };
    assert_eq!(ticket.reason, "outside the workgroup");
}

#[tokio::test]
async fn approval_lets_the_same_call_by_the_same_caller_through_once() {
    let g = gateway();
    let Verdict::Hold(ticket) = g.judge(&call("send_message")).unwrap() else {
        panic!("should be held");
    };
    let answered = g.answer_ticket(ticket.id, Answer::Approve).await.unwrap();
    assert!(answered.is_some());
    assert!(
        g.answer_ticket(ticket.id, Answer::Approve)
            .await
            .unwrap()
            .is_none()
    );

    let mut other_caller = call("send_message");
    other_caller.actor = "worker".into();
    g.grant("worker", ANY, ANY).unwrap();
    assert!(matches!(g.judge(&other_caller).unwrap(), Verdict::Hold(_)));
    let other_input = Call {
        input: json!({ "to": "b@example.com" }),
        ..call("send_message")
    };
    assert!(matches!(g.judge(&other_input).unwrap(), Verdict::Hold(_)));

    assert!(is_allow(&g.judge(&call("send_message")).unwrap()));
    assert!(matches!(
        g.judge(&call("send_message")).unwrap(),
        Verdict::Hold(_)
    ));
}

#[tokio::test]
async fn always_allow_lets_the_tool_run_from_then_on() {
    let g = gateway();
    let Verdict::Hold(ticket) = g.judge(&call("send_message")).unwrap() else {
        panic!("should be held");
    };
    g.answer_ticket(ticket.id, Answer::AlwaysAllow)
        .await
        .unwrap();
    let other = Call {
        input: json!({ "to": "c@example.com" }),
        ..call("send_message")
    };
    assert!(is_allow(&g.judge(&other).unwrap()));
}

fn builtin(tool: &str) -> Call {
    Call {
        marks: Some(Marks {
            read_only: Some(true),
            ..Marks::default()
        }),
        ..Call::new("secretary", "", tool, json!({ "command": "ls" }))
    }
}

#[test]
fn a_builtin_tool_follows_the_owners_permission_over_its_marks() {
    let g = gateway();
    assert!(is_allow(&g.judge(&builtin("Bash")).unwrap()));
    g.set_permission(OWNER, BUILTINS, "Bash", Some(Permission::Forbid))
        .unwrap();
    assert!(matches!(
        g.judge(&builtin("Bash")).unwrap(),
        Verdict::Deny { .. }
    ));
    assert_eq!(
        g.permissions().scoped(BUILTINS, "Bash").unwrap(),
        Some(Permission::Forbid)
    );
}

#[tokio::test]
async fn always_allow_on_a_builtin_tool_sets_the_builtins_permission() {
    let g = gateway();
    let held = Call {
        marks: Some(Marks::default()),
        ..builtin("Write")
    };
    let Verdict::Hold(ticket) = g.judge(&held).unwrap() else {
        panic!("should be held");
    };
    g.answer_ticket(ticket.id, Answer::AlwaysAllow)
        .await
        .unwrap();
    assert_eq!(
        g.permissions().scoped(BUILTINS, "Write").unwrap(),
        Some(Permission::Allow)
    );
}

#[tokio::test]
async fn a_rejected_call_stays_held_next_time() {
    let g = gateway();
    let Verdict::Hold(ticket) = g.judge(&call("send_message")).unwrap() else {
        panic!("should be held");
    };
    g.answer_ticket(ticket.id, Answer::Reject).await.unwrap();
    assert!(matches!(
        g.judge(&call("send_message")).unwrap(),
        Verdict::Hold(_)
    ));
}

#[test]
fn judgements_are_logged_with_the_caller() {
    let g = gateway();
    g.judge(&call("send_message")).unwrap();
    g.judge(&call("search_threads")).unwrap();
    let entries = g.log().recent(10).unwrap();
    assert_eq!(
        entries.len(),
        1,
        "an allowed call is logged by who makes it"
    );
    assert_eq!(entries[0].actor, "secretary");
    assert_eq!(entries[0].decision, "confirm");
    assert!(entries[0].reason.starts_with("held as #"));
}

#[test]
fn held_tickets_are_told_to_subscribers() {
    let g = gateway();
    let mut held = g.subscribe_held();
    g.judge(&call("send_message")).unwrap();
    assert_eq!(held.try_recv().unwrap().tool, "send_message");
    assert_eq!(g.held_tickets().unwrap().len(), 1);
}

#[test]
fn a_ticket_keeps_what_its_call_was_made_for() {
    let g = gateway();
    let made_for = Call {
        origin: Some("7".into()),
        ..call("send_message")
    };
    let Verdict::Hold(ticket) = g.judge(&made_for).unwrap() else {
        panic!("should be held");
    };
    assert_eq!(ticket.origin.as_deref(), Some("7"));
    assert_eq!(g.ticket(ticket.id).unwrap(), Some(ticket));
    g.judge(&call("send_message")).unwrap();
    assert_eq!(g.held_tickets().unwrap()[1].origin, None);
}

#[test]
fn a_ticket_that_waited_too_long_lapses() {
    let g = gateway();
    let Verdict::Hold(ticket) = g.judge(&call("send_message")).unwrap() else {
        panic!("should be held");
    };
    g.store
        .conn()
        .execute(
            "UPDATE tickets SET created_at = ?1 WHERE id = ?2",
            params![now() - TICKET_LIFETIME_SECS - 1, ticket.id],
        )
        .unwrap();
    assert_eq!(g.ticket(ticket.id).unwrap().unwrap().status, "lapsed");
    assert!(g.held_tickets().unwrap().is_empty());
}

/// An assessor that answers the same each time and counts what it weighed.
struct Fixed {
    answer: anyhow::Result<Assessment, String>,
    weighed: std::sync::Mutex<Vec<Value>>,
}

impl Assessor for Fixed {
    fn assess<'a>(&'a self, call: &'a Call) -> BoxFuture<'a, anyhow::Result<Assessment>> {
        self.weighed.lock().unwrap().push(call.input.clone());
        let answer = self.answer.clone().map_err(anyhow::Error::msg);
        Box::pin(async move { answer })
    }
}

fn assessed_by(g: &Gateway, answer: anyhow::Result<Assessment, String>) -> Arc<Fixed> {
    let assessor = Arc::new(Fixed {
        answer,
        weighed: Default::default(),
    });
    g.set_assessor(assessor.clone());
    assessor
}

fn lets_it_run() -> anyhow::Result<Assessment, String> {
    Ok(Assessment::Allow {
        reason: "as the owner asked".into(),
    })
}

#[tokio::test]
async fn auto_runs_a_reading_tool_without_asking_the_assessor() {
    let g = gateway();
    let assessor = assessed_by(&g, lets_it_run());

    assert!(is_allow(
        &g.judge_assessed(&call("search_threads")).await.unwrap()
    ));
    assert!(assessor.weighed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn auto_runs_a_writing_tool_the_assessor_lets_run_and_logs_why() {
    let g = gateway();
    let assessor = assessed_by(&g, lets_it_run());

    let verdict = g.judge_assessed(&call("create_draft")).await.unwrap();

    let Verdict::Allow { reason } = verdict else {
        panic!("should run: {verdict:?}");
    };
    assert!(reason.contains("as the owner asked"), "{reason}");
    assert_eq!(assessor.weighed.lock().unwrap().len(), 1);
    let entry = &g.log_entries(1).unwrap()[0];
    assert_eq!(
        (
            entry.actor.as_str(),
            entry.decision.as_str(),
            entry.tool.as_str()
        ),
        (ASSESSOR, "allow", "create_draft")
    );
    assert!(entry.reason.contains("as the owner asked"));
}

#[tokio::test]
async fn auto_holds_a_writing_tool_the_assessor_asks_about_with_its_reason() {
    let g = gateway();
    assessed_by(
        &g,
        Ok(Assessment::Ask {
            reason: "not what the owner asked".into(),
        }),
    );

    let Verdict::Hold(ticket) = g.judge_assessed(&call("create_draft")).await.unwrap() else {
        panic!("should be held");
    };
    assert!(
        ticket.reason.contains("not what the owner asked"),
        "{}",
        ticket.reason
    );
}

#[tokio::test]
async fn auto_holds_a_call_beyond_undoing_even_when_the_assessor_would_let_it_run() {
    let g = gateway();
    let assessor = assessed_by(&g, lets_it_run());

    for tool in ["send_message", "delete_thread"] {
        assert!(
            matches!(
                g.judge_assessed(&call(tool)).await.unwrap(),
                Verdict::Hold(_)
            ),
            "{tool}"
        );
    }
    let marked = Call {
        marks: Some(Marks {
            read_only: Some(false),
            always_ask: true,
        }),
        ..call("create_draft")
    };
    assert!(matches!(
        g.judge_assessed(&marked).await.unwrap(),
        Verdict::Hold(_)
    ));
    assert!(assessor.weighed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn auto_holds_a_writing_tool_when_no_assessor_answers() {
    let g = gateway();
    let Verdict::Hold(ticket) = g.judge_assessed(&call("create_draft")).await.unwrap() else {
        panic!("should be held without an assessor");
    };
    assert!(ticket.reason.contains("no assessor"), "{}", ticket.reason);

    assessed_by(&g, Err("over its cap".into()));
    let Verdict::Hold(ticket) = g.judge_assessed(&call("create_draft")).await.unwrap() else {
        panic!("should be held when the assessor fails");
    };
    assert!(ticket.reason.contains("over its cap"), "{}", ticket.reason);
}

#[tokio::test]
async fn the_owners_own_choice_comes_before_auto() {
    let g = gateway();
    let assessor = assessed_by(
        &g,
        Ok(Assessment::Ask {
            reason: "unsure".into(),
        }),
    );
    g.set_permission(OWNER, "conn:mail", "send_message", Some(Permission::Allow))
        .unwrap();
    g.set_permission(
        OWNER,
        "conn:mail",
        "search_threads",
        Some(Permission::Confirm),
    )
    .unwrap();

    assert!(is_allow(
        &g.judge_assessed(&call("send_message")).await.unwrap()
    ));
    assert!(matches!(
        g.judge_assessed(&call("search_threads")).await.unwrap(),
        Verdict::Hold(_)
    ));
    assert!(assessor.weighed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_assessor_does_not_see_a_secret_field() {
    let g = gateway();
    let assessor = assessed_by(&g, lets_it_run());
    let secret = Call {
        secret_fields: vec!["to".into()],
        ..call("create_draft")
    };

    g.judge_assessed(&secret).await.unwrap();

    assert_eq!(assessor.weighed.lock().unwrap()[0], json!({ "to": "…" }));
}

#[tokio::test]
async fn a_caller_that_cannot_ask_is_refused_where_the_owner_would_be_asked() {
    let g = gateway();
    assessed_by(&g, lets_it_run());
    let unasked = |tool: &str| Call {
        never_ask: true,
        ..call(tool)
    };

    assert!(is_allow(
        &g.judge_assessed(&unasked("search_threads")).await.unwrap()
    ));
    assert!(is_allow(
        &g.judge_assessed(&unasked("create_draft")).await.unwrap()
    ));
    let Verdict::Deny { reason } = g.judge_assessed(&unasked("send_message")).await.unwrap() else {
        panic!("should be refused");
    };
    assert!(reason.starts_with(NEEDS_OWNER), "{reason}");
    assert!(g.held_tickets().unwrap().is_empty());
}

#[tokio::test]
async fn a_caller_that_cannot_ask_does_not_use_up_an_approval() {
    let g = gateway();
    let Verdict::Hold(ticket) = g.judge(&call("send_message")).unwrap() else {
        panic!("should be held");
    };
    g.answer_ticket(ticket.id, Answer::Approve).await.unwrap();

    let unasked = Call {
        never_ask: true,
        ..call("send_message")
    };
    assert!(matches!(g.judge(&unasked).unwrap(), Verdict::Deny { .. }));
    assert!(is_allow(&g.judge(&call("send_message")).unwrap()));
}

#[tokio::test]
async fn a_withdrawn_ticket_can_be_answered_no_more_and_is_found_by_its_call() {
    let g = gateway();
    let Verdict::Hold(ticket) = g.judge(&call("send_message")).unwrap() else {
        panic!("should be held");
    };
    assert_eq!(
        g.ticket_for(&call("send_message")).unwrap().unwrap().id,
        ticket.id
    );
    assert!(g.ticket_for(&call("create_draft")).unwrap().is_none());

    assert!(g.withdraw_ticket(ticket.id).unwrap());

    assert!(!g.withdraw_ticket(ticket.id).unwrap());
    assert!(
        g.answer_ticket(ticket.id, Answer::Approve)
            .await
            .unwrap()
            .is_none()
    );
}
