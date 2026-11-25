//! When the dream runs and how far it tidies memory, as the owner sets them
//! on the console. Kept as one JSON value in the `state` table, since the
//! values are read together and change together.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};

use crate::store::Store;

const KEY: &str = "setting.dream";
/// The day the last scheduled dream succeeded or was given up; an early
/// dream does not count.
pub const LAST_KEY: &str = "dream.last";

/// The most core lines the owner may ask for. A core longer than this
/// would no longer be a summary read in every conversation.
pub const MAX_CORE_LINES: u32 = 1000;
/// The most waiting notes the early dream may wait for.
pub const MAX_EARLY_AT: u32 = 1000;
/// The most days a person may go unseen before they rest.
pub const MAX_REST_AFTER: u32 = 3650;

/// How often the scheduled dream runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Frequency {
    Nightly,
    EveryOtherDay,
    /// Once a week on `weekday`, 0 being Sunday.
    Weekly {
        weekday: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DreamSettings {
    /// The time of day the scheduled dream runs, in the owner's time zone.
    pub hour: u32,
    pub minute: u32,
    pub frequency: Frequency,
    /// Dream before the scheduled time once this many notes wait; 0 never.
    pub early_at: u32,
    /// How many lines core may hold: the dream keeps it within this, and a
    /// new session is handed no more than this.
    pub core_lines: u32,
    /// Whether the dream also tidies records.
    pub tidy_records: bool,
    /// How many days a person goes unseen before they rest; 0 never.
    pub rest_after: u32,
}

impl Default for DreamSettings {
    fn default() -> Self {
        Self {
            hour: 4,
            minute: 0,
            frequency: Frequency::Nightly,
            early_at: 30,
            core_lines: 150,
            tidy_records: true,
            rest_after: 60,
        }
    }
}

impl DreamSettings {
    /// The settings as saved, or the defaults when none are.
    pub fn load(store: &Store) -> anyhow::Result<Self> {
        match store.state(KEY)? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Self::default()),
        }
    }

    /// Save the settings, refusing values out of range.
    pub fn save(&self, store: &Store) -> anyhow::Result<()> {
        self.check()?;
        store.set_state(KEY, serde_json::to_string(self)?)
    }

    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.at().is_some(),
            "the time must be between 0:00 and 23:59"
        );
        if let Frequency::Weekly { weekday } = self.frequency {
            anyhow::ensure!(weekday <= 6, "the weekday must be 0 (Sunday) to 6");
        }
        anyhow::ensure!(
            self.early_at <= MAX_EARLY_AT,
            "the early dream waits for at most {MAX_EARLY_AT} notes"
        );
        anyhow::ensure!(
            (1..=MAX_CORE_LINES).contains(&self.core_lines),
            "core holds 1 to {MAX_CORE_LINES} lines"
        );
        anyhow::ensure!(
            self.rest_after <= MAX_REST_AFTER,
            "a person rests after at most {MAX_REST_AFTER} days"
        );
        Ok(())
    }

    fn at(&self) -> Option<NaiveTime> {
        NaiveTime::from_hms_opt(self.hour, self.minute, 0)
    }

    /// Whether the scheduled dream may run on `day`, given the day it last counted for.
    fn runs_on(&self, day: NaiveDate, last: Option<NaiveDate>) -> bool {
        if last == Some(day) {
            return false;
        }
        match self.frequency {
            Frequency::Nightly => true,
            Frequency::EveryOtherDay => last.is_none_or(|last| (day - last).num_days() >= 2),
            Frequency::Weekly { weekday } => day.weekday().num_days_from_sunday() == weekday,
        }
    }

    /// Whether the scheduled dream is due at `now`, local time.
    pub fn due(&self, last: Option<NaiveDate>, now: NaiveDateTime) -> bool {
        self.at().is_some_and(|at| now.time() >= at) && self.runs_on(now.date(), last)
    }

    /// Whether this many waiting notes call for a dream before its time.
    pub fn early(&self, waiting: usize) -> bool {
        self.early_at > 0 && waiting >= self.early_at as usize
    }

    /// When the scheduled dream runs next, after `now`.
    pub fn next(&self, last: Option<NaiveDate>, now: NaiveDateTime) -> NaiveDateTime {
        let at = self.at().unwrap_or_default();
        let today = now.date();
        (0..=14)
            .map(|offset| today + Duration::days(offset))
            .find(|&day| (day > today || now.time() < at) && self.runs_on(day, last))
            .unwrap_or(today + Duration::days(1))
            .and_time(at)
    }

    /// The same as `next`, in words: 今日 4:00, 明日 4:00, 11月29日（日） 4:00.
    pub fn next_in_words(&self, last: Option<NaiveDate>, now: NaiveDateTime) -> String {
        let next = self.next(last, now);
        let time = format!("{}:{:02}", next.hour(), next.minute());
        let day = match (next.date() - now.date()).num_days() {
            0 => "今日".to_owned(),
            1 => "明日".to_owned(),
            _ => {
                let weekday = ["日", "月", "火", "水", "木", "金", "土"]
                    [next.weekday().num_days_from_sunday() as usize];
                format!("{}月{}日（{weekday}）", next.month(), next.day())
            }
        };
        format!("{day} {time}")
    }
}

/// The day the last scheduled dream succeeded or was given up.
pub fn last(store: &Store) -> anyhow::Result<Option<NaiveDate>> {
    Ok(store.state(LAST_KEY)?.and_then(|d| d.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        // 2026-11-22 is a Sunday.
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
        day(d).and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn without_a_choice_it_dreams_nightly_at_four() {
        let store = Store::open_in_memory().unwrap();
        let s = DreamSettings::load(&store).unwrap();
        assert_eq!(s, DreamSettings::default());
        assert_eq!(
            (s.hour, s.minute, s.early_at, s.core_lines, s.rest_after),
            (4, 0, 30, 150, 60)
        );
        assert_eq!(s.frequency, Frequency::Nightly);
    }

    #[test]
    fn saved_settings_are_read_back() {
        let store = Store::open_in_memory().unwrap();
        let s = DreamSettings {
            hour: 23,
            minute: 30,
            frequency: Frequency::Weekly { weekday: 3 },
            early_at: 0,
            core_lines: 80,
            tidy_records: false,
            rest_after: 0,
        };
        s.save(&store).unwrap();
        assert_eq!(DreamSettings::load(&store).unwrap(), s);
    }

    #[test]
    fn values_out_of_range_are_not_saved() {
        let store = Store::open_in_memory().unwrap();
        let good = DreamSettings::default();
        for bad in [
            DreamSettings { hour: 24, ..good },
            DreamSettings { minute: 60, ..good },
            DreamSettings {
                frequency: Frequency::Weekly { weekday: 7 },
                ..good
            },
            DreamSettings {
                early_at: MAX_EARLY_AT + 1,
                ..good
            },
            DreamSettings {
                core_lines: 0,
                ..good
            },
            DreamSettings {
                core_lines: MAX_CORE_LINES + 1,
                ..good
            },
            DreamSettings {
                rest_after: MAX_REST_AFTER + 1,
                ..good
            },
        ] {
            assert!(bad.save(&store).is_err(), "{bad:?}");
        }
        assert_eq!(DreamSettings::load(&store).unwrap(), good);
    }

    #[test]
    fn nightly_it_is_due_once_a_day_from_its_time() {
        let s = DreamSettings::default();
        assert!(!s.due(Some(day(22)), at(23, 3, 59)));
        assert!(s.due(Some(day(22)), at(23, 4, 0)));
        assert!(s.due(None, at(23, 4, 0)));
        assert!(!s.due(Some(day(23)), at(23, 22, 0)));
    }

    #[test]
    fn every_other_day_it_skips_the_day_after_a_dream() {
        let s = DreamSettings {
            frequency: Frequency::EveryOtherDay,
            ..Default::default()
        };
        assert!(!s.due(Some(day(22)), at(23, 5, 0)));
        assert!(s.due(Some(day(22)), at(24, 5, 0)));
        assert!(s.due(Some(day(20)), at(23, 5, 0)));
        assert!(s.due(None, at(23, 5, 0)));
    }

    #[test]
    fn weekly_it_is_due_only_on_its_weekday() {
        let s = DreamSettings {
            frequency: Frequency::Weekly { weekday: 3 },
            ..Default::default()
        };
        // 2026-11-25 is a Wednesday.
        assert!(!s.due(None, at(24, 5, 0)));
        assert!(s.due(None, at(25, 5, 0)));
        assert!(!s.due(Some(day(25)), at(25, 6, 0)));
    }

    #[test]
    fn the_early_dream_waits_for_its_count_and_zero_turns_it_off() {
        let s = DreamSettings::default();
        assert!(!s.early(29));
        assert!(s.early(30));
        let off = DreamSettings { early_at: 0, ..s };
        assert!(!off.early(0));
        assert!(!off.early(1000));
    }

    #[test]
    fn the_next_dream_is_said_in_words() {
        let s = DreamSettings::default();
        assert_eq!(s.next_in_words(Some(day(22)), at(23, 1, 0)), "今日 4:00");
        assert_eq!(s.next_in_words(Some(day(23)), at(23, 5, 0)), "明日 4:00");
        let weekly = DreamSettings {
            hour: 23,
            minute: 5,
            frequency: Frequency::Weekly { weekday: 0 },
            ..s
        };
        assert_eq!(
            weekly.next_in_words(Some(day(22)), at(23, 5, 0)),
            "11月29日（日） 23:05"
        );
        let other = DreamSettings {
            frequency: Frequency::EveryOtherDay,
            ..s
        };
        assert_eq!(
            other.next_in_words(Some(day(23)), at(23, 5, 0)),
            "11月25日（水） 4:00"
        );
    }
}
