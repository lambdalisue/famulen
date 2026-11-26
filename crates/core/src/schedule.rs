//! What starts a turn besides the owner: routines, which are records of the
//! notebook, and one-off wake-ups the agent sets for itself.

mod cron_line;

pub use cron_line::*;

use anyhow::Context as _;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use famulen_notebook::records::{flag, minute, record_of};
use rusqlite::{OptionalExtension, params};

use crate::input::{Input, Sender, Source};
use crate::notebook::{Writer, link_target};
use crate::records::{Patch, ROUTINES, Record, Records, SECRETARY, TASKS, is_over};
use crate::store::Store;

/// The most runs looked through for the next one that is not skipped.
const MOST_RUNS: usize = 500;

/// A routine, with when it runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Routine {
    pub record: Record,
    pub tz: Tz,
    pub enabled: bool,
    pub cron: Vec<String>,
    pub next_run: Option<DateTime<Tz>>,
    pub last_run: Option<DateTime<Tz>>,
    /// Why it never runs, when something in it cannot be read.
    pub problem: Option<String>,
}

impl Routine {
    fn skips(&self) -> Vec<DateTime<Tz>> {
        self.record
            .body
            .values("Skip")
            .iter()
            .filter_map(|t| self.tz.from_local_datetime(&minute(t)?).earliest())
            .collect()
    }

    /// The runs from `after` on, skipped ones among them.
    pub fn runs(&self, after: DateTime<Utc>, count: usize) -> Vec<(DateTime<Tz>, bool)> {
        let skips = self.skips();
        let Ok(rule) = Rule::of(&self.record, self.tz) else {
            return Vec::new();
        };
        rule.after(after)
            .take(count)
            .map(|at| (at, skips.contains(&at)))
            .collect()
    }

    /// When it runs, in words.
    pub fn when(&self) -> String {
        self.cron
            .iter()
            .map(|c| describe_cron(c))
            .collect::<Vec<_>>()
            .join("、")
    }
}

/// When a routine runs.
struct Rule {
    crons: Vec<cron::Schedule>,
    tz: Tz,
    start: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
}

impl Rule {
    fn of(record: &Record, tz: Tz) -> anyhow::Result<Self> {
        let crons = crons_of(record)
            .iter()
            .map(|c| parse_cron(c))
            .collect::<anyhow::Result<Vec<_>>>()?;
        anyhow::ensure!(!crons.is_empty(), "there is no Schedule");
        let bound = |name: &str, last: bool| -> anyhow::Result<Option<DateTime<Utc>>> {
            let Some(text) = record.body.value(name) else {
                return Ok(None);
            };
            let at = match minute(&text) {
                Some(at) => at,
                None => {
                    let day = NaiveDate::parse_from_str(&text, "%Y-%m-%d")
                        .with_context(|| format!("{name} is no date: {text}"))?;
                    let time = if last { (23, 59, 59) } else { (0, 0, 0) };
                    day.and_hms_opt(time.0, time.1, time.2)
                        .context("no such time")?
                }
            };
            Ok(tz
                .from_local_datetime(&at)
                .earliest()
                .map(|t| t.with_timezone(&Utc)))
        };
        Ok(Self {
            crons,
            tz,
            start: bound("Start", false)?,
            until: bound("Until", true)?,
        })
    }

    /// Every run after a time, in order.
    fn after(&self, after: DateTime<Utc>) -> impl Iterator<Item = DateTime<Tz>> + '_ {
        let mut at = match self.start {
            // A run at the very start counts.
            Some(start) if start > after => start - chrono::Duration::seconds(1),
            _ => after,
        };
        std::iter::from_fn(move || {
            let next = self
                .crons
                .iter()
                .filter_map(|c| c.after(&at.with_timezone(&self.tz)).next())
                .min()?;
            at = next.with_timezone(&Utc);
            self.until.is_none_or(|until| at <= until).then_some(next)
        })
    }
}

/// The cron lines of a routine. A line with commas is written as a nested
/// item, so a value on the field's own line is one line.
fn crons_of(record: &Record) -> Vec<String> {
    match record.body.field("Schedule") {
        Some(item) if !item.items.is_empty() => item.values(),
        // One line with commas of its own, or several lines between commas.
        Some(item) if parse_cron(&item.value()).is_err() => item.values(),
        Some(item) => vec![item.value()],
        None => Vec::new(),
    }
}

/// A wake-up that has not fired yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wakeup {
    pub id: i64,
    pub at: DateTime<Tz>,
    pub note: String,
    /// What it follows up, for the owner to read; empty when not given.
    pub why: String,
}

#[derive(Clone)]
pub struct Scheduler {
    store: Store,
    records: Records,
    tz: Tz,
}

impl Scheduler {
    pub fn new(store: Store, records: Records, tz: Tz) -> Self {
        Self { store, records, tz }
    }

    pub fn timezone(&self) -> Tz {
        self.tz
    }

    /// Every routine, with its next run worked out. A routine that changed
    /// gets a new next run; one that did not keeps the one it had.
    pub fn list(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<Routine>> {
        let mut routines = Vec::new();
        for record in self.records.list(ROUTINES)? {
            routines.push(self.routine(record, now)?);
        }
        Ok(routines)
    }

    fn routine(&self, record: Record, now: DateTime<Utc>) -> anyhow::Result<Routine> {
        let mut problem = None;
        let tz = match record.body.value("Timezone") {
            Some(name) => name.parse::<Tz>().unwrap_or_else(|_| {
                problem = Some(format!("unknown time zone {name}"));
                self.tz
            }),
            None => self.tz,
        };
        let cron = crons_of(&record);
        let text = |name: &str| record.body.value(name).unwrap_or_default();
        let rule_text = format!(
            "{}|{}|{}|{}",
            cron.join(";"),
            // As written, so that putting right a name which cannot be read
            // is a change too.
            record
                .body
                .value("Timezone")
                .unwrap_or_else(|| tz.name().to_owned()),
            text("Start"),
            text("Until")
        );
        let rule = Rule::of(&record, tz);
        if let Err(error) = &rule {
            problem = Some(format!("{error:#}"));
        }
        let kept: Option<(String, Option<i64>, Option<i64>)> = self
            .store
            .conn()
            .query_row(
                "SELECT rule, next_run, last_run FROM routine_runs WHERE routine = ?1",
                [&record.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (next, last) = match kept {
            Some((kept, next, last)) if kept == rule_text => (next, last),
            kept => {
                let next = match (&problem, &rule) {
                    (None, Ok(rule)) => rule.after(now).next().map(|t| t.timestamp()),
                    _ => None,
                };
                self.store.conn().execute(
                    "INSERT INTO routine_runs (routine, rule, next_run) VALUES (?1, ?2, ?3)
                     ON CONFLICT (routine) DO UPDATE SET rule = ?2, next_run = ?3",
                    params![record.id, rule_text, next],
                )?;
                (next, kept.and_then(|k| k.2))
            }
        };
        let local = |t: i64| tz.timestamp_opt(t, 0).single();
        Ok(Routine {
            enabled: flag(&text("Enabled")).unwrap_or(true),
            record,
            tz,
            cron,
            next_run: next.and_then(local),
            last_run: last.and_then(local),
            problem,
        })
    }

    pub fn get(&self, id: &str, now: DateTime<Utc>) -> anyhow::Result<Routine> {
        self.routine(self.records.need(ROUTINES, id)?, now)
    }

    /// Skip the run at `at`. Asked twice, it is still the one run that is
    /// skipped.
    pub fn skip(&self, writer: Writer, id: &str, at: DateTime<Utc>) -> anyhow::Result<()> {
        let routine = self.get(id, Utc::now())?;
        let at = routine
            .runs(at - chrono::Duration::seconds(1), 1)
            .first()
            .map(|(t, _)| *t)
            .filter(|t| t.timestamp() == at.timestamp())
            .with_context(|| format!("{} does not run then", routine.record.title()))?;
        let mut skips = routine.record.body.values("Skip");
        let text = at.format("%Y-%m-%d %H:%M").to_string();
        if skips.contains(&text) {
            return Ok(());
        }
        skips.push(text);
        self.set_skips(writer, &routine.record.id, skips)
    }

    /// Run a skipped run after all.
    pub fn unskip(&self, writer: Writer, id: &str, at: DateTime<Utc>) -> anyhow::Result<()> {
        let routine = self.get(id, Utc::now())?;
        let text = at
            .with_timezone(&routine.tz)
            .format("%Y-%m-%d %H:%M")
            .to_string();
        let mut skips = routine.record.body.values("Skip");
        skips.retain(|s| minute(s) != minute(&text));
        self.set_skips(writer, &routine.record.id, skips)
    }

    fn set_skips(&self, writer: Writer, id: &str, skips: Vec<String>) -> anyhow::Result<()> {
        let field = crate::records::Field {
            name: "Skip".into(),
            value: String::new(),
            values: skips,
        };
        self.records.change(
            writer,
            ROUTINES,
            id,
            Patch {
                fields: vec![field],
                ..Patch::default()
            },
        )?;
        Ok(())
    }

    /// Set a one-off wake-up. Returns its id.
    pub fn wake_at(
        &self,
        at: DateTime<Utc>,
        note: impl AsRef<str>,
        why: impl AsRef<str>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<i64> {
        anyhow::ensure!(
            at > now,
            "the wake-up time {} has already passed",
            at.with_timezone(&self.tz)
        );
        let conn = self.store.conn();
        conn.execute(
            "INSERT INTO wakeups (at, note, why) VALUES (?1, ?2, ?3)",
            params![at.timestamp(), note.as_ref(), why.as_ref()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// The wake-ups that have not fired, soonest first.
    pub fn pending_wakeups(&self) -> anyhow::Result<Vec<Wakeup>> {
        let rows: Vec<(i64, i64, String, String)> = {
            let conn = self.store.conn();
            let mut statement = conn
                .prepare("SELECT id, at, note, why FROM wakeups WHERE fired = 0 ORDER BY at, id")?;
            statement
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<Result<_, _>>()?
        };
        Ok(rows
            .into_iter()
            .map(|(id, at, note, why)| Wakeup {
                id,
                at: self.local(at),
                note,
                why,
            })
            .collect())
    }

    /// Drop a wake-up before it fires.
    pub fn cancel_wakeup(&self, id: i64) -> anyhow::Result<()> {
        let removed = self.store.conn().execute(
            "DELETE FROM wakeups WHERE id = ?1 AND fired = 0",
            params![id],
        )?;
        anyhow::ensure!(removed == 1, "no wake-up {id} is waiting");
        Ok(())
    }

    /// Everything that has come due by `now`, as inputs. Each routine fires
    /// once however many runs it missed, then moves on from `now`. A routine
    /// that is not the secretary's becomes a task instead.
    pub fn due(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<Input>> {
        let mut inputs = Vec::new();
        for wakeup in self
            .pending_wakeups()?
            .into_iter()
            .filter(|w| w.at.timestamp() <= now.timestamp())
        {
            self.store.conn().execute(
                "UPDATE wakeups SET fired = 1 WHERE id = ?1",
                params![wakeup.id],
            )?;
            inputs.push(Input {
                sender: Sender::System,
                source: Source::Wake {
                    id: wakeup.id,
                    at: wakeup.at.to_rfc3339(),
                },
                text: wakeup.note,
            });
        }
        for routine in self.list(now)? {
            let Some(next) = routine
                .next_run
                .filter(|n| n.timestamp() <= now.timestamp())
            else {
                continue;
            };
            match self.fire(&routine, next, now) {
                Ok(input) => inputs.extend(input),
                Err(error) => {
                    tracing::warn!(%error, routine = %routine.record.title(), "a routine could not run")
                }
            }
        }
        Ok(inputs)
    }

    fn fire(
        &self,
        routine: &Routine,
        next: DateTime<Tz>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<Input>> {
        let id = &routine.record.id;
        let rule = Rule::of(&routine.record, routine.tz)?;
        let following = rule.after(now).next().map(|t| t.timestamp());
        self.store.conn().execute(
            "UPDATE routine_runs SET next_run = ?2 WHERE routine = ?1",
            params![id, following],
        )?;
        if !routine.enabled {
            return Ok(None);
        }
        // Of the runs missed while nothing ran, the last one not skipped.
        let skips = routine.skips();
        let first = next.with_timezone(&Utc) - chrono::Duration::seconds(1);
        let missed: Vec<DateTime<Tz>> = rule
            .after(first)
            .take_while(|t| t.timestamp() <= now.timestamp())
            .take(MOST_RUNS)
            .collect();
        let run = missed.iter().rev().find(|t| !skips.contains(t)).copied();
        let mut lines: Vec<String> = missed
            .iter()
            .filter(|t| skips.contains(t))
            .map(|t| format!("{} の回を飛ばした", t.format("%Y-%m-%d %H:%M")))
            .collect();
        let left: Vec<String> = routine
            .record
            .body
            .values("Skip")
            .into_iter()
            .filter(|s| {
                minute(s)
                    .and_then(|t| routine.tz.from_local_datetime(&t).earliest())
                    .is_none_or(|t| t.timestamp() > now.timestamp())
            })
            .collect();
        let mut fired = None;
        if let Some(run) = run {
            let (why, notes) = self.held_back(routine)?;
            lines.extend(notes);
            match why {
                Some(why) => lines.push(format!(
                    "{} の回を飛ばした（{why}）",
                    run.format("%Y-%m-%d %H:%M")
                )),
                None => fired = Some(self.start(routine, run, now)?),
            }
        }
        if !lines.is_empty() {
            self.records.note_routine(id, &lines, &left)?;
        }
        Ok(fired.flatten())
    }

    fn start(
        &self,
        routine: &Routine,
        run: DateTime<Tz>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<Input>> {
        let record = &routine.record;
        let assignee = record
            .body
            .value("Assignee")
            .unwrap_or_else(|| SECRETARY.to_owned());
        let (input, made) = if assignee == SECRETARY {
            let twins = self
                .records
                .list(ROUTINES)?
                .iter()
                .filter(|r| r.title() == record.title())
                .count();
            let name = if twins > 1 {
                format!(
                    "{} ({})",
                    record.title(),
                    self.records.code(ROUTINES, &record.id)?
                )
            } else {
                record.title()
            };
            let notes = record.notes();
            let input = Input {
                sender: Sender::System,
                source: Source::Schedule {
                    name,
                    at: run.to_rfc3339(),
                    routine: record.id.clone(),
                },
                text: if notes.is_empty() {
                    record.title()
                } else {
                    notes
                },
            };
            let key = format!("input:{}", input.dedup_key());
            (Some(input), key)
        } else {
            let task = self.records.add(
                Writer::Secretary,
                TASKS,
                Patch {
                    notes: Some(record.notes()),
                    labels: Some(record.labels()),
                    workgroups: Some(record.workgroups()),
                    source: "定期タスク".into(),
                    ..Patch::titled(record.title())
                }
                .with("Assignee", assignee)
                .with("Start", run.format("%Y-%m-%d %H:%M").to_string())
                .with("Related", record.link()),
            )?;
            crate::disclosure::carry_routine(&self.store, &record.id, &task.id)?;
            (None, task.id)
        };
        self.store.conn().execute(
            "UPDATE routine_runs SET last_run = ?2, task = ?3 WHERE routine = ?1",
            params![record.id, now.timestamp(), made],
        )?;
        Ok(input)
    }

    /// Why a run is skipped, if a condition says so, and what the owner
    /// should know about the conditions.
    fn held_back(&self, routine: &Routine) -> anyhow::Result<(Option<String>, Vec<String>)> {
        let mut notes = Vec::new();
        for item in routine.record.body.items("Skip if") {
            let text = item.plain();
            if text.trim().eq_ignore_ascii_case("previous unfinished") {
                if self.previous_unfinished(&routine.record.id)? {
                    return Ok((Some("前の回がまだ済んでいない".into()), notes));
                }
                continue;
            }
            let Some((TASKS, id)) = record_of(link_target(&text)) else {
                notes.push(format!("飛ばす条件が読めない：{text}"));
                continue;
            };
            let Some(task) = self.records.get(TASKS, id)? else {
                notes.push(format!("飛ばす条件のタスクがない：{text}"));
                continue;
            };
            let status = task.body.value("Status").unwrap_or_default();
            let among = item
                .nested("Status")
                .map(|i| i.values())
                .unwrap_or_default();
            if among.contains(&status) {
                return Ok((Some(format!("{} が {status}", task.title())), notes));
            }
        }
        Ok((None, notes))
    }

    fn previous_unfinished(&self, id: &str) -> anyhow::Result<bool> {
        let made: Option<String> = self
            .store
            .conn()
            .query_row(
                "SELECT task FROM routine_runs WHERE routine = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let Some(made) = made else { return Ok(false) };
        match made.strip_prefix("input:") {
            Some(key) => Ok(self
                .store
                .conn()
                .query_row(
                    "SELECT 1 FROM inputs WHERE dedup_key = ?1 AND status IN ('pending', 'running')",
                    [key],
                    |_| Ok(()),
                )
                .optional()?
                .is_some()),
            None => Ok(self
                .records
                .get(TASKS, &made)?
                .is_some_and(|t| !is_over(&t))),
        }
    }

    fn local(&self, timestamp: i64) -> DateTime<Tz> {
        self.tz
            .timestamp_opt(timestamp, 0)
            .single()
            .unwrap_or_else(|| self.tz.timestamp_opt(0, 0).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::testing::TempDir;
    use crate::records::{Entry, Field, OWNER, SectionContent, log_lines, testing::records};

    const TOKYO: Tz = chrono_tz::Asia::Tokyo;

    fn setup(name: &str) -> (TempDir, Scheduler) {
        let (tmp, records) = records(name);
        (
            tmp,
            Scheduler::new(Store::open_in_memory().unwrap(), records, TOKYO),
        )
    }

    fn scheduler() -> Scheduler {
        static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // The directory may go: wake-ups touch no record.
        setup(&format!("schedule-wake-{n}")).1
    }

    fn tokyo(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        TOKYO
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn routine(s: &Scheduler, title: &str, cron: &str, patch: Patch) -> String {
        let patch = Patch {
            title: Some(title.into()),
            notes: Some(format!("run {title}")),
            ..patch
        };
        s.records
            .add(Writer::Owner, ROUTINES, patch.with("Schedule", cron))
            .unwrap()
            .id
    }

    fn next(s: &Scheduler, id: &str, now: DateTime<Utc>) -> Option<String> {
        s.get(id, now).unwrap().next_run.map(|t| t.to_rfc3339())
    }

    fn log(s: &Scheduler, id: &str) -> Vec<String> {
        log_lines(&s.records.need(ROUTINES, id).unwrap())
            .into_iter()
            .map(|l| l.text)
            .collect()
    }

    #[test]
    fn a_due_routine_becomes_an_input_and_moves_to_its_next_run() {
        let (_tmp, s) = setup("schedule-due");
        let id = routine(&s, "morning", "0 8 * * *", Patch::default());
        let seven = tokyo(2026, 11, 21, 7, 0);
        assert_eq!(
            next(&s, &id, seven).as_deref(),
            Some("2026-11-21T08:00:00+09:00")
        );
        assert!(s.due(tokyo(2026, 11, 21, 7, 59)).unwrap().is_empty());
        let inputs = s.due(tokyo(2026, 11, 21, 8, 0)).unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].sender, Sender::System);
        assert_eq!(inputs[0].text, "run morning");
        assert_eq!(
            inputs[0].render().lines().next().unwrap(),
            "[schedule] name: morning at: 2026-11-21T08:00:00+09:00"
        );
        assert!(s.due(tokyo(2026, 11, 21, 8, 1)).unwrap().is_empty());
        assert_eq!(
            next(&s, &id, seven).as_deref(),
            Some("2026-11-22T08:00:00+09:00")
        );
        assert!(s.get(&id, seven).unwrap().last_run.is_some());
    }

    #[test]
    fn missed_runs_fire_once_after_downtime() {
        let (_tmp, s) = setup("schedule-missed");
        let id = routine(&s, "patrol", "*/30 * * * *", Patch::default());
        s.list(tokyo(2026, 11, 21, 9, 0)).unwrap();
        let now = tokyo(2026, 11, 21, 12, 10);
        assert_eq!(s.due(now).unwrap().len(), 1);
        assert_eq!(
            next(&s, &id, now).as_deref(),
            Some("2026-11-21T12:30:00+09:00")
        );
    }

    #[test]
    fn a_weekday_routine_skips_the_weekend_and_several_lines_all_count() {
        let (_tmp, s) = setup("schedule-weekday");
        // 2026-11-21 is a Saturday.
        let now = tokyo(2026, 11, 21, 10, 0);
        let one = routine(&s, "a", "0 9 * * Mon-Fri", Patch::default());
        assert_eq!(
            next(&s, &one, now).as_deref(),
            Some("2026-11-23T09:00:00+09:00")
        );
        let lists = Patch {
            fields: vec![Field {
                name: "Schedule".into(),
                value: String::new(),
                values: vec!["0 9 * * 1,3".into(), "0 22 * * 6".into()],
            }],
            ..Patch::titled("b")
        };
        let two = s.records.add(Writer::Owner, ROUTINES, lists).unwrap().id;
        assert_eq!(
            next(&s, &two, now).as_deref(),
            Some("2026-11-21T22:00:00+09:00")
        );
    }

    #[test]
    fn a_routine_keeps_to_its_own_time_zone_and_its_start_and_end() {
        let (_tmp, s) = setup("schedule-bounds");
        let now = tokyo(2026, 11, 21, 10, 0);
        let patch = Patch::default()
            .with("Timezone", "Europe/Paris")
            .with("Start", "2026-12-01 08:00")
            .with("Until", "2026-12-02");
        let id = routine(&s, "a", "0 8 * * *", patch);
        let r = s.get(&id, now).unwrap();
        let runs: Vec<String> = r
            .runs(now, 9)
            .into_iter()
            .map(|(t, _)| t.to_rfc3339())
            .collect();
        assert_eq!(
            runs,
            ["2026-12-01T08:00:00+01:00", "2026-12-02T08:00:00+01:00"]
        );
    }

    #[test]
    fn an_unchanged_routine_keeps_its_next_run_and_a_changed_one_gets_a_new_one() {
        let (_tmp, s) = setup("schedule-keep");
        let id = routine(&s, "a", "0 8 * * *", Patch::default());
        s.list(tokyo(2026, 11, 21, 7, 0)).unwrap();
        let again = Scheduler::new(s.store.clone(), s.records.clone(), TOKYO);
        let nine = tokyo(2026, 11, 21, 9, 0);
        assert_eq!(
            next(&again, &id, nine).as_deref(),
            Some("2026-11-21T08:00:00+09:00")
        );
        s.records
            .change(
                Writer::Owner,
                ROUTINES,
                &id,
                Patch::default().with("Schedule", "0 10 * * *"),
            )
            .unwrap();
        assert_eq!(
            next(&s, &id, nine).as_deref(),
            Some("2026-11-21T10:00:00+09:00")
        );
        let paris = Scheduler::new(s.store.clone(), s.records.clone(), chrono_tz::Europe::Paris);
        assert_eq!(
            next(&paris, &id, nine).as_deref(),
            Some("2026-11-21T10:00:00+01:00")
        );
    }

    #[test]
    fn a_routine_that_is_stopped_or_unreadable_does_not_run() {
        let (_tmp, s) = setup("schedule-stopped");
        routine(&s, "a", "0 8 * * *", Patch::default().with("Enabled", "no"));
        let bad = routine(&s, "b", "daily", Patch::default());
        s.list(tokyo(2026, 11, 21, 7, 0)).unwrap();
        assert!(s.due(tokyo(2026, 11, 21, 8, 0)).unwrap().is_empty());
        assert!(s.get(&bad, Utc::now()).unwrap().problem.is_some());
    }

    #[test]
    fn a_routine_whose_time_zone_is_put_right_runs_again() {
        let (_tmp, s) = setup("schedule-timezone");
        let id = routine(
            &s,
            "朝",
            "0 9 * * *",
            Patch::default().with("Timezone", "Asia/Tokio"),
        );
        let now = tokyo(2026, 11, 23, 8, 0);
        assert_eq!(next(&s, &id, now), None);

        let right = Patch::default().with("Timezone", "Asia/Tokyo");
        s.records
            .change(Writer::Owner, ROUTINES, &id, right)
            .unwrap();

        assert!(next(&s, &id, now).is_some());
    }

    #[test]
    fn a_skipped_run_does_not_run_and_the_next_one_does() {
        let (_tmp, s) = setup("schedule-skip");
        let id = routine(&s, "a", "0 8 * * *", Patch::default());
        let now = tokyo(2026, 11, 21, 7, 0);
        s.list(now).unwrap();
        let eight = tokyo(2026, 11, 21, 8, 0);
        s.skip(Writer::Owner, &id, eight).unwrap();
        s.skip(Writer::Owner, &id, eight).unwrap();
        assert!(
            s.skip(Writer::Owner, &id, tokyo(2026, 11, 21, 8, 30))
                .is_err()
        );
        assert_eq!(
            s.get(&id, now)
                .unwrap()
                .runs(now, 2)
                .iter()
                .map(|(_, skipped)| *skipped)
                .collect::<Vec<_>>(),
            [true, false]
        );

        assert!(s.due(eight).unwrap().is_empty());
        assert_eq!(log(&s, &id), ["2026-11-21 08:00 の回を飛ばした"]);
        assert!(
            s.records
                .need(ROUTINES, &id)
                .unwrap()
                .body
                .values("Skip")
                .is_empty()
        );
        assert_eq!(s.due(tokyo(2026, 11, 22, 8, 0)).unwrap().len(), 1);

        let third = tokyo(2026, 11, 23, 8, 0);
        s.skip(Writer::Owner, &id, third).unwrap();
        s.unskip(Writer::Owner, &id, third).unwrap();
        assert_eq!(s.due(third).unwrap().len(), 1);
    }

    fn with_condition(text: String, status: &str) -> Patch {
        let mut entry = Entry {
            text,
            fields: Vec::new(),
        };
        if !status.is_empty() {
            entry.fields.push(Field::new("Status", status));
        }
        Patch {
            sections: vec![SectionContent {
                name: "Skip if".into(),
                items: vec![entry],
                ..SectionContent::default()
            }],
            ..Patch::default()
        }
    }

    #[test]
    fn runs_are_skipped_while_a_task_is_as_the_condition_says() {
        let (_tmp, s) = setup("schedule-condition");
        let task = s
            .records
            .add(Writer::Owner, TASKS, Patch::titled("見積もり"))
            .unwrap();
        let id = routine(
            &s,
            "a",
            "0 8 * * *",
            with_condition(task.link(), "pending, waiting"),
        );
        s.list(tokyo(2026, 11, 21, 7, 0)).unwrap();

        assert!(s.due(tokyo(2026, 11, 21, 8, 0)).unwrap().is_empty());
        assert!(log(&s, &id)[0].contains("見積もり が pending"));
        s.records
            .change(
                Writer::Owner,
                TASKS,
                &task.id,
                Patch::default().with("Status", "done"),
            )
            .unwrap();
        assert_eq!(s.due(tokyo(2026, 11, 22, 8, 0)).unwrap().len(), 1);
        s.records.remove(Writer::Owner, TASKS, &task.id).unwrap();
        assert_eq!(s.due(tokyo(2026, 11, 23, 8, 0)).unwrap().len(), 1);
        assert!(log(&s, &id).last().unwrap().contains("タスクがない"));
    }

    #[test]
    fn a_routine_of_the_owners_makes_a_task_and_waits_for_the_one_before() {
        let (_tmp, s) = setup("schedule-task");
        let patch = with_condition("previous unfinished".into(), "").with("Assignee", OWNER);
        routine(&s, "掃除", "0 8 * * *", patch);
        s.list(tokyo(2026, 11, 21, 7, 0)).unwrap();

        assert!(s.due(tokyo(2026, 11, 21, 8, 0)).unwrap().is_empty());
        s.due(tokyo(2026, 11, 22, 8, 0)).unwrap();
        let tasks = s.records.list(TASKS).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].body.value("Assignee").as_deref(), Some(OWNER));
        s.records
            .change(
                Writer::Owner,
                TASKS,
                &tasks[0].id,
                Patch::default().with("Status", "done"),
            )
            .unwrap();
        s.due(tokyo(2026, 11, 23, 8, 0)).unwrap();
        assert_eq!(s.records.list(TASKS).unwrap().len(), 2);
    }

    #[test]
    fn a_routine_of_the_secretarys_waits_while_its_input_is_not_done() {
        let (_tmp, s) = setup("schedule-input");
        let patch = with_condition("previous unfinished".into(), "");
        routine(&s, "a", "0 8 * * *", patch);
        s.list(tokyo(2026, 11, 21, 7, 0)).unwrap();
        let first = s.due(tokyo(2026, 11, 21, 8, 0)).unwrap();
        s.store.enqueue(&first[0]).unwrap();

        assert!(s.due(tokyo(2026, 11, 22, 8, 0)).unwrap().is_empty());
    }

    #[test]
    fn a_wakeup_fires_once_when_due() {
        let s = scheduler();
        let now = tokyo(2026, 11, 21, 12, 0);
        let id = s
            .wake_at(tokyo(2026, 11, 21, 13, 0), "check the deploy", "", now)
            .unwrap();
        assert!(s.due(tokyo(2026, 11, 21, 12, 59)).unwrap().is_empty());
        let inputs = s.due(tokyo(2026, 11, 21, 13, 0)).unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].text, "check the deploy");
        assert!(matches!(inputs[0].source, Source::Wake { id: got, .. } if got == id));
        assert!(s.due(tokyo(2026, 11, 21, 14, 0)).unwrap().is_empty());
    }

    #[test]
    fn a_wakeup_in_the_past_is_refused() {
        let s = scheduler();
        let now = tokyo(2026, 11, 21, 12, 0);
        assert!(
            s.wake_at(tokyo(2026, 11, 21, 11, 0), "late", "", now)
                .is_err()
        );
    }

    #[test]
    fn pending_wakeups_leave_out_the_fired_and_the_cancelled_soonest_first() {
        let s = scheduler();
        let now = tokyo(2026, 11, 21, 12, 0);
        s.wake_at(tokyo(2026, 11, 21, 12, 30), "fires", "", now)
            .unwrap();
        let cancelled = s
            .wake_at(tokyo(2026, 11, 21, 15, 0), "cancelled", "", now)
            .unwrap();
        s.wake_at(
            tokyo(2026, 11, 21, 18, 0),
            "ask whether the quote arrived",
            "the owner is waiting for a quote",
            now,
        )
        .unwrap();
        s.wake_at(tokyo(2026, 11, 21, 14, 0), "check the deploy", "", now)
            .unwrap();
        s.due(tokyo(2026, 11, 21, 13, 0)).unwrap();
        s.cancel_wakeup(cancelled).unwrap();

        let pending = s.pending_wakeups().unwrap();
        let notes: Vec<&str> = pending.iter().map(|w| w.note.as_str()).collect();
        assert_eq!(notes, ["check the deploy", "ask whether the quote arrived"]);
        assert_eq!(pending[1].why, "the owner is waiting for a quote");
        assert_eq!(pending[1].at.to_rfc3339(), "2026-11-21T18:00:00+09:00");
    }

    #[test]
    fn a_cancelled_wakeup_does_not_fire() {
        let s = scheduler();
        let now = tokyo(2026, 11, 21, 12, 0);
        let id = s
            .wake_at(tokyo(2026, 11, 21, 13, 0), "check the deploy", "", now)
            .unwrap();
        s.cancel_wakeup(id).unwrap();
        assert!(s.due(tokyo(2026, 11, 21, 14, 0)).unwrap().is_empty());
    }

    #[test]
    fn only_a_wakeup_that_is_still_pending_can_be_cancelled() {
        let s = scheduler();
        let now = tokyo(2026, 11, 21, 12, 0);
        let id = s
            .wake_at(tokyo(2026, 11, 21, 13, 0), "check the deploy", "", now)
            .unwrap();
        s.due(tokyo(2026, 11, 21, 13, 0)).unwrap();
        assert!(s.cancel_wakeup(id).is_err(), "it has already fired");
        assert!(
            s.cancel_wakeup(id + 100).is_err(),
            "there is no such wake-up"
        );
    }
}
