//! Each agent's log read through the API a page at a time, newest first,
//! with `before` naming the oldest row already shown.

use famulen_core::lookout::Item;
use serde_json::{Value, json};

use crate::support::{call, open, temp};

fn id(row: &Value) -> i64 {
    row["id"].as_str().unwrap().parse().unwrap()
}

fn no_more(page: &Value) -> bool {
    page.get("more").is_none_or(|m| m == false)
}

#[tokio::test]
async fn the_secretarys_log_reads_older_turns_from_before_the_last_one_shown() {
    let dir = temp();
    let api = open(&dir);
    for n in 1..=3 {
        api.app()
            .journal
            .record("input", format!("[console] turn {n}"));
        api.app()
            .journal
            .record("tool", "mcp__famulen__list_records {}");
        api.app().journal.record("text", format!("reply {n}"));
        api.app().journal.record("dream", format!("night {n}"));
    }

    let first = call(&api, "LogService/ListLog", json!({ "limit": 4 })).await;
    let items = first["items"].as_array().unwrap();
    assert_eq!(items.len(), 4, "{first}");
    assert_eq!(first["more"], true);
    assert_eq!(items[0]["event"]["detail"], "night 3");
    assert_eq!(items[1]["turn"]["reply"], "reply 3");
    assert_eq!(items[3]["turn"]["reply"], "reply 2");

    let rest = call(
        &api,
        "LogService/ListLog",
        json!({ "limit": 4, "before": id(&items[3]) }),
    )
    .await;
    let older = rest["items"].as_array().unwrap();
    assert_eq!(older.len(), 2, "{rest}");
    assert!(no_more(&rest), "{rest}");
    assert_eq!(older[0]["event"]["detail"], "night 1");
    assert_eq!(older[1]["turn"]["reply"], "reply 1");
    assert_eq!(
        older[1]["turn"]["steps"].as_array().unwrap().len(),
        1,
        "a turn on the next page keeps its steps: {rest}"
    );

    let talk = call(
        &api,
        "LogService/ListLog",
        json!({ "show": "talk", "limit": 2 }),
    )
    .await;
    let turns = talk["items"].as_array().unwrap();
    assert_eq!(turns.len(), 2, "{talk}");
    assert_eq!(talk["more"], true);
    let last = call(
        &api,
        "LogService/ListLog",
        json!({ "show": "talk", "limit": 2, "before": id(&turns[1]) }),
    )
    .await;
    assert_eq!(last["items"].as_array().unwrap().len(), 1, "{last}");
    assert!(no_more(&last));
}

#[tokio::test]
async fn triage_tells_what_it_decided_of_each_message_a_page_at_a_time() {
    let dir = temp();
    let api = open(&dir);
    for (title, said) in [("a", true), ("b", false), ("c", true)] {
        let item = Item {
            title: title.into(),
            body: format!("{title} body"),
        };
        api.app()
            .triage
            .weigh("slack", &item, |_| {
                std::future::ready(anyhow::Ok(Some(said)))
            })
            .await
            .unwrap();
    }

    let first = call(&api, "LogService/ListTriageLog", json!({ "limit": 2 })).await;
    let decisions = first["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 2, "{first}");
    assert_eq!(first["more"], true);
    assert_eq!(decisions[0]["title"], "c");
    assert_eq!(decisions[0]["addressed"], true);
    assert!(decisions[1].get("addressed").is_none_or(|a| a == false));
    assert_eq!(decisions[1]["reason"], "秘書宛てではない");
    assert!(decisions[0]["at"].as_str().is_some());

    let rest = call(
        &api,
        "LogService/ListTriageLog",
        json!({ "limit": 2, "before": id(&decisions[1]) }),
    )
    .await;
    let older = rest["decisions"].as_array().unwrap();
    assert_eq!(older.len(), 1, "{rest}");
    assert_eq!(older[0]["title"], "a");
    assert!(no_more(&rest));
}

#[tokio::test]
async fn the_dreams_come_newest_first_a_page_at_a_time() {
    let dir = temp();
    let api = open(&dir);
    for n in 1..=3 {
        famulen_core::dreaming::dream(
            &api.app().store,
            &api.app().records,
            &api.app().memory_dir,
            chrono::NaiveDate::from_ymd_opt(2026, 11, n).unwrap(),
            move |_, _| async move { anyhow::bail!("night {n}") },
        )
        .await
        .unwrap();
    }

    let first = call(&api, "LogService/ListDreamLog", json!({ "limit": 2 })).await;
    let dreams = first["dreams"].as_array().unwrap();
    assert_eq!(dreams.len(), 2, "{first}");
    assert_eq!(first["more"], true);
    assert_eq!(dreams[0]["status"], "見られなかった（night 3）");
    let rest = call(
        &api,
        "LogService/ListDreamLog",
        json!({ "limit": 2, "before": id(&dreams[1]) }),
    )
    .await;
    assert_eq!(
        rest["dreams"][0]["status"], "見られなかった（night 1）",
        "{rest}"
    );
    assert!(no_more(&rest));
}

#[tokio::test]
async fn a_lookout_that_has_read_nothing_has_an_empty_log() {
    let dir = temp();
    let api = open(&dir);
    let page = call(&api, "LogService/ListLookoutLog", json!({})).await;
    assert!(
        page.get("decisions")
            .is_none_or(|d| d.as_array().unwrap().is_empty()),
        "{page}"
    );
    assert!(no_more(&page));
}
