//! Taking a finishing through its steps, from when the agent's turn is over
//! until the agent or the owner is needed again.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::task::AbortHandle;

use super::{Finishing, Hands, Next, Stage};

/// How long after a step that went wrong it is tried again, at first.
const ERROR_AGAIN: Duration = Duration::from_secs(30);
/// How often in a row a step may go wrong before the owner is told.
const MOST_ERRORS: u32 = 8;

/// How a finishing under way stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Seen {
    Working,
    /// The owner is asked.
    Waiting,
    /// A step goes wrong, and is tried again.
    Trouble(String),
    /// Nothing more is done here until it is driven again.
    Over(Next),
}

/// A finishing being driven. It is dropped to stop it.
pub(crate) struct Driving {
    abort: AbortHandle,
    seen: Arc<Mutex<Seen>>,
}

impl Driving {
    pub(crate) fn start<H: Hands + 'static>(
        finishing: Finishing,
        hands: H,
        over: impl FnOnce() + Send + 'static,
    ) -> Self {
        let seen = Arc::new(Mutex::new(Seen::Working));
        let task = tokio::spawn({
            let seen = seen.clone();
            async move {
                let next = drive(finishing, hands, &seen, ERROR_AGAIN).await;
                *lock(&seen) = Seen::Over(next);
                over();
            }
        });
        Self {
            abort: task.abort_handle(),
            seen,
        }
    }

    pub(crate) fn seen(&self) -> Seen {
        let seen = lock(&self.seen).clone();
        match seen {
            Seen::Over(_) => seen,
            // A task that went away without a word is driven again.
            _ if self.abort.is_finished() => Seen::Over(Next::Again),
            _ => seen,
        }
    }
}

impl Drop for Driving {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

fn lock(seen: &Mutex<Seen>) -> MutexGuard<'_, Seen> {
    seen.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn drive<H: Hands>(
    mut finishing: Finishing,
    hands: H,
    seen: &Mutex<Seen>,
    again: Duration,
) -> Next {
    let mut errors = 0;
    loop {
        let stepped = finishing.step(&hands, crate::store::now()).await;
        // Kept whether or not the step went through: what it counted stays.
        let kept = hands.keep(&finishing);
        match stepped.and_then(|next| kept.map(|()| next)) {
            Ok(Next::Again) => {
                errors = 0;
                *lock(seen) = Seen::Working;
            }
            Ok(Next::Wait(wait)) => {
                errors = 0;
                *lock(seen) = if finishing.asking() {
                    Seen::Waiting
                } else {
                    Seen::Working
                };
                tokio::time::sleep(wait).await;
            }
            Ok(next) => return next,
            Err(error) => {
                errors += 1;
                tracing::warn!(%error, stage = finishing.stage.as_str(), "could not take a step of finishing work");
                if errors >= MOST_ERRORS {
                    let why = format!(
                        "Finishing stopped while {}: {error:#}",
                        finishing.stage.as_str()
                    );
                    finishing.stage = Stage::Stopped;
                    finishing.note = Some(why.clone());
                    if let Err(error) = hands.keep(&finishing) {
                        tracing::warn!(%error, "could not keep that finishing stopped");
                    }
                    return Next::Stop(why);
                }
                *lock(seen) = Seen::Trouble(format!("{error:#}"));
                tokio::time::sleep(again * 2u32.pow(errors - 1)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests;
