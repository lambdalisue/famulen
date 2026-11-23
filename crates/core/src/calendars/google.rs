//! Reading the events a Google calendar answers.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use serde_json::Value;

use super::occurrences::day_start;
use super::{Attendee, Event};

/// The events of Google Calendar `list_events` answers (the Calendar API's
/// event resources). Cancelled events are left out.
pub fn google_events(answers: &[String], tz: Tz) -> anyhow::Result<Vec<Event>> {
    let mut out = Vec::new();
    for answer in answers {
        let page: Value = serde_json::from_str(answer)
            .map_err(|e| anyhow::anyhow!("not a list of events: {e}"))?;
        let events = page
            .get("events")
            .or_else(|| page.get("items"))
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("no events in the answer"))?;
        out.extend(events.iter().filter_map(|e| google_event(e, tz)));
    }
    out.sort_by(|a, b| (a.start, &a.uid).cmp(&(b.start, &b.uid)));
    out.dedup_by(|a, b| a.uid == b.uid);
    Ok(out)
}

fn google_event(e: &Value, tz: Tz) -> Option<Event> {
    let text = |v: &Value, key: &str| -> String {
        v.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let status = text(e, "status");
    if status == "cancelled" {
        return None;
    }
    let bound = |key: &str| -> Option<(DateTime<Utc>, bool)> {
        let b = e.get(key)?;
        if let Some(t) = b.get("dateTime").and_then(Value::as_str) {
            return Some((DateTime::parse_from_rfc3339(t).ok()?.to_utc(), false));
        }
        let d = NaiveDate::parse_from_str(b.get("date")?.as_str()?, "%Y-%m-%d").ok()?;
        Some((day_start(d, tz)?, true))
    };
    let (start, all_day) = bound("start")?;
    let (end, _) = bound("end")?;
    let video = e
        .pointer("/conferenceData/entryPoints")
        .and_then(Value::as_array)
        .and_then(|points| {
            points
                .iter()
                .find(|p| text(p, "entryPointType") == "video")
                .map(|p| text(p, "uri"))
        })
        .unwrap_or_default();
    let hangout = text(e, "hangoutLink");
    let organizer = e.pointer("/organizer/email").and_then(Value::as_str);
    let attendees: Vec<Attendee> = e
        .get("attendees")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|a| a.get("resource").and_then(Value::as_bool) != Some(true))
        .map(|a| {
            let email = text(a, "email");
            Attendee {
                name: text(a, "displayName"),
                response: answer(&text(a, "responseStatus")),
                organizer: organizer == Some(email.as_str()),
                optional: a.get("optional").and_then(Value::as_bool) == Some(true),
                email,
                party: String::new(),
            }
        })
        .collect();
    let response = e
        .get("attendees")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|a| a.get("self").and_then(Value::as_bool) == Some(true))
        .map(|a| answer(&text(a, "responseStatus")))
        .unwrap_or_default();
    let series = text(e, "recurringEventId");
    let uid = text(e, "id");
    Some(Event {
        calendar: String::new(),
        uid: if uid.is_empty() {
            start.timestamp().to_string()
        } else {
            uid
        },
        start,
        end,
        all_day,
        title: text(e, "summary"),
        status: if status.is_empty() {
            "confirmed".into()
        } else {
            status
        },
        busy: text(e, "transparency") != "transparent",
        response,
        location: text(e, "location"),
        map: String::new(),
        meeting: if video.is_empty() { hangout } else { video },
        notes: markdown_of(&text(e, "description")),
        attendees,
        url: text(e, "htmlLink"),
        repeats: !series.is_empty(),
        series,
        record: String::new(),
        labels: Vec::new(),
        workgroups: Vec::new(),
    })
}

fn answer(google: &str) -> String {
    match google {
        "needsAction" => "needs_action".into(),
        other => other.into(),
    }
}

/// Google keeps descriptions as a little HTML; the rest of Famulen reads
/// Markdown.
fn markdown_of(html: &str) -> String {
    if !html.contains('<') && !html.contains('&') {
        return html.trim().to_owned();
    }
    let mut out = String::new();
    let mut href: Option<String> = None;
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let Some(end) = rest[at..].find('>') else {
            rest = &rest[at..];
            break;
        };
        let tag = &rest[at + 1..at + end];
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_lowercase();
        let closing = tag.starts_with('/');
        match (name.as_str(), closing) {
            ("br", _) => out.push('\n'),
            ("p" | "div" | "ul" | "ol" | "h1" | "h2" | "h3", _) => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            ("li", false) => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("- ");
            }
            ("b" | "strong", _) => out.push_str("**"),
            ("a", false) => {
                href = tag
                    .split_once("href=")
                    .map(|(_, v)| v.trim_start_matches(['"', '\'']))
                    .and_then(|v| v.split(['"', '\'', ' ']).next())
                    .map(str::to_owned);
                if href.is_some() {
                    out.push('[');
                }
            }
            ("a", true) => {
                if let Some(href) = href.take() {
                    out.push_str(&format!("](<{href}>)"));
                }
            }
            ("", _) => out.push_str(&rest[at..=at + end]),
            _ => {}
        }
        rest = &rest[at + end + 1..];
    }
    out.push_str(rest);
    for (entity, text) in [
        ("&nbsp;", " "),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&amp;", "&"),
    ] {
        out = out.replace(entity, text);
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::calendars::tests::{PAGE, TZ};

    #[test]
    fn googles_events_are_read_with_everything_they_say() {
        let events = google_events(&[PAGE.to_owned()], TZ).unwrap();

        assert_eq!(events.len(), 2);
        let e = &events[0];
        assert_eq!((e.title.as_str(), e.location.as_str()), ("定例", "渋谷"));
        assert_eq!(e.meeting, "https://meet.example/x");
        assert_eq!(e.url, "https://calendar.example/a");
        assert_eq!(e.response, "needs_action");
        assert!(!e.busy && e.repeats);
        assert_eq!(e.attendees.len(), 2);
        assert!(e.attendees[0].organizer && e.attendees[1].optional);
        assert_eq!(
            e.notes,
            "議題\n- 一つ\n[資料](<https://x.example/?a=1&b=2>)"
        );
        assert!(events[1].all_day);
        assert_eq!(events[1].end - events[1].start, Duration::days(1));
    }

    #[test]
    fn an_event_on_two_pages_is_read_once_even_among_others_at_its_time() {
        let event = |id: &str| serde_json::json!({ "id": id, "start": { "date": "2026-11-23" }, "end": { "date": "2026-11-24" } });
        let page = |ids: &[&str]| {
            serde_json::json!({ "items": ids.iter().map(|id| event(id)).collect::<Vec<_>>() })
                .to_string()
        };
        let events =
            google_events(&[page(&["a", "b"]), page(&["a", "c"])], chrono_tz::UTC).unwrap();
        let ids: Vec<&str> = events.iter().map(|e| e.uid.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }
}
