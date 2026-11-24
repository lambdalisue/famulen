//! Which delegated task runs next: a pure function of the tasks, how their
//! runs stand, the limits and the time.

use chrono::{NaiveDate, NaiveDateTime};
use famulen_notebook::records::minute;

use super::order::Runner;
use super::runs::State;

/// How a start the owner was asked of stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    NotYet,
    Approved,
}

/// A delegated task that is not over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The task's id, which sorts by when it was made.
    pub task: String,
    /// `high`, `normal`, `low` or nothing.
    pub priority: String,
    /// As written: a date, or a date and a time.
    pub due: String,
    pub start: String,
    pub blocked: bool,
    /// Its `Status` is `pending`.
    pub pending: bool,
    /// What runs it, or why it cannot be run as it is written.
    pub runner: Result<Runner, String>,
    /// How its open run stands.
    pub run: Option<State>,
    pub asked: Asked,
    /// A start that failed may be tried again by now.
    pub retry_due: bool,
}

/// What bounds the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub slots: usize,
    pub paused: bool,
    /// How many more agents may start today; none is bound when not said.
    pub agents_left: Option<usize>,
}

/// How a delegated task stands in the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// Nothing is in its way: it starts now, or once a slot is free.
    Next,
    Paused,
    /// The worker role reached its cap for today.
    Capped,
    /// Its `Start` has not come.
    Later,
    Blocked,
    /// It cannot be run as it is written.
    Unfit(String),
    /// Its start waits for the owner's approval.
    Asking,
    /// Approved, it starts once a slot is free.
    Approved,
    Running,
    /// Its session waits for the owner.
    Waiting,
    /// Its start failed, or was cut off, and is tried again in a while.
    Retrying,
    /// Adroit cannot be asked of it.
    Unreachable,
    /// No run is open and it does not ask for one: its `Status` is not
    /// `pending`.
    Still,
}

/// A task in the queue, and whether it is started now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub task: String,
    pub standing: Standing,
    pub start: bool,
}

fn rank(priority: &str) -> u8 {
    match priority {
        "high" => 0,
        "low" => 2,
        _ => 1,
    }
}

/// A date or a minute as a time; a date is its first or last moment.
fn time(text: &str, last: bool) -> Option<NaiveDateTime> {
    minute(text).or_else(|| {
        let day = NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok()?;
        if last {
            day.and_hms_opt(23, 59, 59)
        } else {
            day.and_hms_opt(0, 0, 0)
        }
    })
}

fn capped(c: &Candidate, limits: &Limits) -> bool {
    limits.agents_left == Some(0) && c.runner == Ok(Runner::Agent)
}

/// Whether a `Start` as written has not come.
pub fn is_later(start: &str, now: NaiveDateTime) -> bool {
    time(start, false).is_some_and(|start| start > now)
}

/// What keeps a task from starting, whether asked of already or not.
fn held(c: &Candidate, now: NaiveDateTime) -> Option<Standing> {
    if c.blocked {
        return Some(Standing::Blocked);
    }
    if c.start.trim().is_empty() {
        return None;
    }
    if time(&c.start, false).is_none() {
        return Some(Standing::Unfit(format!("Start is no date: {}", c.start)));
    }
    is_later(&c.start, now).then_some(Standing::Later)
}

fn standing(c: &Candidate, limits: &Limits, now: NaiveDateTime) -> Standing {
    let held = held(c, now);
    match (c.run, c.asked) {
        (Some(State::Asking), Asked::NotYet) => return Standing::Asking,
        (Some(State::Asking | State::Starting), _) if held.is_some() || !c.pending => {}
        (Some(State::Asking), Asked::Approved) if limits.paused => return Standing::Paused,
        (Some(State::Asking), Asked::Approved) => return Standing::Approved,
        (Some(State::Waiting), _) => return Standing::Waiting,
        (Some(State::Unreachable), _) => return Standing::Unreachable,
        (Some(State::Starting), _) if limits.paused => return Standing::Paused,
        (Some(State::Starting), _) if capped(c, limits) => return Standing::Capped,
        (Some(State::Starting), _) => return Standing::Retrying,
        (Some(_), _) => return Standing::Running,
        (None, _) => {}
    }
    if !c.pending {
        return Standing::Still;
    }
    if let Err(why) = &c.runner {
        return Standing::Unfit(why.clone());
    }
    if let Some(held) = held {
        return held;
    }
    if limits.paused {
        return Standing::Paused;
    }
    if capped(c, limits) {
        return Standing::Capped;
    }
    Standing::Next
}

/// The queue in the order its tasks start: by priority, then the due date,
/// then the oldest.
pub fn plan(candidates: &[Candidate], limits: &Limits, now: NaiveDateTime) -> Vec<Placed> {
    let mut order: Vec<&Candidate> = candidates.iter().collect();
    order.sort_by(|a, b| {
        let due = |c: &Candidate| time(&c.due, true).unwrap_or(NaiveDateTime::MAX);
        rank(&a.priority)
            .cmp(&rank(&b.priority))
            .then_with(|| due(a).cmp(&due(b)))
            .then_with(|| a.task.cmp(&b.task))
    });
    let taken = candidates
        .iter()
        .filter(|c| c.run.is_some_and(State::takes_slot))
        .count();
    let mut free = limits.slots.saturating_sub(taken);
    // One found not to have started may come back when its slot was given to
    // another: it waits like the rest.
    let mut going = candidates
        .iter()
        .filter(|c| c.run == Some(State::Running))
        .count();
    let mut limits = *limits;
    order
        .into_iter()
        .map(|c| {
            let standing = standing(c, &limits, now);
            let start = match standing {
                Standing::Retrying if going < limits.slots => {
                    going += 1;
                    c.retry_due
                }
                Standing::Next | Standing::Approved if free > 0 => {
                    free -= 1;
                    true
                }
                _ => false,
            };
            // What starts now counts for those after it.
            if start && c.runner == Ok(Runner::Agent) {
                limits.agents_left = limits.agents_left.map(|n| n.saturating_sub(1));
            }
            Placed {
                task: c.task.clone(),
                standing,
                start,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str) -> Candidate {
        Candidate {
            task: id.into(),
            priority: String::new(),
            due: String::new(),
            start: String::new(),
            blocked: false,
            pending: true,
            runner: Ok(Runner::Agent),
            run: None,
            asked: Asked::NotYet,
            retry_due: false,
        }
    }

    fn limits(slots: usize) -> Limits {
        Limits {
            slots,
            paused: false,
            agents_left: None,
        }
    }

    fn now() -> NaiveDateTime {
        minute("2026-11-24 12:00").unwrap()
    }

    fn started(candidates: &[Candidate], limits: Limits) -> Vec<String> {
        plan(candidates, &limits, now())
            .into_iter()
            .filter(|p| p.start)
            .map(|p| p.task)
            .collect()
    }

    fn standing_of(c: Candidate, limits: Limits) -> Standing {
        plan(&[c], &limits, now()).remove(0).standing
    }

    #[test]
    fn tasks_start_by_priority_then_due_then_the_oldest() {
        let with = |id: &str, priority: &str, due: &str| Candidate {
            priority: priority.into(),
            due: due.into(),
            ..task(id)
        };
        let tasks = [
            with("a", "low", "2026-11-25"),
            with("b", "", ""),
            with("c", "normal", "2026-12-01"),
            with("d", "", "2026-11-30 09:00"),
            with("e", "high", ""),
            with("f", "", ""),
            with("g", "", "2026-11-30"),
        ];
        assert_eq!(
            started(&tasks, limits(10)),
            ["e", "d", "g", "c", "b", "f", "a"]
        );
    }

    #[test]
    fn no_more_run_at_once_than_there_are_slots() {
        let run = |id: &str, state: State| Candidate {
            run: Some(state),
            pending: false,
            ..task(id)
        };
        let tasks = [
            run("a", State::Running),
            run("b", State::Starting),
            task("c"),
            task("d"),
        ];
        assert_eq!(started(&tasks, limits(2)), Vec::<String>::new());
        assert_eq!(started(&tasks, limits(3)), ["c"]);
        assert_eq!(started(&tasks, limits(1)), Vec::<String>::new());
    }

    #[test]
    fn a_run_that_waits_on_the_owner_takes_no_slot() {
        for state in [State::Waiting, State::Asking, State::Ending] {
            let waiting = Candidate {
                run: Some(state),
                pending: false,
                ..task("a")
            };
            assert_eq!(
                started(&[waiting, task("b")], limits(1)),
                ["b"],
                "{state:?}"
            );
        }
    }

    #[test]
    fn an_approved_start_takes_its_place_by_priority_and_needs_a_slot() {
        let approved = Candidate {
            run: Some(State::Asking),
            asked: Asked::Approved,
            runner: Ok(Runner::Adroit),
            ..task("b")
        };
        let urgent = Candidate {
            priority: "high".into(),
            ..task("c")
        };
        let tasks = [task("a"), approved.clone(), urgent];
        assert_eq!(started(&tasks, limits(1)), ["c"]);
        assert_eq!(started(&tasks, limits(3)), ["c", "a", "b"]);
        assert_eq!(standing_of(approved, limits(0)), Standing::Approved);
    }

    #[test]
    fn a_failed_start_is_tried_again_in_its_slot_when_due_and_not_paused_or_capped() {
        let retry = |due: bool| Candidate {
            run: Some(State::Starting),
            retry_due: due,
            ..task("a")
        };
        assert!(started(&[retry(false)], limits(1)).is_empty());
        assert_eq!(started(&[retry(true), task("b")], limits(1)), ["a"]);
        for held in [
            Limits {
                paused: true,
                ..limits(1)
            },
            Limits {
                agents_left: Some(0),
                ..limits(1)
            },
        ] {
            assert!(started(&[retry(true)], held).is_empty(), "{held:?}");
        }
    }

    #[test]
    fn a_start_tried_again_after_its_slot_was_given_away_waits_for_one() {
        let retry = Candidate {
            run: Some(State::Starting),
            retry_due: true,
            ..task("a")
        };
        let running = Candidate {
            run: Some(State::Running),
            pending: false,
            ..task("b")
        };
        let tasks = [
            retry.clone(),
            running,
            Candidate {
                task: "c".into(),
                ..retry
            },
        ];
        assert!(started(&tasks, limits(1)).is_empty());
        assert_eq!(started(&tasks, limits(2)), ["a"]);
        assert_eq!(started(&tasks, limits(3)), ["a", "c"]);
    }

    #[test]
    fn a_start_not_yet_made_is_not_made_once_its_task_is_not_pending() {
        for run in [State::Asking, State::Starting] {
            let c = Candidate {
                run: Some(run),
                asked: Asked::Approved,
                retry_due: true,
                pending: false,
                ..task("a")
            };
            assert_eq!(standing_of(c.clone(), limits(5)), Standing::Still);
            assert!(started(&[c], limits(5)).is_empty());
        }
    }

    #[test]
    fn a_run_that_cannot_be_seen_takes_no_slot_and_is_not_started_again() {
        let away = Candidate {
            run: Some(State::Unreachable),
            retry_due: true,
            ..task("a")
        };
        assert_eq!(started(&[away, task("b")], limits(1)), ["b"]);
    }

    #[test]
    fn no_more_agents_start_than_the_day_has_left() {
        let left = |n| Limits {
            agents_left: Some(n),
            ..limits(5)
        };
        let session = Candidate {
            runner: Ok(Runner::Adroit),
            ..task("c")
        };
        let tasks = [task("a"), task("b"), session, task("d")];
        assert_eq!(started(&tasks, left(1)), ["a", "c"]);
        assert_eq!(started(&tasks, left(2)), ["a", "b", "c"]);
        assert_eq!(plan(&tasks, &left(1), now())[1].standing, Standing::Capped);
    }

    #[test]
    fn a_start_put_off_or_blocked_is_not_made_even_once_approved_or_to_be_tried_again() {
        let put_off = |run| Candidate {
            run: Some(run),
            asked: Asked::Approved,
            retry_due: true,
            start: "2026-11-25".into(),
            ..task("a")
        };
        for run in [State::Asking, State::Starting] {
            let later = put_off(run);
            assert_eq!(standing_of(later.clone(), limits(5)), Standing::Later);
            let blocked = Candidate {
                blocked: true,
                start: String::new(),
                ..later.clone()
            };
            assert_eq!(standing_of(blocked.clone(), limits(5)), Standing::Blocked);
            for c in [later, blocked] {
                assert!(started(&[c], limits(5)).is_empty(), "{run:?}");
            }
        }
    }

    #[test]
    fn what_stands_in_a_tasks_way_is_told() {
        let open = limits(1);
        let paused = Limits {
            paused: true,
            ..open
        };
        let capped = Limits {
            agents_left: Some(0),
            ..open
        };
        let cases: Vec<(Candidate, Limits, Standing)> = vec![
            (task("a"), open, Standing::Next),
            (task("a"), paused, Standing::Paused),
            (task("a"), capped, Standing::Capped),
            (
                Candidate {
                    runner: Ok(Runner::Adroit),
                    ..task("a")
                },
                capped,
                Standing::Next,
            ),
            (
                Candidate {
                    blocked: true,
                    ..task("a")
                },
                open,
                Standing::Blocked,
            ),
            (
                Candidate {
                    start: "2026-11-24 12:01".into(),
                    ..task("a")
                },
                open,
                Standing::Later,
            ),
            (
                Candidate {
                    start: "2026-11-24".into(),
                    ..task("a")
                },
                open,
                Standing::Next,
            ),
            (
                Candidate {
                    start: "someday".into(),
                    ..task("a")
                },
                open,
                Standing::Unfit("Start is no date: someday".into()),
            ),
            (
                Candidate {
                    runner: Err("no runner".into()),
                    ..task("a")
                },
                open,
                Standing::Unfit("no runner".into()),
            ),
            (
                Candidate {
                    pending: false,
                    ..task("a")
                },
                open,
                Standing::Still,
            ),
            (
                Candidate {
                    run: Some(State::Asking),
                    ..task("a")
                },
                open,
                Standing::Asking,
            ),
            (
                Candidate {
                    run: Some(State::Asking),
                    asked: Asked::Approved,
                    ..task("a")
                },
                paused,
                Standing::Paused,
            ),
            (
                Candidate {
                    run: Some(State::Running),
                    pending: false,
                    ..task("a")
                },
                paused,
                Standing::Running,
            ),
        ];
        for (c, limits, expected) in cases {
            let starts = matches!(expected, Standing::Next);
            let placed = plan(std::slice::from_ref(&c), &limits, now()).remove(0);
            assert_eq!(
                (placed.standing, placed.start),
                (expected, starts),
                "{c:?} {limits:?}"
            );
        }
    }
}
