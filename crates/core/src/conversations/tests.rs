use serde_json::json;

use super::*;
use crate::input::ReplyTo;

struct Bench {
    store: Store,
    log: ConsoleLog,
    all: Conversations,
}

fn bench() -> Bench {
    let store = Store::open_in_memory().unwrap();
    Bench {
        log: ConsoleLog::new(store.clone()),
        all: Conversations::new(store.clone()),
        store,
    }
}

impl Bench {
    fn input(&self, sender: Sender, source: Source, text: &str) -> StoredInput {
        let input = Input {
            sender,
            source,
            text: text.into(),
        };
        let id = self.store.enqueue(&input).unwrap().unwrap();
        self.store.get(id).unwrap().unwrap()
    }

    /// The owner says something on the console, in reply to `to`.
    fn says(&self, text: &str, to: Option<&Message>) -> (Message, StoredInput) {
        let meta = json!({ "origin": "コンソール", "workgroup": "work" });
        let m = match to {
            Some(to) => self.log.reply(Who::Owner, text, meta, to),
            None => self.log.add(Who::Owner, text, meta),
        }
        .unwrap();
        let source = Source::Console {
            message: m.id,
            workgroup: "work".into(),
            view: String::new(),
            selection: None,
            attached: Vec::new(),
            reply_to: to.map(ReplyTo::of),
        };
        let row = self.input(Sender::Owner, source, text);
        (m, row)
    }

    fn on_slack(&self, ts: &str, thread: Option<&str>, sender: Sender) -> StoredInput {
        let source = Source::Slack {
            workspace: "w".into(),
            channel: "D".into(),
            ts: ts.into(),
            thread_ts: thread.map(Into::into),
            user: "U".into(),
            unmentioned: false,
            others_before: false,
            attached: Vec::new(),
        };
        let row = self.input(sender, source, &format!("slack {ts}"));
        let meta = json!({ "origin": "Slack · w", "input": row.id });
        self.log.add(Who::Owner, &row.input.text, meta).unwrap();
        row
    }

    fn lookout(&self, item: i64) -> StoredInput {
        let source = Source::Lookout {
            item,
            source: "1".into(),
        };
        self.input(Sender::System, source, "届いた")
    }

    fn wake(&self, id: i64) -> StoredInput {
        let source = Source::Wake {
            id,
            at: "now".into(),
        };
        self.input(Sender::System, source, "見直す")
    }

    fn settle(&self, row: &StoredInput) -> Conversation {
        self.all.settle(row, |_| None).unwrap()
    }

    /// Run the turn of `row` in its conversation, in which `does` happens.
    fn turn(&self, row: &StoredInput, does: impl Fn()) -> i64 {
        let c = self.settle(row);
        assert!(self.store.start_turn(row.id, "work").unwrap());
        self.all.began(c.id, &row.input, "work").unwrap();
        does();
        self.all.set_session(c.id, &format!("s{}", c.id)).unwrap();
        self.all.ended(c.id, row).unwrap();
        self.store.end_turn().unwrap();
        c.id
    }

    /// What the secretary says in the running turn.
    fn answers(&self, text: &str) -> Message {
        let m = self
            .log
            .add(Who::Aide, text, json!({ "workgroup": "work" }))
            .unwrap();
        self.all.link_to_running([message_key(m.id)]).unwrap();
        m
    }

    /// `secs` pass.
    fn pass(&self, secs: i64) {
        let conn = self.store.conn();
        conn.execute("UPDATE conversations SET said_at = said_at - ?1", [secs])
            .unwrap();
    }

    fn told(&self, row: &StoredInput) -> String {
        let c = self.settle(row);
        self.all.said_before(&self.log, &c, row, "work").unwrap()
    }
}

#[test]
fn a_reply_goes_on_in_the_conversation_of_its_thread_and_two_threads_do_not_mix() {
    let b = bench();
    let (a, first) = b.says("A の話", None);
    let of_a = b.turn(&first, || {});
    b.pass(GOES_ON_SECS + 1);
    let (z, second) = b.says("Z の話", None);
    let of_z = b.turn(&second, || {});
    assert_ne!(of_a, of_z);
    for (to, of) in [(&a, of_a), (&z, of_z), (&a, of_a)] {
        let (_, row) = b.says("続き", Some(to));
        assert_eq!(b.turn(&row, || {}), of);
        assert_eq!(b.told(&row), "", "a thread is told nothing of the other");
    }
}

#[test]
fn a_reply_to_what_the_secretary_said_goes_on_where_it_said_it() {
    let b = bench();
    let notice = std::cell::RefCell::new(None);
    let of = b.turn(&b.lookout(1), || {
        *notice.borrow_mut() = Some(b.answers("届きました"));
    });
    let notice = notice.into_inner().unwrap();
    let (_, row) = b.says("返しておいて", Some(&notice));
    assert_eq!(b.settle(&row).id, of);
}

#[test]
fn what_is_said_outside_a_thread_goes_on_for_a_while_and_then_begins_anew() {
    let b = bench();
    let of = b.turn(&b.says("明日の予定は？", None).1, || {});
    assert_eq!(b.turn(&b.says("それを動かして", None).1, || {}), of);
    b.pass(GOES_ON_SECS + 1);
    assert_ne!(b.settle(&b.says("別の話", None).1).id, of);
}

#[test]
fn a_reply_in_another_thread_does_not_change_what_goes_on_outside() {
    let b = bench();
    let (old, first) = b.says("前の話", None);
    let of_old = b.turn(&first, || {});
    b.pass(GOES_ON_SECS + 1);
    let of_new = b.turn(&b.says("今の話", None).1, || {});
    b.turn(&b.says("ところで", Some(&old)).1, || {});
    assert_ne!(of_old, of_new);
    assert_eq!(b.settle(&b.says("続き", None).1).id, of_new);
}

#[test]
fn another_workgroup_is_another_conversation() {
    let b = bench();
    let of = b.turn(&b.says("仕事の話", None).1, || {});
    let private = Source::Console {
        message: 99,
        workgroup: "personal".into(),
        view: String::new(),
        selection: None,
        attached: Vec::new(),
        reply_to: None,
    };
    assert_ne!(b.settle(&b.input(Sender::Owner, private, "私事")).id, of);
}

#[test]
fn a_slack_thread_is_a_conversation() {
    let b = bench();
    let of = b.turn(&b.on_slack("1.0", None, Sender::Owner), || {});
    b.pass(GOES_ON_SECS + 1);
    let other = b.turn(&b.on_slack("2.0", None, Sender::Owner), || {
        let keys = [slack_key("w", "D", "2.1")];
        b.all.link_to_running(keys).unwrap();
    });
    assert_ne!(of, other);
    for (ts, thread, expected) in [("3.0", "1.0", of), ("4.0", "2.1", other)] {
        let row = b.on_slack(ts, Some(thread), Sender::Owner);
        assert_eq!(b.settle(&row).id, expected);
    }
}

#[test]
fn what_arrives_by_itself_and_what_the_owner_let_be_answered_are_each_on_their_own() {
    let b = bench();
    let of = b.turn(&b.says("話", None).1, || {});
    let asked = Source::Asked {
        request: 1,
        workspace: "w".into(),
        channel: "D".into(),
        ts: "1.0".into(),
        thread_ts: None,
        user: "U2".into(),
        granted: true,
    };
    let arrival = Source::Arrival {
        connection: "adroit".into(),
        receiver: "r".into(),
        id: "1".into(),
        about: "s".into(),
    };
    let schedule = Source::Schedule {
        name: "朝".into(),
        at: "now".into(),
    };
    let mut seen = vec![of];
    for row in [
        b.lookout(1),
        b.lookout(2),
        b.input(Sender::System, arrival, "x"),
        b.input(Sender::System, schedule, "x"),
        b.input(Sender::Owner, asked, "x"),
    ] {
        let id = b.settle(&row).id;
        assert!(!seen.contains(&id));
        seen.push(id);
        if matches!(row.input.source, Source::Asked { .. }) {
            assert_eq!(b.told(&row), "", "the owner's talk is not handed over");
        }
    }
}

#[test]
fn a_wake_up_an_approval_and_the_end_of_a_task_come_back_to_where_they_were_made() {
    let b = bench();
    b.turn(&b.says("その前の話", None).1, || {});
    let (asking, row) = b.says("調べておいて", None);
    let of = b.turn(&row, || {
        let keys = [wake_key(7), task_key("01T")];
        b.all.link_to_running(keys).unwrap();
    });
    b.turn(&b.lookout(1), || {});
    assert_ne!(b.settle(&b.wake(8)).id, of);

    b.store
        .conn()
        .execute(
            "INSERT INTO worker_runs (id, task, runner, state, attempt, spec, created_at)
             VALUES (3, '01T', 'agent', 'ended', 1, '{}', 0)",
            [],
        )
        .unwrap();
    let ended = Source::Worker {
        run: 3,
        task: "task-1".into(),
        outcome: "done".into(),
        workgroup: "work".into(),
        again: 0,
    };
    assert_eq!(b.settle(&b.input(Sender::System, ended, "x")).id, of);

    let answered = Source::Approval {
        id: 5,
        user: "U".into(),
    };
    let answered = b.input(Sender::Owner, answered, "approved");
    let origin = |request: i64| (request == 5).then_some(row.id);
    assert_eq!(b.all.settle(&answered, origin).unwrap().id, of);

    for row in [b.wake(7), answered] {
        let asked = b.all.asked_with(&row, origin).unwrap().unwrap();
        assert!(
            matches!(asked.source, Source::Console { message, .. } if message == asking.id),
            "what comes back is answered where the owner asked"
        );
    }
}

#[test]
fn a_task_ordered_anew_comes_back_to_where_it_was_ordered_last() {
    let b = bench();
    let of = b.turn(&b.says("委ねて", None).1, || {
        b.all.move_to_running(task_key("01T")).unwrap();
    });
    let again = b.turn(&b.lookout(1), || {
        b.all.move_to_running(task_key("01T")).unwrap();
    });
    assert_ne!(of, again);
    assert_eq!(b.all.of(&task_key("01T")).unwrap(), Some(again));
}

#[test]
fn an_input_given_again_stays_in_its_conversation() {
    let b = bench();
    let (_, row) = b.says("話", None);
    let of = b.settle(&row).id;
    b.store.start_turn(row.id, "work").unwrap();
    b.store.interrupt_turn().unwrap();
    b.pass(GOES_ON_SECS + 1);
    assert_eq!(b.settle(&row).id, of);
}

#[test]
fn a_new_conversation_is_told_what_was_said_lately_and_no_more_than_a_bound() {
    let b = bench();
    let of = b.turn(&b.says("りんごの話", None).1, || {
        b.answers("りんごは赤い");
    });
    b.pass(GOES_ON_SECS + 1);
    let (_, row) = b.says("それは何色？", None);
    let told = b.told(&row);
    assert_ne!(b.settle(&row).id, of);
    assert!(told.contains("> #1 owner: りんごの話\n> #2 you: りんごは赤い\n"));
    assert!(!told.contains("何色"), "what is asked is not told twice");

    for n in 0..50 {
        b.log
            .add(Who::Aide, format!("{n} {}", "長".repeat(900)), json!({}))
            .unwrap();
    }
    b.pass(GOES_ON_SECS + 1);
    let told = b.told(&b.says("それは？", None).1);
    assert!(told.contains("49 長") && !told.contains("\n> #4 "));
    assert!(told.chars().count() < ELSEWHERE.all + 200);
}

#[test]
fn a_conversation_that_goes_on_is_told_only_what_was_said_elsewhere_since() {
    let b = bench();
    b.turn(&b.says("前の話", None).1, || {
        b.answers("前の答え");
    });
    b.turn(&b.lookout(1), || {
        b.answers("荷物が届きました");
    });
    let told = b.told(&b.says("それ、受け取って", None).1);
    assert!(told.contains("荷物が届きました"), "{told}");
    assert!(!told.contains("前の"), "{told}");
}

#[test]
fn what_others_said_and_were_answered_is_told_to_no_conversation() {
    let b = bench();
    b.on_slack("1.0", None, Sender::Other);
    let meta = json!({ "origin": "Slack · w" });
    b.log.add(Who::Aide, "他の人への返事", meta).unwrap();
    b.on_slack("2.0", None, Sender::Owner);
    let told = b.told(&b.says("何かあった？", None).1);
    assert!(told.contains("slack 2.0"), "{told}");
    assert!(!told.contains("slack 1.0") && !told.contains("他の人"));
}

#[test]
fn what_was_said_in_another_workgroup_is_not_told() {
    let b = bench();
    b.log
        .add(Who::Owner, "私事", json!({ "workgroup": "personal" }))
        .unwrap();
    assert!(!b.told(&b.says("話", None).1).contains("私事"));
}

#[test]
fn a_conversation_whose_session_is_gone_is_told_what_was_said_in_it() {
    let b = bench();
    let (m, row) = b.says("最初の話", None);
    let of = b.turn(&row, || {
        b.answers("最初の答え");
    });
    b.all.forget_session(of).unwrap();
    let (_, reply) = b.says("続き", Some(&m));
    assert_eq!(b.settle(&reply).id, of);
    let told = b.told(&reply);
    assert!(
        told.contains("最初の答え") && !told.contains("続き"),
        "{told}"
    );
}

#[test]
fn a_thread_from_before_conversations_were_told_apart_is_told_to_a_reply_in_it() {
    let b = bench();
    let root = b.log.add(Who::Aide, "昔の報告", json!({})).unwrap();
    let reply = b
        .log
        .reply(Who::Aide, "昔の補足", json!({}), &root)
        .unwrap();
    let told = b.told(&b.says("これは？", Some(&reply)).1);
    assert!(
        told.contains("昔の報告") && !told.contains("> #2 "),
        "{told}"
    );
}

#[test]
fn what_is_under_way_when_conversations_are_first_told_apart_goes_on_in_the_session_before() {
    let b = bench();
    let (_, row) = b.says("頼んだ", None);
    let guest = b.on_slack("1.0", None, Sender::Other);
    b.store.set_state(BEFORE, "old").unwrap();
    b.store
        .conn()
        .execute_batch(
            "INSERT INTO wakeups (id, at, note) VALUES (1, 0, 'n');
             INSERT INTO wakeups (id, at, note, fired) VALUES (2, 0, 'n', 1);
             INSERT INTO delegations (task, digest, by, workgroup) VALUES ('01T', 'd', 'secretary', 'work');
             INSERT INTO delegations (task, digest, by, workgroup) VALUES ('02T', 'd', 'owner', '');",
        )
        .unwrap();
    b.all.adopt_the_one_before().unwrap();
    b.all.adopt_the_one_before().unwrap();

    let old = b.settle(&row);
    assert_eq!(old.session.as_deref(), Some("old"));
    for (key, of) in [
        (wake_key(1), Some(old.id)),
        (wake_key(2), None),
        (task_key("01T"), Some(old.id)),
        (task_key("02T"), None),
        (input_key(guest.id), None),
    ] {
        assert_eq!(b.all.of(&key).unwrap(), of, "{key}");
    }
    assert_ne!(
        b.settle(&b.says("新しい話", None).1).id,
        old.id,
        "what is said from now on is not added to it"
    );
}

#[test]
fn the_messages_of_a_conversation_are_those_on_the_console_and_on_slack() {
    let b = bench();
    let row = b.on_slack("1.0", None, Sender::Owner);
    let of = b.turn(&row, || {
        b.answers("答え");
    });
    b.turn(&b.lookout(1), || {
        b.answers("別の話");
    });
    let said: Vec<_> = b
        .all
        .messages(&b.log, of)
        .unwrap()
        .into_iter()
        .map(|m| m.text)
        .collect();
    assert_eq!(said, ["slack 1.0", "答え"]);
}

#[test]
fn what_each_conversation_used_in_a_period_is_summed() {
    let b = bench();
    let of = b.settle(&b.lookout(1)).id;
    let other = b.settle(&b.lookout(2)).id;
    let usage = |input| crate::models::Usage {
        input_tokens: Some(input),
        output_tokens: Some(1),
        calls: 1,
    };
    let cached = Cached {
        read: 5,
        written: 3,
    };
    for (id, input) in [(of, 10), (of, 10), (other, 100)] {
        b.all.spent(id, &usage(input), cached).unwrap();
    }
    let now = now();
    let spent = b.all.spent_between(now - 5, now + 5).unwrap();
    let order: Vec<_> = spent.iter().map(|(id, _)| *id).collect();
    assert_eq!(order, [other, of], "the most input first");
    assert_eq!(spent[1].1, b.all.spent_by(of).unwrap());
    assert_eq!(spent[1].1.input_tokens, 20);
    assert_eq!(spent[1].1.cached_tokens, 10);
    assert!(b.all.spent_between(now + 5, now + 9).unwrap().is_empty());
}

#[test]
fn the_conversations_used_last_come_first_a_page_at_a_time() {
    let b = bench();
    let ids: Vec<i64> = (0..3).map(|n| b.turn(&b.lookout(n), || {})).collect();
    let page = b.all.recent("work", None, 2).unwrap();
    let listed: Vec<_> = page.iter().map(|c| c.id).collect();
    assert_eq!(listed, [ids[2], ids[1]]);
    let rest = b.all.recent("", page.last(), 2).unwrap();
    assert_eq!(rest.len(), 1);
    assert!(b.all.recent("personal", None, 2).unwrap().is_empty());
}

#[test]
fn the_conversation_the_owner_chose_is_gone_on_with() {
    let b = bench();
    let of = b.turn(&b.says("話", None).1, || {});
    let (m, row) = b.says("新しい話", None);
    let chosen = b.all.begin_anew(&row.input).unwrap();
    b.all.choose(chosen, &row.input, m.id).unwrap();
    assert_eq!(b.settle(&row).id, chosen);
    assert_ne!(chosen, of);
    assert_eq!(b.told(&row), "", "it is about something else");
    assert_eq!(
        b.settle(&b.says("続き", None).1).id,
        chosen,
        "what is said after goes on there"
    );
    assert!(b.all.waiting(chosen).unwrap());
}

#[test]
fn what_is_answered_in_a_channel_is_told_nothing_said_elsewhere() {
    let b = bench();
    b.turn(&b.says("内輪の話", None).1, || {});
    let slack = |channel: &str| {
        let source = Source::Slack {
            workspace: "w".into(),
            channel: channel.into(),
            ts: "1.0".into(),
            thread_ts: None,
            user: "U".into(),
            unmentioned: false,
            others_before: false,
            attached: Vec::new(),
        };
        b.told(&b.input(Sender::Owner, source, "@秘書 教えて"))
    };
    assert_eq!(slack("C1"), "");
    assert!(slack("D1").contains("内輪の話"));
}

#[test]
fn what_comes_back_more_than_once_is_answered_where_it_was_first_asked() {
    let b = bench();
    let row = b.on_slack("1.0", None, Sender::Owner);
    let of = b.turn(&row, || {
        b.all.link_to_running([wake_key(7)]).unwrap();
    });
    let first = b.wake(7);
    let again = b.turn(&first, || {
        b.all.link_to_running([wake_key(8)]).unwrap();
    });
    assert_eq!(of, again);
    let asked = b.all.asked_with(&b.wake(8), |_| None).unwrap().unwrap();
    assert_eq!(asked.place().unwrap().ts, "1.0");
    b.turn(&b.lookout(1), || {
        b.all.link_to_running([wake_key(9)]).unwrap();
    });
    assert_eq!(b.all.asked_with(&b.wake(9), |_| None).unwrap(), None);
}

#[test]
fn what_was_answered_to_someone_else_with_the_owners_leave_is_told_to_no_conversation() {
    let b = bench();
    let asked = Source::Asked {
        request: 1,
        workspace: "w".into(),
        channel: "C".into(),
        ts: "1.0".into(),
        thread_ts: None,
        user: "U2".into(),
        granted: true,
    };
    b.turn(&b.input(Sender::Owner, asked, "x"), || {
        let meta = json!({ "origin": "Slack · w", "workgroup": "work" });
        let m = b.log.add(Who::Aide, "許された答え", meta).unwrap();
        b.all.link_to_running([message_key(m.id)]).unwrap();
    });
    assert_eq!(b.told(&b.says("話", None).1), "");
}

#[test]
fn what_was_said_elsewhere_is_handed_over_until_a_turn_that_had_it_ended() {
    let b = bench();
    let of = b.turn(&b.says("話", None).1, || {
        b.all.link_to_running([wake_key(7)]).unwrap();
    });
    b.turn(&b.lookout(1), || {
        b.answers("荷物が届きました");
    });
    b.turn(&b.wake(7), || {});
    let (_, row) = b.says("それ、受け取って", None);
    assert_eq!(b.settle(&row).id, of);
    b.store.start_turn(row.id, "work").unwrap();
    b.all.began(of, &row.input, "work").unwrap();
    b.store.interrupt_turn().unwrap();
    assert!(b.told(&row).contains("荷物"), "its turn was cut off");
    b.turn(&row, || {});
    assert!(!b.told(&b.says("次", None).1).contains("荷物"));
}

#[test]
fn what_is_said_while_something_is_still_done_goes_on_however_long_it_takes() {
    let b = bench();
    let (_, row) = b.says("長い仕事", None);
    let of = b.settle(&row).id;
    b.pass(GOES_ON_SECS + 1);
    assert_eq!(b.settle(&b.says("これも足して", None).1).id, of);
}

#[test]
fn a_conversation_the_owner_chose_is_where_they_talk_from_then_on() {
    let b = bench();
    let of = b.turn(&b.lookout(1), || {});
    b.turn(&b.says("別の話", None).1, || {});
    let (m, row) = b.says("届いたものについて", None);
    b.all.choose(of, &row.input, m.id).unwrap();
    b.turn(&row, || {});
    assert_eq!(b.settle(&b.says("続き", None).1).id, of);
}

#[test]
fn a_conversation_waiting_for_its_first_turn_shows_only_in_its_workgroup() {
    let b = bench();
    b.settle(&b.says("仕事の話", None).1);
    assert!(b.all.recent("personal", None, 9).unwrap().is_empty());
    assert_eq!(b.all.recent("work", None, 9).unwrap().len(), 1);
}
