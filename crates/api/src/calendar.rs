//! Calendars, and their events as each workgroup shows them.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::workgroups::CalendarMode as Mode;

use crate::pb::{self, *};
use crate::{Api, OrFail, changed, given, writer};

/// A time given as RFC 3339, YYYY-MM-DDTHH:MM or YYYY-MM-DD, in `tz`.
fn parse_bound(s: &str, tz: chrono_tz::Tz) -> anyhow::Result<DateTime<Utc>> {
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Ok(d.with_timezone(&Utc));
    }
    let local = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .or_else(|_| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d").map(|day| day.and_time(NaiveTime::MIN))
        })
        .map_err(|_| anyhow::anyhow!("times are RFC 3339, YYYY-MM-DDTHH:MM or YYYY-MM-DD: {s}"))?;
    tz.from_local_datetime(&local)
        .earliest()
        .map(|d| d.with_timezone(&Utc))
        .ok_or_else(|| anyhow::anyhow!("no such local time: {s}"))
}

pub(crate) fn mode(m: Mode) -> pb::CalendarMode {
    match m {
        Mode::Show => pb::CalendarMode::CALENDAR_MODE_SHOW,
        Mode::Faint => pb::CalendarMode::CALENDAR_MODE_FAINT,
        Mode::Hide => pb::CalendarMode::CALENDAR_MODE_HIDE,
    }
}

pub(crate) fn mode_from(m: pb::CalendarMode) -> Option<Mode> {
    match m {
        pb::CalendarMode::CALENDAR_MODE_SHOW => Some(Mode::Show),
        pb::CalendarMode::CALENDAR_MODE_FAINT => Some(Mode::Faint),
        pb::CalendarMode::CALENDAR_MODE_HIDE => Some(Mode::Hide),
        _ => None,
    }
}

impl Api {
    fn calendar(&self, c: famulen_core::calendars::Calendar) -> Calendar {
        let (google_connection, google_calendar) = c.google.clone().unwrap_or_default();
        let source = match &c.google {
            Some((conn, id)) => {
                let account = self
                    .app
                    .connections
                    .get(conn)
                    .ok()
                    .flatten()
                    .map(|c| c.account)
                    .unwrap_or_else(|| conn.to_owned());
                if id == "primary" {
                    format!("Google · {account}")
                } else {
                    format!("Google · {account} · {id}")
                }
            }
            None => "手帳".to_owned(),
        };
        Calendar {
            id: c.id,
            name: c.name,
            source,
            color: c.color,
            fetched_at: c.fetched_at.unwrap_or(0),
            error: c.error.unwrap_or_default(),
            google_connection,
            google_calendar,
            default_for: c.default_for,
            ..Default::default()
        }
    }
}

/// Where a place opens on a map, when the event names no map of its own.
fn map_of(e: &famulen_core::calendars::Event) -> String {
    if !e.map.is_empty() || e.location.is_empty() || e.location.contains("://") {
        return e.map.clone();
    }
    let query: String = url::form_urlencoded::byte_serialize(e.location.as_bytes()).collect();
    format!("https://www.google.com/maps/search/?api=1&query={query}")
}

impl CalendarService for Api {
    async fn list_events(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListEventsRequest>,
    ) -> ServiceResult<ListEventsResponse> {
        let r = request.to_owned_message();
        let tz = self.app.tz();
        let from = parse_bound(&r.from, tz).or_fail()?;
        let to = parse_bound(&r.to, tz).or_fail()?;
        let workgroup = self.app.workgroups.resolve(Some(&r.workgroup)).or_fail()?;
        let calendars = self.app.calendars.list().or_fail()?;
        let accounts = self.app.records.accounts().or_fail()?;
        let party_of = |email: &str| -> String {
            accounts
                .iter()
                .find(|(_, all)| {
                    all.iter()
                        .any(|a| a.confirmed && a.handle.eq_ignore_ascii_case(email))
                })
                .map(|(r, _)| r.id.clone())
                .unwrap_or_default()
        };
        let mut events = Vec::new();
        for e in self.app.calendars.events(from, to, tz).or_fail()? {
            let cal = calendars.iter().find(|c| c.id == e.calendar);
            let outside = !e.workgroups.is_empty()
                && !famulen_core::records::Records::in_workgroup(&e.workgroups, &workgroup.id);
            let m = match cal.map(|c| c.mode(&workgroup.id)).unwrap_or(Mode::Show) {
                Mode::Show if outside => Mode::Faint,
                m => m,
            };
            if m == Mode::Hide {
                continue;
            }
            let mut out = CalendarEvent {
                calendar: e.calendar.clone(),
                calendar_name: cal.map(|c| c.name.clone()).unwrap_or_default(),
                color: cal.map(|c| c.color.clone()).unwrap_or_default(),
                uid: e.uid.clone(),
                start: e.start.with_timezone(&tz).to_rfc3339(),
                end: e.end.with_timezone(&tz).to_rfc3339(),
                all_day: e.all_day,
                mode: mode(m).into(),
                status: e.status.clone(),
                busy: e.busy,
                stale: cal.and_then(|c| c.error.clone()).unwrap_or_default(),
                ..Default::default()
            };
            if m == Mode::Show {
                out.map = map_of(&e);
                out.google = cal.is_some_and(|c| c.google.is_some());
                out.series = if e.series.is_empty() { e.uid } else { e.series };
                out.title = e.title;
                out.location = e.location;
                out.response = e.response;
                out.meeting = e.meeting;
                out.notes = e.notes;
                out.url = e.url;
                out.repeats = e.repeats;
                out.record = e.record;
                out.labels = e.labels;
                out.attendees = e
                    .attendees
                    .into_iter()
                    .map(|a| Attendee {
                        party: if a.party.is_empty() && !a.email.is_empty() {
                            party_of(&a.email)
                        } else {
                            a.party
                        },
                        name: a.name,
                        email: a.email,
                        response: a.response,
                        organizer: a.organizer,
                        optional: a.optional,
                        ..Default::default()
                    })
                    .collect();
            }
            events.push(out);
        }
        Response::ok(ListEventsResponse {
            timezone: tz.name().to_owned(),
            events,
            no_calendars: calendars.is_empty(),
            ..Default::default()
        })
    }

    async fn list_calendars(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListCalendarsResponse> {
        let calendars = self
            .app
            .calendars
            .list()
            .or_fail()?
            .into_iter()
            .map(|c| self.calendar(c))
            .collect();
        Response::ok(ListCalendarsResponse {
            calendars,
            ..Default::default()
        })
    }

    async fn add_calendar(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddCalendarRequest>,
    ) -> ServiceResult<Calendar> {
        let r = request.to_owned_message();
        let color = if r.color.is_empty() {
            let colors = famulen_core::workgroups::COLORS;
            let n = self.app.calendars.list().or_fail()?.len();
            colors[n % colors.len()].1.to_owned()
        } else {
            r.color.clone()
        };
        let id = given(&r.google_calendar).unwrap_or("primary");
        let google =
            (!r.google_connection.is_empty()).then_some((r.google_connection.as_str(), id));
        let c = self
            .app
            .calendars
            .add(writer(&ctx), &r.name, google, &color)
            .or_fail()?;
        self.app.refresh_calendars_later();
        Response::ok(self.calendar(c))
    }

    async fn remove_calendar(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Empty> {
        self.app
            .calendars
            .remove(writer(&ctx), request.id)
            .or_fail()?;
        changed(self);
        Response::ok(Empty::default())
    }

    async fn refresh_calendars(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Empty> {
        self.app.refresh_calendars_later();
        Response::ok(Empty::default())
    }
}
