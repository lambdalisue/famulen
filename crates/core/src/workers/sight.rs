//! What can be seen of a run while it goes on: the plan its worker keeps, and
//! what it wrote and called. Kept in memory alone; it is the worker's words,
//! and nothing else is decided by it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::agents::{Body, PlanStep};

/// How many runs are kept, the newest.
const MOST_RUNS: usize = 50;
/// How many lines of one run are kept, the newest.
const MOST_LINES: usize = 500;
/// How much of one line is kept.
const LINE_CHARS: usize = 6000;
/// How much is said of what a run does now.
const DOING_CHARS: usize = 120;

/// One step of a worker's plan.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Step {
    pub id: String,
    /// The step it is part of, or nothing.
    pub parent: String,
    pub title: String,
    /// `pending`, `in_progress`, `completed` or `blocked`.
    pub status: String,
    /// Its size beside its siblings.
    pub weight: f64,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// What the worker was asked.
    Asked,
    /// What it wrote.
    Said,
    /// A tool it called.
    Called,
    /// What a tool answered.
    Answered,
}

impl LineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LineKind::Asked => "asked",
            LineKind::Said => "said",
            LineKind::Called => "called",
            LineKind::Answered => "answered",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// What tells it from the other lines of its run, however many are read
    /// again or left out.
    pub id: u64,
    /// Unix milliseconds.
    pub at: i64,
    /// The turn of the conversation it is in.
    pub turn: u64,
    pub kind: LineKind,
    pub text: String,
}

impl Line {
    pub fn new(at: i64, turn: u64, kind: LineKind, text: &str) -> Self {
        Self {
            id: 0,
            at,
            turn,
            kind,
            text: text.trim().chars().take(LINE_CHARS).collect(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sight {
    pub steps: Vec<Step>,
    pub lines: Vec<Line>,
    /// What an Adroit session waits on the owner for, as Adroit tells it.
    pub question: Option<Value>,
    /// The question answered last: what was read before it was answered may
    /// come after.
    answered: String,
    /// The id the newest line was given.
    last: u64,
}

impl Sight {
    /// How much of the plan is done, from 0 to 1; `None` without a plan.
    pub fn done(&self) -> Option<f64> {
        fn share(steps: &[Step], parent: &str, depth: usize) -> Option<f64> {
            // A plan that goes in circles is not followed forever.
            if depth > steps.len() {
                return None;
            }
            let (mut all, mut done) = (0.0, 0.0);
            let mut any = false;
            for step in steps.iter().filter(|s| s.parent == parent) {
                any = true;
                let weight = step.weight.max(0.0);
                all += weight;
                done += weight
                    * if step.status == "completed" {
                        1.0
                    } else if step.id.is_empty() {
                        0.0
                    } else {
                        share(steps, &step.id, depth + 1).unwrap_or(0.0)
                    };
            }
            any.then(|| if all > 0.0 { done / all } else { 0.0 })
        }
        share(&self.steps, "", 0)
    }

    /// What it does now: the step under way, or the last thing it wrote.
    pub fn doing(&self) -> String {
        let under_way = self
            .steps
            .iter()
            .filter(|s| s.status == "in_progress")
            // The innermost: the one no other step under way is part of.
            .rfind(|s| {
                !self
                    .steps
                    .iter()
                    .any(|c| c.status == "in_progress" && c.parent == s.id && c.id != s.id)
            });
        let text = match under_way {
            Some(step) => step.title.as_str(),
            None => self
                .lines
                .iter()
                .rfind(|l| l.kind == LineKind::Said)
                .and_then(|l| l.text.lines().find(|t| !t.trim().is_empty()))
                .unwrap_or_default(),
        };
        text.trim().chars().take(DOING_CHARS).collect()
    }

    /// Take in what was read anew of a conversation: of each turn, what was
    /// asked and the last thing written. What was written before it and seen
    /// then is kept.
    fn take_in(&mut self, read: Vec<Line>) {
        for line in read.into_iter().filter(|l| !l.text.is_empty()) {
            let before = self
                .lines
                .iter()
                .rposition(|l| l.turn == line.turn && l.kind == line.kind);
            match before {
                Some(at) if grows(&self.lines[at].text, &line.text) => {
                    self.lines[at].text = line.text;
                }
                Some(_) if line.kind == LineKind::Asked => {}
                _ => {
                    let at = self.lines.partition_point(|l| l.turn <= line.turn);
                    self.put(at, line);
                }
            }
        }
        self.trim();
    }

    fn put(&mut self, at: usize, line: Line) {
        // From the clock, so that a run forgotten and read again gives none
        // anew to another line.
        let now = chrono::Utc::now().timestamp_micros().unsigned_abs();
        self.last = now.max(self.last + 1);
        let id = self.last;
        self.lines.insert(at, Line { id, ..line });
    }

    fn trim(&mut self) {
        let over = self.lines.len().saturating_sub(MOST_LINES);
        self.lines.drain(..over);
    }
}

/// Whether `now` is `before`, or goes on from it.
fn grows(before: &str, now: &str) -> bool {
    now.starts_with(before)
}

#[derive(Default)]
struct Kept {
    sight: Sight,
    read_at: Option<Instant>,
    /// When it was last used, among all.
    used: u64,
}

#[derive(Default)]
struct All {
    runs: BTreeMap<i64, Kept>,
    changed: bool,
    uses: u64,
}

impl All {
    /// What is kept of a run. Those used longest ago give way to it: not the
    /// oldest, which may go on still.
    fn of(&mut self, run: i64) -> &mut Kept {
        if !self.runs.contains_key(&run) && self.runs.len() >= MOST_RUNS {
            let unused = self
                .runs
                .iter()
                .min_by_key(|(_, k)| k.used)
                .map(|(r, _)| *r);
            self.runs.remove(&unused.unwrap_or_default());
        }
        self.uses += 1;
        let kept = self.runs.entry(run).or_default();
        kept.used = self.uses;
        kept
    }
}

/// What can be seen of each run.
#[derive(Clone, Default)]
pub struct Sights(Arc<Mutex<All>>);

impl Sights {
    fn lock(&self) -> MutexGuard<'_, All> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn change(&self, run: i64, change: impl FnOnce(&mut Sight)) {
        let mut all = self.lock();
        let sight = &mut all.of(run).sight;
        let standing = |s: &Sight| (s.steps.clone(), s.doing(), s.question.clone());
        let before = standing(sight);
        change(sight);
        let changed = standing(sight) != before;
        all.changed |= changed;
    }

    pub fn of(&self, run: i64) -> Sight {
        self.lock()
            .runs
            .get(&run)
            .map(|k| k.sight.clone())
            .unwrap_or_default()
    }

    /// Whether what stands beside a run in a list changed since this was last
    /// asked.
    pub(super) fn take_changed(&self) -> bool {
        std::mem::take(&mut self.lock().changed)
    }

    pub(super) fn plan(&self, run: i64, steps: Vec<Step>) {
        self.change(run, |s| s.steps = steps);
    }

    pub(super) fn line(&self, run: i64, kind: LineKind, text: &str) {
        let at = chrono::Utc::now().timestamp_millis();
        let line = Line::new(at, 0, kind, text);
        if line.text.is_empty() {
            return;
        }
        self.change(run, |s| {
            s.put(s.lines.len(), line);
            s.trim();
        });
    }

    /// What was read of an Adroit session.
    pub(super) fn read(&self, run: i64, session: &Value) {
        self.change(run, |s| {
            s.steps = steps_of(session);
            s.take_in(lines_of(session));
            s.question = question_of(session).filter(|q| q["id"] != s.answered.as_str());
        });
    }

    /// A question was answered: it is asked no more.
    pub(super) fn answered(&self, run: i64, question: &str) {
        self.change(run, |s| {
            s.question = None;
            s.answered = question.to_owned();
        });
    }

    /// Whether a run was not read in the last `every`; if so, it is now.
    pub(super) fn due(&self, run: i64, every: Duration) -> bool {
        let mut all = self.lock();
        let kept = all.of(run);
        if kept.read_at.is_some_and(|at| at.elapsed() < every) {
            return false;
        }
        kept.read_at = Some(Instant::now());
        true
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

fn each<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value[key].as_array().into_iter().flatten()
}

fn question_of(session: &Value) -> Option<Value> {
    let question = &session["question"];
    (session["state"] == "waiting" && question["id"].is_string()).then(|| question.clone())
}

/// The timeline of an Adroit session.
fn steps_of(session: &Value) -> Vec<Step> {
    each(session, "timeline")
        .map(|t| Step {
            id: text(t, "id"),
            parent: text(t, "parent"),
            title: text(t, "title"),
            status: text(t, "status"),
            weight: t["weight"].as_f64().unwrap_or(1.0),
            note: text(t, "note"),
        })
        .collect()
}

fn lines_of(session: &Value) -> Vec<Line> {
    each(session, "turns")
        .flat_map(|t| {
            let turn = t["turn"].as_u64().unwrap_or_default();
            let at = |key: &str| t[key].as_i64().unwrap_or_default();
            [
                Line::new(at("began_ms"), turn, LineKind::Asked, &text(t, "prompt")),
                Line::new(at("last_ms"), turn, LineKind::Said, &text(t, "reply")),
            ]
        })
        .collect()
}

/// The fields of a tool's input that say most of what a call does.
const TELLING: &[&str] = &[
    "command",
    "file_path",
    "notebook_path",
    "pattern",
    "path",
    "query",
    "url",
];

fn called(name: &str, input: &Value) -> String {
    let what = TELLING
        .iter()
        .find_map(|key| input[key].as_str().map(str::to_owned))
        .unwrap_or_else(|| match input.as_object() {
            Some(fields) if !fields.is_empty() => input.to_string(),
            _ => String::new(),
        });
    format!("{name} {what}")
}

impl Sights {
    /// One event of an agent. Whether it changed the plan: that alone is
    /// worth telling at once, as what an agent writes changes all the time
    /// and every telling has each open console read everything again.
    pub(super) fn event(&self, run: i64, event: &Body) -> bool {
        match event {
            Body::Said { text, .. } => self.line(run, LineKind::Said, text),
            Body::ToolCalled { tool, input, .. } => {
                self.line(run, LineKind::Called, &called(tool, input));
            }
            Body::ToolAnswered { output, .. } => self.line(run, LineKind::Answered, output),
            Body::Planned { steps } => {
                let before = self.of(run).steps;
                self.plan(run, steps.iter().cloned().map(Step::from).collect());
                return self.of(run).steps != before;
            }
            _ => {}
        }
        false
    }
}

impl From<PlanStep> for Step {
    fn from(step: PlanStep) -> Self {
        let PlanStep {
            id,
            parent,
            title,
            status,
            weight,
            note,
        } = step;
        Self {
            id,
            parent,
            title,
            status,
            weight,
            note,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::agents::Adapter as _;
    use crate::agents::claude::Claude;

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

    fn texts(sight: &Sight) -> Vec<&str> {
        sight.lines.iter().map(|l| l.text.as_str()).collect()
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
            let sights = Sights::default();
            sights.read(1, &session("", steps));
            assert_eq!(sights.of(1).done(), done, "{steps:?}");
        }
    }

    #[test]
    fn what_a_run_does_is_its_innermost_step_or_what_it_wrote_last() {
        let sights = Sights::default();
        sights.read(1, &session("\n調べる。\n続き", &[]));
        assert_eq!(sights.of(1).doing(), "調べる。");
        let steps = [
            ("g", "", "in_progress", 1.0),
            ("a", "g", "in_progress", 1.0),
            ("b", "g", "pending", 1.0),
        ];
        sights.read(1, &session("調べる。", &steps));
        assert_eq!(sights.of(1).doing(), "step a");
    }

    #[test]
    fn what_a_session_wrote_before_is_kept_as_it_goes_on() {
        let sights = Sights::default();
        for reply in ["", "調べ", "調べる。", "調べる。", "直す。"] {
            sights.read(1, &session(reply, &[]));
        }
        assert_eq!(texts(&sights.of(1)), ["直して", "調べる。", "直す。"]);
        let mut next = session("戻した。", &[]);
        next["turns"][0]["turn"] = json!(2);
        sights.read(1, &next);
        let again = Sights::default();
        again.read(1, &next);
        let given: Vec<_> = sights.of(1).lines.iter().map(|l| l.id).collect();
        assert!(again.of(1).lines.iter().all(|l| !given.contains(&l.id)));
        assert_eq!(
            texts(&sights.of(1)),
            ["直して", "調べる。", "直す。", "直して", "戻した。"]
        );
    }

    #[test]
    fn an_agents_stream_is_seen_as_its_plan_and_what_it_wrote_and_called() {
        let sights = Sights::default();
        for (kind, block) in [
            ("assistant", json!({ "type": "text", "text": "読む。" })),
            (
                "assistant",
                json!({ "type": "tool_use", "name": "Read", "input": { "file_path": "/a" } }),
            ),
            (
                "user",
                json!({ "type": "tool_result", "content": [{ "type": "text", "text": "中身" }] }),
            ),
            (
                "assistant",
                json!({ "type": "tool_use", "name": "TodoWrite", "input": { "todos": [
                    { "content": "読む", "status": "completed" }, { "content": "書く", "status": "in_progress" },
                ] } }),
            ),
        ] {
            let plans = block["name"] == "TodoWrite";
            let line = json!({ "type": kind, "message": { "content": [block] } }).to_string();
            let changed = Claude
                .codec()
                .read(&line)
                .iter()
                .any(|event| sights.event(1, event));
            assert_eq!(changed, plans, "only a plan is told at once");
        }
        let sight = sights.of(1);
        assert_eq!(texts(&sight), ["読む。", "Read /a", "中身"]);
        assert_eq!((sight.done(), sight.doing()), (Some(0.5), "書く".into()));
    }

    #[test]
    fn a_change_of_what_stands_in_a_list_is_told_once() {
        let sights = Sights::default();
        sights.line(1, LineKind::Said, "a");
        assert!(sights.take_changed());
        assert!(!sights.take_changed());
        sights.line(1, LineKind::Answered, "b");
        assert!(!sights.take_changed(), "only its own page shows it");
    }

    #[test]
    fn the_runs_used_last_and_the_newest_lines_alone_are_kept() {
        let sights = Sights::default();
        for run in 0..=MOST_RUNS as i64 {
            sights.line(run, LineKind::Said, "a");
            // The oldest goes on all along.
            sights.line(0, LineKind::Said, "b");
        }
        assert!(!sights.of(0).lines.is_empty());
        assert_eq!(sights.of(1), Sight::default());
        for n in 0..=MOST_LINES {
            sights.line(1, LineKind::Said, &n.to_string());
        }
        let sight = sights.of(1);
        assert_eq!(sight.lines.len(), MOST_LINES);
        assert_eq!(sight.lines[MOST_LINES - 1].text, MOST_LINES.to_string());
        assert!(sight.lines.is_sorted_by(|a, b| a.id < b.id));
    }

    #[test]
    fn a_run_is_read_no_more_often_than_asked() {
        let sights = Sights::default();
        let long = Duration::from_secs(600);
        assert!(sights.due(1, long));
        assert!(!sights.due(1, long));
        assert!(sights.due(1, Duration::ZERO));
        assert!(sights.due(2, long));
    }
}
