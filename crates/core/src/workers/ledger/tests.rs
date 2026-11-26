use rusqlite::params;

use super::*;

fn ledger() -> (Store, Ledger) {
    let store = Store::open_in_memory().unwrap();
    (store.clone(), Ledger::new(store))
}

fn said(text: &str) -> Item {
    Item::Said {
        message: "m".into(),
        text: text.into(),
    }
}

fn step(id: &str, status: &str) -> Step {
    Step {
        id: id.into(),
        title: format!("step {id}"),
        status: status.into(),
        weight: 1.0,
        ..Step::default()
    }
}

fn plan(steps: &[(&str, &str)]) -> Vec<Step> {
    steps.iter().map(|(id, status)| step(id, status)).collect()
}

fn texts(ledger: &Ledger, run: i64) -> Vec<String> {
    ledger
        .since(run, 0, 10_000)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.item {
            Item::Said { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn what_is_kept_under_a_key_is_kept_once_and_what_grew_takes_its_place() {
    let (_, ledger) = ledger();
    let first = ledger.put(1, "a", 1, said("調べ")).unwrap().unwrap();
    assert_eq!(first.seq, 1);
    assert_eq!(ledger.put(1, "a", 1, said("調べ")).unwrap(), None);
    let grown = ledger.put(1, "a", 2, said("調べる。")).unwrap().unwrap();
    assert_eq!(grown.seq, 1, "it keeps its number");
    let next = ledger.put(1, "b", 3, said("直す。")).unwrap().unwrap();
    assert_eq!(next.seq, 2);
    ledger.put(2, "a", 3, said("ほかの回")).unwrap();
    assert_eq!(texts(&ledger, 1), ["調べる。", "直す。"]);
    assert_eq!(ledger.since(1, 1, 100).unwrap().len(), 1);
    assert_eq!(ledger.last_said(1).unwrap().as_deref(), Some("直す。"));
}

#[test]
fn what_is_kept_is_told_and_what_is_being_written_is_told_alone() {
    let (_, ledger) = ledger();
    let (mut live, _) = ledger.watch(1);
    ledger.part(1, 1, PartOf::Said("m".into()), "調".into());
    ledger.part(1, 1, PartOf::Said("m".into()), String::new());
    ledger.put(1, "a", 2, said("調べる。")).unwrap();
    assert!(matches!(live.try_recv().unwrap(), Live::Part { text, .. } if text == "調"));
    assert!(matches!(live.try_recv().unwrap(), Live::Entry(e) if e.seq == 1));
    assert!(live.try_recv().is_err(), "an empty part is not told");
    assert_eq!(ledger.since(1, 0, 100).unwrap().len(), 1, "no part is kept");
}

#[test]
fn a_long_text_is_cut_and_said_to_be() {
    let (_, ledger) = ledger();
    let long = "あ".repeat(ENTRY_CHARS + 10);
    let entry = ledger.put(1, "a", 1, said(&long)).unwrap().unwrap();
    assert!(entry.cut);
    assert_eq!(texts(&ledger, 1)[0].chars().count(), ENTRY_CHARS);
    let input = serde_json::json!({ "command": "x".repeat(ENTRY_CHARS + 1), "n": 1 });
    let called = Item::Called {
        call: "c".into(),
        tool: "Bash".into(),
        input,
    };
    let entry = ledger.put(1, "b", 1, called).unwrap().unwrap();
    assert!(entry.cut);
    assert!(!ledger.put(1, "c", 1, said("短い")).unwrap().unwrap().cut);
}

#[test]
fn once_a_run_kept_all_it_may_what_comes_is_cut_short_and_said_once() {
    let (_, ledger) = ledger();
    let big = "x".repeat(ENTRY_CHARS);
    let mut n = 0;
    while ledger
        .since(1, 0, 10_000)
        .unwrap()
        .iter()
        .all(|e| e.item != Item::Full)
    {
        ledger.put(1, &n.to_string(), 1, said(&big)).unwrap();
        n += 1;
        assert!(n < 1000, "it never fills");
    }
    for key in ["late", "later"] {
        let entry = ledger.put(1, key, 1, said(&big)).unwrap().unwrap();
        assert!(entry.cut);
    }
    let all = ledger.since(1, 0, 10_000).unwrap();
    assert_eq!(all.iter().filter(|e| e.item == Item::Full).count(), 1);
    assert_eq!(
        texts(&ledger, 1).last().unwrap().chars().count(),
        FULL_CHARS
    );
    assert!(all.is_sorted_by(|a, b| a.seq < b.seq));
}

#[test]
fn the_plan_is_the_one_best_trusted_and_its_steps_tell_how_they_changed() {
    let (_, ledger) = ledger();
    assert_eq!(ledger.plan(1).unwrap(), Plan::default());
    let put = |key: &str, at, from, steps| {
        ledger
            .put(1, key, at, Item::Plan { from, steps })
            .unwrap()
            .unwrap()
            .seq
    };
    put("n1", 1, PlanFrom::Notes, plan(&[("0", "in_progress")]));
    assert_eq!(ledger.plan(1).unwrap().from, Some(PlanFrom::Notes));
    let first = put("t1", 2, PlanFrom::Timeline, plan(&[("a", "pending")]));
    put("n2", 3, PlanFrom::Notes, plan(&[("0", "completed")]));
    let second = put(
        "t2",
        4,
        PlanFrom::Timeline,
        plan(&[("a", "in_progress"), ("b", "pending")]),
    );
    let mut renamed = plan(&[("a", "completed")]);
    renamed[0].title = "改めた".into();
    renamed[0].note = "済んだ".into();
    let third = put("t3", 5, PlanFrom::Timeline, renamed.clone());

    let now = ledger.plan(1).unwrap();
    assert_eq!(now.from, Some(PlanFrom::Timeline));
    assert_eq!(now.steps, renamed);
    let status = |from: &str, to: &str| Change::Status {
        from: from.into(),
        to: to.into(),
    };
    let changes: Vec<_> = now
        .changes
        .iter()
        .map(|c| (c.seq, c.step.as_str(), c.change.clone()))
        .collect();
    assert_eq!(
        changes,
        [
            (first, "a", Change::Added),
            (second, "a", status("pending", "in_progress")),
            (second, "b", Change::Added),
            (third, "a", status("in_progress", "completed")),
            (third, "a", Change::Title),
            (third, "a", Change::Note),
            (third, "b", Change::Removed),
        ]
    );
    assert_eq!(
        ledger.plan_from(1, PlanFrom::Notes).unwrap(),
        plan(&[("0", "completed")])
    );
}

#[test]
fn a_run_that_ended_long_ago_forgets_what_it_did_but_its_plan() {
    let (store, ledger) = ledger();
    let day = 24 * 60 * 60;
    let now = 1000 * day;
    for (run, ended) in [
        (1, Some(now - KEEP_DAYS * day - 1)),
        (2, Some(now - day)),
        (3, None),
    ] {
        store
            .conn()
            .execute(
                "INSERT INTO worker_runs (id, task, runner, state, attempt, spec, created_at, ended_at)
                 VALUES (?1, 't', 'agent', ?2, 1, '{}', 0, ?3)",
                params![run, if ended.is_some() { "ended" } else { "running" }, ended],
            )
            .unwrap();
        ledger.put(run, "s", 1, said("書いた")).unwrap();
        ledger
            .put(
                run,
                "p",
                1,
                Item::Plan {
                    from: PlanFrom::Timeline,
                    steps: plan(&[("a", "completed")]),
                },
            )
            .unwrap();
    }
    assert_eq!(ledger.forget_old(now).unwrap(), 1);
    assert!(texts(&ledger, 1).is_empty());
    assert_eq!(ledger.plan(1).unwrap().steps.len(), 1);
    for run in [2, 3] {
        assert_eq!(texts(&ledger, run), ["書いた"]);
    }
}

#[test]
fn what_is_kept_outlives_the_ledger_it_was_kept_by() {
    let (store, ledger) = ledger();
    ledger.put(1, "a", 1, said("残る")).unwrap();
    drop(ledger);
    let again = Ledger::new(store);
    assert_eq!(texts(&again, 1), ["残る"]);
    assert_eq!(again.put(1, "a", 1, said("残る")).unwrap(), None);
}

#[test]
fn one_who_begins_to_watch_is_told_what_is_being_written_until_it_settles() {
    let (_, ledger) = ledger();
    ledger.part(1, 1, PartOf::Said("m".into()), "調".into());
    ledger.part(1, 1, PartOf::Said("m".into()), "べ".into());
    ledger.part(1, 1, PartOf::Output("c".into()), "ok".into());
    ledger.part(2, 1, PartOf::Said("n".into()), "ほか".into());
    let (mut live, now) = ledger.watch(1);
    let texts: Vec<_> = now
        .iter()
        .map(|l| match l {
            Live::Part { text, .. } => text.as_str(),
            Live::Entry(_) => "",
        })
        .collect();
    assert_eq!(texts, ["調べ", "ok"]);
    assert!(
        live.try_recv().is_err(),
        "nothing told before is told again"
    );

    ledger.put(1, "a", 2, said("調べる。")).unwrap();
    assert_eq!(
        ledger.watch(1).1.len(),
        1,
        "what settled is not being written"
    );
    ledger
        .put(
            1,
            "e",
            3,
            Item::Ended {
                reason: "done".into(),
            },
        )
        .unwrap();
    assert!(ledger.watch(1).1.is_empty());
    assert_eq!(ledger.watch(2).1.len(), 1);
}

#[test]
fn what_is_read_anew_after_a_restart_ends_what_was_being_written() {
    let (_, ledger) = ledger();
    ledger.put(1, "a", 1, said("調べる。")).unwrap();
    // Famulen reads the run from its start again.
    ledger.part(1, 1, PartOf::Said("m".into()), "調".into());
    assert_eq!(ledger.put(1, "a", 1, said("調べる。")).unwrap(), None);
    assert!(ledger.watch(1).1.is_empty());
}

#[test]
fn what_was_kept_whole_before_a_run_filled_up_stays_whole() {
    let (_, ledger) = ledger();
    let big = "x".repeat(ENTRY_CHARS);
    ledger.put(1, "first", 1, said(&big)).unwrap();
    let mut n = 0;
    while !ledger
        .since(1, 0, 10_000)
        .unwrap()
        .iter()
        .any(|e| e.item == Item::Full)
    {
        ledger.put(1, &n.to_string(), 1, said(&big)).unwrap();
        n += 1;
    }
    assert_eq!(ledger.put(1, "first", 1, said(&big)).unwrap(), None);
    assert_eq!(texts(&ledger, 1)[0].chars().count(), ENTRY_CHARS);
}

#[test]
fn a_run_fills_up_by_the_bytes_it_keeps() {
    let (_, ledger) = ledger();
    // Three bytes each in UTF-8.
    let big = "あ".repeat(ENTRY_CHARS);
    let mut n = 0;
    while !ledger
        .since(1, 0, 10_000)
        .unwrap()
        .iter()
        .any(|e| e.item == Item::Full)
    {
        ledger.put(1, &n.to_string(), 1, said(&big)).unwrap();
        n += 1;
    }
    // Counted by characters, it would take three times as many.
    assert!(
        n * ENTRY_CHARS * 3 <= RUN_BYTES as usize + 2 * ENTRY_CHARS * 3,
        "{n}"
    );
}

#[test]
fn the_newest_plan_of_each_source_is_read_whichever_is_followed() {
    let (_, ledger) = ledger();
    let put = |key: &str, from, steps| ledger.put(1, key, 1, Item::Plan { from, steps }).unwrap();
    put("n", PlanFrom::Notes, plan(&[("0", "pending")]));
    put("t", PlanFrom::Timeline, plan(&[("a", "pending")]));
    assert_eq!(
        ledger.plan_from(1, PlanFrom::Notes).unwrap(),
        plan(&[("0", "pending")])
    );
    assert!(ledger.plan_from(1, PlanFrom::Adroit).unwrap().is_empty());
}

#[test]
fn changes_of_the_timeline_made_at_once_each_start_from_the_other() {
    let (_, ledger) = ledger();
    std::thread::scope(|s| {
        for id in ["a", "b", "c", "d"] {
            let ledger = &ledger;
            s.spawn(move || {
                ledger
                    .change_timeline(1, |mut steps| {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        steps.push(step(id, "pending"));
                        (steps, ())
                    })
                    .unwrap();
            });
        }
    });
    assert_eq!(ledger.plan(1).unwrap().steps.len(), 4);
}

#[test]
fn one_who_watches_a_run_is_told_of_that_run_alone() {
    let (_, ledger) = ledger();
    let (mut live, _) = ledger.watch(1);
    ledger.part(2, 1, PartOf::Said("n".into()), "ほか".into());
    ledger.put(2, "a", 1, said("ほかの回")).unwrap();
    ledger.put(1, "a", 1, said("この回")).unwrap();
    assert!(matches!(live.try_recv().unwrap(), Live::Entry(e) if e.run == 1));
    assert!(live.try_recv().is_err());
}

#[test]
fn an_entry_that_cannot_be_read_keeps_its_place_and_what_follows_is_read() {
    let (store, ledger) = ledger();
    ledger.put(1, "a", 1, said("前")).unwrap();
    store
        .conn()
        .execute(
            "INSERT INTO run_events (run, seq, key, at, kind, body) VALUES (1, 2, 'x', 1, 'new', '{\"kind\":\"new\"}')",
            [],
        )
        .unwrap();
    ledger.put(1, "b", 1, said("後")).unwrap();
    let all = ledger.since(1, 0, 2).unwrap();
    assert_eq!(all.len(), 2, "a page is as long as asked");
    assert!(matches!(all[1].item, Item::Warned { .. }));
    assert_eq!(texts(&ledger, 1), ["前", "後"]);
}

#[test]
fn the_plan_followed_now_is_read_alone_from_where_it_is_best_trusted() {
    let (_, ledger) = ledger();
    assert_eq!(ledger.current(1).unwrap(), (None, Vec::new()));
    let put = |key: &str, from, steps| ledger.put(1, key, 1, Item::Plan { from, steps }).unwrap();
    put("t", PlanFrom::Timeline, plan(&[("a", "pending")]));
    put("n", PlanFrom::Notes, plan(&[("0", "pending")]));
    put("t2", PlanFrom::Timeline, plan(&[("a", "completed")]));
    put("n2", PlanFrom::Notes, plan(&[("0", "completed")]));
    assert_eq!(
        ledger.current(1).unwrap(),
        (Some(PlanFrom::Timeline), plan(&[("a", "completed")]))
    );
}

#[test]
fn what_was_kept_after_a_run_filled_up_stays_short_as_it_grows() {
    let (_, ledger) = ledger();
    let big = "x".repeat(ENTRY_CHARS);
    let mut n = 0;
    while !ledger
        .since(1, 0, 10_000)
        .unwrap()
        .iter()
        .any(|e| e.item == Item::Full)
    {
        ledger.put(1, &n.to_string(), 1, said(&big)).unwrap();
        n += 1;
    }
    ledger.put(1, "late", 1, said("短い")).unwrap();
    let grown = ledger.put(1, "late", 1, said(&big)).unwrap().unwrap();
    assert!(grown.cut);
    assert_eq!(
        texts(&ledger, 1).last().unwrap().chars().count(),
        FULL_CHARS
    );
}

#[test]
fn what_is_being_written_is_held_no_longer_than_an_entry_keeps() {
    let (_, ledger) = ledger();
    let long = "x".repeat(ENTRY_CHARS);
    ledger.part(1, 1, PartOf::Output("c".into()), long.clone());
    ledger.part(1, 1, PartOf::Output("c".into()), "end".into());
    ledger.part(1, 1, PartOf::Said("m".into()), long);
    ledger.part(1, 1, PartOf::Said("m".into()), "end".into());
    let held: Vec<_> = ledger
        .watch(1)
        .1
        .into_iter()
        .map(|l| match l {
            Live::Part { text, .. } => text,
            Live::Entry(_) => String::new(),
        })
        .collect();
    assert!(held[0].ends_with("end") && held[0].chars().count() == ENTRY_CHARS);
    assert!(!held[1].ends_with("end") && held[1].chars().count() == ENTRY_CHARS);
    ledger.forget_writing(1);
    assert!(ledger.watch(1).1.is_empty());
}
