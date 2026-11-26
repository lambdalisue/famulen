//! What can be seen of a run while it goes on: the plan its worker keeps, and
//! what it wrote and called, taken into the ledger as it comes. It is the
//! worker's words, and nothing else is decided by it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ledger::{Item, Ledger, PartOf, PlanFrom, fresh_key};
use crate::agents::{Ask, Body, Counted, Event, PlanStep};
use crate::store::Store;

/// How much is said of what a run does now.
const DOING_CHARS: usize = 120;

/// One step of a worker's plan.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    /// The step it is part of, or nothing.
    pub parent: String,
    pub title: String,
    /// `pending`, `in_progress`, `completed` or `blocked`.
    pub status: String,
    /// Its size beside its siblings.
    pub weight: f64,
    #[serde(default)]
    pub description: String,
    pub note: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sight {
    pub steps: Vec<Step>,
    pub from: Option<PlanFrom>,
    /// What it wrote last.
    pub said: String,
    /// What an Adroit session waits on the owner for, as Adroit tells it.
    pub question: Option<Value>,
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
                .said
                .lines()
                .find(|t| !t.trim().is_empty())
                .unwrap_or_default(),
        };
        text.trim().chars().take(DOING_CHARS).collect()
    }
}

/// What is kept in memory of an Adroit session, which Adroit tells again.
#[derive(Default)]
struct Kept {
    question: Option<Value>,
    /// The question answered last: what was read before it was answered may
    /// come after.
    answered: String,
    read_at: Option<Instant>,
    /// Why it could not be read last, until it is.
    trouble: String,
    /// When it was last used, among all.
    used: u64,
}

#[derive(Default)]
struct All {
    runs: BTreeMap<i64, Kept>,
    changed: bool,
    uses: u64,
}

/// How many runs are remembered of what Adroit tells.
const MOST_RUNS: usize = 200;

impl All {
    /// What is remembered of a run. The one used longest ago gives way to it:
    /// not the oldest, which may go on still.
    fn kept(&mut self, run: i64) -> &mut Kept {
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
#[derive(Clone)]
pub struct Sights {
    ledger: Ledger,
    all: Arc<Mutex<All>>,
}

impl Sights {
    pub fn new(store: Store) -> Self {
        Self {
            ledger: Ledger::new(store),
            all: Arc::default(),
        }
    }

    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    fn lock(&self) -> MutexGuard<'_, All> {
        self.all.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn of(&self, run: i64) -> Sight {
        let (from, steps) = self.ledger.current(run).unwrap_or_else(|error| {
            tracing::debug!(run, %error, "could not read a run's plan");
            Default::default()
        });
        Sight {
            steps,
            from,
            said: self
                .ledger
                .last_said(run)
                .ok()
                .flatten()
                .unwrap_or_default(),
            question: self.lock().runs.get(&run).and_then(|k| k.question.clone()),
        }
    }

    /// Whether what stands beside a run in a list changed since this was last
    /// asked.
    pub(super) fn take_changed(&self) -> bool {
        std::mem::take(&mut self.lock().changed)
    }

    /// What stands beside a run in a list changed.
    pub(super) fn changed(&self) {
        self.lock().changed = true;
    }

    /// Keep an entry; whether it is new or grew.
    fn put(&self, run: i64, key: &str, at: i64, item: Item) -> bool {
        match self.ledger.put(run, key, at, item) {
            Ok(entry) => entry.is_some(),
            Err(error) => {
                tracing::warn!(run, %error, "could not keep what a worker did");
                false
            }
        }
    }

    /// Keep a plan when it differs from the one it would follow.
    fn plan(&self, run: i64, key: &str, at: i64, from: PlanFrom, steps: Vec<Step>) -> bool {
        if self.ledger.plan_from(run, from).ok().as_ref() == Some(&steps) {
            return false;
        }
        if !self.put(run, key, at, Item::Plan { from, steps }) {
            return false;
        }
        // Told only when it is the plan the run follows.
        let followed = self
            .ledger
            .current(run)
            .is_ok_and(|(now, _)| now == Some(from));
        if followed {
            self.changed();
        }
        followed
    }

    /// What a worker was asked to do.
    pub(super) fn asked(&self, run: i64, at: i64, text: &str) {
        self.put(run, "asked", at, Item::Asked { text: text.into() });
    }

    /// What a run Famulen runs itself waits on the owner for, or that it
    /// waits on them no more.
    pub(super) fn owe(&self, run: i64, question: Option<Value>) {
        let mut all = self.lock();
        if question.is_none() && !all.runs.contains_key(&run) {
            return;
        }
        let kept = all.kept(run);
        if kept.question != question {
            kept.question = question;
            all.changed = true;
        }
    }

    /// A run's process is gone, and why.
    pub(super) fn ended(&self, run: i64, start: i64, at: i64, reason: String) {
        self.put(run, &format!("exit:{start}"), at, Item::Ended { reason });
    }

    /// How a question was decided.
    pub(super) fn decided(&self, run: i64, at: i64, key: &str, item: Item) {
        self.put(run, key, at, item);
    }

    /// What was read of an Adroit session.
    pub(super) fn read(&self, run: i64, session: &Value) {
        let now = chrono::Utc::now().timestamp_millis();
        let steps = steps_of(session);
        if !steps.is_empty()
            || self
                .ledger
                .current(run)
                .is_ok_and(|(from, _)| from.is_some())
        {
            self.plan(run, &fresh_key("adroit:plan"), now, PlanFrom::Adroit, steps);
        }
        // What was forgotten for its age is not kept again from what Adroit
        // still tells.
        if self.ledger.forgotten(run).unwrap_or(false) {
            return;
        }
        let mut grew = false;
        for turn in each(session, "turns") {
            let n = turn["turn"].as_u64().unwrap_or_default();
            let at = |key: &str| turn[key].as_i64().unwrap_or(now);
            let prompt = text(turn, "prompt");
            if !prompt.trim().is_empty() {
                self.put(
                    run,
                    &format!("adroit:{n}:asked"),
                    at("began_ms"),
                    Item::Asked { text: prompt },
                );
            }
            let reply = text(turn, "reply");
            if !reply.trim().is_empty() {
                // A reply that goes on from the one kept takes its place; one
                // written anew is kept beside it.
                let said = format!("adroit:{n}:said");
                let key = match self.ledger.last_said_under(run, &said) {
                    Ok(Some((key, before))) if reply.starts_with(&before) => key,
                    Ok(Some(_)) => fresh_key(&said),
                    _ => said,
                };
                grew |= self.put(
                    run,
                    &key,
                    at("last_ms"),
                    Item::Said {
                        message: format!("turn-{n}"),
                        text: reply,
                    },
                );
            }
        }
        let (asked, gone) = {
            let mut all = self.lock();
            let kept = all.kept(run);
            let question = question_of(session).filter(|q| q["id"] != kept.answered.as_str());
            let before = std::mem::replace(&mut kept.question, question.clone());
            // Waited for no more without being answered here: answered
            // elsewhere, or taken back.
            let gone = before
                .as_ref()
                .filter(|b| question.as_ref().is_none_or(|q| q["id"] != b["id"]))
                .filter(|b| b["id"] != kept.answered.as_str())
                .map(|b| text(b, "id"));
            all.changed |= before != question || grew;
            (question.filter(|q| before.as_ref() != Some(q)), gone)
        };
        if let Some(id) = gone {
            self.put(run, &fresh_key("adroit:gone"), now, Item::Withdrawn { id });
        }
        if let Some(q) = asked {
            let id = text(&q, "id");
            let tool = text(&q, "tool");
            self.put(
                run,
                &fresh_key("adroit:question"),
                now,
                Item::Question {
                    what: what_of(&tool, &q["input"]),
                    id,
                    tool,
                },
            );
        }
    }

    /// A question was answered: it is asked no more.
    pub(super) fn answered(&self, run: i64, question: &str, allowed: bool, why: &str) {
        {
            let mut all = self.lock();
            let kept = all.kept(run);
            kept.question = None;
            kept.answered = question.to_owned();
            all.changed = true;
        }
        self.decided(
            run,
            chrono::Utc::now().timestamp_millis(),
            // A question may be asked and answered again.
            &fresh_key(&format!("decided:{question}")),
            Item::Decided {
                id: question.to_owned(),
                allowed,
                why: why.to_owned(),
            },
        );
    }

    /// Why a run could not be read when it last was; nothing once it was.
    pub(super) fn troubled(&self, run: i64, why: String) {
        self.lock().kept(run).trouble = why;
    }

    pub(super) fn trouble(&self, run: i64) -> String {
        self.lock()
            .runs
            .get(&run)
            .map(|k| k.trouble.clone())
            .unwrap_or_default()
    }

    /// Whether a run was not read in the last `every`; if so, it is now.
    pub(super) fn due(&self, run: i64, every: Duration) -> bool {
        let mut all = self.lock();
        let kept = all.kept(run);
        if kept.read_at.is_some_and(|at| at.elapsed() < every) {
            return false;
        }
        kept.read_at = Some(Instant::now());
        true
    }

    /// One event of an agent. Whether it changed the plan: that alone is
    /// worth telling at once, as what an agent writes changes all the time
    /// and every telling has each open console read everything again.
    /// `now` when it is told as it comes, not read again from before; `start`
    /// tells the process it is of from another started for the same run.
    pub(super) fn event(&self, run: i64, start: i64, event: &Event, now: bool) -> bool {
        let at = event.at;
        let key = format!("a:{start}:{}.{}", event.seq, event.part);
        let part = |of: PartOf, text: &str| {
            if now {
                self.ledger.part(run, at, of, text.to_owned());
            }
        };
        let item = match &event.body {
            Body::Said { message, text } => Item::Said {
                message: message.clone(),
                text: text.clone(),
            },
            Body::Thought { message, text } => Item::Thought {
                message: message.clone(),
                text: text.clone(),
            },
            Body::SaidMore { message, text } => {
                part(PartOf::Said(message.clone()), text);
                return false;
            }
            Body::ThoughtMore { message, text } => {
                part(PartOf::Thought(message.clone()), text);
                return false;
            }
            Body::ToolCalledMore { call, input } => {
                part(PartOf::Input(call.clone()), input);
                return false;
            }
            Body::ToolWorks { call, output, .. } => {
                part(PartOf::Output(call.clone()), output);
                return false;
            }
            Body::ToolCalled { call, tool, input } => Item::Called {
                call: call.clone(),
                tool: tool.clone(),
                input: input.clone(),
            },
            Body::ToolAnswered { call, ok, output } => Item::Answered {
                call: call.clone(),
                ok: *ok,
                output: output.clone(),
            },
            Body::FileChanged { call, path, diff } => Item::Changed {
                call: call.clone(),
                path: path.clone(),
                diff: diff.clone(),
            },
            Body::Asked { question } => match &question.ask {
                Ask::Tool { tool, input, .. } => Item::Question {
                    id: question.id.clone(),
                    tool: tool.clone(),
                    what: what_of(tool, input),
                },
                // Its calls are seen as the agent tells of them.
                Ask::Server { .. } => return false,
                Ask::Unknown { what } => Item::Question {
                    id: question.id.clone(),
                    tool: String::new(),
                    what: what.clone(),
                },
            },
            Body::Withdrawn { question } => Item::Withdrawn {
                id: question.clone(),
            },
            Body::Planned { steps } => {
                let steps = steps.iter().cloned().map(Step::from).collect();
                return self.plan(run, &key, at, PlanFrom::Notes, steps);
            }
            Body::Used {
                of: Counted::Message(message),
                model,
                input,
                cached,
                output,
            } => {
                self.put(
                    run,
                    &format!("used:{message}"),
                    at,
                    Item::Used {
                        message: message.clone(),
                        model: model.clone().unwrap_or_default(),
                        input: *input,
                        cached: *cached,
                        output: *output,
                    },
                );
                return false;
            }
            Body::Warned { what } => Item::Warned { what: what.clone() },
            Body::TurnEnded { ok, text } => Item::TurnEnded {
                ok: *ok,
                text: text.clone(),
            },
            Body::Ended { reason } => Item::Ended {
                reason: reason.clone(),
            },
            Body::Started { .. } | Body::Stands { .. } | Body::Used { .. } => return false,
        };
        if self.put(run, &key, at, item) && matches!(event.body, Body::Said { .. }) {
            self.changed();
        }
        false
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
            description: text(t, "description"),
            note: text(t, "note"),
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
    "plan",
];

/// What a call asks to do, in a line or a few.
fn what_of(tool: &str, input: &Value) -> String {
    let questions: Vec<String> = each(input, "questions")
        .map(|q| text(q, "question"))
        .filter(|q| !q.is_empty())
        .collect();
    if !questions.is_empty() {
        return questions.join("\n");
    }
    TELLING
        .iter()
        .find_map(|key| input[key].as_str().map(str::to_owned))
        .unwrap_or_else(|| match input.as_object() {
            Some(fields) if !fields.is_empty() => input.to_string(),
            _ => tool.to_owned(),
        })
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
            description: String::new(),
            note,
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
impl Sights {
    /// What a run wrote, oldest first.
    pub(crate) fn said(&self, run: i64) -> Vec<String> {
        self.ledger
            .since(run, 0, 10_000)
            .unwrap()
            .into_iter()
            .filter_map(|e| match e.item {
                Item::Said { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }
}
