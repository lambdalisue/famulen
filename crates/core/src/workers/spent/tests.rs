use std::io::Write as _;

use chrono::{NaiveDate, Utc};
use chrono_tz::Asia::Tokyo;

use super::*;
use crate::models::{ModelKind, ModelSpec, Role, UsageReport, UsageRow, UsageSpan};
use crate::workers::{Order, Runner, Runs};

struct Desk {
    dir: PathBuf,
    store: Store,
    models: Models,
    sessions: Sessions,
}

fn unique() -> String {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{next}", std::process::id())
}

impl Drop for Desk {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Desk {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("famulen-spent-{}", unique()));
        fs::create_dir_all(&dir).unwrap();
        let store = Store::open_in_memory().unwrap();
        let models = Models::new(store.clone());
        let logs = Logs {
            claude: Some(dir.join("projects")),
            codex: Some(dir.join("sessions")),
        };
        let sessions = Sessions::new(store.clone(), models.clone(), logs);
        Self {
            dir,
            store,
            models,
            sessions,
        }
    }

    /// A run whose session is named by `task-<code>-run<id>`.
    fn run(&self, code: &str, directory: &str) -> i64 {
        let order = Order {
            code: format!("task-{code}"),
            directory: directory.into(),
            runner: Runner::Adroit,
            ..Order::default()
        };
        let runs = Runs::new(self.store.clone());
        runs.begin(code, &order).unwrap().id
    }

    fn write(&self, path: &str, lines: &[String]) {
        let path = self.dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = File::options()
            .append(true)
            .create(true)
            .open(path)
            .unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    fn report(&self) -> UsageReport {
        let day: NaiveDate = "2026-11-26".parse().unwrap();
        self.models
            .usage(UsageSpan::Month, day, Tokyo, Utc::now())
            .unwrap()
    }

    fn rows(&self) -> Vec<UsageRow> {
        self.report().rows
    }
}

fn said(id: &str, model: &str, output: u64) -> String {
    format!(
        r#"{{"type":"assistant","timestamp":"2026-11-25T16:06:18.539Z","message":{{"id":"{id}","model":"{model}","content":[{{"type":"text","text":"secret words"}}],"usage":{{"input_tokens":2,"cache_creation_input_tokens":30,"cache_read_input_tokens":68,"output_tokens":{output}}}}}}}"#
    )
}

fn title(text: &str) -> String {
    format!(r#"{{"type":"custom-title","customTitle":"{text}","sessionId":"s"}}"#)
}

fn begun_in(cwd: &str) -> String {
    format!(r#"{{"type":"session_meta","payload":{{"id":"t","cwd":"{cwd}"}}}}"#)
}

fn turn(model: &str) -> String {
    format!(
        r#"{{"timestamp":"2026-11-25T16:00:00Z","type":"turn_context","payload":{{"model":"{model}"}}}}"#
    )
}

fn answered(id: &str) -> String {
    format!(
        r#"{{"timestamp":"2026-11-25T16:00:01Z","type":"token_usage_record","payload":{{"response_id":"{id}","usage":{{"input_tokens":50,"cached_input_tokens":40,"output_tokens":5}}}}}}"#
    )
}

#[test]
fn what_works_in_the_worktree_of_a_run_is_counted_as_the_workers_with_its_subagents() {
    let mut desk = Desk::new();
    let run = desk.run("abc123", "/repo");
    let dir = format!("projects/-repo--claude-worktrees-task-abc123-run{run}-some-words");
    desk.write(
        &format!("{dir}/s1.jsonl"),
        &[
            said("m1", "claude-opus-5-5", 10),
            said("m2", "claude-opus-5-5", 5),
        ],
    );
    desk.write(
        &format!("{dir}/s1/subagents/agent-a.jsonl"),
        &[said("m3", "claude-haiku-5-5", 1)],
    );
    desk.sessions.count().unwrap();
    let report = desk.report();
    let rows: Vec<_> = report
        .rows
        .iter()
        .map(|r| {
            (
                r.role,
                r.model.as_str(),
                r.calls,
                r.input_tokens,
                r.output_tokens,
                r.cached_tokens,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (Role::Worker, "claude-haiku-5-5", 1, 100, 1, Some(68)),
            (Role::Worker, "claude-opus-5-5", 1, 200, 15, Some(136)),
        ]
    );
    assert_eq!(report.runs.len(), 1);
    assert_eq!(report.runs[0].run, run);
    assert_eq!(report.runs[0].rows, report.rows);
    assert_eq!(desk.models.run_usage(run).unwrap(), report.rows);
}

#[test]
fn what_goes_on_is_counted_once_however_often_it_is_told() {
    let mut desk = Desk::new();
    let run = desk.run("abc123", "/repo");
    let file = format!("projects/-repo-task-abc123-run{run}/s1.jsonl");
    desk.write(&file, &[said("m1", "claude-opus-5-5", 1)]);
    desk.sessions.count().unwrap();
    desk.sessions.count().unwrap();
    desk.write(
        &file,
        &[
            said("m1", "claude-opus-5-5", 9),
            said("m2", "claude-opus-5-5", 1),
        ],
    );
    desk.sessions.count().unwrap();
    // A conversation branched off repeats what came before it.
    desk.write(
        &format!("projects/-repo-task-abc123-run{run}/s2.jsonl"),
        &[
            said("m1", "claude-opus-5-5", 9),
            said("m3", "claude-opus-5-5", 1),
        ],
    );
    desk.sessions.count().unwrap();
    let rows = desk.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].calls, rows[0].output_tokens), (2, 11));
}

#[test]
fn a_restart_goes_on_from_where_it_was_read_to() {
    let mut desk = Desk::new();
    let run = desk.run("abc123", "/repo");
    let cwd = format!("/repo/.claude/worktrees/task-abc123-run{run}");
    let file = "sessions/2026/11/25/rollout-a.jsonl";
    desk.write(file, &[begun_in(&cwd), turn("gpt-6.1-sol"), answered("r1")]);
    desk.sessions.count().unwrap();
    desk.sessions = Sessions::new(
        desk.store.clone(),
        desk.models.clone(),
        desk.sessions.logs.clone(),
    );
    desk.write(file, &[answered("r2")]);
    desk.sessions.count().unwrap();
    let rows = desk.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].model, "codex-gpt-6.1-sol");
    assert_eq!((rows[0].calls, rows[0].input_tokens), (1, 100));
    assert_eq!(
        (rows[0].output_tokens, rows[0].cached_tokens),
        (10, Some(80))
    );
}

#[test]
fn a_line_being_written_is_counted_once_it_is_whole() {
    let mut desk = Desk::new();
    let run = desk.run("abc123", "/repo");
    let file = format!("projects/-repo-task-abc123-run{run}/s1.jsonl");
    desk.write(&file, &[said("m1", "claude-opus-5-5", 1)]);
    let whole = said("m2", "claude-opus-5-5", 2);
    let (head, tail) = whole.split_at(40);
    let path = desk.dir.join(&file);
    let mut open = File::options().append(true).open(path).unwrap();
    write!(open, "{head}").unwrap();
    desk.sessions.count().unwrap();
    assert_eq!(desk.rows()[0].output_tokens, 1);
    writeln!(open, "{tail}").unwrap();
    desk.sessions.count().unwrap();
    assert_eq!(desk.rows()[0].output_tokens, 3);
}

#[test]
fn without_a_worktree_only_what_is_titled_by_the_run_is_its_own() {
    let mut desk = Desk::new();
    let run = desk.run("abc123", "/home/me/repo");
    desk.write(
        "projects/-home-me-repo/mine.jsonl",
        &[
            title(&format!("task-abc123-run{run}-some-words")),
            said("m1", "claude-opus-5-5", 1),
        ],
    );
    desk.write(
        "projects/-home-me-repo/mine/subagents/agent-a.jsonl",
        &[said("m2", "claude-opus-5-5", 2)],
    );
    desk.write(
        "projects/-home-me-repo/owners.jsonl",
        &[title("my own work"), said("m3", "claude-opus-5-5", 4)],
    );
    desk.write(
        "projects/-home-me-repo/owners/subagents/agent-b.jsonl",
        &[said("m4", "claude-opus-5-5", 8)],
    );
    desk.write(
        "sessions/2026/11/25/rollout-a.jsonl",
        &[
            begun_in("/home/me/repo"),
            turn("gpt-6.1-sol"),
            answered("r1"),
        ],
    );
    desk.sessions.count().unwrap();
    let rows = desk.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].calls, rows[0].output_tokens), (2, 3));
}

#[test]
fn a_conversation_titled_after_it_was_looked_at_is_counted_whole() {
    let mut desk = Desk::new();
    let run = desk.run("abc123", "/repo");
    let file = "projects/-repo/s1.jsonl";
    desk.write(file, &[said("m1", "claude-opus-5-5", 1)]);
    desk.sessions.count().unwrap();
    assert!(desk.rows().is_empty());
    desk.write(file, &[title(&format!("task-abc123-run{run}"))]);
    desk.sessions.count().unwrap();
    assert_eq!(desk.rows()[0].output_tokens, 1);
}

#[test]
fn each_run_of_a_task_has_its_own_and_one_is_not_taken_for_another() {
    let mut desk = Desk::new();
    let first = desk.run("abc123", "/repo");
    assert_eq!(first, 1);
    for run in [1, 12] {
        desk.write(
            &format!("projects/-repo-task-abc123-run{run}-x/s.jsonl"),
            &[said(&format!("m{run}"), "claude-opus-5-5", run)],
        );
    }
    desk.sessions.count().unwrap();
    assert_eq!(desk.rows()[0].output_tokens, 1);
}

#[test]
fn what_a_session_used_takes_nothing_of_the_workers_cap() {
    let mut desk = Desk::new();
    let run = desk.run("abc123", "/repo");
    desk.write(
        &format!("projects/-repo-task-abc123-run{run}/s1.jsonl"),
        &[said("m1", "claude-opus-5-5", 1)],
    );
    desk.sessions.count().unwrap();
    let now = "2026-11-25T17:00:00Z".parse().unwrap();
    let used = desk.models.used_today(Role::Worker, Tokyo, now).unwrap();
    assert_eq!(used, (0.0, 0));
    assert_eq!(desk.rows()[0].calls, 1);
}

#[test]
fn a_priced_model_costs_what_its_tokens_do() {
    let mut desk = Desk::new();
    let spec = ModelSpec {
        id: "claude-opus-5-5".into(),
        name: "Opus".into(),
        kind: ModelKind::AgentCli,
        cli: Default::default(),
        endpoint: None,
        model: "claude-opus-5-5".into(),
        price: Price {
            input_per_mtok_yen: Some(10_000.0),
            ..Price::default()
        },
    };
    desk.models.register(spec, None).unwrap();
    let run = desk.run("abc123", "/repo");
    desk.write(
        &format!("projects/-repo-task-abc123-run{run}/s1.jsonl"),
        &[said("m1", "claude-opus-5-5", 1)],
    );
    desk.sessions.count().unwrap();
    let rows = desk.rows();
    assert_eq!((rows[0].cost_yen, rows[0].metered), (1.0, true));
}

#[test]
fn nothing_but_the_counts_is_kept() {
    let mut desk = Desk::new();
    let run = desk.run("abc123", "/repo");
    desk.write(
        &format!("projects/-repo-task-abc123-run{run}/s1.jsonl"),
        &[said("m1", "claude-opus-5-5", 1)],
    );
    desk.sessions.count().unwrap();
    let copy = desk.dir.join("copy.sqlite3");
    desk.store
        .conn()
        .execute("VACUUM INTO ?1", [copy.to_string_lossy()])
        .unwrap();
    let kept = fs::read(copy).unwrap();
    assert!(!kept.windows(12).any(|w| w == b"secret words"));
}

#[test]
fn the_logs_are_where_the_agents_are_told_to_keep_them() {
    let logs = |set: &[(&str, &str)]| {
        let set: HashMap<String, String> = set
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Logs::from_env(|name| set.get(name).cloned())
    };
    let home = logs(&[("HOME", "/h")]);
    assert_eq!(home.claude, Some("/h/.claude/projects".into()));
    assert_eq!(home.codex, Some("/h/.codex/sessions".into()));
    let set = logs(&[("CLAUDE_CONFIG_DIR", "/c"), ("CODEX_HOME", "/x")]);
    assert_eq!(set.claude, Some("/c/projects".into()));
    assert_eq!(set.codex, Some("/x/sessions".into()));
}

#[test]
fn of_worktrees_in_one_another_the_inner_ones_run_is_counted() {
    let mut desk = Desk::new();
    let outer = desk.run("abc123", "/repo");
    let inner = desk.run("def456", "/repo");
    let work = format!("task-abc123-run{outer}/.claude/worktrees/task-def456-run{inner}");
    desk.write(
        &format!(
            "projects/{}/s.jsonl",
            project(Path::new(&format!("/r/{work}")))
        ),
        &[said("m1", "claude-opus-5-5", 1)],
    );
    desk.write(
        "sessions/2026/11/25/rollout-a.jsonl",
        &[
            begun_in(&format!("/r/{work}")),
            turn("gpt-6.1-sol"),
            answered("r1"),
        ],
    );
    desk.sessions.count().unwrap();
    let runs = desk.report().runs;
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].run, runs[0].rows.len()), (inner, 2));
}
