//! The owner's calendars and events. Calendars are records of the notebook.
//! An event of a calendar kept in Google stays there: it is read through the
//! connection, with a cache so that showing it does not wait. Every other
//! event, holds among them, is a record of the notebook.

mod google;
mod occurrences;

pub use google::*;

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::notebook::Writer;
use crate::records::{CALENDARS, EVENTS, Entry, Field, Patch, Record, Records, SectionContent};
use crate::store::{Store, now};
use crate::workgroups::{ALL, CalendarMode};

/// How far back and ahead a refresh keeps events.
const KEEP_BEFORE_DAYS: i64 = 31;
const KEEP_AFTER_DAYS: i64 = 180;

const SHOW: &str = "Show";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    /// The record's id.
    pub id: String,
    pub name: String,
    pub color: String,
    /// The connection and calendar id of a calendar kept in Google.
    pub google: Option<(String, String)>,
    /// The workgroups whose new events go here.
    pub default_for: Vec<String>,
    pub show: BTreeMap<String, CalendarMode>,
    pub fetched_at: Option<i64>,
    pub error: Option<String>,
}

impl Calendar {
    /// How it shows in a workgroup.
    pub fn mode(&self, workgroup: &str) -> CalendarMode {
        if workgroup == ALL {
            return CalendarMode::Show;
        }
        self.show
            .get(workgroup)
            .copied()
            .unwrap_or(CalendarMode::Show)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Attendee {
    pub name: String,
    pub email: String,
    pub response: String,
    pub organizer: bool,
    pub optional: bool,
    /// The party's record id, for one the notebook names.
    pub party: String,
}

/// One occurrence of an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// The calendar's record id; empty for none.
    pub calendar: String,
    pub uid: String,
    pub start: DateTime<Utc>,
    /// For an event of whole days, the start of the day after the last.
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub title: String,
    /// hold, tentative, confirmed or cancelled.
    pub status: String,
    pub busy: bool,
    /// The owner's own answer.
    pub response: String,
    pub location: String,
    pub map: String,
    pub meeting: String,
    /// Markdown.
    pub notes: String,
    pub attendees: Vec<Attendee>,
    /// Where it opens outside.
    pub url: String,
    pub repeats: bool,
    /// The id the series goes by, for an occurrence of one.
    pub series: String,
    /// The record that holds it, or that adds to it.
    pub record: String,
    pub labels: Vec<String>,
    pub workgroups: Vec<String>,
}

#[derive(Clone)]
pub struct Calendars {
    store: Store,
    records: Records,
}

impl Calendars {
    pub fn new(store: Store, records: Records) -> Self {
        Self { store, records }
    }

    /// The span a refresh keeps, around `now_at`.
    pub fn window(now_at: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
        (
            now_at - Duration::days(KEEP_BEFORE_DAYS),
            now_at + Duration::days(KEEP_AFTER_DAYS),
        )
    }

    /// Add a calendar, kept in Google when `google` names its connection and
    /// calendar id.
    pub fn add(
        &self,
        writer: Writer,
        name: &str,
        google: Option<(&str, &str)>,
        color: &str,
    ) -> anyhow::Result<Calendar> {
        if let Some((conn, id)) = google {
            let there = self
                .list()?
                .into_iter()
                .find(|c| c.google.as_ref().is_some_and(|g| g.0 == conn && g.1 == id));
            if let Some(there) = there {
                return Ok(there);
            }
        }
        let record = self.records.add(
            writer,
            CALENDARS,
            Patch {
                google: google.map(|(conn, id)| format!("{conn}/{id}")),
                ..Patch::titled(name)
            }
            .with("Color", color),
        )?;
        self.get(&record.id)?
            .ok_or_else(|| anyhow::anyhow!("the calendar vanished"))
    }

    pub fn list(&self) -> anyhow::Result<Vec<Calendar>> {
        let reads: BTreeMap<String, (Option<i64>, Option<String>)> = {
            let conn = self.store.conn();
            let mut stmt =
                conn.prepare("SELECT calendar, fetched_at, error FROM calendar_reads")?;
            stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
                .collect::<Result<_, _>>()?
        };
        Ok(self
            .records
            .list(CALENDARS)?
            .iter()
            .map(|r| {
                let (fetched_at, error) = reads.get(&r.id).cloned().unwrap_or_default();
                Calendar {
                    id: r.id.clone(),
                    name: r.title(),
                    color: r.body.value("Color").unwrap_or_default(),
                    google: r.meta.text("google").and_then(|g| {
                        let (conn, id) = g.split_once('/')?;
                        Some((conn.to_owned(), id.to_owned()))
                    }),
                    default_for: r.body.values("Default for"),
                    show: r
                        .body
                        .items(SHOW)
                        .into_iter()
                        .filter_map(|i| {
                            let mode = CalendarMode::parse(&i.nested("Mode")?.value())?;
                            Some((i.plain(), mode))
                        })
                        .collect(),
                    fetched_at,
                    error,
                }
            })
            .collect())
    }

    pub fn get(&self, id: &str) -> anyhow::Result<Option<Calendar>> {
        Ok(self.list()?.into_iter().find(|c| c.id == id))
    }

    /// How a calendar shows in a workgroup.
    pub fn set_mode(
        &self,
        writer: Writer,
        calendar: &str,
        workgroup: &str,
        mode: CalendarMode,
    ) -> anyhow::Result<()> {
        let mut show = self
            .get(calendar)?
            .ok_or_else(|| anyhow::anyhow!("no calendar {calendar}"))?
            .show;
        show.insert(workgroup.to_owned(), mode);
        self.save_show(writer, calendar, show)
    }

    fn save_show(
        &self,
        writer: Writer,
        calendar: &str,
        show: BTreeMap<String, CalendarMode>,
    ) -> anyhow::Result<()> {
        self.records.change(
            writer,
            CALENDARS,
            calendar,
            Patch {
                sections: vec![SectionContent {
                    name: SHOW.into(),
                    items: show
                        .into_iter()
                        .map(|(workgroup, mode)| Entry {
                            text: workgroup,
                            fields: vec![Field::new("Mode", mode.as_str())],
                        })
                        .collect(),
                    ..SectionContent::default()
                }],
                ..Patch::default()
            },
        )?;
        Ok(())
    }

    /// Take a workgroup that is gone out of every calendar.
    pub fn forget_workgroup(&self, writer: Writer, workgroup: &str) -> anyhow::Result<()> {
        for mut c in self.list()? {
            if c.show.remove(workgroup).is_some() {
                self.save_show(writer, &c.id, c.show)?;
            }
        }
        Ok(())
    }

    /// Remove a calendar. Events in Google stay there, and events of the
    /// notebook stay without a calendar.
    pub fn remove(&self, writer: Writer, id: &str) -> anyhow::Result<()> {
        for link in self.records.backlinks(CALENDARS, id)? {
            if link.kind == EVENTS && link.from == "Calendar" {
                let without = Patch::default().with("Calendar", "");
                self.records.change(writer, EVENTS, &link.id, without)?;
            }
        }
        self.records.remove(writer, CALENDARS, id)?;
        let conn = self.store.conn();
        conn.execute("DELETE FROM calendar_cache WHERE calendar = ?1", [id])?;
        conn.execute("DELETE FROM calendar_reads WHERE calendar = ?1", [id])?;
        Ok(())
    }

    /// Replace a calendar's cached events.
    pub fn store_events(&self, calendar: &str, events: &[Event]) -> anyhow::Result<usize> {
        let mut conn = self.store.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM calendar_cache WHERE calendar = ?1", [calendar])?;
        for e in events {
            tx.execute(
                "INSERT OR REPLACE INTO calendar_cache (calendar, uid, start, end, event)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    calendar,
                    e.uid,
                    e.start.timestamp(),
                    e.end.timestamp(),
                    serde_json::to_string(e)?
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO calendar_reads (calendar, fetched_at, error) VALUES (?1, ?2, NULL)
             ON CONFLICT (calendar) DO UPDATE SET fetched_at = ?2, error = NULL",
            params![calendar, now()],
        )?;
        tx.commit()?;
        Ok(events.len())
    }

    /// Record a failed read; the events read before stay.
    pub fn store_error(&self, calendar: &str, error: impl AsRef<str>) -> anyhow::Result<()> {
        self.store.conn().execute(
            "INSERT INTO calendar_reads (calendar, error) VALUES (?1, ?2)
             ON CONFLICT (calendar) DO UPDATE SET error = ?2",
            params![calendar, error.as_ref()],
        )?;
        Ok(())
    }

    /// Events overlapping `[from, to)`, by start: Google's from the cache,
    /// with what the notebook adds to them, and the notebook's own.
    pub fn events(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        tz: Tz,
    ) -> anyhow::Result<Vec<Event>> {
        let known = self.list()?;
        let mut out: Vec<Event> = {
            let conn = self.store.conn();
            let mut stmt = conn.prepare(
                "SELECT calendar, event FROM calendar_cache WHERE start < ?2 AND end > ?1",
            )?;
            let rows = stmt.query_map(params![from.timestamp(), to.timestamp()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut events = Vec::new();
            for row in rows {
                let (calendar, event) = row?;
                if !known.iter().any(|c| c.id == calendar) {
                    continue;
                }
                let mut event: Event = serde_json::from_str(&event)?;
                event.calendar = calendar;
                events.push(event);
            }
            events
        };
        for record in self.records.list(EVENTS)? {
            match record.meta.text("google") {
                Some(id) => {
                    for e in out.iter_mut().filter(|e| e.uid == id || e.series == id) {
                        add_to(e, &record);
                    }
                }
                None => match occurrences::occurrences(&record, from, to, tz) {
                    Ok(mine) => out.extend(mine),
                    Err(error) => {
                        tracing::debug!(%error, event = %record.id, "an event is not readable")
                    }
                },
            }
        }
        out.sort_by(|a, b| (a.start, &a.title).cmp(&(b.start, &b.title)));
        Ok(out)
    }
}

/// What a record adds to an event kept in Google.
fn add_to(event: &mut Event, record: &Record) {
    event.record = record.id.clone();
    event.labels = record.labels();
    event.workgroups = record.workgroups();
    let notes = record.notes();
    if !notes.is_empty() {
        event.notes = if event.notes.is_empty() {
            notes
        } else {
            format!("{}\n\n---\n\n{notes}", event.notes)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::occurrences::read_time;
    use super::*;
    use crate::notebook::testing::TempDir;
    use crate::records::testing::records;

    pub(super) const TZ: Tz = chrono_tz::Asia::Tokyo;

    fn calendars(name: &str) -> (TempDir, Calendars) {
        let (tmp, records) = records(name);
        (
            tmp,
            Calendars::new(Store::open_in_memory().unwrap(), records),
        )
    }

    fn at(text: &str) -> DateTime<Utc> {
        read_time(text, TZ).unwrap().0
    }

    fn write(c: &Calendars, body: &str) -> String {
        let id = famulen_notebook::records::fresh_id();
        let dir = c.records.notebook().root().join("records/events");
        std::fs::write(
            dir.join(format!("{id}.md")),
            format!("---\nid: {id}\n---\n\n{body}"),
        )
        .unwrap();
        id
    }

    pub(super) const PAGE: &str = r#"{"items":[
        {"id":"a","summary":"定例","status":"confirmed","htmlLink":"https://calendar.example/a",
         "start":{"dateTime":"2026-11-23T14:00:00+09:00"},"end":{"dateTime":"2026-11-23T14:30:00+09:00"},
         "location":"渋谷","hangoutLink":"https://meet.example/x","transparency":"transparent",
         "description":"議題<br><ul><li>一つ</li></ul><a href=\"https://x.example/?a=1&amp;b=2\">資料</a>",
         "organizer":{"email":"t@example.com"},"recurringEventId":"series",
         "attendees":[{"email":"t@example.com","displayName":"田中","responseStatus":"accepted"},
                      {"email":"me@example.com","self":true,"responseStatus":"needsAction","optional":true},
                      {"email":"room@example.com","resource":true}]},
        {"id":"b","summary":"休み","start":{"date":"2026-11-24"},"end":{"date":"2026-11-25"}},
        {"id":"c","status":"cancelled","start":{"date":"2026-11-24"},"end":{"date":"2026-11-25"}}
    ]}"#;

    #[test]
    fn a_calendar_is_a_record_and_shows_as_each_workgroup_was_told() {
        let (_tmp, c) = calendars("calendars-records");
        let cal = c
            .add(
                Writer::Owner,
                "仕事",
                Some(("google-2", "primary")),
                "#2a78d6",
            )
            .unwrap();
        let again = c
            .add(
                Writer::Owner,
                "別名",
                Some(("google-2", "primary")),
                "#000000",
            )
            .unwrap();
        assert_eq!(cal.id, again.id);

        c.set_mode(Writer::Owner, &cal.id, "personal", CalendarMode::Faint)
            .unwrap();

        let cal = c.get(&cal.id).unwrap().unwrap();
        assert_eq!(cal.google, Some(("google-2".into(), "primary".into())));
        assert_eq!(cal.mode("personal"), CalendarMode::Faint);
        assert_eq!(cal.mode("work"), CalendarMode::Show);
        assert_eq!(cal.mode(ALL), CalendarMode::Show);
        c.forget_workgroup(Writer::Owner, "personal").unwrap();
        assert!(c.get(&cal.id).unwrap().unwrap().show.is_empty());
    }

    #[test]
    fn events_in_google_are_not_copied_and_a_record_adds_its_notes_to_one() {
        let (_tmp, c) = calendars("calendars-google");
        let cal = c
            .add(Writer::Owner, "仕事", Some(("g", "primary")), "#2a78d6")
            .unwrap();
        c.store_events(&cal.id, &google_events(&[PAGE.to_owned()], TZ).unwrap())
            .unwrap();
        c.store_error(&cal.id, "読めない").unwrap();
        assert!(c.records.list(EVENTS).unwrap().is_empty());

        c.records
            .add(
                Writer::Owner,
                EVENTS,
                Patch {
                    google: Some("series".into()),
                    notes: Some("持ち物".into()),
                    labels: Some(vec!["準備".into()]),
                    ..Patch::titled("定例")
                },
            )
            .unwrap();

        let events = c.events(at("2026-11-23"), at("2026-11-26"), TZ).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events[0].notes.ends_with("持ち物"));
        assert_eq!(events[0].labels, ["準備"]);
        assert_eq!(c.list().unwrap()[0].error.as_deref(), Some("読めない"));
        c.remove(Writer::Owner, &cal.id).unwrap();
        assert!(
            c.events(at("2026-11-23"), at("2026-11-26"), TZ)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn events_of_the_notebook_show_in_their_time_and_whole_days_to_their_last() {
        let (_tmp, c) = calendars("calendars-notebook");
        write(
            &c,
            "# 歯医者\n\n- Status: hold\n- Start: 2026-11-25 14:00\n- End: 2026-11-25 15:00\n- Map: <https://maps.example/x>\n\n## Attendees\n\n- [[records/parties/p|田中]]\n  - Response: accepted\n",
        );
        write(&c, "# 旅行\n\n- Start: 2026-11-25\n- End: 2026-11-26\n");
        write(&c, "# やめた\n\n- Status: cancelled\n- Start: 2026-11-25\n");
        write(&c, "# 読めない\n\n- Start: いつか\n");

        let events = c.events(at("2026-11-25"), at("2026-11-26"), TZ).unwrap();

        let titles: Vec<&str> = events.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["旅行", "歯医者"]);
        assert_eq!(events[0].end, at("2026-11-27"));
        assert_eq!(events[1].status, "hold");
        assert_eq!(events[1].map, "https://maps.example/x");
        assert_eq!(events[1].attendees[0].party, "p");
        assert_eq!(events[1].attendees[0].name, "田中");
        assert!(
            c.events(at("2026-11-27"), at("2026-11-28"), TZ)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_repeating_event_is_shown_by_its_occurrences_with_the_ones_that_differ() {
        let (_tmp, c) = calendars("calendars-repeat");
        write(
            &c,
            "# 定例\n\n- Start: 2026-11-25 14:00\n- End: 2026-11-25 15:00\n- Repeat: FREQ=WEEKLY;BYDAY=WE\n\n## Exceptions\n\n- 2026-12-02 14:00\n  - Status: cancelled\n- 2026-12-09 14:00\n  - Title: 拡大\n  - Start: 2026-12-10 10:00\n  - End: 2026-12-10 12:00\n- 2026-12-12 09:00\n",
        );

        let events = c.events(at("2026-11-24"), at("2026-12-17"), TZ).unwrap();

        let shown: Vec<(String, &str)> = events
            .iter()
            .map(|e| {
                (
                    e.start.with_timezone(&TZ).format("%m-%d %H:%M").to_string(),
                    e.title.as_str(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("11-25 14:00".to_owned(), "定例"),
                ("12-10 10:00".to_owned(), "拡大"),
                ("12-12 09:00".to_owned(), "定例"),
                ("12-16 14:00".to_owned(), "定例"),
            ]
        );
        assert_eq!(events[1].end - events[1].start, Duration::hours(2));
        assert!(events.iter().all(|e| e.repeats));
    }

    #[test]
    fn a_window_reaches_back_a_month_and_ahead_half_a_year() {
        let (from, to) = Calendars::window(at("2026-11-25"));
        assert!(from < at("2026-10-30") && to > at("2027-05-01"));
    }
}
