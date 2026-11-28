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
use crate::workers::runners::{After, Left};
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
    /// The titles of ended runs whose sessions went on.
    went_on: Mutex<Vec<String>>,
    /// The titles of those asked to go on.
    asked_on: Mutex<Vec<String>>,
    /// What each was asked to go on after.
    after: Mutex<Vec<After>>,
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
                Runner::Agent | Runner::Direct => String::new(),
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

    fn went_on<'a>(&'a self, ended: &'a [(Run, bool)]) -> BoxFuture<'a, Vec<i64>> {
        Box::pin(async move {
            let went = self.went_on.lock().unwrap();
            ended
                .iter()
                .filter(|(r, _)| went.contains(&r.order.title))
                .map(|(r, _)| r.id)
                .collect()
        })
    }

    fn go_on<'a>(&'a self, run: &'a Run, after: After) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.after.lock().unwrap().push(after);
            self.asked_on.lock().unwrap().push(run.order.title.clone());
            anyhow::ensure!(
                !self.fails.load(std::sync::atomic::Ordering::SeqCst),
                "no answer"
            );
            Ok(())
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

    /// The session `s<run>` ends its turn so.
    async fn turn_ends(&self, session: &str, title: &str, looked: Looked) {
        self.fake.look_as(title, looked);
        self.app.workers.heard_of(session).unwrap();
        self.pool.tick().await.unwrap();
    }

    /// The sessions of ended runs with these titles go on.
    async fn go_on(&self, titles: &[&str]) {
        *self.fake.went_on.lock().unwrap() = titles.iter().map(|t| (*t).to_owned()).collect();
        *self.pool.looked_back.lock().unwrap() = None;
        self.pool.tick().await.unwrap();
        self.fake.went_on.lock().unwrap().clear();
    }

    /// What was to wait until a session was idle for long has waited.
    async fn idle_over(&self) {
        self.app
            .store
            .conn()
            .execute("UPDATE worker_idle SET due_at = 1 WHERE due_at != 0", [])
            .unwrap();
        self.pool.looked.lock().unwrap().clear();
        self.pool.tick().await.unwrap();
    }

    /// Its session ends its turn so, and nothing happens for long.
    async fn stays_idle(&self, left: Left) {
        self.turn_ends("s1", "直す", Looked::Idle(left)).await;
        self.idle_over().await;
    }

    fn asked_on(&self) -> usize {
        self.fake.asked_on.lock().unwrap().len()
    }

    /// What was to wait until now has waited.
    async fn due(&self) {
        self.app
            .store
            .conn()
            .execute("UPDATE worker_again SET due_at = 1 WHERE due_at != 0", [])
            .unwrap();
        self.pool.looked.lock().unwrap().clear();
        self.pool.tick().await.unwrap();
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
async fn a_turn_that_ends_with_its_plan_unfinished_neither_ends_the_task_nor_holds_a_slot() {
    let b = bench("pool-idle", Fake::default());
    b.slots(1);
    let id = b.adroit("直す");
    b.adroit("次");
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun(), ["直す"]);

    b.turn_ends("s1", "直す", Looked::Idle(Left::default()))
        .await;
    assert_eq!(b.runs()[0].state, State::Idle);
    assert_eq!(b.status(&id), "in_progress");
    assert!(b.told().is_empty());
    assert_eq!(
        b.fake.begun(),
        ["直す", "次"],
        "it waits on someone, as one asking does"
    );

    b.turn_ends("s1", "直す", Looked::Working).await;
    assert_eq!(b.runs()[0].state, State::Running);
    b.turn_ends("s1", "直す", Looked::Done("済んだ".into()))
        .await;
    assert_eq!(b.status(&id), "done");
    let statuses: Vec<String> = b.log(&id);
    assert_eq!(
        statuses.iter().filter(|l| l.contains("委託 #1")).count(),
        2,
        "begun and done, nothing in between: {statuses:?}"
    );
}

#[tokio::test]
async fn a_session_closed_before_its_plan_was_finished_is_left_for_someone_to_check() {
    let b = bench("pool-unsure", Fake::default());
    let id = b.adroit("直す");
    b.pool.tick().await.unwrap();
    b.turn_ends(
        "s1",
        "直す",
        Looked::Unsure("closed early\n---\nまず調べます。".into()),
    )
    .await;

    let run = &b.runs()[0];
    assert_eq!(run.outcome, Some(Outcome::Unsure));
    assert_eq!(b.status(&id), "waiting");
    assert_eq!(
        b.task(&id).body.value("Waiting for").as_deref(),
        Some("済んだか確かめる委託")
    );
    assert!(
        b.log(&id)
            .iter()
            .any(|l| l.contains("済んだかは分からない")),
        "{:?}",
        b.log(&id)
    );
    let told = b.told();
    assert_eq!(told.len(), 1);
    assert!(told[0].text.contains("cannot be told"), "{}", told[0].text);
    assert!(told[0].text.contains("まず調べます。"), "{}", told[0].text);
    b.pool.tick().await.unwrap();
    assert_eq!(b.fake.begun(), ["直す"], "not started again by itself");

    b.go_on(&["直す"]).await;
    assert_eq!(b.runs()[0].state, State::Running, "its session went on");
    assert_eq!(b.status(&id), "in_progress");
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
            repository: String::new(),
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

fn cap_worker(app: &App, calls: f64) {
    let worker = crate::models::Role::Worker;
    let model = app.models.assignment(worker).unwrap().unwrap().model;
    app.models.assign(worker, model, Some(calls), None).unwrap();
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
    cap_worker(&b.app, 1.0);
    let id = b.agent("x");
    b.pool.tick().await.unwrap();
    let spec = b.app.models.registered().unwrap()[0].spec.clone();
    let usage = crate::models::Usage {
        calls: 1,
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
    let prompt = held[0].shown["prompt"].as_str().unwrap();
    assert!(prompt.starts_with("Do it.\n"), "{prompt}");
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
async fn a_session_without_a_model_runs_on_the_workers_unless_another_agent_is_named() {
    let dir = TempDir::new("pool-real-model");
    let app = crate::runtime::testing::app(dir.path());
    app.gateway
        .add_defined("adroit", Default::default(), None)
        .unwrap();
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    let src = at("/src/api");
    for (title, fields) in [
        ("書かない", vec![]),
        ("claude", vec![("Agent", "claude")]),
        ("選ぶ", vec![("Agent model", "opus")]),
        ("codex", vec![("Agent", "codex")]),
    ] {
        let mut fields = fields;
        fields.extend([("Runner", "adroit"), ("Directory", src.as_str())]);
        app.records
            .add(Writer::Secretary, TASKS, delegated(title, &fields))
            .unwrap();
    }
    pool.tick().await.unwrap();

    let held = app.gateway.held_tickets().unwrap();
    let model_of = |title: &str| {
        let shown = &held
            .iter()
            .find(|t| t.shown["name"].as_str().unwrap().ends_with(title))
            .unwrap_or_else(|| panic!("{title} was not started"))
            .shown;
        (shown["agent"].clone(), shown["model"].clone())
    };
    assert_eq!(model_of("書かない"), (json!("claude"), json!("sonnet")));
    assert_eq!(model_of("claude"), (json!("claude"), json!("sonnet")));
    assert_eq!(model_of("選ぶ"), (serde_json::Value::Null, json!("opus")));
    assert_eq!(model_of("codex"), (json!("codex"), serde_json::Value::Null));

    // A start held for approval is made again as it was asked.
    let worker = crate::models::Role::Worker;
    app.models
        .assign(worker, "claude-opus", None, None)
        .unwrap();
    let kept = app.workers.runs.open().unwrap();
    let kept = kept
        .iter()
        .find(|r| r.order.title.ends_with("書かない"))
        .unwrap();
    assert_eq!(kept.order.model, "sonnet");
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
    $got{bash} = may("Bash", { command => "ls" });
    $got{task} = may("Task", { prompt => "x" });
    $got{web} = may("WebSearch", { query => "x" });
    $got{read} = may("Read", { file_path => "note.md" });
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
    cap_worker(&app, 1.0);
    let worker = crate::models::Role::Worker;
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    pool.tick().await.unwrap();
    assert_eq!(app.workers.runs.recent(9).unwrap().len(), 1);
    let used = app
        .models
        .used_today(worker, app.tz(), Utc::now())
        .unwrap()
        .1;
    assert_eq!(used, 1);
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
        ],
    )
    .await;

    let task = app.records.need(TASKS, &id).unwrap();
    assert_eq!(task.body.value("Status").as_deref(), Some("done"));
    let run = app.workers.runs.get(1).unwrap().unwrap();
    let (answer, refused) = run.result.split_once("\n\n").unwrap();
    let got: serde_json::Value = serde_json::from_str(answer).unwrap();
    assert!(got["told"].as_str().unwrap().contains("Do it."), "{got}");
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
    let texts = app.workers.sights.said(1);
    assert_eq!(
        (texts, sight.done()),
        (vec!["読む。".to_owned()], Some(1.0))
    );
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

/// Adroit as far as it starts a session and tells what one asks, from a file.
const ASKING_ADROIT: &str = r#"
format = 1
id = "adroit"
name = "Adroit"

[account]
shows = { label = "Shows" }

[transport.command]
program = "perl"

[[commands]]
name = "start_session"
description = "Start one."
args = ["-e", "print qq({{\"id\":\"s1\"}})"]

[[commands]]
name = "read_session"
description = "Read it."
args = ["-e", "open my $f, '<', shift or die; print <$f>", "--", "{account.shows}"]
read_only = true

[[commands]]
name = "put_question"
description = "Answer it."
args = ["-e", "1"]

[commands.each_time]
reads = "read_session"
at = "/question"
same = { "/question/id" = "{args.question}" }
questions = "/input/questions"
fills = "replies"

[permissions]
start_session = "confirm"
read_session = "allow"
"#;

#[tokio::test]
async fn a_request_about_what_a_session_asks_no_more_is_taken_back() {
    let dir = TempDir::new("pool-asks-no-more");
    let services = dir.path().join("gateway/services");
    std::fs::create_dir_all(&services).unwrap();
    std::fs::write(services.join("adroit.toml"), ASKING_ADROIT).unwrap();
    let app = crate::runtime::testing::app(dir.path());
    let shows = dir.path().join("shows");
    let show = |state: &str, question: &str| {
        let question = json!({ "id": question, "kind": "question", "input": { "questions": [{ "question": "どちら？" }] } });
        std::fs::write(
            &shows,
            json!({ "state": state, "question": question }).to_string(),
        )
        .unwrap();
    };
    let values = [("shows".to_owned(), shows.to_str().unwrap().to_owned())];
    app.gateway
        .add_defined("adroit", values.into(), None)
        .unwrap();
    app.gateway.read_tools("adroit").await.unwrap();
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
    let start = app.gateway.held_tickets().unwrap()[0].id;
    app.gateway
        .answer_ticket(start, Answer::Approve)
        .await
        .unwrap();
    pool.tick().await.unwrap();
    let held = || app.gateway.held_tickets().unwrap().len();
    let look = async || {
        app.workers.heard_of("s1").unwrap();
        pool.tick().await.unwrap();
    };

    show("waiting", "q1");
    look().await;
    crate::workers::adroit::ask_again(&app, 1, "q1")
        .await
        .unwrap();
    look().await;
    assert_eq!(held(), 1, "it is asked still");
    // What the secretary put to them, naming the session by its code.
    let by_code = Call::new(
        crate::permissions::SECRETARY,
        "adroit",
        "put_question",
        json!({ "session": "k7", "question": "q1", "summary": "x" }),
    );
    app.gateway.withdraw_decided("adroit", "q1").unwrap();
    app.gateway.call(&by_code).await.unwrap();
    assert_eq!(held(), 1);
    show("waiting", "q2");
    look().await;
    assert_eq!(held(), 0);
    crate::workers::adroit::ask_again(&app, 1, "q2")
        .await
        .unwrap();
    std::fs::write(&shows, json!({ "state": "working" }).to_string()).unwrap();
    look().await;
    assert_eq!(held(), 0, "answered in Adroit");
    assert!(
        crate::workers::adroit::ask_again(&app, 1, "q2")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn an_ended_run_whose_session_goes_on_is_under_way_again_and_its_end_is_told_anew() {
    for (outcome, status) in [
        (Looked::Failed("broke".into()), "waiting"),
        (Looked::Done("half".into()), "done"),
    ] {
        let b = bench(&format!("pool-goes-on-{status}"), Fake::default());
        let id = b.adroit("直す");
        b.pool.tick().await.unwrap();
        b.turn_ends("s1", "直す", outcome).await;
        assert_eq!(b.status(&id), status);

        b.fake.look_as("直す", Looked::Working);
        b.go_on(&["直す"]).await;
        assert_eq!(b.status(&id), "in_progress");
        let task = b.task(&id);
        assert_eq!(task.body.value("Finished").unwrap_or_default(), "");
        assert_eq!(task.body.value("Waiting for").unwrap_or_default(), "");
        assert_eq!(b.runs().len(), 1, "no second run");
        assert_eq!(b.runs()[0].state, State::Running);
        assert_eq!(b.told().len(), 1);

        b.turn_ends("s1", "直す", Looked::Done("直した".into()))
            .await;
        b.pool.tick().await.unwrap();
        assert_eq!(b.status(&id), "done");
        let told = b.told();
        assert_eq!(told.len(), 2);
        assert!(told[1].text.contains("went on since"), "{}", told[1].text);
        assert!(told[1].text.contains("直した"));

        b.fake.look_as("直す", Looked::Working);
        b.go_on(&["直す"]).await;
        b.turn_ends("s1", "直す", Looked::Failed("broke".into()))
            .await;
        assert_eq!(b.status(&id), "waiting");
        assert_eq!(b.told().len(), 3);
        assert_eq!(b.fake.begun(), ["直す"]);
    }
}

#[tokio::test]
async fn a_task_put_back_cancelled_or_taken_back_is_not_taken_up_by_its_old_session() {
    for (field, value) in [
        ("Status", "cancelled"),
        ("Status", "pending"),
        ("Assignee", "owner"),
    ] {
        let b = bench(&format!("pool-not-taken-up-{value}"), Fake::default());
        b.pause(false);
        let id = b.adroit("直す");
        b.pool.tick().await.unwrap();
        b.turn_ends("s1", "直す", Looked::Failed("broke".into()))
            .await;
        b.pause(true);
        b.set(&id, field, value);
        b.go_on(&["直す"]).await;
        assert_eq!(b.runs()[0].state, State::Ended, "{value}");
        assert_eq!(b.task(&id).body.value(field).as_deref(), Some(value));
    }
}

#[tokio::test]
async fn a_task_run_anew_is_not_run_by_its_old_session_too() {
    let b = bench("pool-run-anew", Fake::default());
    let id = b.adroit("直す");
    b.pool.tick().await.unwrap();
    b.turn_ends("s1", "直す", Looked::Failed("broke".into()))
        .await;
    b.set(&id, "Status", "pending");
    b.fake.look_as("直す", Looked::Working);
    b.go_on(&["直す"]).await;
    let states: Vec<State> = b.runs().iter().map(|r| r.state).collect();
    assert_eq!(states, [State::Ended, State::Running]);
    b.turn_ends("s2", "直す", Looked::Done("ok".into())).await;
    b.fake.look_as("直す", Looked::Working);
    b.go_on(&["直す"]).await;
    assert_eq!(b.runs()[0].state, State::Ended);
    assert_eq!(
        b.runs()[1].state,
        State::Running,
        "the newer session went on"
    );
}

#[tokio::test]
async fn a_done_task_whose_session_goes_on_does_not_stop_what_waited_for_it() {
    let b = bench("pool-goes-on-blocker", Fake::default());
    let first = b.adroit("先");
    let next = b.adroit("後");
    let link = b.task(&first).link();
    b.app
        .records
        .change(
            Writer::Owner,
            TASKS,
            &next,
            Patch {
                fields: vec![crate::records::Field {
                    name: "Blocked by".into(),
                    value: String::new(),
                    values: vec![link],
                }],
                ..Patch::default()
            },
        )
        .unwrap();
    b.pool.tick().await.unwrap();
    b.turn_ends("s1", "先", Looked::Done("ok".into())).await;
    assert_eq!(b.status(&next), "in_progress");
    b.go_on(&["先"]).await;
    b.pool.tick().await.unwrap();
    assert_eq!(b.status(&first), "done");
    assert_eq!(b.runs()[1].state, State::Running);
}

#[tokio::test]
async fn a_passing_fault_has_the_session_go_on_a_few_times_with_time_between_then_fails() {
    let b = bench("pool-stumbles", Fake::default());
    let id = b.adroit("直す");
    b.pool.tick().await.unwrap();
    let asked = || b.fake.asked_on.lock().unwrap().len();
    let stumble = async || {
        b.turn_ends("s1", "直す", Looked::Stumbled("overloaded".into()))
            .await;
    };
    for times in 1..=GO_ON_AFTER_SECS.len() {
        stumble().await;
        stumble().await;
        assert_eq!(asked(), times - 1, "not at once");
        b.due().await;
        assert_eq!(asked(), times);
        assert_eq!(b.status(&id), "in_progress");
        assert!(b.told().is_empty());
        b.turn_ends("s1", "直す", Looked::Working).await;
    }
    stumble().await;
    assert_eq!(b.status(&id), "waiting");
    assert_eq!(b.told().len(), 1);
    assert_eq!(asked(), GO_ON_AFTER_SECS.len());
    assert_eq!(b.fake.begun(), ["直す"]);
}

#[tokio::test]
async fn faults_far_apart_are_not_counted_together() {
    let b = bench("pool-stumbles-apart", Fake::default());
    b.adroit("直す");
    b.pool.tick().await.unwrap();
    for times in 1..=GO_ON_AFTER_SECS.len() + 2 {
        b.turn_ends("s1", "直す", Looked::Stumbled("overloaded".into()))
            .await;
        b.due().await;
        assert_eq!(b.fake.asked_on.lock().unwrap().len(), times);
        b.turn_ends("s1", "直す", Looked::Working).await;
        b.app
            .store
            .conn()
            .execute(
                "UPDATE worker_again SET continued_at = continued_at - ?1",
                [IN_A_ROW_SECS + 1],
            )
            .unwrap();
    }
    assert!(b.told().is_empty());
}

#[tokio::test]
async fn a_session_that_went_on_by_itself_before_it_was_asked_is_not_asked() {
    let b = bench("pool-stumbles-itself", Fake::default());
    b.adroit("直す");
    b.pool.tick().await.unwrap();
    b.turn_ends("s1", "直す", Looked::Stumbled("overloaded".into()))
        .await;
    b.turn_ends("s1", "直す", Looked::Working).await;
    b.due().await;
    b.turn_ends("s1", "直す", Looked::Stumbled("overloaded".into()))
        .await;
    assert!(b.fake.asked_on.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_session_that_cannot_be_asked_to_go_on_is_tried_no_more_often() {
    let b = bench("pool-stumbles-unasked", Fake::default());
    let id = b.adroit("直す");
    b.pool.tick().await.unwrap();
    b.fake
        .fails
        .store(true, std::sync::atomic::Ordering::SeqCst);
    for _ in GO_ON_AFTER_SECS {
        b.turn_ends("s1", "直す", Looked::Stumbled("overloaded".into()))
            .await;
        b.due().await;
    }
    b.turn_ends("s1", "直す", Looked::Stumbled("overloaded".into()))
        .await;
    assert_eq!(b.status(&id), "waiting");
    assert_eq!(
        b.fake.asked_on.lock().unwrap().len(),
        GO_ON_AFTER_SECS.len()
    );
}

/// Adroit as far as it starts a session, lists and reads it from a file, and
/// keeps what it is asked.
const GOING_ADROIT: &str = r#"
format = 1
id = "adroit"
name = "Adroit"

[account]
shows = { label = "Shows" }

[transport.command]
program = "perl"

[[commands]]
name = "start_session"
description = "Start one."
args = ["-e", "print qq({{\"id\":\"s1\"}})"]

[[commands]]
name = "read_session"
description = "Read it."
args = ["-e", "open my $f, '<', shift or die; print <$f>", "--", "{account.shows}"]
read_only = true

[[commands]]
name = "list_sessions"
description = "List them."
args = ["-e", "open my $f, '<', shift or die; my $s = <$f>; print '[', $s =~ /ended/ ? '' : $s, ']'", "--", "{account.shows}"]
read_only = true

[[commands]]
name = "send_to_session"
description = "Ask it."
args = ["-e", "my ($p, $s, $t) = @ARGV; open my $f, '>>', \"$p.sent\" or die; print $f \"$s: $t\\n\"; print '{{}}'", "--", "{account.shows}", "{args.session}", "{args.prompt}"]

[commands.input]
type = "object"
required = ["session", "prompt"]

[commands.input.properties]
session = { type = "string" }
prompt = { type = "string" }

[permissions]
start_session = "allow"
read_session = "allow"
list_sessions = "allow"
send_to_session = "confirm"
"#;

#[tokio::test]
async fn a_session_in_adroit_is_asked_to_go_on_unless_forbidden_and_seen_going_on_after_its_end() {
    let dir = TempDir::new("pool-real-goes-on");
    let services = dir.path().join("gateway/services");
    std::fs::create_dir_all(&services).unwrap();
    std::fs::write(services.join("adroit.toml"), GOING_ADROIT).unwrap();
    let app = crate::runtime::testing::app(dir.path());
    let shows = dir.path().join("shows");
    let sent = || std::fs::read_to_string(dir.path().join("shows.sent")).unwrap_or_default();
    let show = |state: &str, began: i64| {
        let session = json!({
            "id": "s1", "state": state, "message": "API Error: 529 overloaded_error",
            "last_active_ms": began * 1000,
            "turns": [{ "turn": 1, "prompt": "p", "reply": "ok", "began_ms": began * 1000 }],
            "timeline": [{ "id": "g", "parent": null, "status": "completed" }],
            "undelivered": 0,
        });
        std::fs::write(&shows, session.to_string()).unwrap();
    };
    let values = [("shows".to_owned(), shows.to_str().unwrap().to_owned())];
    app.gateway
        .add_defined("adroit", values.into(), None)
        .unwrap();
    app.gateway.read_tools("adroit").await.unwrap();
    let pool = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    let directory = at("/src/api");
    let order = [("Runner", "adroit"), ("Directory", directory.as_str())];
    let task = app
        .records
        .add(Writer::Secretary, TASKS, delegated("直す", &order))
        .unwrap()
        .id;
    let run = {
        let order = Order::of(&app.records.need(TASKS, &task).unwrap(), "task-x").unwrap();
        let run = app.workers.runs.begin(&task, &order).unwrap();
        app.workers.runs.through(run.id, "adroit").unwrap();
        app.workers.runs.running(run.id, "s1").unwrap();
        run.id
    };
    let status = || {
        let task = app.records.need(TASKS, &task).unwrap();
        task.body.value("Status").unwrap_or_default()
    };
    let state = || app.workers.runs.get(run).unwrap().unwrap().state;
    let look = async || {
        app.workers.heard_of("s1").unwrap();
        pool.tick().await.unwrap();
    };
    let due = |run| app.workers.runs.due(run, 1).unwrap();
    let now = crate::store::now();

    show("failed", now - 60);
    look().await;
    due(run);
    look().await;
    assert!(
        sent().starts_with("s1: "),
        "confirmed at its start: {}",
        sent()
    );
    assert!(app.gateway.held_tickets().unwrap().is_empty());
    look().await;
    assert_eq!(sent().lines().count(), 1, "it has yet to take it up");
    assert_eq!(state(), State::Running);

    app.gateway
        .set_permission(
            "owner",
            "conn:adroit",
            "send_to_session",
            Some(crate::gateway::permissions::Permission::Forbid),
        )
        .unwrap();
    show("failed", now + 1);
    look().await;
    due(run);
    look().await;
    assert_eq!(sent().lines().count(), 1, "forbidden");

    show("done", now - 30);
    look().await;
    assert_eq!(state(), State::Ended);
    look().await;
    assert_eq!(state(), State::Ended, "it did nothing since");
    show("working", now + 9999);
    look().await;
    assert_eq!(state(), State::Running);
    assert_ne!(status(), "done");

    // It failed, went on and was closed while Famulen was not running.
    app.workers.runs.due(run, 0).unwrap();
    app.workers
        .runs
        .ending(run, Outcome::Failed, "broke")
        .unwrap();
    app.workers.runs.ended(run).unwrap();
    show("ended", now + 9999);
    app.store
        .conn()
        .execute("UPDATE worker_runs SET ended_at = ended_at - 60", [])
        .unwrap();
    let anew = Pool::new(app.clone(), Arc::new(Real::new(app.clone())));
    anew.tick().await.unwrap();
    assert_eq!(state(), State::Running);
    anew.tick().await.unwrap();
    assert_eq!(status(), "done");
    let told = app.store.recent_inputs(0, 50).unwrap().len();
    anew.tick().await.unwrap();
    assert_eq!(app.store.recent_inputs(0, 50).unwrap().len(), told);
}

/// Adroit that lists more ended sessions than a command's output is cut to,
/// with the one a run started last.
const LONG_LISTED_ADROIT: &str = r#"
format = 1
id = "adroit"
name = "Adroit"

[transport.command]
program = "perl"

[[commands]]
name = "list_sessions"
description = "List them."
args = ["-e", 'print "[", join(",", (map {{ qq({{"id":"old$_","name":"task-y-run$_ 前の仕事","state":"ended"}}) }} 1 .. 2000), q({{"id":"s9","name":"task-x-run1 直す","state":"ended"}})), "]"']
read_only = true

[permissions]
list_sessions = "allow"
"#;

#[tokio::test]
async fn a_session_whose_start_was_not_told_is_found_however_many_sessions_ended_before() {
    let dir = TempDir::new("pool-real-finds");
    let services = dir.path().join("gateway/services");
    std::fs::create_dir_all(&services).unwrap();
    std::fs::write(services.join("adroit.toml"), LONG_LISTED_ADROIT).unwrap();
    let app = crate::runtime::testing::app(dir.path());
    app.gateway
        .add_defined("adroit", Default::default(), None)
        .unwrap();
    app.gateway.read_tools("adroit").await.unwrap();
    let directory = at("/src/api");
    let order = [("Runner", "adroit"), ("Directory", directory.as_str())];
    let task = app
        .records
        .add(Writer::Secretary, TASKS, delegated("直す", &order))
        .unwrap()
        .id;
    let order = Order::of(&app.records.need(TASKS, &task).unwrap(), "task-x").unwrap();
    let run = app.workers.runs.begin(&task, &order).unwrap();
    app.workers.runs.through(run.id, "adroit").unwrap();
    let run = app.workers.runs.get(run.id).unwrap().unwrap();

    let found = crate::workers::adroit::find(&app, &run).await.unwrap();
    assert_eq!(found, Some(Began::Running("s9".into())));
}

#[cfg(unix)]
mod again;
#[cfg(unix)]
mod direct;
mod finishing;

/// Sends in its first turn; makes the same call twice in a turn it is told
/// of an approval; reports once it is told of a rejection, or was not held.
#[cfg(unix)]
const FAKE_CLAUDE_THAT_WAITS: &str = r#"#!/usr/bin/env perl
use strict;
use JSON::PP;
$| = 1;
my (@said, @told);
sub ask {
    my ($id, $request) = @_;
    print encode_json({ type => "control_request", request_id => $id, request => $request }), "\n";
    while (my $line = <STDIN>) {
        my $m = decode_json($line);
        next unless ($m->{type} // "") eq "control_response";
        return $m->{response}{response};
    }
}
my $calls = 0;
sub send_it {
    $calls++;
    my $r = ask("call-$calls", { subtype => "mcp_message", server_name => "adroit",
        message => { jsonrpc => "2.0", id => $calls, method => "tools/call",
                     params => { name => "send_to_session", arguments => { session => "s", prompt => "p" } } } });
    $r->{mcp_response}{result}{content}[0]{text} // "";
}
while (my $line = <STDIN>) {
    next unless $line =~ /"type":"user"/;
    my $told = decode_json($line)->{message}{content};
    push @told, $told;
    push @said, send_it() unless $told =~ /rejected/;
    push @said, send_it() if $told =~ /was approved/;
    my $result = JSON::PP->new->canonical->encode({ said => \@said, told => \@told });
    print encode_json({ type => "result", is_error => JSON::PP::false, session_id => "w",
        result => $result, usage => { input_tokens => 1, output_tokens => 1 } }), "\n";
}
"#;

#[cfg(unix)]
async fn held_by(app: &App, pool: &Pool, run: i64, after: i64) -> famulen_gateway::Ticket {
    for _ in 0..200 {
        pool.tick().await.unwrap();
        let waiting = app
            .workers
            .runs
            .get(run)
            .unwrap()
            .is_some_and(|r| r.state == State::Waiting);
        let ticket = app
            .gateway
            .tickets_of(crate::permissions::job_actor(run))
            .unwrap()
            .into_iter()
            .find(|t| t.id > after && t.status == "held");
        if let Some(ticket) = ticket.filter(|_| waiting) {
            return ticket;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("it never waited");
}

#[cfg(unix)]
#[tokio::test]
async fn an_agent_told_to_wait_goes_on_once_answered_and_its_approval_goes_through_once() {
    let dir = TempDir::new("pool-agent-waits");
    let fields = [
        ("Runner", "agent"),
        ("Tools", "mcp__adroit__send_to_session"),
        ("Wait", "yes"),
    ];
    let (app, pool, _) = agent_app(&dir, FAKE_CLAUDE_THAT_WAITS, &fields);

    let first = held_by(&app, &pool, 1, 0).await;
    // A call that waits was not made: nothing of others' was read.
    assert!(!crate::workers::notebook::read_outside(&app.store, 1));
    app.gateway
        .answer_ticket(first.id, famulen_gateway::Answer::Approve)
        .await
        .unwrap();
    // The same call made again used the approval up: the next is held anew.
    let second = held_by(&app, &pool, 1, first.id).await;
    assert!(!app.gateway.has_pass(first.id).unwrap());
    app.gateway
        .answer_ticket(second.id, famulen_gateway::Answer::Reject)
        .await
        .unwrap();
    for _ in 0..200 {
        pool.tick().await.unwrap();
        if app.workers.runs.open().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let run = app.workers.runs.get(1).unwrap().unwrap();
    let got: serde_json::Value = serde_json::from_str(&run.result).unwrap();
    let told: Vec<&str> = got["told"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t.as_str())
        .collect();
    assert_eq!(told.len(), 3, "{got}");
    assert!(
        told[1].contains(&format!("#{} ", first.id)) && told[1].contains("approved"),
        "{got}"
    );
    assert!(told[2].contains("rejected"), "{got}");
}

const FAKE_CLAUDE_THAT_READS: &str = r#"#!/usr/bin/env perl
use strict;
use JSON::PP;
$| = 1;
sub ask {
    my ($id, $request) = @_;
    print encode_json({ type => "control_request", request_id => $id, request => $request }), "\n";
    while (my $line = <STDIN>) {
        my $m = decode_json($line);
        next unless ($m->{type} // "") eq "control_response";
        return $m->{response}{response};
    }
}
my $calls = 0;
sub call {
    my ($name, $arguments) = @_;
    $calls++;
    my $r = ask("call-$calls", { subtype => "mcp_message", server_name => "famulen",
        message => { jsonrpc => "2.0", id => $calls, method => "tools/call",
                     params => { name => $name, arguments => $arguments } } });
    $r->{mcp_response}{result}{content}[0]{text} // "";
}
while (my $line = <STDIN>) {
    next unless $line =~ /"type":"user"/;
    my %got = (
        may => ask("use-read", { subtype => "can_use_tool",
            tool_name => "mcp__famulen__notebook_read", input => {} })->{behavior},
        named => call("notebook_read", { path => "records/memos/README.md" }),
        other => call("notebook_read", { path => "records/tasks/README.md" }),
        search => call("notebook_search", { text => "memo" }),
        proposed => call("notebook_propose", { title => "learned", content => "a fact" }),
    );
    print encode_json({ type => "result", is_error => JSON::PP::false, session_id => "r",
        result => JSON::PP->new->canonical->encode(\%got),
        usage => { input_tokens => 1, output_tokens => 1 } }), "\n";
}
"#;

#[cfg(unix)]
#[tokio::test]
async fn an_agent_reads_what_was_named_and_what_it_proposes_goes_to_the_secretary_unwritten() {
    let fields = [
        ("Runner", "agent"),
        ("Notebook", "propose"),
        ("Files", "records/memos/README.md"),
    ];
    let dir = TempDir::new("pool-agent-reads");
    let (app, pool, _) = agent_app(&dir, FAKE_CLAUDE_THAT_READS, &fields);
    approve_until_over(&app, &pool).await;

    let run = app.workers.runs.get(1).unwrap().unwrap();
    let got: serde_json::Value = serde_json::from_str(&run.result).unwrap();
    assert_eq!(got["may"], "allow", "{got}");
    let named =
        std::fs::read_to_string(app.notebook.root().join("records/memos/README.md")).unwrap();
    assert_eq!(got["named"], named.as_str());
    assert!(got["other"].as_str().unwrap().contains("may read"), "{got}");
    assert!(got["search"].as_str().unwrap().contains("no tool"), "{got}");
    assert!(app.records.list(crate::records::MEMOS).unwrap().is_empty());
    let told = app.store.recent_inputs(0, 10).unwrap();
    assert!(
        told.iter()
            .any(|(i, _)| i.input.text.contains("## learned\na fact")),
        "the proposal is told to the secretary"
    );
    assert!(crate::workers::notebook::proposals(&app.store, 1).is_empty());
}

#[cfg(unix)]
async fn approve_until_over(app: &App, pool: &Pool) {
    for _ in 0..200 {
        pool.tick().await.unwrap();
        for ticket in app.gateway.held_tickets().unwrap() {
            let approve = crate::approvals::Answer::Approve;
            app.gateway
                .answer_ticket(ticket.id, approve.into())
                .await
                .unwrap();
        }
        if app.workers.runs.open().unwrap().is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the work did not end");
}

#[cfg(unix)]
#[tokio::test]
async fn work_that_mixes_the_notebook_with_others_words_starts_only_once_the_owner_approves() {
    let script = r#"#!/bin/sh
read -r init; read -r ask
echo '{"type":"result","subtype":"success","is_error":false,"result":"done"}'
"#;
    let fields = [
        ("Runner", "agent"),
        ("Files", "records/memos/README.md"),
        ("Tools", "mcp__adroit__list_sessions"),
    ];
    for (approve, ended) in [(true, "done"), (false, "failed")] {
        let dir = TempDir::new(&format!("pool-agent-mixes-{approve}"));
        let (app, pool, _) = agent_app(&dir, script, &fields);
        pool.tick().await.unwrap();
        pool.tick().await.unwrap();

        let run = app.workers.runs.get(1).unwrap().unwrap();
        assert_eq!(run.state, State::Asking);
        assert!(!app.workers.dir.join("1").exists(), "nothing was started");
        let held = app.gateway.held_tickets().unwrap();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].actor, crate::permissions::job_actor(1));

        if approve {
            approve_until_over(&app, &pool).await;
        } else {
            let reject = crate::approvals::Answer::Reject;
            app.gateway
                .answer_ticket(held[0].id, reject.into())
                .await
                .unwrap();
            pool.tick().await.unwrap();
        }
        let run = app.workers.runs.get(1).unwrap().unwrap();
        assert_eq!(run.result == "done", ended == "done", "{}", run.result);
        assert!(app.workers.runs.open().unwrap().is_empty());
    }
}

const FAKE_CLAUDE_THAT_MIXES: &str = r#"#!/usr/bin/env perl
use strict;
use JSON::PP;
$| = 1;
my $asked = 0;
sub ask {
    my ($request) = @_;
    my $id = "ask-" . ++$asked;
    print encode_json({ type => "control_request", request_id => $id, request => $request }), "\n";
    while (my $line = <STDIN>) {
        my $m = decode_json($line);
        next unless ($m->{type} // "") eq "control_response";
        return $m->{response}{response};
    }
}
sub call {
    my ($server, $name, $arguments) = @_;
    my $r = ask({ subtype => "mcp_message", server_name => $server,
        message => { jsonrpc => "2.0", id => $asked, method => "tools/call",
                     params => { name => $name, arguments => $arguments } } });
    $r->{mcp_response}{result}{content}[0]{text} // "";
}
while (my $line = <STDIN>) {
    next unless $line =~ /"type":"user"/;
    my %got;
    $got{before} = call("famulen", "notebook_read", { path => "records/memos/README.md" });
    call("adroit", "list_sessions", {});
    $got{after} = call("famulen", "notebook_read", { path => "records/memos/README.md" });
    $got{send} = call("adroit", "send_to_session", { session => "s", prompt => "p" });
    $got{web} = ask({ subtype => "can_use_tool", tool_name => "WebSearch",
        input => { query => "x" } })->{behavior};
    print encode_json({ type => "result", is_error => JSON::PP::false, session_id => "m",
        result => JSON::PP->new->canonical->encode(\%got),
        usage => { input_tokens => 1, output_tokens => 1 } }), "\n";
}
"#;

#[cfg(unix)]
#[tokio::test]
async fn work_that_read_others_words_reads_no_more_of_the_notebook_and_sends_nothing_unasked() {
    let fields = [
        ("Runner", "agent"),
        ("Files", "records/memos/README.md"),
        (
            "Tools",
            "WebSearch, mcp__adroit__list_sessions, mcp__adroit__send_to_session",
        ),
    ];
    let dir = TempDir::new("pool-agent-after-outside");
    let (app, pool, id) = agent_app(&dir, FAKE_CLAUDE_THAT_MIXES, &fields);
    let task = app.records.need(TASKS, &id).unwrap();
    crate::workers::written(&app, &task, Writer::Owner).unwrap();
    app.gateway
        .set_permission(
            famulen_gateway::OWNER,
            "conn:adroit",
            "send_to_session",
            Some(famulen_gateway::permissions::Permission::Allow),
        )
        .unwrap();
    approve_until_over(&app, &pool).await;

    let run = app.workers.runs.get(1).unwrap().unwrap();
    let answer = run.result.split("\n\n").next().unwrap();
    let got: serde_json::Value = serde_json::from_str(answer).unwrap();
    assert!(
        got["before"].as_str().unwrap().contains("code: memo"),
        "{got}"
    );
    assert!(
        got["after"].as_str().unwrap().contains("others wrote"),
        "{got}"
    );
    assert!(got["send"].as_str().unwrap().contains("approval"), "{got}");
    assert_eq!(got["web"], "deny", "{got}");
}

fn waits_on(decision: &str, open: bool) -> Left {
    Left {
        open,
        blocked: vec![decision.into()],
        ..Left::default()
    }
}

async fn delegated_to_adroit(name: &str) -> (Bench, String) {
    let b = bench(name, Fake::default());
    let id = b.adroit("直す");
    b.pool.tick().await.unwrap();
    (b, id)
}

#[tokio::test]
async fn a_session_left_idle_is_asked_to_go_on_only_after_a_while() {
    let (b, id) = delegated_to_adroit("pool-nudge").await;

    b.turn_ends("s1", "直す", Looked::Idle(Left::default()))
        .await;
    b.turn_ends("s1", "直す", Looked::Idle(Left::default()))
        .await;
    assert_eq!(b.asked_on(), 0);

    b.idle_over().await;
    assert_eq!(b.asked_on(), 1);
    assert_eq!(*b.fake.after.lock().unwrap(), [After::Idle]);
    assert!(b.log(&id).iter().any(|l| l.contains("続けるよう伝えた")));
    assert!(b.told().is_empty());
}

#[tokio::test]
async fn a_session_that_goes_on_by_itself_is_waited_for_anew() {
    let (b, _) = delegated_to_adroit("pool-nudge-anew").await;
    b.turn_ends("s1", "直す", Looked::Idle(Left::default()))
        .await;
    b.turn_ends("s1", "直す", Looked::Working).await;
    b.idle_over().await;
    b.turn_ends("s1", "直す", Looked::Idle(Left::default()))
        .await;
    assert_eq!(b.asked_on(), 0);
}

#[tokio::test]
async fn a_session_that_waits_or_that_the_owner_stopped_is_not_asked_to_go_on() {
    let stopped = Left {
        interrupted: true,
        ..Left::default()
    };
    for (name, looked) in [
        ("pool-nudge-waiting", Looked::Waiting),
        ("pool-nudge-working", Looked::Working),
        ("pool-nudge-stopped", Looked::Idle(stopped)),
    ] {
        let (b, _) = delegated_to_adroit(name).await;
        b.turn_ends("s1", "直す", looked).await;
        b.idle_over().await;
        b.idle_over().await;
        assert_eq!(b.asked_on(), 0, "{name}");
        assert!(b.told().is_empty(), "{name}");
    }
}

#[tokio::test]
async fn nothing_is_asked_to_go_on_while_delegation_is_paused() {
    let (b, _) = delegated_to_adroit("pool-nudge-paused").await;
    b.pause(true);
    b.stays_idle(Left::default()).await;
    b.idle_over().await;
    assert_eq!(b.asked_on(), 0);

    b.pause(false);
    b.stays_idle(Left::default()).await;
    assert_eq!(b.asked_on(), 1);
}

#[tokio::test]
async fn a_plan_that_only_waits_on_decisions_is_told_to_the_secretary_once() {
    let (b, id) = delegated_to_adroit("pool-nudge-blocked").await;
    b.stays_idle(waits_on("マージ: 直さずに入れるか", false))
        .await;
    b.idle_over().await;
    b.stays_idle(waits_on("マージ: 直さずに入れるか", false))
        .await;

    assert_eq!(b.asked_on(), 0);
    let told = b.told();
    assert_eq!(told.len(), 1);
    assert!(
        told[0].text.contains("直さずに入れるか"),
        "{}",
        told[0].text
    );
    assert_eq!(b.status(&id), "in_progress");
    assert_eq!(b.runs()[0].state, State::Idle);
}

#[tokio::test]
async fn steps_that_wait_on_no_decision_are_gone_on_with() {
    let (b, _) = delegated_to_adroit("pool-nudge-partly").await;
    b.stays_idle(waits_on("マージ: 入れるか", true)).await;
    assert_eq!(b.asked_on(), 1);
    assert_eq!(b.told().len(), 1);
}

#[tokio::test]
async fn asking_without_progress_is_given_up_and_the_secretary_told() {
    let (b, id) = delegated_to_adroit("pool-nudge-cap").await;
    for _ in 0..6 {
        b.stays_idle(Left::default()).await;
        b.turn_ends("s1", "直す", Looked::Working).await;
    }
    assert_eq!(b.asked_on(), 3);
    assert_eq!(b.told().len(), 1);
    assert_eq!(b.status(&id), "in_progress");
}

#[tokio::test]
async fn progress_of_the_plan_counts_anew() {
    let (b, _) = delegated_to_adroit("pool-nudge-progress").await;
    for done in 0..5 {
        b.stays_idle(Left {
            done,
            ..Left::default()
        })
        .await;
        b.turn_ends("s1", "直す", Looked::Working).await;
    }
    assert_eq!(b.asked_on(), 5);
    assert!(b.told().is_empty());
}

#[tokio::test]
async fn a_message_not_taken_in_is_not_followed_by_another_and_is_told() {
    let (b, _) = delegated_to_adroit("pool-nudge-undelivered").await;
    b.stays_idle(Left::default()).await;
    let left = Left {
        undelivered: true,
        ..Left::default()
    };
    b.stays_idle(left.clone()).await;
    b.stays_idle(left).await;

    assert_eq!(b.asked_on(), 1);
    assert_eq!(b.told().len(), 1);
}

#[tokio::test]
async fn a_message_not_taken_in_is_told_once_though_the_plan_moved() {
    let (b, _) = delegated_to_adroit("pool-nudge-moved-undelivered").await;
    b.stays_idle(Left::default()).await;
    let left = Left {
        done: 1,
        undelivered: true,
        ..Left::default()
    };
    for _ in 0..3 {
        b.stays_idle(left.clone()).await;
    }
    assert_eq!(b.told().len(), 1);
}

#[tokio::test]
async fn a_turn_not_seen_going_by_is_waited_for_anew() {
    let (b, _) = delegated_to_adroit("pool-nudge-new-turn").await;
    b.turn_ends("s1", "直す", Looked::Idle(Left::default()))
        .await;
    b.app
        .store
        .conn()
        .execute("UPDATE worker_idle SET due_at = 1", [])
        .unwrap();
    let next = Left {
        turn: 2,
        ..Left::default()
    };
    b.turn_ends("s1", "直す", Looked::Idle(next)).await;
    assert_eq!(b.asked_on(), 0);
}

#[tokio::test]
async fn a_restart_keeps_the_count_and_asks_nothing_twice() {
    let (mut b, _) = delegated_to_adroit("pool-nudge-restart").await;
    for _ in 0..2 {
        b.stays_idle(Left::default()).await;
        b.pool = Pool::new(b.app.clone(), b.fake.clone());
        b.turn_ends("s1", "直す", Looked::Idle(Left::default()))
            .await;
    }
    assert_eq!(b.asked_on(), 2);
    for _ in 0..4 {
        b.pool = Pool::new(b.app.clone(), b.fake.clone());
        b.stays_idle(Left::default()).await;
    }
    assert_eq!(b.asked_on(), 3);
    assert_eq!(b.told().len(), 1);
}
