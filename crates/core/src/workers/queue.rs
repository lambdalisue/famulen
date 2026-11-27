//! The queue as it stands: the delegated tasks with their runs.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};

use super::{Asked, Candidate, Limits, Order, Placed, Run, Runner, Settings, Sight, State, plan};
use crate::app::App;
use crate::conversations::task_key;
use crate::models::{CapUnit, Role};
pub use crate::records::is_delegated;
use crate::records::{OWNER, Record, Records, SECRETARY, TASKS, is_over};

/// `order` as it runs when who delegated it named no runner: directly for
/// work on code that Famulen can also finish, if the owner made that the
/// default, else in Adroit.
pub fn with_default_runner(app: &App, order: Order) -> Order {
    let direct = Order {
        runner: Runner::Direct,
        // Work run directly has a workspace of its own anyway.
        worktree: false,
        ..order.clone()
    };
    let finished = app.workers.direct_by_default
        && super::direct::checkout(app.workers.direct.as_ref(), &direct).is_ok_and(
            |(checkout, _)| {
                app.workers.finishing.iter().any(|rule| {
                    rule.repository
                        .canonicalize()
                        .is_ok_and(|named| named == checkout)
                })
            },
        );
    if finished && direct.check().is_ok() && fit(app, &direct).is_ok() {
        direct
    } else {
        Order {
            runner: Runner::Adroit,
            ..order
        }
    }
}

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
    if order.runner == super::Runner::Direct {
        use super::direct;
        let (checkout, _) = direct::checkout(app.workers.direct.as_ref(), order)?;
        app.workers.may_work_in(&direct::worktrees(&checkout))?;
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
    let ordered = app
        .workers
        .runs
        .delegated(&task.id, &order.digest()?, who, &workgroup)?;
    if ordered && who == SECRETARY {
        app.conversations.move_to_running(task_key(&task.id))?;
    }
    Ok(())
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
        None => Some(0),
        Some(_) if !app.models.within_cap(Role::Worker, app.tz(), now)? => Some(0),
        Some(a) if a.cap_unit == CapUnit::Calls => a
            .cap
            .map(|cap| (cap - used as f64).ceil().max(0.0) as usize),
        // What a run will cost is not known before it ends.
        Some(_) => None,
    };
    Ok(Limits {
        slots: settings.slots as usize,
        paused: settings.paused,
        agents_left,
    })
}

/// How often a session is read while it is watched, and once its run ended.
const WATCH_EVERY: Duration = Duration::from_secs(4);
const WATCH_ENDED_EVERY: Duration = Duration::from_secs(30);

/// A run with what can be seen of it, read anew; and why it could not be.
pub async fn watch(app: &App, id: i64) -> anyhow::Result<(Run, Sight, String)> {
    let run = app
        .workers
        .runs
        .get(id)?
        .ok_or_else(|| anyhow::anyhow!("no run #{id}"))?;
    let every = if run.state == State::Ended {
        WATCH_ENDED_EVERY
    } else {
        WATCH_EVERY
    };
    if !run.session.is_empty() && app.workers.sights.due(id, every) {
        let why = match super::adroit::watch(app, &run).await {
            Ok(()) => String::new(),
            Err(error) => format!("{error:#}"),
        };
        app.workers.sights.troubled(id, why);
    }
    let sight = app.workers.sights.of(id);
    let why = app.workers.sights.trouble(id);
    Ok((run, sight, why))
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

#[cfg(test)]
mod default_runner_tests {
    use super::*;
    use crate::notebook::testing::TempDir;
    use crate::workers::{Direct, Rule};

    #[test]
    fn work_is_run_directly_by_default_only_where_famulen_may_finish_it() {
        let dir = TempDir::new("default-runner");
        let repo = TempDir::new("default-runner-repo");
        let checkout = repo.path().canonicalize().unwrap();
        std::fs::create_dir_all(checkout.join(".jj/repo")).unwrap();
        let mut app = crate::runtime::testing::app(dir.path());
        let direct = Direct {
            repository: checkout.clone(),
            jj: "jj".into(),
            fenced: true,
        };
        let rule = Rule {
            repository: checkout.clone(),
            checks: Vec::new(),
            reviews: 0,
            merge: false,
            deploy: None,
            forge: vec!["gh".to_owned()],
        };
        let order = Order {
            title: "Add b".into(),
            request: "add b".into(),
            runner: Runner::Adroit,
            directory: checkout.display().to_string(),
            worktree: true,
            ..Order::default()
        };
        eprintln!("{:?}", {
            app.workers = app.workers.clone().with_direct(Some(direct.clone()));
            Order {
                runner: Runner::Direct,
                worktree: false,
                ..order.clone()
            }
            .check()
        });
        let mut chosen = |by_default: bool, rules: Vec<Rule>, order: Order| {
            app.workers = app
                .workers
                .clone()
                .with_direct(Some(direct.clone()))
                .with_finishing(rules)
                .with_direct_by_default(by_default);
            with_default_runner(&app, order).runner
        };

        let rules = || vec![rule.clone()];
        assert_eq!(chosen(true, rules(), order.clone()), Runner::Direct);
        assert_eq!(chosen(false, rules(), order.clone()), Runner::Adroit);
        assert_eq!(chosen(true, Vec::new(), order.clone()), Runner::Adroit);
        for other in [
            Order {
                permissions: "read_only".into(),
                ..order.clone()
            },
            Order {
                agent: "codex".into(),
                ..order.clone()
            },
            Order {
                directory: "/elsewhere".into(),
                ..order.clone()
            },
        ] {
            assert_eq!(chosen(true, rules(), other), Runner::Adroit);
        }
    }
}
