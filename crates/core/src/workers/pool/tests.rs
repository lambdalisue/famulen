use std::collections::VecDeque;

use futures_util::future::BoxFuture;
use serde_json::json;

use super::*;
use crate::gateway::{Answer, Call, Outcome as CallOutcome};
use crate::notebook::Writer;
use crate::notebook::testing::TempDir;
use crate::paths::at;
use crate::records::{Patch, log_lines};
use crate::workers::order::tests::delegated;
use crate::workers::{Finish, Order, Settings, WORKER};

type Meanwhile = Box<dyn FnOnce(&App) + Send>;

/// Runners that do what they are told to, and remember what was asked.
#[derive(Default)]
struct Fake {
    app: Mutex<Option<App>>,
    begins: Mutex<Vec<String>>,
    stopped: Mutex<Vec<String>>,
    /// Starts are held for the owner through the app's gateway.
    asks: bool,
    fails: std::sync::atomic::AtomicBool,
    /// Adroit cannot be asked.
    away: std::sync::atomic::AtomicBool,
    found: Mutex<VecDeque<Began>>,
    /// Done to the tasks while Adroit is asked of a run.
    meanwhile: Mutex<Option<Meanwhile>>,
    /// How each run stands, by title; `Working` when not said.
    looks: Mutex<HashMap<String, Looked>>,
}

impl Fake {
    fn look_as(&self, title: &str, looked: Looked) {
        self.looks.lock().unwrap().insert(title.into(), looked);
    }

    fn begun(&self) -> Vec<String> {
        self.begins.lock().unwrap().clone()
    }
}

impl Runners for Fake {
    fn begin<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Began>> {
        Box::pin(async move {
            self.begins.lock().unwrap().push(run.order.title.clone());
            anyhow::ensure!(
                !self.fails.load(std::sync::atomic::Ordering::SeqCst),
                "no such program"
            );
            if self.asks {
                let app = self.app.lock().unwrap().clone().unwrap();
                let call = Call::new(
                    WORKER,
                    "adroit",
                    "start_session",
                    json!({ "dir": "/src", "prompt": "p", "name": format!("{}", run.id) }),
                );
                return Ok(match app.gateway.judge(&call)? {
                    crate::gateway::Verdict::Hold(t) => Began::Asking(t.id),
                    _ => Began::Running(format!("s{}", run.id)),
                });
            }
            Ok(Began::Running(match run.order.runner {
                Runner::Adroit => format!("s{}", run.id),
                Runner::Agent => String::new(),
            }))
        })
    }

    fn find<'a>(&'a self, _run: &'a Run) -> BoxFuture<'a, anyhow::Result<Option<Began>>> {
        Box::pin(async move {
            if let Some(meanwhile) = self.meanwhile.lock().unwrap().take() {
                meanwhile(self.app.lock().unwrap().as_ref().unwrap());
            }
            anyhow::ensure!(
                !self.away.load(std::sync::atomic::Ordering::SeqCst),
                "no answer"
            );
            Ok(self.found.lock().unwrap().pop_front())
        })
    }

    fn look<'a>(&'a self, run: &'a Run) -> BoxFuture<'a, anyhow::Result<Looked>> {
        Box::pin(async move {
            if let Some(meanwhile) = self.meanwhile.lock().unwrap().take() {
                meanwhile(self.app.lock().unwrap().as_ref().unwrap());
            }
            anyhow::ensure!(
                !self.away.load(std::sync::atomic::Ordering::SeqCst),
                "no answer"
            );
            let looks = self.looks.lock().unwrap();
            Ok(looks
                .get(&run.order.title)
                .cloned()
                .unwrap_or(Looked::Working))
        })
    }

    fn stop(&self, run: &Run) {
        self.stopped.lock().unwrap().push(run.order.title.clone());
    }
}

struct Bench {
    app: App,
    fake: Arc<Fake>,
    pool: Pool,
    _dir: TempDir,
}

fn bench(name: &str, fake: Fake) -> Bench {
    let dir = TempDir::new(name);
    let app = crate::runtime::testing::app(dir.path());
    *fake.app.lock().unwrap() = Some(app.clone());
    let fake = Arc::new(fake);
    Bench {
        pool: Pool::new(app.clone(), fake.clone()),
        app,
        fake,
        _dir: dir,
    }
}

impl Bench {
    fn delegate(&self, title: &str, fields: &[(&str, &str)]) -> String {
        self.app
            .records
            .add(Writer::Secretary, TASKS, delegated(title, fields))
            .unwrap()
            .id
    }

    fn agent(&self, title: &str) -> String {
        self.delegate(title, &[("Runner", "agent")])
    }

    fn adroit(&self, title: &str) -> String {
        self.delegate(title, &[("Runner", "adroit"), ("Directory", &at("/src"))])
    }

    fn slots(&self, slots: u32) {
        Settings {
            slots,
            ..Settings::default()
        }
        .save(&self.app.store)
        .unwrap();
    }

    fn pause(&self, paused: bool) {
        Settings {
            paused,
            ..Settings::default()
        }
        .save(&self.app.store)
        .unwrap();
    }

    /// The time until a failed start is tried again has passed.
    fn wait_over(&self) {
        self.app
            .store
            .conn()
            .execute("UPDATE worker_runs SET retry_at = 0", [])
            .unwrap();
        self.pool.looked.lock().unwrap().clear();
    }

    fn task(&self, id: &str) -> Record {
        self.app.records.need(TASKS, id).unwrap()
    }

    fn status(&self, id: &str) -> String {
        self.task(id).body.value("Status").unwrap_or_default()
    }

    fn set(&self, id: &str, field: &str, value: &str) {
        self.app
            .records
            .change(
                Writer::Owner,
                TASKS,
                id,
                Patch::default().with(field, value),
            )
            .unwrap();
    }

    fn log(&self, id: &str) -> Vec<String> {
        log_lines(&self.task(id))
            .into_iter()
            .map(|l| format!("{}: {}", l.who, l.text))
            .collect()
    }

    fn told(&self) -> Vec<Input> {
        self.app
            .store
            .recent_inputs(0, 50)
            .unwrap()
            .into_iter()
            .rev()
            .map(|(stored, _)| stored.input)
            .filter(|i| matches!(i.source, Source::Worker { .. }))
            .collect()
    }

    fn runs(&self) -> Vec<Run> {
        let mut runs = self.app.workers.runs.recent(50).unwrap();
        runs.reverse();
        runs
    }

    /// Famulen stopped and came up again: what it held in memory is gone.
    fn restart(&mut self, fake: Fake) {
        *fake.app.lock().unwrap() = Some(self.app.clone());
        self.fake = Arc::new(fake);
        self.pool = Pool::new(self.app.clone(), self.fake.clone());
    }
}

#[tokio::test]
async fn a_delegated_task_is_run_marked_done_and_told_to_the_secretary_once() {
    let b = bench("pool-done", Fake::default());
    let id = b.agent("調べる");

    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun(), ["調べる"]);
    assert_eq!(b.status(&id), "in_progress");

    b.fake.look_as("調べる", Looked::Done("見つけた。".into()));
    b.pool.tick().await.unwrap();
    b.pool.tick().await.unwrap();

    assert_eq!(b.status(&id), "done");
    assert!(b.task(&id).body.value("Finished").is_some());
    let told = b.told();
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].sender, Sender::System);
    assert!(told[0].text.contains("見つけた。"), "{}", told[0].text);
    assert_eq!(b.runs()[0].state, State::Ended);
    assert!(
        b.log(&id).iter().all(|l| !l.contains("見つけた")),
        "what a worker wrote does not reach the notebook by itself"
    );
    assert!(b.log(&id).iter().any(|l| l == "famulen: 委託 #1 が済んだ"));
}

#[tokio::test]
async fn tasks_not_delegated_and_tasks_that_do_not_say_how_are_left_alone() {
    let b = bench("pool-left", Fake::default());
    for assignee in ["owner", "secretary"] {
        let mut patch = delegated("x", &[("Runner", "agent")]);
        patch.fields.clear();
        b.app
            .records
            .add(Writer::Secretary, TASKS, patch.with("Assignee", assignee))
            .unwrap();
    }
    b.delegate("how?", &[]);
    b.pool.tick().await.unwrap();
    assert!(b.fake.begun().is_empty());
}

#[tokio::test]
async fn no_more_run_at_once_than_the_owner_allows_and_the_next_follows() {
    let b = bench("pool-slots", Fake::default());
    b.slots(2);
    for title in ["a", "b", "c"] {
        b.agent(title);
    }
    b.pool.tick().await.unwrap();
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun(), ["a", "b"]);

    b.fake.look_as("a", Looked::Done("ok".into()));
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun(), ["a", "b", "c"]);
}

#[tokio::test]
async fn nothing_starts_while_the_pool_is_paused_and_what_runs_goes_on() {
    let b = bench("pool-paused", Fake::default());
    let a = b.agent("a");
    b.pool.tick().await.unwrap();
    Settings {
        paused: true,
        ..Settings::default()
    }
    .save(&b.app.store)
    .unwrap();
    b.agent("b");
    b.fake.look_as("a", Looked::Done("ok".into()));
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun(), ["a"]);
    assert_eq!(b.status(&a), "done");
}

#[tokio::test]
async fn work_on_code_is_handed_the_owners_steps_to_finish_it_and_the_secretary_checks_them() {
    let b = bench("pool-finish", Fake::default());
    Settings {
        finishes: vec![Finish {
            directory: at("/src"),
            steps: "Merge it.".into(),
        }],
        ..Settings::default()
    }
    .save(&b.app.store)
    .unwrap();
    b.adroit("直す");
    b.agent("調べる");
    b.pool.tick().await.unwrap();
    let finishes: Vec<String> = b
        .app
        .workers
        .runs
        .open()
        .unwrap()
        .into_iter()
        .map(|r| r.order.finish)
        .collect();
    assert_eq!(finishes, ["Merge it.", ""]);

    b.fake.look_as("直す", Looked::Done("マージした".into()));
    b.fake.look_as("調べる", Looked::Done("書いた".into()));
    b.app.workers.heard_of("s1").unwrap();
    b.pool.tick().await.unwrap();
    let told = b.told();
    assert_eq!(told.len(), 2);
    for Input { text, .. } in told {
        assert_eq!(
            text.contains("steps to finish it"),
            text.contains("マージした"),
            "{text}"
        );
    }
}

#[tokio::test]
async fn the_owner_is_asked_once_and_the_approved_start_is_made() {
    let mut b = bench(
        "pool-asked",
        Fake {
            asks: true,
            ..Fake::default()
        },
    );
    b.slots(1);
    let id = b.adroit("直す");
    b.pool.tick().await.unwrap();
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun().len(), 1, "asked once");
    assert_eq!(b.app.gateway.held_tickets().unwrap().len(), 1);
    assert_eq!(b.status(&id), "pending");

    b.restart(Fake {
        asks: true,
        ..Fake::default()
    });
    b.pool.tick().await.unwrap();
    assert!(b.fake.begun().is_empty(), "not asked again after a restart");

    let ticket = b.runs()[0].ticket.unwrap();
    b.app
        .gateway
        .answer_ticket(ticket, Answer::Approve)
        .await
        .unwrap();
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun().len(), 1);
    assert_eq!(b.runs()[0].state, State::Running);
    assert_eq!(b.runs()[0].session, "s1");
    assert_eq!(b.status(&id), "in_progress");
    assert!(b.app.gateway.held_tickets().unwrap().is_empty());
}

#[tokio::test]
async fn a_start_waiting_for_approval_leaves_its_slot_to_others() {
    let b = bench(
        "pool-asking-slot",
        Fake {
            asks: true,
            ..Fake::default()
        },
    );
    b.slots(1);
    b.adroit("a");
    b.adroit("b");
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun(), ["a", "b"]);
    for run in b.runs() {
        b.app
            .gateway
            .answer_ticket(run.ticket.unwrap(), Answer::Approve)
            .await
            .unwrap();
    }
    b.pool.tick().await.unwrap();
    let states: Vec<State> = b.runs().iter().map(|r| r.state).collect();
    assert_eq!(
        states,
        [State::Running, State::Asking],
        "approved starts still keep to the slots"
    );
}

#[tokio::test]
async fn a_rejected_start_puts_the_task_on_hold_and_tells_the_secretary() {
    let b = bench(
        "pool-rejected",
        Fake {
            asks: true,
            ..Fake::default()
        },
    );
    let id = b.adroit("直す");
    b.pool.tick().await.unwrap();
    let ticket = b.runs()[0].ticket.unwrap();
    b.app
        .gateway
        .answer_ticket(ticket, Answer::Reject)
        .await
        .unwrap();
    b.pool.tick().await.unwrap();
    b.pool.tick().await.unwrap();

    assert_eq!(b.status(&id), "waiting");
    assert!(b.task(&id).body.value("Waiting for").is_some());
    assert_eq!(b.told().len(), 1);
    assert!(b.told()[0].text.contains("did not approve"));
    assert_eq!(b.fake.begun().len(), 1, "not started, not asked again");
}

#[tokio::test]
async fn a_start_that_fails_is_tried_a_few_times_with_time_between_then_told() {
    let b = bench(
        "pool-fails",
        Fake {
            fails: true.into(),
            ..Fake::default()
        },
    );
    let id = b.agent("x");
    for _ in 0..3 {
        b.pool.tick().await.unwrap();
    }
    assert_eq!(b.fake.begun().len(), 1, "not again at once");

    b.pause(true);
    b.wait_over();
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun().len(), 1, "nor while paused");
    b.pause(false);

    for _ in 0..6 {
        b.wait_over();
        b.pool.tick().await.unwrap();
    }
    assert_eq!(b.fake.begun().len(), MOST_TRIES as usize);
    assert_eq!(b.status(&id), "waiting");
    assert_eq!(b.told().len(), 1);
    assert!(b.told()[0].text.contains("no such program"));
}

#[tokio::test]
async fn an_approved_start_that_keeps_failing_does_not_ask_the_owner_forever() {
    let b = bench(
        "pool-asks",
        Fake {
            asks: true,
            ..Fake::default()
        },
    );
    let id = b.adroit("直す");
    let mut asked = 0;
    for _ in 0..40 {
        b.wait_over();
        b.pool.tick().await.unwrap();
        for ticket in b.app.gateway.held_tickets().unwrap() {
            asked += 1;
            b.app
                .gateway
                .answer_ticket(ticket.id, Answer::Approve)
                .await
                .unwrap();
            // The call uses its approval up, and then fails.
            b.app
                .gateway
                .judge(&Call::new(
                    WORKER,
                    "adroit",
                    "start_session",
                    ticket.shown.clone(),
                ))
                .unwrap();
        }
    }
    assert!(asked <= MOST_ASKS + 1, "asked {asked} times");
    assert!(b.app.gateway.held_tickets().unwrap().is_empty());
    assert_eq!(b.status(&id), "waiting");
    assert_eq!(b.told().len(), 1);
}

#[tokio::test]
async fn a_start_not_yet_made_stops_once_its_task_is_blocked_or_not_pending() {
    for (name, field) in [
        ("blocked", "Blocked by"),
        ("waiting", "Status"),
        ("in_progress", "Status"),
    ] {
        let b = bench(
            &format!("pool-stop-{name}"),
            Fake {
                asks: true,
                ..Fake::default()
            },
        );
        let first = b
            .app
            .records
            .add(Writer::Owner, TASKS, Patch::titled("先"))
            .unwrap();
        let id = b.adroit("直す");
        b.pool.tick().await.unwrap();
        let value = match field {
            "Status" => name.to_owned(),
            _ => first.link(),
        };
        b.set(&id, field, &value);
        b.pool.tick().await.unwrap();
        assert!(b.app.gateway.held_tickets().unwrap().is_empty(), "{name}");
        assert_eq!(b.runs()[0].outcome, Some(Outcome::Stopped), "{name}");
        assert_eq!(b.runs().len(), 1, "{name}");
    }
}

#[tokio::test]
async fn a_running_task_that_comes_to_be_blocked_stops_its_agent_and_is_not_marked_done() {
    for adroit in [false, true] {
        let b = bench(&format!("pool-block-running-{adroit}"), Fake::default());
        let first = b
            .app
            .records
            .add(Writer::Owner, TASKS, Patch::titled("先"))
            .unwrap();
        let id = if adroit {
            b.adroit("直す")
        } else {
            b.agent("直す")
        };
        b.pool.tick().await.unwrap();
        b.set(&id, "Blocked by", &first.link());
        b.fake.look_as("直す", Looked::Done("済んだ".into()));
        b.pool.tick().await.unwrap();
        assert_eq!(b.runs()[0].outcome, Some(Outcome::Stopped));
        assert_ne!(b.status(&id), "done");
        let left = b.log(&id).iter().any(|l| l.contains("動いたまま"));
        assert_eq!(left, adroit);
    }
}

#[tokio::test]
async fn a_task_taken_back_while_its_session_is_read_is_left_as_the_owner_has_it() {
    let b = bench("pool-taken-back", Fake::default());
    let id = b.adroit("直す");
    b.pool.tick().await.unwrap();
    b.fake.look_as("直す", Looked::Done("済んだ".into()));
    let task = id.clone();
    *b.fake.meanwhile.lock().unwrap() = Some(Box::new(move |app| {
        let patch = Patch::default().with("Assignee", "secretary");
        app.records
            .change(Writer::Owner, TASKS, &task, patch)
            .unwrap();
    }));
    b.pool.tick().await.unwrap();
    assert_eq!(b.status(&id), "in_progress");
    assert_eq!(b.runs()[0].outcome, Some(Outcome::Stopped));
    assert!(b.told().is_empty());
}

#[tokio::test]
async fn a_task_changed_while_an_earlier_try_is_looked_for_is_not_started() {
    for (field, value) in [
        ("Status", "cancelled"),
        ("Assignee", "secretary"),
        ("Start", "2999-01-01"),
    ] {
        let fake = Fake {
            fails: true.into(),
            ..Fake::default()
        };
        let b = bench(&format!("pool-changed-{field}"), fake);
        let id = b.adroit("直す");
        b.pool.tick().await.unwrap();
        b.wait_over();
        b.fake
            .fails
            .store(false, std::sync::atomic::Ordering::SeqCst);
        *b.fake.meanwhile.lock().unwrap() = Some(Box::new(move |app| {
            let patch = Patch::default().with(field, value);
            app.records
                .change(Writer::Owner, TASKS, &id, patch)
                .unwrap();
        }));
        b.pool.tick().await.unwrap();
        assert_eq!(b.fake.begun().len(), 1, "{field}");
        assert_eq!(b.runs()[0].outcome, Some(Outcome::Stopped), "{field}");
    }
}

#[tokio::test]
async fn a_start_put_off_takes_its_request_back_and_is_asked_anew_when_it_comes() {
    let b = bench(
        "pool-put-off",
        Fake {
            asks: true,
            ..Fake::default()
        },
    );
    let id = b.adroit("直す");
    b.pool.tick().await.unwrap();
    let ticket = b.runs()[0].ticket.unwrap();
    b.app
        .gateway
        .answer_ticket(ticket, Answer::Approve)
        .await
        .unwrap();
    b.set(&id, "Start", "2999-01-01");
    b.pool.tick().await.unwrap();
    assert_eq!(b.runs()[0].outcome, Some(Outcome::Stopped));
    assert_eq!(b.status(&id), "pending");
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun().len(), 1);

    b.set(&id, "Start", "");
    b.pool.tick().await.unwrap();
    assert_eq!(b.app.gateway.held_tickets().unwrap().len(), 1);
}

#[tokio::test]
async fn a_start_put_off_while_it_is_unsure_is_not_made_twice() {
    let mut b = bench("pool-put-off-unsure", Fake::default());
    let id = b.adroit("直す");
    let order = Order::of(&b.task(&id), &b.app.records.code(TASKS, &id).unwrap()).unwrap();
    let run = b.app.workers.runs.begin(&id, &order).unwrap();
    b.app.workers.runs.trying(run.id).unwrap();
    b.restart(Fake {
        away: true.into(),
        ..Fake::default()
    });
    b.fake
        .found
        .lock()
        .unwrap()
        .push_back(Began::Running("s9".into()));
    b.set(&id, "Start", "2999-01-01");
    b.pool.tick().await.unwrap();
    assert_eq!(b.runs()[0].state, State::Unreachable);

    b.fake
        .away
        .store(false, std::sync::atomic::Ordering::SeqCst);
    b.wait_over();
    b.pool.tick().await.unwrap();
    assert_eq!(b.runs()[0].outcome, Some(Outcome::Stopped));
    assert_eq!(b.runs()[0].session, "s9");
    assert!(b.fake.begun().is_empty());
}

#[tokio::test]
async fn the_ended_runs_of_a_task_or_a_workgroup_are_found_however_many_others_came_after() {
    let b = bench("pool-ended-of", Fake::default());
    let mut orders = Vec::new();
    for (title, workgroup) in [("古い", "home"), ("新しい", "work")] {
        let id = b.agent(title);
        let code = b.app.records.code(TASKS, &id).unwrap();
        let mut order = Order::of(&b.task(&id), &code).unwrap();
        order.workgroups = vec![workgroup.into()];
        orders.push((id, order));
    }
    let runs = &b.app.workers.runs;
    for (n, (id, order)) in orders.iter().enumerate() {
        for _ in 0..=n * 3 {
            let run = runs.begin(id, order).unwrap();
            runs.ending(run.id, Outcome::Done, "x").unwrap();
            runs.ended(run.id).unwrap();
        }
    }
    let titles = |task: Option<&str>, workgroup: Option<&str>| {
        runs.ended_of(task, workgroup, 2)
            .unwrap()
            .into_iter()
            .map(|r| r.order.title)
            .collect::<Vec<_>>()
    };
    assert_eq!(titles(None, None), ["新しい", "新しい"]);
    assert_eq!(titles(Some(&orders[0].0), None), ["古い"]);
    assert_eq!(titles(None, Some("home")), ["古い"]);
}

#[tokio::test]
async fn a_task_cancelled_while_its_start_is_unsure_says_what_is_known_of_the_session() {
    for found in [true, false] {
        let mut b = bench(&format!("pool-unsure-{found}"), Fake::default());
        let id = b.adroit("直す");
        let order = Order::of(&b.task(&id), &b.app.records.code(TASKS, &id).unwrap()).unwrap();
        let run = b.app.workers.runs.begin(&id, &order).unwrap();
        b.app.workers.runs.trying(run.id).unwrap();
        b.restart(Fake {
            away: (!found).into(),
            ..Fake::default()
        });
        b.fake
            .found
            .lock()
            .unwrap()
            .push_back(Began::Running("s9".into()));
        b.set(&id, "Status", "cancelled");
        b.pool.tick().await.unwrap();

        assert_eq!(b.runs()[0].outcome, Some(Outcome::Stopped));
        let log = b.log(&id).join("\n");
        if found {
            assert!(
                log.contains("セッションは動いたまま") && log.contains("s9"),
                "{log}"
            );
        } else {
            assert!(log.contains("始まったかは分からない"), "{log}");
        }
    }
}

#[tokio::test]
async fn a_run_is_not_failed_for_an_adroit_that_is_away_and_goes_on_when_it_is_back() {
    let b = bench("pool-away", Fake::default());
    b.slots(1);
    let id = b.adroit("a");
    b.agent("b");
    b.pool.tick().await.unwrap();
    b.fake.away.store(true, std::sync::atomic::Ordering::SeqCst);
    for _ in 0..MOST_MISSES {
        b.app.workers.heard_of("s1").unwrap();
        b.pool.tick().await.unwrap();
    }
    assert_eq!(b.runs()[0].state, State::Unreachable);
    assert_eq!(b.fake.begun(), ["a", "b"], "its slot is free meanwhile");
    assert_eq!(b.status(&id), "in_progress");

    b.fake
        .away
        .store(false, std::sync::atomic::Ordering::SeqCst);
    b.fake.look_as("a", Looked::Done("ok".into()));
    b.app.workers.heard_of("s1").unwrap();
    b.pool.tick().await.unwrap();
    assert_eq!(b.status(&id), "done");
}

#[tokio::test]
async fn a_start_that_cannot_be_asked_of_is_neither_made_again_nor_called_failed() {
    let mut b = bench("pool-away-start", Fake::default());
    let id = b.adroit("a");
    let order = Order::of(&b.task(&id), &b.app.records.code(TASKS, &id).unwrap()).unwrap();
    let run = b.app.workers.runs.begin(&id, &order).unwrap();
    b.app.workers.runs.trying(run.id).unwrap();
    b.restart(Fake {
        away: true.into(),
        ..Fake::default()
    });
    for _ in 0..6 {
        b.wait_over();
        b.pool.tick().await.unwrap();
    }
    assert!(b.fake.begun().is_empty());
    assert_eq!(b.runs()[0].state, State::Unreachable);
    assert!(b.told().is_empty());
}

#[tokio::test]
async fn what_is_told_of_a_session_is_the_pools_even_after_its_run_ended() {
    let b = bench("pool-ended-session", Fake::default());
    b.adroit("a");
    b.pool.tick().await.unwrap();
    b.fake.look_as("a", Looked::Done("ok".into()));
    b.app.workers.heard_of("s1").unwrap();
    b.pool.tick().await.unwrap();
    assert!(b.app.workers.heard_of("s1").unwrap());
    assert!(!b.app.workers.heard_of("another").unwrap());
}

#[tokio::test]
async fn a_task_is_bound_to_where_it_was_delegated_and_to_nowhere_when_that_is_not_known() {
    let b = bench("pool-scopes", Fake::default());
    let id = b.agent("x");
    let scopes_now = |b: &Bench| {
        let task = b.task(&id);
        let order = Order::of(&task, "").unwrap();
        scopes(&b.app, &id, &order).unwrap()
    };
    assert!(scopes_now(&b).is_empty(), "written past Famulen's API");

    crate::workers::written(&b.app, &b.task(&id), Writer::Worker).unwrap();
    assert!(scopes_now(&b).is_empty(), "a worker delegates nothing");

    crate::workers::written(&b.app, &b.task(&id), Writer::Owner).unwrap();
    assert_eq!(scopes_now(&b), [crate::workgroups::ALL]);

    // The secretary works in `work` and names another workgroup.
    let input = Input {
        sender: Sender::Owner,
        source: Source::Wake {
            id: 1,
            at: String::new(),
        },
        text: String::new(),
    };
    let turn = b.app.store.enqueue(&input).unwrap().unwrap();
    b.app.store.start_turn(turn, "work").unwrap();
    let moved = b
        .app
        .records
        .change(
            Writer::Secretary,
            TASKS,
            &id,
            Patch {
                workgroups: Some(vec!["personal".into()]),
                ..Patch::default()
            },
        )
        .unwrap();
    assert!(scopes_now(&b).is_empty(), "not as it was delegated");
    crate::workers::written(&b.app, &moved, Writer::Secretary).unwrap();
    assert_eq!(scopes_now(&b), ["personal", "work"]);
    for scopes in [vec![], scopes_now(&b)] {
        assert!(
            b.app
                .permissions
                .ask_outside("adroit", &scopes)
                .unwrap()
                .is_some()
        );
    }
}

#[tokio::test]
async fn agents_without_a_model_of_their_own_are_bounded_a_day() {
    let b = bench("pool-unassigned", Fake::default());
    b.agent("x");
    let spec = b.app.models.registered().unwrap()[0].spec.clone();
    for _ in 0..crate::workers::UNASSIGNED_RUNS {
        let usage = crate::models::Usage {
            calls: 1,
            ..Default::default()
        };
        b.app
            .models
            .record(crate::models::Role::Worker, &spec, &usage)
            .unwrap();
    }
    b.pool.tick().await.unwrap();
    assert!(b.fake.begun().is_empty());
}

#[tokio::test]
async fn a_start_tried_again_is_bounded_as_what_was_ordered_then() {
    let b = bench(
        "pool-runner-changed",
        Fake {
            fails: true.into(),
            ..Fake::default()
        },
    );
    let id = b.agent("x");
    b.pool.tick().await.unwrap();
    let spec = b.app.models.registered().unwrap()[0].spec.clone();
    let usage = crate::models::Usage {
        calls: crate::workers::UNASSIGNED_RUNS as _,
        ..Default::default()
    };
    b.app
        .models
        .record(crate::models::Role::Worker, &spec, &usage)
        .unwrap();
    let adroit = delegated("x", &[("Runner", "adroit"), ("Directory", &at("/src"))]);
    let patch = Patch {
        sections: adroit.sections,
        ..Patch::default()
    };
    b.app
        .records
        .change(Writer::Owner, TASKS, &id, patch)
        .unwrap();
    b.wait_over();
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun().len(), 1);
}

#[tokio::test]
async fn a_failed_run_is_not_run_again_until_it_is_asked_for() {
    let b = bench("pool-failed", Fake::default());
    let id = b.agent("x");
    b.pool.tick().await.unwrap();
    b.fake.look_as("x", Looked::Failed("broke".into()));
    for _ in 0..3 {
        b.pool.tick().await.unwrap();
    }
    assert_eq!(b.status(&id), "waiting");
    assert_eq!(b.fake.begun().len(), 1);

    b.fake.look_as("x", Looked::Working);
    b.set(&id, "Status", "pending");
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun().len(), 2);
    assert_eq!(b.runs()[1].attempt, 1);
}

#[tokio::test]
async fn an_agent_cut_off_by_a_restart_is_run_once_more_and_no_further() {
    let mut b = bench("pool-interrupted", Fake::default());
    let id = b.agent("x");
    b.pool.tick().await.unwrap();

    let lost = || {
        let fake = Fake::default();
        fake.look_as("x", Looked::Lost);
        fake
    };
    b.restart(lost());
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun(), ["x"]);
    assert_eq!(b.runs()[1].attempt, 2);
    assert!(b.told().is_empty(), "run again by itself");
    assert!(
        b.log(&id)
            .iter()
            .any(|l| l.contains("途中で切れた。もう一度行う"))
    );

    b.restart(lost());
    b.pool.tick().await.unwrap();
    b.pool.tick().await.unwrap();
    assert!(b.fake.begun().is_empty());
    assert_eq!(b.status(&id), "waiting");
    assert_eq!(b.told().len(), 1);
}

#[tokio::test]
async fn a_session_is_taken_up_again_by_its_id_after_a_restart() {
    let mut b = bench("pool-reattach", Fake::default());
    let id = b.adroit("直す");
    b.pool.tick().await.unwrap();
    b.restart(Fake::default());
    b.fake.look_as("直す", Looked::Done("直した".into()));
    b.pool.tick().await.unwrap();
    assert!(b.fake.begun().is_empty());
    assert_eq!(b.status(&id), "done");
}

#[tokio::test]
async fn a_start_whose_answer_was_lost_is_found_and_not_made_twice() {
    let mut b = bench("pool-found", Fake::default());
    let id = b.adroit("直す");
    let task = b.task(&id);
    let code = b.app.records.code(TASKS, &id).unwrap();
    // Written down, then stopped before the answer came.
    let run = b
        .app
        .workers
        .runs
        .begin(&id, &Order::of(&task, &code).unwrap())
        .unwrap();
    b.app.workers.runs.trying(run.id).unwrap();

    b.restart(Fake::default());
    b.fake
        .found
        .lock()
        .unwrap()
        .push_back(Began::Running("s9".into()));
    b.pool.tick().await.unwrap();
    assert!(b.fake.begun().is_empty());
    assert_eq!(b.runs()[0].session, "s9");
    assert_eq!(b.runs().len(), 1);
}

#[tokio::test]
async fn cancelling_a_queued_task_keeps_it_from_running() {
    let b = bench("pool-cancel-queued", Fake::default());
    b.slots(1);
    b.agent("a");
    let id = b.agent("b");
    b.pool.tick().await.unwrap();
    b.set(&id, "Status", "cancelled");
    b.fake.look_as("a", Looked::Done("ok".into()));
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun(), ["a"]);
}

#[tokio::test]
async fn cancelling_a_running_task_stops_the_agent_and_leaves_the_session_to_the_owner() {
    let b = bench("pool-cancel-running", Fake::default());
    let agent = b.agent("a");
    let adroit = b.adroit("b");
    let next = b.agent("c");
    b.pool.tick().await.unwrap();
    b.set(&agent, "Status", "cancelled");
    b.set(&adroit, "Assignee", "owner");
    b.pool.tick().await.unwrap();

    assert!(b.fake.stopped.lock().unwrap().contains(&"a".to_owned()));
    assert_eq!(b.runs()[1].outcome, Some(Outcome::Stopped));
    assert!(
        b.log(&adroit)
            .iter()
            .any(|l| l.contains("セッションは動いたまま")),
        "{:?}",
        b.log(&adroit)
    );
    assert_eq!(b.status(&agent), "cancelled");
    assert_eq!(b.status(&next), "in_progress", "the slots are free");
    assert!(b.told().is_empty());
}

#[tokio::test]
async fn cancelling_a_task_waiting_for_approval_takes_the_request_back() {
    let b = bench(
        "pool-cancel-asking",
        Fake {
            asks: true,
            ..Fake::default()
        },
    );
    let id = b.adroit("直す");
    b.pool.tick().await.unwrap();
    b.app.records.remove(Writer::Owner, TASKS, &id).unwrap();
    b.pool.tick().await.unwrap();
    assert!(b.app.gateway.held_tickets().unwrap().is_empty());
    assert_eq!(b.runs()[0].outcome, Some(Outcome::Stopped));
}

#[tokio::test]
async fn a_session_waiting_for_the_owner_frees_its_slot_and_takes_it_again() {
    let b = bench("pool-waiting", Fake::default());
    b.slots(1);
    b.adroit("a");
    b.agent("b");
    b.agent("c");
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun(), ["a"]);

    b.fake.look_as("a", Looked::Waiting);
    b.app.workers.heard_of("s1").unwrap();
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun(), ["a", "b"]);

    b.fake.look_as("a", Looked::Working);
    b.app.workers.heard_of("s1").unwrap();
    b.fake.look_as("b", Looked::Done("ok".into()));
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun(), ["a", "b"], "its slot is taken again");
}

#[tokio::test]
async fn a_session_not_told_of_is_not_looked_at_every_time() {
    let b = bench("pool-slow-look", Fake::default());
    let id = b.adroit("a");
    b.pool.tick().await.unwrap();
    b.pool.tick().await.unwrap();
    b.fake.look_as("a", Looked::Done("ok".into()));
    b.pool.tick().await.unwrap();
    assert_eq!(b.status(&id), "in_progress");
    assert!(b.app.workers.heard_of("s1").unwrap());
    assert!(!b.app.workers.heard_of("someone-elses").unwrap());
    b.pool.tick().await.unwrap();
    assert_eq!(b.status(&id), "done");
}

#[tokio::test]
async fn an_end_cut_off_before_it_was_told_is_told_once() {
    let mut b = bench("pool-settle", Fake::default());
    let id = b.agent("x");
    b.pool.tick().await.unwrap();
    b.app.workers.runs.ending(1, Outcome::Done, "ok").unwrap();
    b.restart(Fake::default());
    b.pool.tick().await.unwrap();
    b.pool.tick().await.unwrap();
    assert_eq!(b.status(&id), "done");
    assert_eq!(b.told().len(), 1);
    let ends = b.log(&id).iter().filter(|l| l.contains("が済んだ")).count();
    assert_eq!(ends, 1);
}

#[tokio::test]
async fn a_start_in_adroit_is_judged_and_held_for_the_owner_with_what_it_would_do() {
    let dir = TempDir::new("pool-real-held");
    let app = crate::runtime::testing::app(dir.path());
    app.gateway
        .add_defined("adroit", Default::default(), None)
        .unwrap();
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    let task = app
        .records
        .add(
            Writer::Secretary,
            TASKS,
            delegated(
                "直す",
                &[("Runner", "adroit"), ("Directory", &at("/src/api"))],
            ),
        )
        .unwrap();
    pool.tick().await.unwrap();

    let held = app.gateway.held_tickets().unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(
        (held[0].actor.as_str(), held[0].tool.as_str()),
        (WORKER, "start_session")
    );
    assert_eq!(held[0].shown["dir"], at("/src/api"));
    assert_eq!(held[0].shown["prompt"], "Do it.");
    let code = app.records.code(TASKS, &task.id).unwrap();
    let name = held[0].shown["name"].as_str().unwrap();
    assert!(name.contains(&code) && name.contains("直す"), "{name}");

    // Another run may not use what was approved for this one.
    app.gateway
        .answer_ticket(held[0].id, Answer::Approve)
        .await
        .unwrap();
    let mut other = Call::new(WORKER, "adroit", "start_session", held[0].shown.clone());
    other.input["prompt"] = json!("something else");
    assert!(matches!(
        app.gateway.call(&other).await.unwrap(),
        CallOutcome::Held(_)
    ));
}

#[tokio::test]
async fn a_start_through_a_connection_outside_the_tasks_workgroup_is_asked_even_when_allowed() {
    let dir = TempDir::new("pool-real-workgroup");
    let app = crate::runtime::testing::app(dir.path());
    let c = app
        .gateway
        .add_defined("adroit", Default::default(), None)
        .unwrap();
    app.permissions
        .set_scoped(
            "owner",
            &format!("conn:{}", c.id),
            "start_session",
            Some(crate::permissions::Permission::Allow),
        )
        .unwrap();
    let mut patch = delegated(
        "直す",
        &[("Runner", "adroit"), ("Directory", &at("/src/api"))],
    );
    patch.workgroups = Some(vec!["work".into()]);
    app.records.add(Writer::Secretary, TASKS, patch).unwrap();

    Pool::new(app.clone(), Arc::new(Real::new(app.clone())))
        .tick()
        .await
        .unwrap();

    assert_eq!(app.gateway.held_tickets().unwrap().len(), 1);
}

#[tokio::test]
async fn a_tool_that_starts_agents_is_not_handed_over() {
    let dir = TempDir::new("pool-launchers");
    let app = crate::runtime::testing::app(dir.path());
    let c = app
        .gateway
        .add_defined("adroit", Default::default(), None)
        .unwrap();
    for (tool, fits) in [
        ("start_session", false),
        ("resume_session", false),
        ("read_session", true),
    ] {
        let order = Order {
            runner: Runner::Agent,
            request: "x".into(),
            tools: vec![format!("mcp__{}__{tool}", c.id)],
            ..Order::default()
        };
        assert_eq!(crate::workers::fit(&app, &order).is_ok(), fits, "{tool}");
    }
}

#[tokio::test]
async fn a_run_keeps_the_connection_it_was_started_through() {
    let dir = TempDir::new("pool-pinned");
    let app = crate::runtime::testing::app(dir.path());
    let first = app
        .gateway
        .add_defined("adroit", Default::default(), None)
        .unwrap();
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    app.records
        .add(
            Writer::Secretary,
            TASKS,
            delegated(
                "直す",
                &[("Runner", "adroit"), ("Directory", &at("/src/api"))],
            ),
        )
        .unwrap();
    pool.tick().await.unwrap();
    app.gateway
        .add_defined("adroit", Default::default(), None)
        .unwrap();
    let run = app.workers.runs.get(1).unwrap().unwrap();
    assert_eq!(run.connection, first.id);
    assert_eq!(run.state, State::Asking);
}

/// A stand-in for Claude Code that tries a few tools and answers with what
/// came of each.
#[cfg(unix)]
const FAKE_CLAUDE: &str = r#"#!/usr/bin/env perl
use strict;
use JSON::PP;
$| = 1;
my %got;
sub ask {
    my ($id, $request) = @_;
    print encode_json({ type => "control_request", request_id => $id, request => $request }), "\n";
    while (my $line = <STDIN>) {
        my $m = decode_json($line);
        next unless ($m->{type} // "") eq "control_response";
        return $m->{response}{response};
    }
}
sub may { my ($tool, $input) = @_; ask("use-$tool", { subtype => "can_use_tool", tool_name => $tool, input => $input })->{behavior} }
while (my $line = <STDIN>) {
    next unless $line =~ /"type":"user"/;
    my $told = decode_json($line)->{message}{content};
    $got{told} = $told;
    $got{file} = -e "handed/records/memos/README.md" ? "there" : "gone";
    $got{bash} = may("Bash", { command => "ls" });
    $got{task} = may("Task", { prompt => "x" });
    $got{web} = may("WebSearch", { query => "x" });
    $got{read} = may("Read", { file_path => "handed/records/memos/README.md" });
    $got{outside} = may("Read", { file_path => "/etc/hosts" });
    $got{own} = may("mcp__famulen__save_record", {});
    my $listed = ask("list", { subtype => "mcp_message", server_name => "adroit",
        message => { jsonrpc => "2.0", id => 1, method => "tools/list" } });
    $got{tools} = [ map { $_->{name} } @{ $listed->{mcp_response}{result}{tools} } ];
    for my $tool ("send_to_session", "end_session", "start_session") {
        my $r = ask("call-$tool", { subtype => "mcp_message", server_name => "adroit",
            message => { jsonrpc => "2.0", id => 2, method => "tools/call",
                         params => { name => $tool, arguments => { session => "s", prompt => "p" } } } });
        $got{$tool} = $r->{mcp_response}{result}{content}[0]{text};
    }
    print encode_json({ type => "result", is_error => JSON::PP::false, session_id => "w",
        result => JSON::PP->new->canonical->encode(\%got), usage => { input_tokens => 3, output_tokens => 2 } }), "\n";
}
"#;

#[cfg(unix)]
fn agent_app(dir: &TempDir, script: &str, fields: &[(&str, &str)]) -> (App, Pool, String) {
    let app = crate::runtime::testing::app(dir.path());
    app.gateway
        .add_defined("adroit", Default::default(), None)
        .unwrap();
    // `install` writes the executable in a process of its own, so that no
    // handle to write it is left open when it is run.
    let (source, bin) = (dir.path().join("claude.pl"), dir.path().join("claude"));
    std::fs::write(&source, script).unwrap();
    assert!(
        std::process::Command::new("install")
            .args(["-m", "755"])
            .arg(&source)
            .arg(&bin)
            .status()
            .unwrap()
            .success()
    );
    app.settings
        .apply(&[(
            crate::settings::Key::ClaudeBin,
            Some(&bin.display().to_string()),
        )])
        .unwrap();
    // The secretary's model names no executable of its own.
    let id = app
        .records
        .add(Writer::Secretary, TASKS, delegated("調べる", fields))
        .unwrap()
        .id;
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    (app, pool, id)
}

#[tokio::test]
async fn an_agent_whose_process_cannot_be_started_is_tried_again_and_not_counted() {
    let dir = TempDir::new("pool-no-process");
    let app = crate::runtime::testing::app(dir.path());
    let none = at("/nowhere/claude");
    app.settings
        .apply(&[(crate::settings::Key::ClaudeBin, Some(&none))])
        .unwrap();
    let task = delegated("調べる", &[("Runner", "agent")]);
    app.records.add(Writer::Secretary, TASKS, task).unwrap();
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    pool.tick().await.unwrap();
    let run = app.workers.runs.recent(1).unwrap().remove(0);
    assert_eq!((run.state, run.tries), (State::Starting, 1));
    assert!(run.note.contains("could not start"), "{}", run.note);
    let worker = crate::models::Role::Worker;
    let used = app
        .models
        .used_today(worker, app.tz(), Utc::now())
        .unwrap()
        .1;
    assert_eq!(used, 0);
}

#[tokio::test]
async fn an_agent_counts_before_the_next_is_started() {
    let dir = TempDir::new("pool-count");
    let app = crate::runtime::testing::app(dir.path());
    // Any program there is will do.
    let bin = std::env::current_exe().unwrap().display().to_string();
    app.settings
        .apply(&[(crate::settings::Key::ClaudeBin, Some(&bin))])
        .unwrap();
    for title in ["調べる", "次"] {
        let task = delegated(title, &[("Runner", "agent")]);
        app.records.add(Writer::Secretary, TASKS, task).unwrap();
    }
    let spec = app.models.registered().unwrap()[0].spec.clone();
    let usage = crate::models::Usage {
        calls: (crate::workers::UNASSIGNED_RUNS - 1) as _,
        ..Default::default()
    };
    let worker = crate::models::Role::Worker;
    app.models.record(worker, &spec, &usage).unwrap();
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    pool.tick().await.unwrap();
    assert_eq!(app.workers.runs.recent(9).unwrap().len(), 1);
    let used = app
        .models
        .used_today(worker, app.tz(), Utc::now())
        .unwrap()
        .1;
    assert_eq!(used, crate::workers::UNASSIGNED_RUNS);
}

#[cfg(unix)]
async fn run_agent(name: &str, script: &str, fields: &[(&str, &str)]) -> (App, TempDir, String) {
    let dir = TempDir::new(name);
    let (app, pool, id) = agent_app(&dir, script, fields);
    for _ in 0..200 {
        pool.tick().await.unwrap();
        if app.workers.runs.open().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    (app, dir, id)
}

#[cfg(unix)]
#[tokio::test]
async fn an_agent_uses_only_what_it_was_handed_and_asks_nobody() {
    let (app, _dir, id) = run_agent(
        "pool-agent",
        FAKE_CLAUDE,
        &[
            ("Runner", "agent"),
            ("Tools", "Read, Bash, mcp__adroit__send_to_session"),
            ("Files", "records/memos/README.md"),
        ],
    )
    .await;

    let task = app.records.need(TASKS, &id).unwrap();
    assert_eq!(task.body.value("Status").as_deref(), Some("done"));
    let run = app.workers.runs.get(1).unwrap().unwrap();
    let (answer, refused) = run.result.split_once("\n\n").unwrap();
    let got: serde_json::Value = serde_json::from_str(answer).unwrap();
    assert!(got["told"].as_str().unwrap().contains("Do it."), "{got}");
    assert_eq!(got["file"], "there");
    assert_eq!(got["read"], "allow");
    for tool in ["bash", "task", "web", "outside", "own"] {
        assert_eq!(got[tool], "deny", "{tool}: {got}");
    }
    assert_eq!(got["tools"], json!(["send_to_session"]));
    assert!(
        got["send_to_session"]
            .as_str()
            .unwrap()
            .contains("approval")
    );
    assert!(got["end_session"].as_str().unwrap().contains("not handed"));
    assert!(
        got["start_session"]
            .as_str()
            .unwrap()
            .contains("not handed")
    );
    assert!(
        refused.contains("Bash") && refused.contains("mcp__adroit__send_to_session"),
        "what needed the owner is in the result: {refused}"
    );
    assert!(app.gateway.held_tickets().unwrap().is_empty());
    assert!(!app.workers.dir.join("1").exists(), "its files are gone");
    let (_, calls) = app
        .models
        .used_today(crate::models::Role::Worker, app.tz(), Utc::now())
        .unwrap();
    assert_eq!(calls, 1);
}

#[cfg(unix)]
#[tokio::test]
async fn what_an_agent_writes_can_be_watched_and_stays_after_its_end() {
    let script = r#"#!/bin/sh
read -r init; read -r ask
echo '{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"読む。"}]}}'
echo '{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","name":"TodoWrite","input":{"todos":[{"content":"読む","status":"completed"}]}}]}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"済んだ"}'
"#;
    let (app, _dir, _id) = run_agent("pool-agent-watched", script, &[("Runner", "agent")]).await;
    let (run, sight, why) = crate::workers::watch(&app, 1).await.unwrap();
    assert_eq!((run.state, why.as_str()), (State::Ended, ""));
    let texts: Vec<_> = sight.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!((texts, sight.done()), (vec!["読む。"], Some(1.0)));
    assert!(crate::workers::watch(&app, 2).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn a_plan_an_agent_writes_is_told_without_waiting_for_the_next_look() {
    let dir = TempDir::new("pool-agent-told");
    let script = r#"#!/bin/sh
read -r init; read -r ask
echo '{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","name":"TodoWrite","input":{"todos":[{"content":"読む","status":"in_progress"}]}}]}}'
sleep 300
"#;
    let (app, pool, _id) = agent_app(&dir, script, &[("Runner", "agent")]);
    pool.tick().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), app.workers.poked())
        .await
        .expect("the pool is told of the plan");
    assert_eq!(app.workers.sights.of(1).doing(), "読む");
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_ends_an_agent_with_every_process_it_started() {
    let dir = TempDir::new("pool-agent-group");
    let script = format!(
        "#!/bin/sh\nsleep 300 &\necho $! > {}/child\nwait\n",
        dir.path().display()
    );
    let (app, pool, id) = agent_app(&dir, &script, &[("Runner", "agent")]);
    pool.tick().await.unwrap();
    let child = dir.path().join("child");
    let pid = || {
        std::fs::read_to_string(&child)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    for _ in 0..200 {
        if !pid().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", &pid()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    assert!(alive());
    let (_, calls) = app
        .models
        .used_today(crate::models::Role::Worker, app.tz(), Utc::now())
        .unwrap();
    assert_eq!(calls, 1, "counted once it starts");

    app.records
        .change(
            Writer::Owner,
            TASKS,
            &id,
            Patch::default().with("Status", "cancelled"),
        )
        .unwrap();
    pool.tick().await.unwrap();
    for _ in 0..100 {
        if !alive() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!alive());
}

#[cfg(unix)]
#[tokio::test]
async fn an_agent_that_ends_without_a_result_is_a_failure_told_to_the_secretary() {
    let (app, _dir, id) = run_agent(
        "pool-agent-fails",
        "#!/bin/sh\nexit 3\n",
        &[("Runner", "agent")],
    )
    .await;
    let task = app.records.need(TASKS, &id).unwrap();
    assert_eq!(task.body.value("Status").as_deref(), Some("waiting"));
    assert_eq!(
        app.workers.runs.get(1).unwrap().unwrap().outcome,
        Some(Outcome::Failed)
    );
    assert_eq!(app.store.count_pending().unwrap(), 1);
}

#[tokio::test]
async fn the_end_of_a_task_names_the_working_memory_that_speaks_of_it() {
    let b = bench("pool-working", Fake::default());
    let id = b.adroit("直す");
    b.pool.tick().await.unwrap();
    let working = |name: &str, clues: String| {
        let memory = crate::memory::WorkingMemory {
            name: name.into(),
            clues,
            ..Default::default()
        };
        crate::memory::write_working(&b.app.memory_dir, &memory).unwrap();
    };
    working("by-link", b.task(&id).link());
    working("by-session", "セッション s1 の完了待ち".into());
    working("other", "セッション s12 の完了待ち".into());

    b.fake.look_as("直す", Looked::Failed("だめ".into()));
    b.pool.looked.lock().unwrap().clear();
    b.pool.tick().await.unwrap();

    let text = &b.told()[0].text;
    let (said, left) = text.split_once("\n---\n").unwrap();
    assert!(
        said.contains("Working memory that speaks of it: by-link, by-session. "),
        "{text}"
    );
    assert!(said.contains("close_working"));
    assert_eq!(left, "だめ");
    assert_eq!(
        crate::memory::working_memories(&b.app.memory_dir)
            .unwrap()
            .len(),
        3,
        "it is the secretary's to close"
    );
}

#[tokio::test]
async fn the_end_of_a_task_no_working_memory_speaks_of_says_nothing_of_it() {
    let b = bench("pool-no-working", Fake::default());
    b.agent("調べる");
    b.pool.tick().await.unwrap();
    b.fake.look_as("調べる", Looked::Done("ok".into()));
    b.pool.tick().await.unwrap();
    assert!(!b.told()[0].text.contains("Working memory"));
}
