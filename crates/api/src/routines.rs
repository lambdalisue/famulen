//! When routines run, and the wake-ups the secretary sets.

use chrono::Utc;
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::conversations::wake_key;
use famulen_core::schedule::{cron_from, describe_cron};

use crate::pb::*;
use crate::{Api, OrFail, by_secretary, changed, fail, writer};

fn wake(w: famulen_core::schedule::Wakeup) -> Wake {
    Wake {
        id: w.id,
        at: w.at.to_rfc3339(),
        at_unix: w.at.timestamp(),
        note: w.note,
        why: w.why,
        ..Default::default()
    }
}

/// How many of a routine's runs to come are shown.
const SHOWN_RUNS: usize = 5;

impl RoutineService for Api {
    async fn list_routines(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListRoutinesResponse> {
        let now = Utc::now();
        let routines = self
            .app
            .scheduler()
            .list(now)
            .or_fail()?
            .into_iter()
            .map(|r| Routine {
                title: r.record.title(),
                when: r.when(),
                enabled: r.enabled,
                runs: if r.problem.is_some() {
                    Vec::new()
                } else {
                    r.runs(now, SHOWN_RUNS)
                        .into_iter()
                        .map(|(at, skipped)| RoutineRun {
                            at: at.timestamp(),
                            skipped,
                            ..Default::default()
                        })
                        .collect()
                },
                last_run: r.last_run.map_or(0, |t| t.timestamp()),
                problem: r.problem.unwrap_or_default(),
                id: r.record.id,
                ..Default::default()
            })
            .collect();
        Response::ok(ListRoutinesResponse {
            routines,
            ..Default::default()
        })
    }

    async fn skip_run(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SkipRunRequest>,
    ) -> ServiceResult<Empty> {
        let r = request.to_owned_message();
        let at = chrono::DateTime::from_timestamp(r.at, 0).ok_or_else(|| fail("no such time"))?;
        let scheduler = self.app.scheduler();
        let by = writer(&ctx);
        if r.undo {
            scheduler.unskip(by, &r.routine, at)
        } else {
            scheduler.skip(by, &r.routine, at)
        }
        .or_fail()?;
        changed(self);
        Response::ok(Empty::default())
    }

    async fn cron_from(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CronFromRequest>,
    ) -> ServiceResult<CronFromResponse> {
        let r = request.to_owned_message();
        let cron = cron_from(&r.repeat, &r.time, r.weekday, r.day, &r.cron).or_fail()?;
        Response::ok(CronFromResponse {
            when: describe_cron(&cron),
            cron,
            ..Default::default()
        })
    }

    async fn wake_at(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, WakeAtRequest>,
    ) -> ServiceResult<Wake> {
        let r = request.to_owned_message();
        let now = Utc::now();
        let at = match (r.at.is_empty(), r.in_minutes) {
            (false, 0) => chrono::DateTime::parse_from_rfc3339(&r.at)
                .map_err(|e| {
                    fail(format!(
                        "at must be RFC 3339 with an offset, e.g. 2026-11-21T15:00:00+09:00: {e}"
                    ))
                })?
                .with_timezone(&Utc),
            (true, m) if m > 0 => now + chrono::Duration::minutes(m as i64),
            _ => return Err(fail("give exactly one of at or in_minutes")),
        };
        let scheduler = self.app.scheduler();
        let id = scheduler.wake_at(at, &r.note, &r.why, now).or_fail()?;
        // It is set; failing here would have it set twice.
        if by_secretary(&ctx)
            && let Err(error) = self.app.conversations.link_to_running([wake_key(id)])
        {
            tracing::warn!(%error, "could not keep which conversation a wake-up is of");
        }
        changed(self);
        Response::ok(wake(famulen_core::schedule::Wakeup {
            id,
            at: at.with_timezone(&scheduler.timezone()),
            note: r.note,
            why: r.why,
        }))
    }

    async fn list_wakes(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListWakesResponse> {
        let wakes = self
            .app
            .scheduler()
            .pending_wakeups()
            .or_fail()?
            .into_iter()
            .map(wake)
            .collect();
        Response::ok(ListWakesResponse {
            wakes,
            ..Default::default()
        })
    }

    async fn cancel_wake(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ById>,
    ) -> ServiceResult<Empty> {
        self.app.scheduler().cancel_wakeup(request.id).or_fail()?;
        changed(self);
        Response::ok(Empty::default())
    }
}
