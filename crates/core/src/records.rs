//! The owner's records, in the notebook: tasks, routines, memos, events,
//! calendars, parties, holdings and contracts. The notebook says how each is
//! written; here is what Famulen does when one is written, and who may.

mod deadlines;
mod obsolete;
mod parties;

pub use obsolete::CONFIRM_SOURCE;
pub use parties::*;

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context as _;

use famulen_notebook::body::{DETAILS, Item, LOG, NewItem, Value, same};
pub use famulen_notebook::records::{
    Backlink, FieldType, Kind, Names, Obsolete, Record, SectionType,
};
use famulen_notebook::records::{Records as NotebookRecords, record_of};

use crate::notebook::{Hold, Notebook, Writer, link_target};

pub const TASKS: &str = "tasks";
pub const ROUTINES: &str = "routines";
pub const MEMOS: &str = "memos";
pub const EVENTS: &str = "events";
pub const CALENDARS: &str = "calendars";
pub const PARTIES: &str = "parties";
pub const HOLDINGS: &str = "holdings";
pub const CONTRACTS: &str = "contracts";

/// The owner and the secretary, as `Assignee` and the log name them.
pub const OWNER: &str = "owner";
pub const SECRETARY: &str = "secretary";

/// How `Assignee` and the log name who writes a record.
fn name_of(writer: Writer) -> &'static str {
    match writer {
        Writer::Owner => OWNER,
        _ => SECRETARY,
    }
}

/// A field to write, or one read. An empty value removes it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Field {
    pub name: String,
    pub value: String,
    /// The values of a field that holds several.
    pub values: Vec<String>,
}

impl Field {
    pub fn new(name: &str, value: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            value: value.into(),
            values: Vec::new(),
        }
    }

    fn of(item: &Item) -> Option<Self> {
        let (name, _) = item.field()?;
        Some(Self {
            name: name.to_owned(),
            value: item.value(),
            values: item.values(),
        })
    }

    fn to_value(&self) -> Value {
        if self.values.is_empty() {
            Value::One(self.value.clone())
        } else {
            Value::Many(self.values.clone())
        }
    }
}

/// One item of a section of items.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    pub text: String,
    pub fields: Vec<Field>,
}

impl Entry {
    fn of(item: &Item) -> Self {
        Self {
            text: item.plain(),
            fields: item.items.iter().filter_map(Field::of).collect(),
        }
    }

    fn to_item(&self) -> NewItem {
        self.fields
            .iter()
            .fold(NewItem::new(&self.text), |item, f| {
                item.with(&f.name, f.to_value())
            })
    }
}

/// A section as it is, or as it should be.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SectionContent {
    pub name: String,
    pub fields: Vec<Field>,
    pub items: Vec<Entry>,
}

/// What to change in a record. What is left out stays.
#[derive(Debug, Clone, Default)]
pub struct Patch {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub labels: Option<Vec<String>>,
    pub workgroups: Option<Vec<String>>,
    /// Set only when the record has none.
    pub source: String,
    pub fields: Vec<Field>,
    /// Each section given is replaced whole.
    pub sections: Vec<SectionContent>,
    /// The owner's own fields, replaced whole.
    pub details: Option<Vec<Field>>,
    /// Lines to add to the log.
    pub log: Vec<String>,
    /// Other metadata: where a Google event or calendar is.
    pub google: Option<String>,
    pub url: Option<String>,
}

impl Patch {
    pub fn titled(title: impl Into<String>) -> Self {
        Self {
            title: Some(title.into()),
            ..Self::default()
        }
    }

    pub fn with(mut self, name: &str, value: impl Into<String>) -> Self {
        self.fields.push(Field::new(name, value));
        self
    }

    fn given(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| same(&f.name, name))
    }

    fn default_field(&mut self, name: &str, value: &str) {
        if self.given(name).is_none() {
            self.fields.push(Field::new(name, value));
        }
    }
}

/// What the worker pool did with a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskNote {
    pub(crate) line: String,
    pub(crate) status: Option<&'static str>,
    pub(crate) waiting_for: Option<&'static str>,
}

/// A line of a record's log.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogLine {
    pub at: String,
    pub who: String,
    pub text: String,
}

/// The sections a record has, as they are.
pub fn sections(record: &Record) -> Vec<SectionContent> {
    record
        .body
        .sections()
        .iter()
        .filter(|s| ![DETAILS, LOG, "Notes"].iter().any(|n| same(n, &s.name)))
        .map(|s| SectionContent {
            name: s.name.clone(),
            fields: s.items.iter().filter_map(Field::of).collect(),
            items: s.items.iter().map(Entry::of).collect(),
        })
        .collect()
}

pub fn fields(record: &Record) -> Vec<Field> {
    record.body.fields().iter().filter_map(Field::of).collect()
}

/// The owner's own fields.
pub fn details(record: &Record) -> Vec<Field> {
    record
        .body
        .items(DETAILS)
        .into_iter()
        .filter_map(Field::of)
        .collect()
}

pub fn log_lines(record: &Record) -> Vec<LogLine> {
    record
        .body
        .items(LOG)
        .into_iter()
        .map(|item| {
            let text = item.plain();
            let parsed = text.get(..16).and_then(|at| {
                famulen_notebook::records::minute(at)?;
                let (who, what) = text.get(17..)?.split_once(':')?;
                Some(LogLine {
                    at: at.to_owned(),
                    who: who.trim().to_owned(),
                    text: what.trim().to_owned(),
                })
            });
            let mut line = parsed.unwrap_or_else(|| LogLine {
                text: text.clone(),
                ..LogLine::default()
            });
            if !item.rest.is_empty() {
                line.text.push('\n');
                line.text.push_str(&item.rest);
            }
            line
        })
        .collect()
}

/// Whether a task is delegated to a worker. It is its owner's, the
/// secretary's and the pool's to change, and no chore's.
pub fn is_delegated(task: &Record) -> bool {
    task.kind == TASKS && task.body.value("Assignee").as_deref() == Some(crate::workers::WORKER)
}

/// Whether a task is over.
pub fn is_over(task: &Record) -> bool {
    matches!(
        task.body.value("Status").as_deref(),
        Some("done" | "cancelled")
    )
}

/// The kinds and ids of the records a record's text links to.
pub fn linked(record: &Record) -> Vec<(String, String)> {
    famulen_notebook::memory::links(record.body.text())
        .iter()
        .filter_map(|t| record_of(t).map(|(k, i)| (k.to_owned(), i.to_owned())))
        .collect()
}

/// The owner's records.
#[derive(Clone)]
pub struct Records {
    notebook: NotebookRecords,
}

impl Records {
    pub fn new(notebook: Notebook) -> Self {
        Self {
            notebook: NotebookRecords::new(notebook),
        }
    }

    /// Held while a record is read, changed and written.
    fn writing(&self) -> anyhow::Result<Hold> {
        self.notebook().hold()
    }

    /// Keep the records from other writers while more than one of them is
    /// read, checked and written.
    pub(crate) fn hold(&self) -> anyhow::Result<Held<'_>> {
        Ok(Held {
            records: self,
            _writing: self.writing()?,
        })
    }

    pub fn notebook(&self) -> &Notebook {
        self.notebook.notebook()
    }

    /// Whether a record belongs to `workgroup`. Every record belongs to `all`, and a
    /// record with no workgroup belongs only to `all`.
    pub fn in_workgroup(workgroups: &[String], workgroup: &str) -> bool {
        workgroup == crate::workgroups::ALL || workgroups.iter().any(|c| c == workgroup)
    }

    pub fn kinds(&self) -> anyhow::Result<Vec<Kind>> {
        self.notebook
            .kind_names()?
            .iter()
            .map(|k| self.notebook.kind(k))
            .collect()
    }

    pub fn list(&self, kind: &str) -> anyhow::Result<Vec<Record>> {
        self.notebook.list(kind)
    }

    /// The record an id, a code or a wiki link names.
    pub fn get(&self, kind: &str, id: &str) -> anyhow::Result<Option<Record>> {
        match self.notebook.resolve(kind, id)? {
            Some(id) => self.notebook.get(kind, &id),
            None => Ok(None),
        }
    }

    pub fn need(&self, kind: &str, id: &str) -> anyhow::Result<Record> {
        self.get(kind, id)?
            .with_context(|| format!("no record {kind}/{id}"))
    }

    pub fn codes(&self, kind: &str) -> anyhow::Result<BTreeMap<String, String>> {
        self.notebook.codes(kind)
    }

    pub fn code(&self, kind: &str, id: &str) -> anyhow::Result<String> {
        self.notebook.code(kind, id)
    }

    pub fn backlinks(&self, kind: &str, id: &str) -> anyhow::Result<Vec<Backlink>> {
        self.notebook.backlinks(kind, id)
    }

    /// The records as a text may name them.
    pub fn names(&self) -> Names {
        Names::new(self.notebook.clone())
    }

    /// Add a record.
    pub fn add(&self, writer: Writer, kind: &str, patch: Patch) -> anyhow::Result<Record> {
        self.hold()?.add(writer, kind, patch)
    }

    /// Refuse what [`Self::add`] would refuse, writing nothing.
    pub fn check_add(&self, writer: Writer, kind: &str, patch: Patch) -> anyhow::Result<()> {
        writer.may_write()?;
        self.notebook.verify(&self.drafted(writer, kind, patch)?)
    }

    /// A record as it would be added.
    fn drafted(&self, writer: Writer, kind: &str, mut patch: Patch) -> anyhow::Result<Record> {
        let title = patch.title.take().unwrap_or_default();
        let mut record = self.notebook.draft(kind, &title)?;
        match kind {
            TASKS => {
                patch.default_field("Status", "pending");
                patch.default_field("Assignee", name_of(writer));
            }
            ROUTINES => {
                patch.fields.retain(|f| !same(&f.name, "Managed by"));
                patch.default_field("Assignee", SECRETARY);
                patch.default_field("Managed by", name_of(writer));
                patch.default_field("Enabled", "yes");
            }
            PARTIES => patch.default_field(ADDED_BY, name_of(writer)),
            _ => {}
        }
        self.apply(writer, &mut record, patch)?;
        if kind == PARTIES {
            self.refuse_same_party(writer, &record, None)?;
        }
        Ok(record)
    }

    /// Change a record, named by its id or code.
    pub fn change(
        &self,
        writer: Writer,
        kind: &str,
        id: &str,
        patch: Patch,
    ) -> anyhow::Result<Record> {
        self.hold()?.change(writer, kind, id, patch)
    }

    /// Say that a record was made into a task.
    pub fn moved_to_task(&self, kind: &str, id: &str, task: &Record) -> anyhow::Result<Record> {
        self.hold()?.moved_to_task(kind, id, task)
    }

    /// A task not over of the same title in the same workgroups. What is put
    /// into a task shows where the task does: in fewer it would be lost from
    /// where it belongs, in more it would show where it does not.
    pub fn open_task_titled(
        &self,
        title: &str,
        workgroups: &[String],
    ) -> anyhow::Result<Option<Record>> {
        let plain = |t: &str| t.split_whitespace().collect::<String>().to_lowercase();
        let set = |w: &[String]| w.iter().cloned().collect::<BTreeSet<_>>();
        Ok(self.list(TASKS)?.into_iter().find(|t| {
            !is_over(t)
                && !is_delegated(t)
                && plain(&t.title()) == plain(title)
                && set(&t.workgroups()) == set(workgroups)
        }))
    }

    /// Remove a record for good.
    pub fn remove(&self, writer: Writer, kind: &str, id: &str) -> anyhow::Result<()> {
        self.hold()?.remove(writer, kind, id)
    }

    /// What the owner decided about a routine is the owner's to change.
    fn may_change(&self, writer: Writer, record: &Record) -> anyhow::Result<()> {
        let owners = record.kind == ROUTINES
            && record.body.value("Managed by").as_deref() != Some(SECRETARY);
        anyhow::ensure!(
            !owners || writer == Writer::Owner,
            "{} was set by the owner; only the owner can change it",
            record.title()
        );
        Ok(())
    }

    fn apply(&self, writer: Writer, record: &mut Record, patch: Patch) -> anyhow::Result<()> {
        let status_before = record.body.value("Status");
        let assignee_before = record.body.value("Assignee");
        let seen_before = last_seen(record);
        let was_resting = is_resting(record);
        let kind = self.notebook.kind(&record.kind)?;
        if let Some(title) = &patch.title {
            record.body.set_title(title)?;
        }
        if let Some(labels) = &patch.labels {
            record.meta.set_list("labels", labels);
        }
        if let Some(workgroups) = &patch.workgroups {
            record.meta.set_list("workgroup", workgroups);
        }
        if !patch.source.trim().is_empty() && record.meta.text("source").is_none() {
            record.meta.set("source", patch.source.trim());
        }
        for (name, value) in [("google", &patch.google), ("url", &patch.url)] {
            if let Some(value) = value {
                record
                    .meta
                    .set_opt(name, Some(value.as_str()).filter(|v| !v.is_empty()));
            }
        }
        if kind.layout.free {
            anyhow::ensure!(
                patch.sections.is_empty() && patch.details.is_none(),
                "a record of {} has no sections or details",
                kind.name
            );
        }
        for field in &patch.fields {
            let name = kind
                .field(&field.name)
                .map(|(n, _)| n.as_str())
                .with_context(|| {
                    format!(
                        "{} has no field {}; it has {}. Put other facts in details",
                        kind.name,
                        field.name,
                        kind.fields
                            .iter()
                            .map(|(n, _)| n.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            anyhow::ensure!(
                !same(name, "Managed by")
                    || writer == Writer::Owner
                    || record.body.field("Managed by").is_none(),
                "who manages a routine is the owner's to change"
            );
            if record.body.value(name).unwrap_or_default() != field.value
                || record.body.values(name) != field.values
            {
                record.body.set(name, field.to_value())?;
            }
        }
        for section in &patch.sections {
            let (name, ty) = kind.section(&section.name).with_context(|| {
                format!(
                    "{} has no section {}; it has {}",
                    kind.name,
                    section.name,
                    kind.layout.sections.join(", ")
                )
            })?;
            match ty {
                SectionType::Fields(_) => set_fields(record, name, &section.fields)?,
                SectionType::Items(_) => set_items(record, name, &section.items)?,
            }
        }
        if let Some(details) = &patch.details {
            set_fields(record, DETAILS, details)?;
        }
        if let Some(notes) = &patch.notes {
            record.body.set_notes(notes)?;
        }
        let who = name_of(writer);
        let at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        if record.kind == TASKS {
            let status = record.body.value("Status");
            if status != status_before {
                if is_over(record) {
                    if patch.given("Finished").is_none() {
                        record.body.set("Finished", at.as_str())?;
                    }
                } else {
                    record.body.set("Finished", "")?;
                }
                if let (Some(before), Some(now)) = (&status_before, &status) {
                    record.body.log(&at, who, &format!("{before} → {now}"))?;
                }
            }
            let assignee = record.body.value("Assignee");
            if let (Some(before), Some(now)) = (&assignee_before, &assignee)
                && before != now
            {
                record
                    .body
                    .log(&at, who, &format!("担当 {before} → {now}"))?;
            }
        }
        if record.kind == PARTIES
            && parties::note_seen(
                record,
                seen_before,
                writer,
                patch.given("Status").is_some(),
                was_resting,
            )?
        {
            record.body.log(&at, who, "また出たので、休みをやめた")?;
        }
        for line in patch.log.iter().filter(|l| !l.trim().is_empty()) {
            record.body.log(&at, who, line)?;
        }
        Ok(())
    }

    /// Write what Famulen did with a routine, whoever set it: lines of its
    /// log, and the runs still to be skipped.
    pub(crate) fn note_routine(
        &self,
        id: &str,
        lines: &[String],
        skips: &[String],
    ) -> anyhow::Result<()> {
        let _writing = self.writing()?;
        let mut record = self.need(ROUTINES, id)?;
        let at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        for line in lines {
            record.body.log(&at, "famulen", line)?;
        }
        record.body.set("Skip", skips.to_vec())?;
        let subject = format!("{} の経過を書いた", record.title());
        self.notebook
            .write(&Writer::Secretary.signature(), &record, &subject)
    }

    /// Write what the worker pool did with a task: a line of its log, and how
    /// it stands unless the task is over or taken back. Writing the same line again does
    /// nothing.
    pub(crate) fn note_task(&self, id: &str, note: &TaskNote) -> anyhow::Result<()> {
        let _writing = self.writing()?;
        let mut record = self.need(TASKS, id)?;
        if log_lines(&record).iter().any(|l| l.text == note.line) {
            return Ok(());
        }
        let at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        // One taken back meanwhile is not the pool's to change.
        let pools = is_delegated(&record) && !is_over(&record);
        if let Some(status) = note.status.filter(|_| pools) {
            let before = record.body.value("Status").unwrap_or_default();
            if before != status {
                record.body.set("Status", status)?;
                record
                    .body
                    .log(&at, "famulen", &format!("{before} → {status}"))?;
            }
            record
                .body
                .set("Waiting for", note.waiting_for.unwrap_or_default())?;
            if is_over(&record) {
                record.body.set("Finished", at.as_str())?;
            }
        }
        record.body.log(&at, "famulen", &note.line)?;
        let subject = format!("{} の経過を書いた", record.title());
        self.notebook
            .write(&Writer::Secretary.signature(), &record, &subject)
    }

    /// Take a workgroup out of every record. The records stay.
    pub fn forget_workgroup(&self, writer: Writer, workgroup: &str) -> anyhow::Result<()> {
        writer.may_write()?;
        let _writing = self.writing()?;
        for kind in self.notebook.kind_names()? {
            for mut record in self.notebook.list(&kind)? {
                let workgroups = record.workgroups();
                if !workgroups.iter().any(|c| c == workgroup) {
                    continue;
                }
                let kept: Vec<String> = workgroups.into_iter().filter(|c| c != workgroup).collect();
                record.meta.set_list("workgroup", &kept);
                let subject = format!("{} をワークグループ {workgroup} から外した", record.title());
                self.notebook
                    .write(&writer.signature(), &record, &subject)?;
            }
        }
        Ok(())
    }

    /// The tasks a task waits for that are not over.
    pub fn blockers(tasks: &BTreeMap<String, Record>, task: &Record) -> Vec<String> {
        task.body
            .values("Blocked by")
            .iter()
            .filter_map(|v| record_of(link_target(v)).map(|(_, id)| id.to_owned()))
            .filter(|id| tasks.get(id).is_some_and(|t| !is_over(t)))
            .collect()
    }
}

/// Make a section of fields hold exactly these.
/// The records kept from other writers.
pub(crate) struct Held<'a> {
    records: &'a Records,
    _writing: Hold,
}

impl Held<'_> {
    pub(crate) fn records(&self) -> &Records {
        self.records
    }

    pub(crate) fn add(&self, writer: Writer, kind: &str, patch: Patch) -> anyhow::Result<Record> {
        writer.may_write()?;
        let r = self.records;
        let record = r.drafted(writer, kind, patch)?;
        let subject = format!("{} を足した", record.title());
        r.notebook.write(&writer.signature(), &record, &subject)?;
        r.need(kind, &record.id)
    }

    pub(crate) fn change(
        &self,
        writer: Writer,
        kind: &str,
        id: &str,
        patch: Patch,
    ) -> anyhow::Result<Record> {
        writer.may_write()?;
        let r = self.records;
        let mut record = r.need(kind, id)?;
        r.may_change(writer, &record)?;
        parties::may_set_added_by(writer, &record, &patch)?;
        let before = record.clone();
        r.apply(writer, &mut record, patch)?;
        if kind == PARTIES {
            r.refuse_same_party(writer, &record, Some(&before))?;
        }
        if record != before {
            let subject = format!("{} を直した", record.title());
            r.notebook.write(&writer.signature(), &record, &subject)?;
        }
        r.need(kind, &record.id)
    }

    pub(crate) fn remove(&self, writer: Writer, kind: &str, id: &str) -> anyhow::Result<()> {
        writer.may_write()?;
        let r = self.records;
        let record = r.need(kind, id)?;
        r.may_change(writer, &record)?;
        let subject = format!("{} を消した", record.title());
        r.notebook
            .remove(&writer.signature(), kind, &record.id, &subject)?;
        Ok(())
    }

    /// It stays as it is, so what links to it keeps resolving.
    pub(crate) fn moved_to_task(
        &self,
        kind: &str,
        id: &str,
        task: &Record,
    ) -> anyhow::Result<Record> {
        let r = self.records;
        let mut record = r.need(kind, id)?;
        record.meta.set("moved", task.target());
        let subject = format!("{} をタスクに移した", record.title());
        r.notebook
            .write(&Writer::Secretary.signature(), &record, &subject)?;
        r.need(kind, id)
    }
}

fn set_fields(record: &mut Record, section: &str, wanted: &[Field]) -> anyhow::Result<()> {
    let now: Vec<Field> = record
        .body
        .items(section)
        .into_iter()
        .filter_map(Field::of)
        .collect();
    for field in &now {
        if !wanted.iter().any(|f| same(&f.name, &field.name)) {
            record.body.set_in(section, &field.name, "")?;
        }
    }
    for field in wanted {
        anyhow::ensure!(
            !field.name.trim().is_empty() && !field.name.contains([':', '\n', '[']),
            "a field's name is a word or a few, without a colon: {:?}",
            field.name
        );
        if !now.contains(field) {
            record
                .body
                .set_in(section, field.name.trim(), field.to_value())?;
        }
    }
    Ok(())
}

/// Make a section of items hold exactly these, leaving those that do not
/// change as they are written.
fn set_items(record: &mut Record, section: &str, wanted: &[Entry]) -> anyhow::Result<()> {
    let now: Vec<Entry> = record
        .body
        .items(section)
        .into_iter()
        .map(Entry::of)
        .collect();
    if now == wanted {
        return Ok(());
    }
    for at in (wanted.len()..now.len()).rev() {
        record.body.remove_item(section, at)?;
    }
    for (at, entry) in wanted.iter().enumerate() {
        match now.get(at) {
            Some(same) if same == entry => {}
            Some(_) => record.body.replace_item(section, at, &entry.to_item())?,
            None => record.body.add_item(section, &entry.to_item())?,
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::notebook::testing::TempDir;

    /// Records in a fresh notebook, with the directory that holds it.
    pub fn records(name: &str) -> (TempDir, Records) {
        let tmp = TempDir::new(name);
        let notebook = crate::notebook::open(tmp.path());
        crate::notebook::ensure(&notebook).unwrap();
        (tmp, Records::new(notebook))
    }
}

#[cfg(test)]
mod tests {
    use super::testing::records;
    use super::*;
    use crate::notebook;

    pub(super) const ME: Writer = Writer::Owner;

    pub(super) fn add(r: &Records, kind: &str, title: &str) -> Record {
        r.add(ME, kind, Patch::titled(title)).unwrap()
    }

    #[test]
    fn a_task_goes_to_whoever_added_it_and_is_committed_in_their_name() {
        let (tmp, r) = records("records-add");

        let mine = add(&r, TASKS, "自分で");
        assert_eq!(mine.body.value("Assignee").as_deref(), Some(OWNER));
        assert_eq!(mine.body.value("Status").as_deref(), Some("pending"));
        assert_eq!(
            notebook::log(tmp.path(), 1).unwrap()[0].author,
            notebook::owner()
        );
        let theirs = r
            .add(Writer::Secretary, TASKS, Patch::titled("秘書が"))
            .unwrap();
        assert_eq!(theirs.body.value("Assignee").as_deref(), Some(SECRETARY));
        assert_eq!(
            notebook::log(tmp.path(), 1).unwrap()[0].author,
            notebook::famulen()
        );
    }

    #[test]
    fn a_finished_task_says_when_and_one_taken_up_again_does_not() {
        let (_tmp, r) = records("records-finish");
        let t = add(&r, TASKS, "x");
        let to = |status: &str| {
            r.change(ME, TASKS, &t.id, Patch::default().with("status", status))
                .unwrap()
        };

        let done = to("done");
        assert!(done.body.value("Finished").is_some());
        assert_eq!(log_lines(&done)[0].text, "pending → done");
        assert_eq!(log_lines(&done)[0].who, OWNER);
        assert!(is_over(&done));
        let again = to("in_progress");
        assert!(again.body.value("Finished").is_none());
        assert_eq!(log_lines(&again).len(), 2);
    }

    #[test]
    fn a_task_can_be_handed_over_and_back_by_its_code() {
        let (_tmp, r) = records("records-hand");
        let t = add(&r, TASKS, "x");
        let code = r.code(TASKS, &t.id).unwrap();

        r.change(
            ME,
            TASKS,
            &code,
            Patch::default().with("Assignee", SECRETARY),
        )
        .unwrap();

        let t = r.need(TASKS, &code).unwrap();
        assert_eq!(t.body.value("Assignee").as_deref(), Some(SECRETARY));
        assert_eq!(log_lines(&t)[0].text, "担当 owner → secretary");
    }

    #[test]
    fn a_task_is_blocked_until_what_it_waits_for_is_over() {
        let (_tmp, r) = records("records-blocked");
        let a = add(&r, TASKS, "先");
        let b = r
            .add(
                ME,
                TASKS,
                Patch {
                    fields: vec![Field {
                        name: "Blocked by".into(),
                        value: String::new(),
                        values: vec![a.link()],
                    }],
                    ..Patch::titled("後")
                },
            )
            .unwrap();
        let all = || -> BTreeMap<String, Record> {
            r.list(TASKS)
                .unwrap()
                .into_iter()
                .map(|t| (t.id.clone(), t))
                .collect()
        };

        assert_eq!(Records::blockers(&all(), &b), std::slice::from_ref(&a.id));
        r.change(
            ME,
            TASKS,
            &a.id,
            Patch::default().with("Status", "cancelled"),
        )
        .unwrap();
        assert!(Records::blockers(&all(), &b).is_empty());
        let circle = Patch::default().with("Blocked by", b.link());
        assert!(r.change(ME, TASKS, &a.id, circle).is_err());
    }

    #[test]
    fn fields_sections_details_notes_and_labels_are_written_and_read_back() {
        let (_tmp, r) = records("records-all");
        let patch = Patch {
            notes: Some("## 経緯\n\n- 一つ".into()),
            labels: Some(vec!["家電".into()]),
            workgroups: Some(vec!["work".into()]),
            source: "コンソール".into(),
            sections: vec![
                SectionContent {
                    name: "acquired".into(),
                    fields: vec![
                        Field::new("On", "2026-03-01"),
                        Field::new("Price", "398000 JPY"),
                    ],
                    ..SectionContent::default()
                },
                SectionContent {
                    name: "Deadlines".into(),
                    items: vec![Entry {
                        text: "保証".into(),
                        fields: vec![Field::new("On", "2027-03-01")],
                    }],
                    ..SectionContent::default()
                },
            ],
            details: Some(vec![Field::new("色", "銀"), Field::new("置き場", "机")]),
            ..Patch::titled("MacBook")
        }
        .with("Category", "Mac");
        let h = r.add(ME, HOLDINGS, patch).unwrap();

        assert_eq!(h.body.value("Category").as_deref(), Some("Mac"));
        assert_eq!(h.notes(), "## 経緯\n\n- 一つ");
        assert_eq!(h.labels(), ["家電"]);
        assert_eq!(
            details(&h)[1],
            Field {
                values: vec!["机".into()],
                ..Field::new("置き場", "机")
            }
        );
        assert_eq!(sections(&h).len(), 2);

        let h = r
            .change(
                ME,
                HOLDINGS,
                &h.id,
                Patch {
                    details: Some(vec![Field::new("色", "黒")]),
                    sections: vec![SectionContent {
                        name: "Deadlines".into(),
                        ..SectionContent::default()
                    }],
                    ..Patch::default()
                },
            )
            .unwrap();
        assert_eq!(details(&h).len(), 1);
        assert_eq!(sections(&h).len(), 1);
        assert_eq!(h.notes(), "## 経緯\n\n- 一つ");
    }

    #[test]
    fn a_field_or_section_the_kind_does_not_have_is_refused_with_the_ones_it_has() {
        let (_tmp, r) = records("records-unknown");

        let wrong = Patch::titled("x").with("Colour", "red");
        let error = r.add(ME, TASKS, wrong).unwrap_err().to_string();
        assert!(error.contains("Blocked by"), "{error}");
        assert!(
            r.add(ME, MEMOS, Patch::titled("x").with("Due", "x"))
                .is_err()
        );
        assert!(r.add(ME, TASKS, Patch::titled("  ")).is_err());
        assert!(
            r.add(ME, TASKS, Patch::titled("x").with("Due", "tomorrow"))
                .is_err()
        );
        assert!(r.list(TASKS).unwrap().is_empty());
    }

    #[test]
    fn the_secretary_cannot_change_a_routine_the_owner_set() {
        let (_tmp, r) = records("records-routine");
        let mine = r
            .add(
                ME,
                ROUTINES,
                Patch::titled("朝").with("Schedule", "0 9 * * *"),
            )
            .unwrap();
        let theirs = r
            .add(
                Writer::Secretary,
                ROUTINES,
                Patch::titled("晩")
                    .with("Schedule", "0 21 * * *")
                    .with("Managed by", OWNER),
            )
            .unwrap();
        assert_eq!(theirs.body.value("Managed by").as_deref(), Some(SECRETARY));

        let off = || Patch::default().with("Enabled", "no");
        assert!(
            r.change(Writer::Secretary, ROUTINES, &mine.id, off())
                .is_err()
        );
        assert!(r.remove(Writer::Secretary, ROUTINES, &mine.id).is_err());
        let seize = Patch::default().with("Managed by", OWNER);
        assert!(
            r.change(Writer::Secretary, ROUTINES, &theirs.id, seize)
                .is_err()
        );
        r.change(Writer::Secretary, ROUTINES, &theirs.id, off())
            .unwrap();
        r.change(ME, ROUTINES, &theirs.id, off()).unwrap();
    }

    #[test]
    fn a_removed_workgroup_leaves_the_records_it_was_on() {
        let (_tmp, r) = records("records-workgroup");
        let m = r
            .add(
                ME,
                MEMOS,
                Patch {
                    workgroups: Some(vec!["work".into(), "home".into()]),
                    ..Patch::titled("x")
                },
            )
            .unwrap();

        r.forget_workgroup(ME, "work").unwrap();

        assert_eq!(r.need(MEMOS, &m.id).unwrap().workgroups(), ["home"]);
        assert!(Records::in_workgroup(&["home".into()], "home"));
        assert!(!Records::in_workgroup(&[], "home"));
        assert!(Records::in_workgroup(&[], crate::workgroups::ALL));
    }

    #[test]
    fn the_lookout_and_a_worker_cannot_write() {
        let (tmp, r) = records("records-refused");
        let t = add(&r, TASKS, "x");
        let commits = notebook::log(tmp.path(), 10).unwrap().len();

        for writer in [Writer::Lookout, Writer::Worker] {
            for refused in [
                r.add(writer, TASKS, Patch::titled("y")).unwrap_err(),
                r.change(writer, TASKS, &t.id, Patch::titled("y"))
                    .unwrap_err(),
                r.remove(writer, TASKS, &t.id).unwrap_err(),
                r.link_account(writer, "p", "s", "h", true, "").unwrap_err(),
                r.answer_account(writer, "s:h", true).unwrap_err(),
                r.forget_workgroup(writer, "work").unwrap_err(),
            ] {
                assert_eq!(
                    refused.downcast_ref::<notebook::Refused>(),
                    Some(&notebook::Refused(writer))
                );
            }
        }
        assert_eq!(notebook::log(tmp.path(), 10).unwrap().len(), commits);
    }
}
