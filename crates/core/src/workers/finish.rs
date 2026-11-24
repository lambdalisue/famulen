//! How the owner wants delegated work on code finished: steps of their own,
//! added to what a worker is asked, so that they do not depend on who wrote
//! the request.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::order::{Order, Runner};

/// The permissions of a session that changes nothing.
const READ_ONLY: &str = "read_only";

/// Steps that finish work on code.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finish {
    /// Work in this directory or below it, as an absolute path; any when
    /// empty.
    pub directory: String,
    pub steps: String,
}

pub(super) fn check_finishes(all: &[Finish]) -> anyhow::Result<()> {
    for (at, finish) in all.iter().enumerate() {
        anyhow::ensure!(
            !finish.steps.trim().is_empty(),
            "a finish needs its steps written"
        );
        anyhow::ensure!(
            finish.directory.is_empty() || Path::new(&finish.directory).is_absolute(),
            "a finish's directory is an absolute path, not {}",
            finish.directory
        );
        anyhow::ensure!(
            !all[..at]
                .iter()
                .any(|f| Path::new(&f.directory) == Path::new(&finish.directory)),
            "{:?} has two finishes",
            finish.directory
        );
    }
    Ok(())
}

/// The steps that finish `order`: those of the directory closest around its
/// own. None for work that changes no code of the owner's.
pub(super) fn steps_for(all: &[Finish], order: &Order) -> String {
    if order.runner != Runner::Adroit || order.permissions == READ_ONLY {
        return String::new();
    }
    let directory = Path::new(&order.directory);
    all.iter()
        .filter(|f| f.directory.is_empty() || directory.starts_with(&f.directory))
        .max_by_key(|f| Path::new(&f.directory).components().count())
        .map(|f| f.steps.trim().to_owned())
        .unwrap_or_default()
}

/// What a session is asked: the request, then how it is finished.
pub(super) fn prompt(order: &Order) -> String {
    if order.finish.is_empty() {
        return order.request.clone();
    }
    format!(
        "{}\n\n---\n\n\
         # How this work is finished\n\n\
         The owner's standing steps, for every piece of work that changes code here. Take them in order once the work above is done, all of them, by yourself: the owner has decided on them, so do not stop in between to ask whether to go on. They stand over what the request says about finishing (a commit, a pull request, whether to merge), unless it says the owner asked otherwise for this work. Put right what fails on the way. Leave out what a change of nothing has no use for. Your turn ends when they are all done, or when one cannot be: then say which, and why. Say in your last answer how each of them went.\n\n{}",
        order.request, order.finish
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::at;

    fn finish(directory: &str, steps: &str) -> Finish {
        Finish {
            directory: directory.into(),
            steps: steps.into(),
        }
    }

    fn order(directory: &str) -> Order {
        Order {
            directory: directory.into(),
            request: "Do it.".into(),
            ..Order::default()
        }
    }

    #[test]
    fn work_is_finished_by_the_steps_of_the_directory_closest_around_it() {
        let all = [
            finish("", "any"),
            finish(&at("/src/api"), "api"),
            finish(&at("/src"), "src"),
        ];
        for (directory, steps) in [
            ("/src/api", "api"),
            ("/src/api/.worktrees/a", "api"),
            ("/src/api-old", "src"),
            ("/home", "any"),
        ] {
            assert_eq!(steps_for(&all, &order(&at(directory))), steps);
        }
        assert_eq!(steps_for(&all[1..], &order(&at("/home"))), "");
    }

    #[test]
    fn what_changes_no_code_has_no_steps_to_finish_it() {
        let all = [finish("", "any")];
        let agent = Order {
            runner: Runner::Agent,
            ..order(&at("/src"))
        };
        let reads = Order {
            permissions: READ_ONLY.into(),
            ..order(&at("/src"))
        };
        for order in [agent, reads] {
            assert_eq!(steps_for(&all, &order), "");
        }
    }

    #[test]
    fn the_steps_stand_below_the_request() {
        let mut order = order(&at("/src"));
        assert_eq!(prompt(&order), "Do it.");
        order.finish = "1. Merge it.".into();
        let prompt = prompt(&order);
        assert!(prompt.starts_with("Do it.\n"), "{prompt}");
        assert!(prompt.ends_with("\n1. Merge it."), "{prompt}");
    }

    #[test]
    fn a_finish_without_steps_or_a_place_is_refused() {
        let good = finish(&at("/src"), "any");
        assert!(check_finishes(&[finish("", "any"), good.clone()]).is_ok());
        for bad in [finish("", " "), finish("src", "any"), good.clone()] {
            assert!(check_finishes(&[good.clone(), bad]).is_err());
        }
    }
}
