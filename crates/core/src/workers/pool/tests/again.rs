//! Agents go on while Famulen is started again.

use std::path::Path;

use tokio::runtime::Runtime;

use super::*;

/// One start of Famulen: all it runs ends when it is dropped, at whatever
/// it was doing, and what agents run under stays.
struct Famulen {
    runtime: Option<Runtime>,
    app: App,
    pool: Pool,
}

impl Famulen {
    fn start(dir: &Path) -> Self {
        let runtime = Runtime::new().unwrap();
        let (app, pool) = runtime.block_on(async {
            let app = crate::runtime::testing::app(dir);
            let real = Arc::new(Real::new(app.clone()));
            real.sweep().await.unwrap();
            let pool = Pool::new(app.clone(), real);
            (app, pool)
        });
        Self {
            runtime: Some(runtime),
            app,
            pool,
        }
    }

    fn block_on<T>(&self, work: impl Future<Output = T>) -> T {
        self.runtime.as_ref().unwrap().block_on(work)
    }

    fn tick(&self) {
        self.block_on(self.pool.tick()).unwrap();
    }

    /// Let it work until `done`, looking at its runs all along.
    fn until(&self, what: &str, done: impl Fn(&App) -> bool) {
        for _ in 0..400 {
            self.tick();
            if done(&self.app) {
                return;
            }
            self.block_on(async { tokio::time::sleep(Duration::from_millis(50)).await });
        }
        panic!(
            "never: {what}; {:?}",
            self.app.workers.runs.recent(9).unwrap()
        );
    }

    fn until_over(&self) {
        self.until("over", |app| app.workers.runs.open().unwrap().is_empty());
    }

    fn calls(&self) -> u32 {
        let worker = crate::models::Role::Worker;
        self.app
            .models
            .used_today(worker, self.app.tz(), Utc::now())
            .unwrap()
            .1
            .try_into()
            .unwrap()
    }

    fn tokens(&self) -> (i64, i64) {
        let conn = self.app.store.conn();
        conn.query_row(
            "SELECT SUM(input_tokens), SUM(output_tokens) FROM usage WHERE role = 'worker'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }
}

impl Drop for Famulen {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(5));
        }
    }
}

struct Bench {
    dir: TempDir,
    famulen: Option<Famulen>,
    task: String,
}

impl Bench {
    /// Famulen with one task, delegated to an agent that runs `script`, whose
    /// `DIR` is the bench's directory.
    fn new(name: &str, script: &str, fields: &[(&str, &str)]) -> Self {
        let dir = TempDir::new(name);
        let script = script.replace("DIR", &dir.path().display().to_string());
        let runtime = Runtime::new().unwrap();
        let (_, _, task) = runtime.block_on(async { agent_app(&dir, &script, fields) });
        drop(runtime);
        let famulen = Some(Famulen::start(dir.path()));
        Self { dir, famulen, task }
    }

    fn famulen(&self) -> &Famulen {
        self.famulen.as_ref().unwrap()
    }

    fn stop_famulen(&mut self) {
        self.famulen = None;
    }

    fn start_famulen(&mut self) -> &Famulen {
        self.famulen = Some(Famulen::start(self.dir.path()));
        self.famulen()
    }

    fn at(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join(name)
    }

    fn wait_for(&self, name: &str) -> String {
        for _ in 0..400 {
            match std::fs::read_to_string(self.at(name)) {
                Ok(text) if !text.is_empty() => return text,
                _ => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        panic!("{name} never came");
    }

    fn make(&self, name: &str) {
        std::fs::write(self.at(name), "x").unwrap();
    }

    fn status(&self) -> String {
        let task = self.famulen().app.records.need(TASKS, &self.task).unwrap();
        task.body.value("Status").unwrap_or_default()
    }

    fn run(&self) -> Run {
        self.famulen().app.workers.runs.recent(1).unwrap().remove(0)
    }

    fn texts(&self) -> Vec<String> {
        let sight = self.famulen().app.workers.sights.of(self.run().id);
        sight.lines.into_iter().map(|l| l.text).collect()
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        let Some(famulen) = &self.famulen else { return };
        for run in famulen.app.workers.runs.recent(9).unwrap() {
            let name = run.id.to_string();
            let executor = famulen.app.workers.executors.of(name);
            famulen.block_on(executor.forget()).ok();
        }
    }
}

/// Says one, asks while nobody is there to be asked, and tells what it was
/// answered before and then.
const ASKS_ALONE: &str = r#"#!/bin/sh
read -r init; read -r ask
echo '{"type":"system","subtype":"init","session_id":"c1"}'
echo '{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"one"}],"usage":{"input_tokens":10,"output_tokens":1}}}'
echo '{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Read","input":{"file_path":"a"}}}'
read -r first
echo "$first" > DIR/first
while [ ! -e DIR/alone ]; do sleep 0.05; done
echo '{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"two"}],"usage":{"input_tokens":10,"output_tokens":5}}}'
echo '{"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"}}}'
echo '{"type":"control_request","request_id":"r3","request":{"subtype":"can_use_tool","tool_name":"Read","input":{"file_path":"b"}}}'
echo x > DIR/asked
read -r second; read -r third
echo "$second" > DIR/second; echo "$third" > DIR/third
echo '{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"three"}],"usage":{"input_tokens":20,"output_tokens":2}}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"done","usage":{"input_tokens":30,"output_tokens":7}}'
read -r never
"#;

#[test]
fn an_agent_goes_on_and_what_it_wrote_and_asked_meanwhile_is_neither_lost_nor_doubled() {
    let mut b = Bench::new(
        "again",
        ASKS_ALONE,
        &[("Runner", "agent"), ("Tools", "Read, Bash")],
    );
    b.famulen().tick();
    b.famulen()
        .until("one", |app| !app.workers.sights.of(1).lines.is_empty());
    assert!(b.wait_for("first").contains("\"allow\""));
    b.stop_famulen();

    // Nobody is there when it goes on.
    b.make("alone");
    b.wait_for("asked");
    assert!(!b.at("second").exists());

    b.start_famulen();
    b.famulen().until_over();
    assert_eq!(b.status(), "done");
    assert_eq!(b.texts(), ["one", "two", "three"]);
    let answers = b.wait_for("second") + &b.wait_for("third");
    let given = |id: &str, behavior: &str| {
        answers
            .lines()
            .any(|l| l.contains(&format!("\"{id}\"")) && l.contains(behavior))
    };
    assert!(
        given("r2", "\"deny\"") && given("r3", "\"allow\""),
        "{answers}"
    );
    let run = b.run();
    assert_eq!(
        (run.id, run.attempt, run.outcome),
        (1, 1, Some(Outcome::Done))
    );
    assert!(run.result.starts_with("done"), "{}", run.result);
    assert_eq!(b.famulen().calls(), 1);
    assert_eq!(b.famulen().tokens(), (30, 7));
    assert_eq!(b.famulen().app.store.count_pending().unwrap(), 1);
}

#[test]
fn what_needed_the_owner_is_told_whichever_famulen_refused_it() {
    let script = r#"#!/bin/sh
read -r init; read -r ask
echo '{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"}}}'
read -r first; echo "$first" > DIR/first
while [ ! -e DIR/alone ]; do sleep 0.05; done
echo '{"type":"result","subtype":"success","is_error":false,"result":"done"}'
echo x > DIR/ended
read -r never
"#;
    let mut b = Bench::new(
        "again-unasked",
        script,
        &[("Runner", "agent"), ("Tools", "Bash")],
    );
    b.famulen().tick();
    b.wait_for("first");
    b.stop_famulen();
    b.make("alone");
    b.wait_for("ended");
    b.start_famulen();
    b.famulen().until_over();
    let result = b.run().result;
    assert!(
        result.contains("Not used") && result.contains("Bash"),
        "{result}"
    );
}

/// A service whose one tool writes down that it was called, and takes long.
const SLOW: &str = r#"
format = 1
id = "slow"
name = "Slow"

[transport.command]
program = "sh"

[[commands]]
name = "send"
description = "Send it."
args = ["-c", "echo x >> DIR/calls; sleep 20"]

[permissions]
send = "allow"
"#;

#[test]
fn a_call_under_way_when_famulen_went_away_is_not_made_again() {
    let script = r#"#!/bin/sh
read -r init; read -r ask
echo '{"type":"control_request","request_id":"r1","request":{"subtype":"mcp_message","server_name":"slow","message":{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"send","arguments":{}}}}}'
read -r answer; echo "$answer" > DIR/answer
echo '{"type":"result","subtype":"success","is_error":false,"result":"done"}'
read -r never
"#;
    let dir = TempDir::new("again-call");
    let here = dir.path().display().to_string();
    let services = dir.path().join("gateway/services");
    std::fs::create_dir_all(&services).unwrap();
    std::fs::write(services.join("slow.toml"), SLOW.replace("DIR", &here)).unwrap();
    let runtime = Runtime::new().unwrap();
    let task = runtime.block_on(async {
        let (app, _, task) = agent_app(
            &dir,
            &script.replace("DIR", &here),
            &[("Runner", "agent"), ("Tools", "mcp__slow__send")],
        );
        app.gateway
            .add_defined("slow", Default::default(), None)
            .unwrap();
        app.gateway.read_tools("slow").await.unwrap();
        let record = app.records.need(TASKS, &task).unwrap();
        let order = crate::workers::ordered(&app, &record, "t").unwrap();
        app.workers
            .runs
            .delegated(&task, &order.digest().unwrap(), "owner", "")
            .unwrap();
        task
    });
    drop(runtime);
    let mut b = Bench {
        famulen: Some(Famulen::start(dir.path())),
        dir,
        task,
    };
    b.famulen().tick();
    b.famulen().until("called", |_| b.at("calls").exists());
    b.stop_famulen();
    assert!(!b.at("answer").exists());

    b.start_famulen();
    b.famulen().until_over();
    assert!(b.wait_for("answer").contains("not known"));
    assert_eq!(std::fs::read_to_string(b.at("calls")).unwrap(), "x\n");
    assert_eq!(b.status(), "done");
}

const SLEEPS: &str = r#"#!/bin/sh
read -r init; read -r ask
echo $PPID > DIR/parent
echo $$ > DIR/pid
echo '{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"one"}]}}'
exec sleep 300
"#;

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

fn gone(pid: &str) {
    for _ in 0..400 {
        if !alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("{pid} is still there");
}

#[test]
fn an_agent_whose_task_was_cancelled_meanwhile_is_ended_and_leaves_nothing() {
    let mut b = Bench::new(
        "again-cancel",
        SLEEPS,
        &[("Runner", "agent"), ("Files", "records/memos/README.md")],
    );
    b.famulen().tick();
    let pid = b.wait_for("pid");
    b.stop_famulen();
    assert!(alive(&pid));
    b.start_famulen();
    b.famulen()
        .app
        .records
        .change(
            Writer::Owner,
            TASKS,
            &b.task,
            Patch::default().with("Status", "cancelled"),
        )
        .unwrap();
    b.famulen().until_over();
    assert!(!b.famulen().app.workers.dir.join("1").exists());
    b.famulen().until("ended", |_| !alive(&pid));
    b.famulen().until("forgotten", |app| {
        app.workers.executors.names().unwrap().is_empty()
    });
    assert_eq!(b.run().outcome, Some(Outcome::Stopped));
}

#[test]
fn what_no_open_run_wants_is_ended_when_famulen_starts() {
    let mut b = Bench::new("again-sweep", SLEEPS, &[("Runner", "agent")]);
    b.famulen().tick();
    let pid = b.wait_for("pid");
    b.famulen().app.workers.runs.ended(1).unwrap();
    b.stop_famulen();
    let famulen = b.start_famulen();
    assert!(famulen.app.workers.executors.names().unwrap().is_empty());
    assert!(!famulen.app.workers.dir.join("1").exists());
    gone(&pid);
}

#[test]
fn an_agent_whose_executor_went_away_is_run_once_more() {
    let mut b = Bench::new("again-gone", SLEEPS, &[("Runner", "agent")]);
    b.famulen().tick();
    let (pid, parent) = (b.wait_for("pid"), b.wait_for("parent"));
    b.stop_famulen();
    let killed = std::process::Command::new("kill")
        .args(["-9", parent.trim()])
        .status()
        .unwrap();
    assert!(killed.success());
    std::fs::remove_file(b.at("pid")).unwrap();

    b.start_famulen();
    b.famulen().until("again", |app| {
        app.workers.runs.recent(9).unwrap().len() == 2
    });
    let runs = b.famulen().app.workers.runs.recent(9).unwrap();
    assert_eq!(runs[1].outcome, Some(Outcome::Interrupted));
    assert_eq!((runs[0].attempt, runs[0].state), (2, State::Running));
    gone(&pid);
    b.wait_for("pid");
}

#[test]
fn a_start_famulen_went_away_from_is_found_and_not_made_twice() {
    let mut b = Bench::new("again-start", SLEEPS, &[("Runner", "agent")]);
    b.famulen().tick();
    b.wait_for("pid");
    let app = &b.famulen().app;
    // As it stood before the start was known to have gone through.
    app.workers.runs.unstarted(1).unwrap();
    app.records
        .change(
            Writer::Owner,
            TASKS,
            &b.task,
            Patch::default().with("Status", "pending"),
        )
        .unwrap();
    b.stop_famulen();
    b.start_famulen();
    b.famulen().until("running", |app| {
        app.workers.runs.recent(1).unwrap()[0].state == State::Running
    });
    assert_eq!(b.famulen().app.workers.runs.recent(9).unwrap().len(), 1);
    assert_eq!(b.famulen().calls(), 1);
}
