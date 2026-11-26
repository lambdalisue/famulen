use serde_json::json;

use super::*;
use crate::store::Store;
use crate::workers::ledger::PlanFrom;

fn ops(value: Value) -> Vec<Op> {
    serde_json::from_value(value).unwrap()
}

fn shape(steps: &[Step]) -> Vec<(&str, &str, &str, f64)> {
    steps
        .iter()
        .map(|s| {
            (
                s.id.as_str(),
                s.parent.as_str(),
                s.status.as_str(),
                s.weight,
            )
        })
        .collect()
}

#[test]
fn operations_are_applied_in_order_and_what_cannot_be_is_skipped_by_name() {
    let (steps, skipped) = apply(
        Vec::new(),
        &ops(json!([
            { "op": "add", "id": "g", "title": "goal" },
            { "op": "add", "id": "a", "parent": "g", "title": "read", "weight": 3, "status": "in_progress" },
            { "op": "add", "id": "b", "parent": "g", "title": "write", "description": "the fix" },
            { "op": "update", "id": "a", "status": "completed", "note": "read it all" },
            { "op": "add", "id": "a", "title": "again" },
            { "op": "add", "id": "x", "parent": "none", "title": "lost" },
            { "op": "add", "id": "y" },
            { "op": "add", "id": "z", "title": "z", "status": "done" },
            { "op": "add", "id": "w", "title": "w", "weight": -1 },
            { "op": "update", "id": "none", "status": "completed" },
            { "op": "remove", "id": "none" },
        ])),
    );
    assert_eq!(
        shape(&steps),
        [
            ("g", "", "pending", 1.0),
            ("a", "g", "completed", 3.0),
            ("b", "g", "pending", 1.0),
        ]
    );
    assert_eq!(
        (steps[1].note.as_str(), steps[2].description.as_str()),
        ("read it all", "the fix")
    );
    let named: Vec<_> = skipped
        .iter()
        .map(|s| s.split(':').next().unwrap())
        .collect();
    assert_eq!(
        named,
        [
            "add a",
            "add x",
            "add y",
            "add z",
            "add w",
            "update none",
            "remove none"
        ]
    );
}

#[test]
fn a_step_goes_with_the_steps_under_it() {
    let (steps, _) = apply(
        Vec::new(),
        &ops(json!([
            { "op": "add", "id": "g", "title": "goal" },
            { "op": "add", "id": "a", "parent": "g", "title": "a" },
            { "op": "add", "id": "a1", "parent": "a", "title": "a1" },
            { "op": "add", "id": "b", "title": "b" },
            { "op": "remove", "id": "g" },
        ])),
    );
    assert_eq!(shape(&steps), [("b", "", "pending", 1.0)]);
}

#[test]
fn what_is_read_back_is_the_tree_with_how_much_is_done() {
    let (steps, _) = apply(
        Vec::new(),
        &ops(json!([
            { "op": "add", "id": "g", "title": "goal", "status": "in_progress" },
            { "op": "add", "id": "a", "parent": "g", "title": "read", "status": "completed", "note": "ok" },
            { "op": "add", "id": "b", "parent": "g", "title": "write" },
        ])),
    );
    assert_eq!(
        render(&steps),
        "50% completed\n- [>] g: goal (w=1)\n  - [x] a: read (w=1) — ok\n  - [ ] b: write (w=1)\n"
    );
    assert_eq!(render(&[]), "The timeline is empty.");
}

async fn call(hub: &Hub, name: &str, arguments: Value) -> Value {
    let message = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    });
    hub.handle(&message).await.unwrap()["result"].clone()
}

#[tokio::test]
async fn a_worker_keeps_its_plan_in_the_ledger_of_its_own_run() {
    let ledger = Ledger::new(Store::open_in_memory().unwrap());
    let told = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hub = hub(ledger.clone(), 7, {
        let told = told.clone();
        Arc::new(move || {
            told.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        })
    });
    let listed = hub
        .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))
        .await
        .unwrap();
    assert_eq!(listed["result"]["tools"][0]["name"], TOOL);

    let add =
        json!({ "ops": [{ "op": "add", "id": "g", "title": "goal" }, { "op": "bad", "id": "q" }] });
    let answer = call(&hub, TOOL, add).await;
    let text = answer["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("[ ] g: goal") && text.contains("Skipped"),
        "{text}"
    );
    let update = json!({ "ops": [{ "op": "update", "id": "g", "status": "completed" }] });
    call(&hub, TOOL, update.clone()).await;
    // What changes nothing keeps nothing.
    call(&hub, TOOL, update).await;

    let plan = ledger.plan(7).unwrap();
    assert_eq!(plan.from, Some(PlanFrom::Timeline));
    assert_eq!(shape(&plan.steps), [("g", "", "completed", 1.0)]);
    assert_eq!(ledger.since(7, 0, 100).unwrap().len(), 2);
    assert!(ledger.plan(8).unwrap().steps.is_empty(), "no other run's");
    assert_eq!(
        told.load(std::sync::atomic::Ordering::Relaxed),
        2,
        "told of each change alone"
    );

    let other = call(&hub, "task_list", json!({})).await;
    assert_eq!(other["isError"], true);
}
