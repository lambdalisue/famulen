//! Records that may no longer be true: the mark, and the task that asks the
//! owner about it.

use famulen_notebook::records::Obsolete;

use super::{Held, OWNER, Patch, Record, Records, TASKS, is_over, linked, name_of};
use crate::notebook::Writer;

/// The source of a task that asks the owner whether a record is still true.
pub const CONFIRM_SOURCE: &str = "夢が、古くなったかもしれない記録を確かめるために足した";

/// What is written of a doubt in one line. A wiki link in it would be
/// refused once it broke, so it goes in as plain text.
fn one_line(reason: &str) -> String {
    reason
        .replace("[[", "")
        .replace("]]", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

impl Records {
    /// Mark a record as doubtful and ask the owner in one task.
    pub fn doubt(&self, kind: &str, id: &str, reason: &str, on: &str) -> anyhow::Result<Record> {
        self.hold()?.doubt(kind, id, reason, on)
    }

    /// Take the mark off: the record was checked. The tasks that asked about
    /// it are done with it.
    pub fn clear_obsolete(&self, writer: Writer, kind: &str, id: &str) -> anyhow::Result<Record> {
        writer.may_write()?;
        let cleared = {
            let _writing = self.writing()?;
            let mut record = self.need(kind, id)?;
            self.may_change(writer, &record)?;
            if record.obsolete().is_some() {
                record.set_obsolete(None);
                self.log_mark(
                    &mut record,
                    name_of(writer),
                    "古くなったかもしれない印を外した",
                )?;
                let subject = format!("{} の古くなったかもしれない印を外した", record.title());
                self.notebook
                    .write(&writer.signature(), &record, &subject)?;
            }
            self.need(kind, &record.id)?
        };
        for task in self.open_questions(&cleared)? {
            self.change(
                writer,
                TASKS,
                &task.id,
                Patch::default().with("Status", "done"),
            )?;
        }
        Ok(cleared)
    }

    /// The tasks not over that ask whether a record is still true.
    fn open_questions(&self, record: &Record) -> anyhow::Result<Vec<Record>> {
        let about = (record.kind.clone(), record.id.clone());
        Ok(self
            .list(TASKS)?
            .into_iter()
            .filter(|t| t.meta.text("source").as_deref() == Some(CONFIRM_SOURCE))
            .filter(|t| !is_over(t) && linked(t).contains(&about))
            .collect())
    }

    fn log_mark(&self, record: &mut Record, who: &str, what: &str) -> anyhow::Result<()> {
        if self.notebook.kind(&record.kind)?.layout.free {
            return Ok(());
        }
        let at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        record.body.log(&at, who, what)
    }
}

impl Held<'_> {
    /// Mark a record as doubtful and ask the owner in one task, leaving what
    /// it says as it is. The task comes first: a mark without one would
    /// never be asked about.
    pub(crate) fn doubt(
        &self,
        kind: &str,
        id: &str,
        reason: &str,
        on: &str,
    ) -> anyhow::Result<Record> {
        let r = self.records;
        let record = r.need(kind, id)?;
        let reason = one_line(reason);
        if r.open_questions(&record)?.is_empty() {
            self.add(
                Writer::Secretary,
                TASKS,
                Patch {
                    notes: Some(format!(
                        "{reason}\n\n今も正しければ、その記録の印を外す。違っていれば記録を直してから外す。印を外すと、このタスクは済になる。"
                    )),
                    workgroups: Some(record.workgroups()),
                    source: CONFIRM_SOURCE.into(),
                    ..Patch::titled(format!("「{}」が今も正しいか確かめる", record.title()))
                }
                .with("Assignee", OWNER)
                .with("Related", record.link()),
            )?;
        }
        let mut record = r.need(kind, id)?;
        record.set_obsolete(Some(&Obsolete {
            on: on.to_owned(),
            reason: reason.clone(),
        }));
        r.log_mark(
            &mut record,
            "secretary",
            &format!("古くなったかもしれない印を付けた：{reason}"),
        )?;
        let subject = format!("{} に古くなったかもしれない印を付けた", record.title());
        r.notebook
            .write(&Writer::Secretary.signature(), &record, &subject)?;
        r.need(kind, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::testing::records;
    use crate::records::tests::{ME, add};
    use crate::records::{CONTRACTS, MEMOS, log_lines};

    fn questions(r: &Records) -> Vec<Record> {
        r.list(TASKS).unwrap()
    }

    #[test]
    fn a_doubt_marks_the_record_and_asks_the_owner_once_changing_nothing_else() {
        let (_tmp, r) = records("obsolete-doubt");
        let c = r
            .add(
                ME,
                CONTRACTS,
                Patch {
                    notes: Some("家の回線".into()),
                    workgroups: Some(vec!["home".into()]),
                    ..Patch::titled("回線")
                }
                .with("Price", "5000 JPY"),
            )
            .unwrap();

        let marked = r
            .doubt(CONTRACTS, &c.id, "料金が\n[[2 年前]]のまま", "2026-11-24")
            .unwrap();
        r.doubt(CONTRACTS, &c.id, "もう一度", "2026-11-25").unwrap();

        assert_eq!(marked.obsolete().unwrap().reason, "料金が 2 年前のまま");
        assert_eq!(marked.body.value("Price").as_deref(), Some("5000 JPY"));
        assert_eq!(marked.notes(), "家の回線");
        assert!(log_lines(&marked)[0].text.contains("2 年前"));
        let asked = questions(&r);
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].body.value("Assignee").as_deref(), Some(OWNER));
        assert_eq!(asked[0].body.values("Related"), [c.link()]);
        assert_eq!(asked[0].workgroups(), ["home"]);
        assert!(asked[0].notes().contains("2 年前"));
    }

    #[test]
    fn taking_the_mark_off_finishes_the_task_that_asked() {
        let (_tmp, r) = records("obsolete-clear");
        let m = add(&r, MEMOS, "店の営業時間");
        let body = r.need(MEMOS, &m.id).unwrap().body;
        r.doubt(MEMOS, &m.id, "三年前のもの", "2026-11-24").unwrap();

        let cleared = r.clear_obsolete(ME, MEMOS, &m.id).unwrap();

        assert_eq!(cleared.obsolete(), None);
        assert_eq!(cleared.body, body, "a memo's text is the owner's");
        assert!(is_over(&questions(&r)[0]));
        assert!(
            r.clear_obsolete(Writer::Lookout, MEMOS, &m.id).is_err(),
            "the lookout writes nothing"
        );
    }
}
