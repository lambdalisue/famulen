//! When an event of the notebook takes place.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use famulen_notebook::body::Item;
use famulen_notebook::records::{flag, minute};

use super::{Attendee, Event};
use crate::notebook::link_target;
use crate::records::Record;

/// The most occurrences of one repeating event shown at once.
const MOST_OCCURRENCES: u16 = 1000;

/// The occurrences of an event of the notebook in `[from, to)`.
pub(super) fn occurrences(
    record: &Record,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    owner_tz: Tz,
) -> anyhow::Result<Vec<Event>> {
    let body = &record.body;
    let tz = match body.value("Timezone") {
        Some(name) => name
            .parse::<Tz>()
            .map_err(|_| anyhow::anyhow!("unknown time zone {name}"))?,
        None => owner_tz,
    };
    let text = |name: &str| body.value(name).unwrap_or_default();
    let url = |name: &str| bare_url(&text(name));
    let start = text("Start");
    let (first, all_day) =
        read_time(&start, tz).ok_or_else(|| anyhow::anyhow!("no start: {start:?}"))?;
    let length = match read_time(&text("End"), tz) {
        Some((end, true)) if all_day => end + Duration::days(1) - first,
        Some((end, false)) if !all_day => end - first,
        _ if all_day => Duration::days(1),
        _ => Duration::hours(1),
    };
    let attendees = body
        .items("Attendees")
        .into_iter()
        .map(attendee)
        .collect::<Vec<_>>();
    let repeat = body.value("Repeat");
    let base = Event {
        calendar: famulen_notebook::records::id_of(&text("Calendar")),
        uid: record.id.clone(),
        start: first,
        end: first + length,
        all_day,
        title: record.title(),
        status: body.value("Status").unwrap_or_else(|| "confirmed".into()),
        busy: flag(&text("Busy")).unwrap_or(true),
        response: text("Response"),
        location: text("Location"),
        map: url("Map"),
        meeting: url("Meeting"),
        notes: record.notes(),
        attendees,
        url: String::new(),
        repeats: repeat.is_some(),
        series: record.id.clone(),
        record: record.id.clone(),
        labels: record.labels(),
        workgroups: record.workgroups(),
    };
    let overlaps = |e: &Event| e.start < to && e.end > from;
    let Some(rule) = repeat else {
        let one = vec![base];
        return Ok(one
            .into_iter()
            .filter(|e| e.status != "cancelled" && overlaps(e))
            .collect());
    };
    let starts = repeat_starts(&rule, first, tz, from - length, to)?;
    let mut exceptions: BTreeMap<DateTime<Utc>, &Item> = body
        .items("Exceptions")
        .into_iter()
        .filter_map(|i| Some((read_time(&i.plain(), tz)?.0, i)))
        .collect();
    let mut out: Vec<Event> = starts
        .into_iter()
        .map(|at| occurrence(&base, at, length, exceptions.remove(&at), tz))
        .collect();
    out.extend(
        exceptions
            .into_iter()
            .map(|(at, item)| occurrence(&base, at, length, Some(item), tz)),
    );
    out.retain(|e| e.status != "cancelled" && overlaps(e));
    Ok(out)
}

fn occurrence(
    base: &Event,
    at: DateTime<Utc>,
    length: Duration,
    exception: Option<&Item>,
    tz: Tz,
) -> Event {
    let mut e = base.clone();
    e.uid = format!("{}@{}", base.uid, at.timestamp());
    e.start = at;
    e.end = at + length;
    let Some(item) = exception else { return e };
    let of = |name: &str| item.nested(name).map(Item::value).filter(|v| !v.is_empty());
    if let Some((start, _)) = of("Start").and_then(|t| read_time(&t, tz)) {
        e.start = start;
        e.end = start + length;
    }
    if let Some((end, whole)) = of("End").and_then(|t| read_time(&t, tz)) {
        e.end = if whole { end + Duration::days(1) } else { end };
    }
    if let Some(busy) = of("Busy").and_then(|t| flag(&t)) {
        e.busy = busy;
    }
    for (slot, name) in [
        (&mut e.title, "Title"),
        (&mut e.status, "Status"),
        (&mut e.response, "Response"),
        (&mut e.location, "Location"),
        (&mut e.notes, "Note"),
    ] {
        if let Some(value) = of(name) {
            *slot = value;
        }
    }
    for (slot, name) in [(&mut e.map, "Map"), (&mut e.meeting, "Meeting")] {
        if let Some(value) = of(name) {
            *slot = bare_url(&value);
        }
    }
    e
}

fn attendee(item: &Item) -> Attendee {
    let of = |name: &str| item.nested(name).map(Item::value).unwrap_or_default();
    let text = item.plain();
    let target = link_target(&text);
    let linked = famulen_notebook::records::record_of(target);
    Attendee {
        name: match linked {
            Some(_) => text
                .split_once('|')
                .map(|(_, t)| t.trim_end_matches("]]").to_owned())
                .unwrap_or_default(),
            None => text.clone(),
        },
        email: of("Email"),
        response: of("Response"),
        organizer: flag(&of("Organizer")).unwrap_or(false),
        optional: flag(&of("Optional")).unwrap_or(false),
        party: linked.map(|(_, id)| id.to_owned()).unwrap_or_default(),
    }
}

fn bare_url(text: &str) -> String {
    text.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_owned()
}

/// A time or a whole day as the notebook writes it.
pub(super) fn read_time(text: &str, tz: Tz) -> Option<(DateTime<Utc>, bool)> {
    if let Some(at) = minute(text) {
        return Some((local(at, tz)?, false));
    }
    let day = NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok()?;
    Some((day_start(day, tz)?, true))
}

fn repeat_starts(
    rule: &str,
    first: DateTime<Utc>,
    tz: Tz,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<Vec<DateTime<Utc>>> {
    let rule = rule.trim().trim_start_matches("RRULE:");
    let set: rrule::RRuleSet = format!(
        "DTSTART;TZID={}:{}\nRRULE:{rule}",
        tz.name(),
        first.with_timezone(&tz).format("%Y%m%dT%H%M%S"),
    )
    .parse()
    .map_err(|e| anyhow::anyhow!("not a rule of repetition: {rule}: {e}"))?;
    let zone = rrule::Tz::Tz(tz);
    let found = set
        .after(from.with_timezone(&zone))
        .before(to.with_timezone(&zone))
        .all(MOST_OCCURRENCES);
    Ok(found.dates.into_iter().map(|d| d.to_utc()).collect())
}

pub(super) fn day_start(day: NaiveDate, tz: Tz) -> Option<DateTime<Utc>> {
    local(day.and_hms_opt(0, 0, 0)?, tz)
}

fn local(at: NaiveDateTime, tz: Tz) -> Option<DateTime<Utc>> {
    tz.from_local_datetime(&at).earliest().map(|t| t.to_utc())
}
