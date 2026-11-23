//! What the models were used for over a week, a month or a year.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;

use super::{Models, role_from_id};
use crate::models::Role;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageSpan {
    /// Monday to Sunday, by day.
    Week,
    /// A calendar month, by day.
    Month,
    /// A calendar year, by month.
    Year,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsagePeriod {
    pub from: NaiveDate,
    /// The last day, included.
    pub to: NaiveDate,
}

/// What a role used of a model.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageRow {
    pub role: Role,
    pub model: String,
    /// Whether the model is counted in yen; otherwise it is counted by calls.
    pub metered: bool,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_yen: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageBucket {
    pub period: UsagePeriod,
    pub rows: Vec<UsageRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageReport {
    pub period: UsagePeriod,
    pub today: NaiveDate,
    pub buckets: Vec<UsageBucket>,
    pub rows: Vec<UsageRow>,
    /// Every model used in the year the period starts in, first used first,
    /// so a model keeps its place whichever part of the year is shown.
    pub models: Vec<String>,
    pub metered_yen: f64,
    /// Every subscription's fee for a month.
    pub fixed_monthly_yen: f64,
    pub calls_today: i64,
}

impl UsageSpan {
    /// The period of this span holding `day`.
    pub fn period(self, day: NaiveDate) -> Option<UsagePeriod> {
        let (from, next) = match self {
            Self::Week => {
                let from =
                    day.checked_sub_days(Days::new(day.weekday().num_days_from_monday().into()))?;
                (from, from.checked_add_days(Days::new(7))?)
            }
            Self::Month => {
                let from = day.with_day(1)?;
                (from, from.checked_add_months(Months::new(1))?)
            }
            Self::Year => {
                let from = NaiveDate::from_ymd_opt(day.year(), 1, 1)?;
                (from, from.checked_add_months(Months::new(12))?)
            }
        };
        Some(UsagePeriod {
            from,
            to: next.pred_opt()?,
        })
    }

    fn buckets(self, period: UsagePeriod) -> Vec<UsagePeriod> {
        match self {
            Self::Week | Self::Month => period
                .from
                .iter_days()
                .take_while(|day| *day <= period.to)
                .map(|day| UsagePeriod { from: day, to: day })
                .collect(),
            Self::Year => (1..=12)
                .filter_map(|month| NaiveDate::from_ymd_opt(period.from.year(), month, 1))
                .filter_map(|first| Self::Month.period(first))
                .collect(),
        }
    }
}

impl UsagePeriod {
    fn holds(&self, day: NaiveDate) -> bool {
        self.from <= day && day <= self.to
    }
}

struct Call {
    day: NaiveDate,
    row: UsageRow,
}

fn sum<'a>(calls: impl Iterator<Item = &'a Call>) -> Vec<UsageRow> {
    let mut rows: BTreeMap<(usize, &str), UsageRow> = BTreeMap::new();
    for Call { row, .. } in calls {
        let role = Role::ALL.iter().position(|r| *r == row.role).unwrap_or(0);
        rows.entry((role, &row.model))
            .and_modify(|sum| {
                sum.calls += row.calls;
                sum.input_tokens += row.input_tokens;
                sum.output_tokens += row.output_tokens;
                sum.cost_yen += row.cost_yen;
            })
            .or_insert_with(|| row.clone());
    }
    rows.into_values().collect()
}

impl Models {
    /// What was used in the period of `span` holding `day`, in the owner's
    /// days.
    pub fn usage(
        &self,
        span: UsageSpan,
        day: NaiveDate,
        tz: Tz,
        now: DateTime<Utc>,
    ) -> anyhow::Result<UsageReport> {
        let period = span
            .period(day)
            .ok_or_else(|| anyhow::anyhow!("no usage can be shown around {day}"))?;
        let year = UsageSpan::Year
            .period(period.from)
            .ok_or_else(|| anyhow::anyhow!("no usage can be shown around {day}"))?;
        let today = now.with_timezone(&tz).date_naive();
        let calls = {
            let mut calls = self.calls(
                UsagePeriod {
                    from: year.from,
                    to: year.to.max(period.to),
                },
                tz,
            )?;
            let mut metered: BTreeSet<String> = calls
                .iter()
                .filter(|c| c.row.cost_yen > 0.0)
                .map(|c| c.row.model.clone())
                .collect();
            metered.extend(
                self.registered()?
                    .into_iter()
                    .filter(|m| m.spec.price.is_metered())
                    .map(|m| m.spec.id),
            );
            for call in &mut calls {
                call.row.metered = metered.contains(&call.row.model);
            }
            calls
        };
        let mut models: Vec<String> = Vec::new();
        for call in &calls {
            if !models.contains(&call.row.model) {
                models.push(call.row.model.clone());
            }
        }
        let rows = sum(calls.iter().filter(|c| period.holds(c.day)));
        Ok(UsageReport {
            buckets: span
                .buckets(period)
                .into_iter()
                .map(|period| UsageBucket {
                    rows: sum(calls.iter().filter(|c| period.holds(c.day))),
                    period,
                })
                .collect(),
            metered_yen: rows.iter().fold(0.0, |sum, r| sum + r.cost_yen),
            fixed_monthly_yen: self
                .registered()?
                .iter()
                .filter_map(|m| m.monthly_yen)
                .fold(0.0, |sum, yen| sum + yen),
            calls_today: self
                .calls(
                    UsagePeriod {
                        from: today,
                        to: today,
                    },
                    tz,
                )?
                .iter()
                .map(|c| c.row.calls)
                .sum(),
            period,
            today,
            rows,
            models,
        })
    }

    /// The calls of a period, oldest first.
    fn calls(&self, period: UsagePeriod, tz: Tz) -> anyhow::Result<Vec<Call>> {
        // A day wider than UTC midnights on both sides covers every time
        // zone; the owner's days are cut below.
        let bound = |day: Option<NaiveDate>| {
            day.and_then(|day| day.and_hms_opt(0, 0, 0))
                .map(|at| at.and_utc().timestamp())
                .ok_or_else(|| anyhow::anyhow!("no such day"))
        };
        let start = bound(period.from.checked_sub_days(Days::new(1)))?;
        let end = bound(period.to.checked_add_days(Days::new(2)))?;
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT at, role, model, input_tokens, output_tokens, calls, cost_yen FROM usage
             WHERE at >= ?1 AND at < ?2 ORDER BY at, id",
        )?;
        let found = stmt.query_map([start, end], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                UsageRow {
                    role: Role::Secretary,
                    model: r.get(2)?,
                    metered: false,
                    input_tokens: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    output_tokens: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    calls: r.get(5)?,
                    cost_yen: r.get(6)?,
                },
            ))
        })?;
        let mut calls = Vec::new();
        for call in found {
            let (at, role, row) = call?;
            let Some(role) = role_from_id(&role) else {
                continue;
            };
            let Some(day) = tz.timestamp_opt(at, 0).single().map(|t| t.date_naive()) else {
                continue;
            };
            if period.holds(day) {
                calls.push(Call {
                    day,
                    row: UsageRow { role, ..row },
                });
            }
        }
        Ok(calls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ModelKind, ModelSpec, Price, Usage, role_id};
    use crate::store::Store;
    use chrono_tz::Asia::Tokyo;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    fn period(from: &str, to: &str) -> UsagePeriod {
        UsagePeriod {
            from: day(from),
            to: day(to),
        }
    }

    fn models() -> Models {
        let models = Models::new(Store::open_in_memory().unwrap());
        let spec = |id: &str, kind, price| ModelSpec {
            id: id.into(),
            name: id.into(),
            kind,
            cli: Default::default(),
            endpoint: None,
            model: id.into(),
            price,
        };
        models
            .register(
                spec("plan", ModelKind::AgentCli, Price::default()),
                Some(15000.0),
            )
            .unwrap();
        let price = Price {
            per_call_yen: Some(10.0),
            ..Default::default()
        };
        models
            .register(spec("api", ModelKind::Api, price), None)
            .unwrap();
        models
    }

    fn call(models: &Models, role: Role, model: &str, when: &str, yen: f64) {
        models
            .store
            .conn()
            .execute(
                "INSERT INTO usage (at, role, model, input_tokens, output_tokens, calls, cost_yen)
                 VALUES (?1, ?2, ?3, 100, 10, 1, ?4)",
                rusqlite::params![at(when).timestamp(), role_id(role), model, yen],
            )
            .unwrap();
    }

    fn report(models: &Models, span: UsageSpan, shown: &str) -> UsageReport {
        models
            .usage(span, day(shown), Tokyo, at("2026-11-23T03:00:00Z"))
            .unwrap()
    }

    #[test]
    fn a_week_is_monday_to_sunday_a_month_and_a_year_are_the_calendars() {
        for (span, shown, from, to, buckets) in [
            (UsageSpan::Week, "2026-11-23", "2026-11-23", "2026-11-29", 7),
            (UsageSpan::Week, "2026-11-29", "2026-11-23", "2026-11-29", 7),
            (UsageSpan::Week, "2027-01-01", "2026-12-28", "2027-01-03", 7),
            (
                UsageSpan::Month,
                "2028-02-29",
                "2028-02-01",
                "2028-02-29",
                29,
            ),
            (
                UsageSpan::Month,
                "2026-12-31",
                "2026-12-01",
                "2026-12-31",
                31,
            ),
            (
                UsageSpan::Year,
                "2026-11-23",
                "2026-01-01",
                "2026-12-31",
                12,
            ),
        ] {
            let found = span.period(day(shown)).unwrap();
            assert_eq!(found, period(from, to), "{span:?} {shown}");
            let parts = span.buckets(found);
            assert_eq!(parts.len(), buckets, "{span:?} {shown}");
            assert_eq!(parts[0].from, found.from);
            assert_eq!(parts[buckets - 1].to, found.to);
        }
    }

    #[test]
    fn days_are_cut_in_the_owners_time_zone() {
        let m = models();
        call(&m, Role::Secretary, "plan", "2026-10-31T14:59:59Z", 0.0);
        call(&m, Role::Secretary, "plan", "2026-10-31T15:00:00Z", 0.0);
        call(&m, Role::Secretary, "plan", "2026-11-30T14:59:59Z", 0.0);
        call(&m, Role::Secretary, "plan", "2026-11-30T15:00:00Z", 0.0);
        let r = report(&m, UsageSpan::Month, "2026-11-10");
        assert_eq!(r.rows[0].calls, 2);
        assert_eq!(r.buckets[0].rows[0].calls, 1);
        assert_eq!(r.buckets[29].rows[0].calls, 1);
        assert!(r.buckets[1].rows.is_empty());
    }

    #[test]
    fn a_past_period_leaves_out_what_came_after_it() {
        let m = models();
        call(&m, Role::Lookout, "api", "2026-10-05T00:00:00Z", 10.0);
        call(&m, Role::Lookout, "api", "2026-11-05T00:00:00Z", 10.0);
        let r = report(&m, UsageSpan::Month, "2026-10-01");
        assert_eq!(r.metered_yen, 10.0);
        assert_eq!(r.rows[0].calls, 1);
    }

    #[test]
    fn a_year_is_summed_by_month() {
        let m = models();
        call(&m, Role::Dream, "plan", "2026-03-01T00:00:00Z", 0.0);
        call(&m, Role::Dream, "plan", "2026-03-31T00:00:00Z", 0.0);
        let r = report(&m, UsageSpan::Year, "2026-11-23");
        assert_eq!(r.buckets[2].period, period("2026-03-01", "2026-03-31"));
        assert_eq!(r.buckets[2].rows[0].calls, 2);
        assert_eq!(r.buckets[2].rows[0].input_tokens, 200);
        assert_eq!(r.buckets[2].rows[0].output_tokens, 20);
    }

    #[test]
    fn rows_keep_each_role_and_model_apart() {
        let m = models();
        call(&m, Role::Lookout, "api", "2026-11-23T00:00:00Z", 10.0);
        call(&m, Role::Secretary, "plan", "2026-11-23T00:00:00Z", 0.0);
        call(&m, Role::Lookout, "plan", "2026-11-23T01:00:00Z", 0.0);
        call(&m, Role::Lookout, "api", "2026-11-23T01:00:00Z", 10.0);
        let r = report(&m, UsageSpan::Week, "2026-11-23");
        let rows: Vec<_> = r
            .rows
            .iter()
            .map(|r| (r.role, r.model.as_str(), r.calls, r.cost_yen))
            .collect();
        assert_eq!(
            rows,
            [
                (Role::Secretary, "plan", 1, 0.0),
                (Role::Lookout, "api", 2, 20.0),
                (Role::Lookout, "plan", 1, 0.0),
            ]
        );
    }

    #[test]
    fn a_priced_model_is_metered_and_stays_so_once_unregistered() {
        let m = models();
        call(&m, Role::Lookout, "api", "2026-11-23T00:00:00Z", 0.0);
        call(&m, Role::Lookout, "gone", "2026-11-22T00:00:00Z", 5.0);
        call(&m, Role::Lookout, "gone", "2026-11-23T00:00:00Z", 0.0);
        call(&m, Role::Lookout, "plan", "2026-11-23T00:00:00Z", 0.0);
        let r = report(&m, UsageSpan::Week, "2026-11-23");
        let metered: Vec<_> = r
            .rows
            .iter()
            .map(|r| (r.model.as_str(), r.metered))
            .collect();
        assert_eq!(metered, [("api", true), ("gone", true), ("plan", false)]);
    }

    #[test]
    fn models_keep_their_place_all_year() {
        let m = models();
        call(&m, Role::Dream, "plan", "2026-02-01T00:00:00Z", 0.0);
        call(&m, Role::Lookout, "api", "2026-11-23T00:00:00Z", 10.0);
        call(&m, Role::Dream, "plan", "2026-11-23T01:00:00Z", 0.0);
        for span in [UsageSpan::Week, UsageSpan::Month, UsageSpan::Year] {
            assert_eq!(report(&m, span, "2026-11-23").models, ["plan", "api"]);
        }
    }

    #[test]
    fn todays_calls_do_not_depend_on_the_period_shown() {
        let m = models();
        call(&m, Role::Dream, "plan", "2026-11-22T14:00:00Z", 0.0);
        call(&m, Role::Dream, "plan", "2026-11-22T16:00:00Z", 0.0);
        assert_eq!(
            report(&m, UsageSpan::Month, "2025-01-01").calls_today,
            1,
            "only what came after the owner's midnight"
        );
    }

    #[test]
    fn metered_costs_and_subscriptions_are_told_apart() {
        let m = models();
        let api = m.model("api").unwrap().unwrap().spec;
        let usage = Usage {
            calls: 3,
            ..Default::default()
        };
        m.record(Role::Lookout, &api, &usage).unwrap();
        let today = Utc::now().date_naive();
        let r = m
            .usage(UsageSpan::Month, today, chrono_tz::UTC, Utc::now())
            .unwrap();
        assert_eq!((r.metered_yen, r.fixed_monthly_yen), (30.0, 15000.0));
    }

    #[test]
    fn an_empty_period_totals_plain_zero() {
        let m = Models::new(Store::open_in_memory().unwrap());
        let r = report(&m, UsageSpan::Week, "2026-11-23");
        assert_eq!(
            format!("{:.0} {:.0}", r.metered_yen, r.fixed_monthly_yen),
            "0 0"
        );
        assert!(r.rows.is_empty());
    }
}
