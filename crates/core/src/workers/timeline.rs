//! The tool a worker keeps its plan with, the same as an Adroit session's
//! timeline: a tree of steps, each weighed beside its siblings, changed by
//! operations applied in order.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};

use super::ledger::Ledger;
use super::sight::{Sight, Step};
use crate::hub::{Hub, OwnTools, ToolDef};

pub const TOOL: &str = "timeline_tasks";
/// The tool as an agent calls it.
pub fn full_name() -> String {
    format!("mcp__{}__{TOOL}", crate::hub::SERVER_NAME)
}

const STATUSES: &[&str] = &["pending", "in_progress", "completed", "blocked"];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Add {
        id: String,
        #[serde(default)]
        parent: Option<String>,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        weight: Option<f64>,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        note: Option<String>,
    },
    Update {
        id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        weight: Option<f64>,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        note: Option<String>,
    },
    Remove {
        id: String,
    },
}

/// Apply `ops` in order. What cannot be applied is skipped, and told by its
/// id and why.
pub fn apply(mut steps: Vec<Step>, ops: &[Op]) -> (Vec<Step>, Vec<String>) {
    let mut skipped = Vec::new();
    for op in ops {
        if let Err(why) = apply_one(&mut steps, op) {
            skipped.push(why);
        }
    }
    (steps, skipped)
}

fn status_of(status: &Option<String>) -> Result<Option<String>, String> {
    match status {
        Some(s) if !STATUSES.contains(&s.as_str()) => Err(format!("no status {s:?}")),
        other => Ok(other.clone()),
    }
}

fn weight_of(weight: Option<f64>) -> Result<Option<f64>, String> {
    match weight {
        Some(w) if !w.is_finite() || w < 0.0 => Err(format!("a weight of {w}")),
        other => Ok(other),
    }
}

fn apply_one(steps: &mut Vec<Step>, op: &Op) -> Result<(), String> {
    match op {
        Op::Add {
            id,
            parent,
            title,
            weight,
            status,
            description,
            note,
        } => {
            let fail = |why: &str| format!("add {id}: {why}");
            if id.trim().is_empty() {
                return Err("add: an id is needed".into());
            }
            if steps.iter().any(|s| &s.id == id) {
                return Err(fail("there is one by that id"));
            }
            let parent = parent.clone().unwrap_or_default();
            if !parent.is_empty() && !steps.iter().any(|s| s.id == parent) {
                return Err(fail(&format!("there is no {parent} to be part of")));
            }
            let Some(title) = title.clone().filter(|t| !t.trim().is_empty()) else {
                return Err(fail("a title is needed"));
            };
            let status = status_of(status).map_err(|why| fail(&why))?;
            let weight = weight_of(*weight).map_err(|why| fail(&why))?;
            steps.push(Step {
                id: id.clone(),
                parent,
                title,
                status: status.unwrap_or_else(|| "pending".into()),
                weight: weight.unwrap_or(1.0),
                description: description.clone().unwrap_or_default(),
                note: note.clone().unwrap_or_default(),
            });
        }
        Op::Update {
            id,
            title,
            weight,
            status,
            description,
            note,
        } => {
            let fail = |why: &str| format!("update {id}: {why}");
            let status = status_of(status).map_err(|why| fail(&why))?;
            let weight = weight_of(*weight).map_err(|why| fail(&why))?;
            let step = steps
                .iter_mut()
                .find(|s| &s.id == id)
                .ok_or_else(|| fail("there is none by that id"))?;
            if let Some(title) = title.clone().filter(|t| !t.trim().is_empty()) {
                step.title = title;
            }
            if let Some(status) = status {
                step.status = status;
            }
            if let Some(weight) = weight {
                step.weight = weight;
            }
            if let Some(description) = description {
                step.description = description.clone();
            }
            if let Some(note) = note {
                step.note = note.clone();
            }
        }
        Op::Remove { id } => {
            if !steps.iter().any(|s| &s.id == id) {
                return Err(format!("remove {id}: there is none by that id"));
            }
            let mut gone = vec![id.clone()];
            while let Some(at) = steps
                .iter()
                .position(|s| gone.contains(&s.id) || gone.contains(&s.parent))
            {
                let step = steps.remove(at);
                if !gone.contains(&step.id) {
                    gone.push(step.id);
                }
            }
        }
    }
    Ok(())
}

/// The plan as the worker reads it back.
pub fn render(steps: &[Step]) -> String {
    fn mark(status: &str) -> &'static str {
        match status {
            "completed" => "[x]",
            "in_progress" => "[>]",
            "blocked" => "[!]",
            _ => "[ ]",
        }
    }
    fn below(steps: &[Step], parent: &str, depth: usize, out: &mut String) {
        if depth > steps.len() {
            return;
        }
        for step in steps.iter().filter(|s| s.parent == parent) {
            out.push_str(&format!(
                "{}- {} {}: {} (w={})",
                "  ".repeat(depth),
                mark(&step.status),
                step.id,
                step.title,
                step.weight
            ));
            if !step.note.is_empty() {
                out.push_str(&format!(" — {}", step.note));
            }
            out.push('\n');
            below(steps, &step.id, depth + 1, out);
        }
    }
    if steps.is_empty() {
        return "The timeline is empty.".into();
    }
    let mut out = String::new();
    let done = Sight {
        steps: steps.to_vec(),
        ..Sight::default()
    }
    .done()
    .unwrap_or(0.0);
    out.push_str(&format!("{:.0}% completed\n", done * 100.0));
    below(steps, "", 0, &mut out);
    out
}

const DESCRIPTION: &str = "Record the plan of the work you were given as a task tree, and keep it current as you work; the owner follows it. Make one top-level task for the goal and put its steps under it, split further where a step is large. Give each task a weight: its size beside its siblings. Mark a task in_progress as you start it, and completed or blocked as soon as it is; put what it will do in description, and what happened in note. ops are applied in order: add {id, parent?, title, weight?, status?, description?, note?}, update {id, status?, title?, weight?, description?, note?}, remove {id} (with the tasks under it). What cannot be applied is skipped and named.";

fn schema() -> Value {
    let field = |kind: &str| json!({ "type": kind });
    json!({
        "type": "object",
        "properties": {
            "ops": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "op": { "type": "string", "enum": ["add", "update", "remove"] },
                        "id": { "type": "string", "description": "A short id you choose, unique in your timeline." },
                        "parent": { "type": "string", "description": "add: the task it is a step of; a new tree when omitted." },
                        "title": field("string"),
                        "weight": { "type": "number", "description": "Its size beside its siblings, zero or more; 1 when omitted." },
                        "status": { "type": "string", "enum": STATUSES },
                        "description": field("string"),
                        "note": field("string"),
                    },
                    "required": ["op", "id"],
                },
            },
        },
        "required": ["ops"],
    })
}

/// The tools Famulen serves to one run.
struct RunTools {
    ledger: Ledger,
    run: i64,
    /// Told when the plan changed.
    told: Arc<dyn Fn() + Send + Sync>,
}

impl OwnTools for RunTools {
    fn list(&self) -> Vec<ToolDef> {
        vec![ToolDef {
            name: TOOL.into(),
            description: DESCRIPTION.into(),
            input_schema: schema(),
            read_only: false,
            confirm: false,
        }]
    }

    fn call(&self, name: &str, arguments: Value) -> BoxFuture<'static, anyhow::Result<String>> {
        let (ledger, run, name) = (self.ledger.clone(), self.run, name.to_owned());
        let told = self.told.clone();
        Box::pin(async move {
            anyhow::ensure!(name == TOOL, "there is no tool {name}");
            #[derive(Deserialize)]
            struct Args {
                ops: Vec<Value>,
            }
            let args: Args = serde_json::from_value(arguments)?;
            let mut skipped = Vec::new();
            let ops: Vec<Op> = args
                .ops
                .into_iter()
                .filter_map(|op| {
                    serde_json::from_value(op.clone())
                        .inspect_err(|error| skipped.push(format!("{op}: {error}")))
                        .ok()
                })
                .collect();
            let (steps, changed) = ledger.change_timeline(run, |before| {
                let (after, more) = apply(before.clone(), &ops);
                skipped.extend(more);
                let changed = after != before;
                (after, changed)
            })?;
            if changed {
                told();
            }
            let mut text = render(&steps);
            if !skipped.is_empty() {
                text.push_str(&format!("\nSkipped: {}", skipped.join("; ")));
            }
            Ok(text)
        })
    }
}

/// The server a run's tools are answered by.
pub fn hub(ledger: Ledger, run: i64, told: Arc<dyn Fn() + Send + Sync>) -> Hub {
    Hub::new(Arc::new(RunTools { ledger, run, told }))
}

#[cfg(test)]
mod tests;
