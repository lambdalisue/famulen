//! The queue as it stands: the delegated tasks with their runs.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use super::{Asked, Candidate, Limits, Order, Placed, Run, Settings, State, plan};
use crate::app::App;
use crate::models::{CapUnit, Role};
pub use crate::records::is_delegated;
use crate::records::{OWNER, Record, Records, SECRETARY, TASKS, is_over};

/// A delegated task in the queue, with the newest of its runs.
#[derive(Debug, Clone)]
pub struct Queued {
    pub task: Record,
    pub code: String,
    pub placed: Placed,
    pub run: Option<Run>,
}

/// What became of the approval a start waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    Waiting,
    Approved,
    Refused,
}

/// How the approval `ticket` stands; one that lapsed counts as refused.
pub fn approval(app: &App, ticket: Option<i64>) -> anyhow::Result<Approval> {
    let Some(ticket) = ticket else {
        return Ok(Approval::Refused);
    };
    Ok(match app.gateway.ticket(ticket)? {
        Some(t) if t.status == "held" => Approval::Waiting,
        Some(t) if matches!(t.status.as_str(), "approved" | "always_allowed") => Approval::Approved,
        _ => Approval::Refused,
    })
}

/// How many runs of agents a day may bring while the worker role has no model
/// of its own, and so no cap the owner chose.
pub const UNASSIGNED_RUNS: i64 = 20;

/// What a task orders, refused when it could not be run as it is.
pub fn ordered(app: &App, task: &Record, code: &str) -> anyhow::Result<Order> {
    let order = Order::of(task, code)?;
    fit(app, &order)?;
    Ok(order)
}

/// Refuse an order that hands over a tool which starts agents, or an agent's
/// directory it may not work in.
pub fn fit(app: &App, order: &Order) -> anyhow::Result<()> {
    for tool in &order.tools {
        if let Some((connection, name)) = crate::permissions::split_tool(tool) {
            anyhow::ensure!(
                !app.permissions.starts_agents(connection, name),
                "{tool} starts agents and is never handed to a worker"
            );
        }
    }
    if order.runner == super::Runner::Agent && !order.directory.is_empty() {
        app.workers
            .may_work_in(std::path::Path::new(&order.directory))?;
    }
    Ok(())
}

/// Keep who wrote a delegated task, and in which workgroup they did.
pub fn written(app: &App, task: &Record, by: crate::notebook::Writer) -> anyhow::Result<()> {
    use crate::notebook::Writer;
    if !is_delegated(task) {
        return Ok(());
    }
    let Ok(order) = Order::of(task, "") else {
        return Ok(());
    };
    let (who, workgroup) = match by {
        Writer::Owner => (OWNER, String::new()),
        Writer::Secretary => (SECRETARY, app.workgroups.working()?),
        Writer::Lookout | Writer::Worker => return Ok(()),
    };
    app.workers
        .runs
        .delegated(&task.id, &order.digest()?, who, &workgroup)
}

/// The workgroups a task's connections must be in to be used without asking:
/// those it is in, and where the secretary worked when it wrote the task, so
/// that naming another workgroup widens nothing. None when the task is not as
/// the owner or the secretary wrote it through Famulen.
pub fn scopes(app: &App, task: &str, order: &Order) -> anyhow::Result<Vec<String>> {
    let Some((by, workgroup)) = app.workers.runs.delegation(task, &order.digest()?)? else {
        return Ok(Vec::new());
    };
    let mut scopes = order.workgroups.clone();
    if by == SECRETARY {
        scopes.push(workgroup);
    }
    if scopes.is_empty() {
        scopes.push(crate::workgroups::ALL.to_owned());
    }
    Ok(scopes)
}

pub fn limits(app: &App, now: DateTime<Utc>) -> anyhow::Result<Limits> {
    let settings = Settings::load(&app.store)?;
    let used = app.models.used_today(Role::Worker, app.tz(), now)?.1;
    let agents_left = match app.models.assignment(Role::Worker)? {
        Some(_) if !app.models.within_cap(Role::Worker, app.tz(), now)? => Some(0),
        Some(a) if a.cap_unit == CapUnit::Calls => a
            .cap
            .map(|cap| (cap - used as f64).ceil().max(0.0) as usize),
        // What a run will cost is not known before it ends.
        Some(_) => None,
        // The secretary's model stands in, whose cap is not the worker's.
        None => Some(usize::try_from(UNASSIGNED_RUNS - used).unwrap_or(0)),
    };
    Ok(Limits {
        slots: settings.slots as usize,
        paused: settings.paused,
        agents_left,
    })
}

/// The delegated tasks that are not over, in the order they start.
pub fn queue(app: &App, now: DateTime<Utc>) -> anyhow::Result<Vec<Queued>> {
    let all: BTreeMap<String, Record> = app
        .records
        .list(TASKS)?
        .into_iter()
        .map(|r| (r.id.clone(), r))
        .collect();
    let codes = app.records.codes(TASKS)?;
    let code_of = |id: &str| codes.get(id).cloned().unwrap_or_else(|| id.to_owned());
    let mut runs: BTreeMap<String, Run> = app
        .workers
        .runs
        .latest()?
        .into_iter()
        .map(|r| (r.task.clone(), r))
        .collect();
    let mut candidates = Vec::new();
    for task in all.values().filter(|t| is_delegated(t) && !is_over(t)) {
        let text = |name: &str| task.body.value(name).unwrap_or_default();
        let open = runs.get(&task.id).filter(|r| r.state != State::Ended);
        let asked = match open {
            Some(run) if run.state == State::Asking => approval(app, run.ticket)?,
            _ => Approval::Waiting,
        };
        candidates.push(Candidate {
            task: task.id.clone(),
            priority: text("Priority"),
            due: text("Due"),
            start: text("Start"),
            blocked: !Records::blockers(&all, task).is_empty(),
            pending: matches!(text("Status").as_str(), "" | "pending"),
            // An open run is started as it was ordered then.
            runner: match open {
                Some(run) => Ok(run.order.runner),
                None => ordered(app, task, &code_of(&task.id))
                    .map(|o| o.runner)
                    .map_err(|e| e.to_string()),
            },
            run: open.map(|r| r.state),
            retry_due: open.is_some_and(|r| r.retry_at <= now.timestamp()),
            asked: if asked == Approval::Approved {
                Asked::Approved
            } else {
                Asked::NotYet
            },
        });
    }
    // A run whose task is gone or no longer delegated takes its slot until it
    // is stopped.
    let slots_of_others = runs
        .values()
        .filter(|r| r.state.takes_slot() && !candidates.iter().any(|c| c.task == r.task))
        .count();
    let mut limits = limits(app, now)?;
    limits.slots = limits.slots.saturating_sub(slots_of_others);
    let local = now.with_timezone(&app.tz()).naive_local();
    Ok(plan(&candidates, &limits, local)
        .into_iter()
        .map(|placed| Queued {
            task: all[&placed.task].clone(),
            code: code_of(&placed.task),
            run: runs.remove(&placed.task),
            placed,
        })
        .collect())
}
