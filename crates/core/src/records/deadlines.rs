//! Deadlines that become tasks.

use chrono::NaiveDate;
use famulen_notebook::records::{flag, titled_link};

use super::{CONTRACTS, HOLDINGS, OWNER, Patch, Record, Records, TASKS};
use crate::notebook::Writer;

/// How long before its date a deadline becomes a task.
const REMIND_DAYS: i64 = 14;

impl Records {
    /// Turn the deadlines of things and contracts due soon into
    /// tasks, each once. What was let go or has ended is left alone.
    pub fn remind_deadlines(&self, today: NaiveDate) -> anyhow::Result<Vec<Record>> {
        let held = self.hold()?;
        let mut made = Vec::new();
        for kind in [HOLDINGS, CONTRACTS] {
            for record in self.list(kind)? {
                let status = record.body.value("Status");
                if matches!(status.as_deref(), Some("released" | "ended")) {
                    continue;
                }
                let due: Vec<(usize, String, NaiveDate)> = record
                    .body
                    .items("Deadlines")
                    .into_iter()
                    .enumerate()
                    .filter_map(|(at, item)| {
                        let on = item.nested("On")?.value();
                        let on = NaiveDate::parse_from_str(&on, "%Y-%m-%d").ok()?;
                        let reminded = item
                            .nested("Reminded")
                            .and_then(|i| flag(&i.value()))
                            .unwrap_or(false);
                        ((on - today).num_days() <= REMIND_DAYS && !reminded)
                            .then(|| (at, item.plain(), on))
                    })
                    .collect();
                for (at, what, on) in due {
                    let task = held.add(
                        Writer::Secretary,
                        TASKS,
                        Patch {
                            workgroups: Some(record.workgroups()),
                            source: format!(
                                "{}の期限から Famulen が足した",
                                if kind == HOLDINGS { "物" } else { "契約" }
                            ),
                            ..Patch::titled(format!(
                                "{}の{what}（{on}）をどうするか決める",
                                record.title()
                            ))
                        }
                        .with("Due", on.to_string())
                        .with("Assignee", OWNER)
                        .with("Related", titled_link(kind, &record.id, &record.title())),
                    )?;
                    let mut record = self.need(kind, &record.id)?;
                    record
                        .body
                        .set_item_field("Deadlines", at, "Reminded", "yes")?;
                    let subject = format!("{} の{what}を知らせた", record.title());
                    self.notebook
                        .write(&Writer::Secretary.signature(), &record, &subject)?;
                    made.push(task);
                }
            }
        }
        Ok(made)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::testing::records;
    use crate::records::tests::ME;
    use crate::records::{Entry, Field, SectionContent};

    #[test]
    fn each_deadline_two_weeks_away_becomes_a_task_once_unless_it_is_over() {
        let (_tmp, r) = records("records-deadlines");
        let deadlines = |dates: &[&str]| {
            vec![SectionContent {
                name: "Deadlines".into(),
                items: dates
                    .iter()
                    .map(|d| Entry {
                        text: "更新".into(),
                        fields: vec![Field::new("On", *d)],
                    })
                    .collect(),
                ..SectionContent::default()
            }]
        };
        let with = |title: &str, dates: &[&str]| Patch {
            sections: deadlines(dates),
            workgroups: Some(vec!["work".into()]),
            ..Patch::titled(title)
        };
        let c = r
            .add(ME, CONTRACTS, with("回線", &["2026-12-01", "2027-12-01"]))
            .unwrap();
        r.add(
            ME,
            HOLDINGS,
            with("古い", &["2026-12-01"]).with("Status", "released"),
        )
        .unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();

        let made = r.remind_deadlines(today).unwrap();

        assert_eq!(made.len(), 1);
        assert_eq!(made[0].body.value("Due").as_deref(), Some("2026-12-01"));
        assert_eq!(made[0].workgroups(), ["work"]);
        assert_eq!(made[0].body.values("Related"), [c.link()]);
        assert!(r.remind_deadlines(today).unwrap().is_empty());
        r.change(
            ME,
            CONTRACTS,
            &c.id,
            Patch {
                sections: deadlines(&["2026-11-25"]),
                ..Patch::default()
            },
        )
        .unwrap();
        assert_eq!(r.remind_deadlines(today).unwrap().len(), 1);
    }
}
