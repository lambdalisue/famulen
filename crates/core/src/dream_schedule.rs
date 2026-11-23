//! When to dream, and when to try again a dream that did not succeed. A
//! scheduled dream counts for its day only once it succeeded; a failed one is
//! tried again a bounded number of times, ever further apart, so a model that
//! stays broken is not asked all night.

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::dream_settings::{self, DreamSettings, LAST_KEY};
use crate::store::Store;

const RETRY_KEY: &str = "dream.retry";

/// How many times a scheduled dream is tried before its day is given up.
pub const MAX_TRIES: u32 = 4;
/// How long after the first failure the next try comes.
const FIRST_WAIT_MINUTES: i64 = 10;
/// The longest wait between tries.
const LONGEST_WAIT_MINUTES: i64 = 6 * 60;

/// A dream that did not succeed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Failed {
    /// The day it was scheduled for; none for an early dream, or when that
    /// day was given up.
    day: Option<NaiveDate>,
    tries: u32,
    /// Not before this, unix seconds.
    after: i64,
}

/// A dream to see now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    /// The day it is scheduled for; none for an early dream.
    pub day: Option<NaiveDate>,
}

fn failed(store: &Store) -> anyhow::Result<Option<Failed>> {
    Ok(store
        .state(RETRY_KEY)?
        .and_then(|json| serde_json::from_str(&json).ok()))
}

/// How long to wait after a number of failed tries.
fn wait(tries: u32) -> TimeDelta {
    let minutes = FIRST_WAIT_MINUTES.saturating_mul(4_i64.saturating_pow(tries.saturating_sub(1)));
    TimeDelta::minutes(minutes.min(LONGEST_WAIT_MINUTES))
}

/// The dream due at `now`, `local` being the same moment in the owner's zone.
pub fn due(
    store: &Store,
    now: DateTime<Utc>,
    local: NaiveDateTime,
    waiting: usize,
) -> anyhow::Result<Option<Due>> {
    let plan = DreamSettings::load(store)?;
    let last = dream_settings::last(store)?;
    let today = local.date();
    let failed = failed(store)?;
    let scheduled = plan.due(last, local);
    if scheduled && failed.is_none_or(|f| f.day != Some(today)) {
        return Ok(Some(Due { day: Some(today) }));
    }
    if let Some(f) = failed {
        if now.timestamp() < f.after {
            return Ok(None);
        }
        if let Some(day) = f.day {
            return Ok(Some(Due { day: Some(day) }));
        }
    }
    Ok(plan.early(waiting).then_some(Due { day: None }))
}

/// Note how a due dream went.
pub fn ended(store: &Store, due: Due, succeeded: bool, now: DateTime<Utc>) -> anyhow::Result<()> {
    if succeeded {
        store.clear_state(RETRY_KEY)?;
        return due
            .day
            .map_or(Ok(()), |day| store.set_state(LAST_KEY, day.to_string()));
    }
    let tries = failed(store)?
        .filter(|f| f.day == due.day)
        .map_or(0, |f| f.tries)
        + 1;
    let mut day = due.day;
    if let Some(given_up) = day.filter(|_| tries >= MAX_TRIES) {
        store.set_state(LAST_KEY, given_up.to_string())?;
        day = None;
    }
    let after = (now + wait(tries)).timestamp();
    let failed = Failed { day, tries, after };
    store.set_state(RETRY_KEY, serde_json::to_string(&failed)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    struct Nights {
        store: Store,
    }

    impl Nights {
        fn new(settings: DreamSettings) -> Self {
            let store = Store::open_in_memory().unwrap();
            settings.save(&store).unwrap();
            Self { store }
        }

        fn at(d: u32, h: u32, m: u32) -> DateTime<Utc> {
            Utc.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap()
        }

        fn due(&self, d: u32, h: u32, m: u32, waiting: usize) -> Option<Due> {
            let now = Self::at(d, h, m);
            due(&self.store, now, now.naive_utc(), waiting).unwrap()
        }

        fn ended(&self, due: Due, ok: bool, d: u32, h: u32, m: u32) {
            ended(&self.store, due, ok, Self::at(d, h, m)).unwrap();
        }
    }

    fn scheduled(d: u32) -> Option<Due> {
        Some(Due {
            day: NaiveDate::from_ymd_opt(2026, 11, d),
        })
    }

    #[test]
    fn a_dream_counts_for_its_day_only_once_it_succeeded() {
        let n = Nights::new(DreamSettings::default());
        assert_eq!(n.due(21, 3, 59, 0), None);
        let due = n.due(21, 4, 0, 0).unwrap();
        assert_eq!(dream_settings::last(&n.store).unwrap(), None);

        n.ended(due, true, 21, 4, 5);

        assert_eq!(n.due(21, 4, 6, 0), None);
        assert_eq!(n.due(22, 4, 0, 0), scheduled(22));
    }

    #[test]
    fn a_failed_dream_is_tried_again_ever_later_and_then_given_up_for_its_day() {
        let n = Nights::new(DreamSettings::default());
        let due = n.due(21, 4, 0, 0).unwrap();
        n.ended(due, false, 21, 4, 0);
        assert_eq!(n.due(21, 4, 9, 0), None);
        assert_eq!(n.due(21, 4, 10, 0), scheduled(21));
        n.ended(due, false, 21, 4, 10);
        assert_eq!(n.due(21, 4, 49, 0), None);
        assert_eq!(n.due(21, 4, 50, 0), scheduled(21));
        n.ended(due, false, 21, 4, 50);
        assert_eq!(n.due(21, 7, 30, 0), scheduled(21));
        n.ended(due, false, 21, 7, 30);

        assert_eq!(n.due(21, 23, 0, 0), None, "given up after {MAX_TRIES}");
        assert_eq!(n.due(22, 4, 0, 0), scheduled(22));
        n.ended(scheduled(22).unwrap(), false, 22, 4, 0);
        assert_eq!(n.due(22, 4, 10, 0), scheduled(22), "counted afresh");
    }

    #[test]
    fn a_retry_that_passes_midnight_still_counts_for_the_day_it_was_for() {
        let n = Nights::new(DreamSettings {
            hour: 23,
            minute: 55,
            ..DreamSettings::default()
        });
        let due = n.due(21, 23, 55, 0).unwrap();
        n.ended(due, false, 21, 23, 55);
        assert_eq!(n.due(22, 0, 5, 0), scheduled(21));
        n.ended(due, true, 22, 0, 6);
        assert_eq!(n.due(22, 0, 7, 0), None);
        assert_eq!(n.due(22, 23, 55, 0), scheduled(22));
    }

    #[test]
    fn an_early_dream_that_failed_is_not_tried_again_every_minute() {
        let n = Nights::new(DreamSettings::default());
        n.ended(scheduled(21).unwrap(), true, 21, 4, 0);
        assert_eq!(n.due(21, 9, 0, 29), None);
        let early = n.due(21, 9, 0, 30).unwrap();
        assert_eq!(early, Due { day: None });
        n.ended(early, false, 21, 9, 0);
        assert_eq!(n.due(21, 9, 1, 30), None);
        assert_eq!(n.due(21, 9, 10, 30), Some(early));
        for _ in 0..5 {
            n.ended(early, false, 21, 10, 0);
        }
        assert_eq!(n.due(21, 15, 59, 30), None);
        assert_eq!(n.due(21, 16, 0, 30), Some(early));
        assert_eq!(n.due(22, 4, 0, 30), scheduled(22));
    }

    #[test]
    fn an_early_dream_leaves_the_scheduled_one_where_it_was() {
        let n = Nights::new(DreamSettings::default());
        n.ended(Due { day: None }, true, 21, 1, 0);
        assert_eq!(n.due(21, 4, 0, 0), scheduled(21));
    }
}
