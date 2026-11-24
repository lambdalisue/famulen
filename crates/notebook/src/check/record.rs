//! What is wrong with a record by itself.

use super::{Findings, has_t};
use crate::body::{DETAILS, Item, LOG, NOTES, Section, same};
use crate::records::{FieldType, Kind, METADATA, Record, SectionType, is_date, is_ulid, minute};

/// The times of a record that follow one another.
const ORDERED: [(&str, &str); 4] = [
    ("Start", "End"),
    ("Start", "Due"),
    ("Start", "Until"),
    ("Started", "Ended"),
];

pub(super) fn check(kind: &Kind, id: &str, text: &str, findings: &mut Findings) {
    let record = match Record::parse(kind, id, text) {
        Ok(record) => record,
        Err(error) => {
            findings.error(1, "unreadable", format!("{error:#}"));
            return;
        }
    };
    metadata(&record, findings);
    // Lines are counted in the file, the body's after the frontmatter.
    let lead = text.matches('\n').count() - record.body.text().matches('\n').count();
    let mut body = BodyCheck {
        kind,
        record: &record,
        lead,
        findings,
    };
    if record.body.title().is_none_or(str::is_empty) {
        body.findings.error(lead + 1, "title", "there is no title");
    }
    if !kind.layout.free {
        body.fields(record.body.fields(), Some(&kind.fields));
        body.sections();
        body.times();
    }
}

fn metadata(record: &Record, findings: &mut Findings) {
    let id = &record.id;
    if !is_ulid(id) {
        let why = format!("the file name is no ULID in lower case: {id}");
        findings.error(1, "id", why);
    }
    match record.meta.text("id") {
        None => findings.error(1, "id", "there is no id"),
        Some(written) if written != *id => {
            findings.error(1, "id", format!("the id {written} is not the file's name"));
        }
        Some(_) => {}
    }
    for name in ["created", "updated"] {
        if let Some(at) = record.meta.text(name)
            && chrono::DateTime::parse_from_rfc3339(&at).is_err()
        {
            findings.error(1, "metadata", format!("{name} is no RFC 3339 time: {at}"));
        }
    }
    if let Some(on) = record.meta.text("obsolete")
        && !is_date(&on)
    {
        findings.error(1, "metadata", format!("obsolete is no date: {on}"));
    }
    if record.meta.text("moved").is_some() && record.moved().is_none() {
        let why = "moved names no record: write records/<kind>/<id>";
        findings.error(1, "metadata", why);
    }
    for (name, _) in record.meta.fields() {
        if !METADATA.contains(&name) {
            findings.warning(1, "metadata", format!("unknown metadata: {name}"));
        }
    }
}

struct BodyCheck<'a, 'f> {
    kind: &'a Kind,
    record: &'a Record,
    /// How many lines of the file stand before the body.
    lead: usize,
    findings: &'a mut Findings<'f>,
}

impl BodyCheck<'_, '_> {
    fn line_of(&self, at: usize) -> usize {
        self.lead + self.record.body.line_of(at)
    }

    /// A set of fields; without a declaration, any field may be there.
    fn fields(&mut self, items: &[Item], declared: Option<&[(String, FieldType)]>) {
        let mut seen: Vec<&str> = Vec::new();
        for item in items {
            let line = self.line_of(item.span.start);
            let Some((name, _)) = item.field() else {
                let why = format!("not `Name: value`: {}", item.text);
                self.findings.error(line, "item", why);
                continue;
            };
            if seen.iter().any(|s| same(s, name)) {
                let why = format!("{name} is there twice");
                self.findings.error(line, "twice", why);
            }
            seen.push(name);
            let Some(declared) = declared else { continue };
            match declared.iter().find(|(n, _)| same(n, name)) {
                None => {
                    let why = format!("unknown field: {name}");
                    self.findings.warning(line, "unknown-field", why);
                }
                Some((proper, ty)) => self.field(item, line, name, proper, ty),
            }
        }
    }

    fn field(&mut self, item: &Item, line: usize, name: &str, proper: &str, ty: &FieldType) {
        if let Some(why) = ty.misfit(item) {
            let why = format!("{proper}: {why}");
            self.findings.error(line, "value", why);
        } else if *ty == FieldType::DateOrMinute && has_t(&item.value()) {
            let why = format!("{proper} is written YYYY-MM-DD HH:MM");
            self.findings.warning(line, "time-style", why);
        }
        self.name_style(line, name, proper);
    }

    fn name_style(&mut self, line: usize, name: &str, proper: &str) {
        if proper != name {
            let why = format!("{name} is written {proper}");
            self.findings.warning(line, "name-style", why);
        }
    }

    fn sections(&mut self) {
        let layout = &self.kind.layout;
        let mut seen: Vec<&str> = Vec::new();
        let mut rank = None;
        for section in self.record.body.sections() {
            let name = section.name.as_str();
            let line = self.line_of(section.heading.start);
            if seen.iter().any(|s| same(s, name)) {
                let why = format!("the section {name} is there twice");
                self.findings.error(line, "twice", why);
            }
            seen.push(name);
            let here = layout.rank(name);
            if here < rank {
                let why = format!("{name} comes earlier");
                self.findings.warning(line, "order", why);
            }
            rank = rank.max(here);
            let proper = layout.proper(name);
            self.name_style(line, name, proper);
            self.section(section, proper);
        }
    }

    fn section(&mut self, section: &Section, proper: &str) {
        if proper == NOTES {
            return;
        }
        for stray in &section.stray {
            let line = self.line_of(stray.start);
            let why = format!("{proper} holds only a list");
            self.findings.error(line, "item", why);
        }
        if proper == DETAILS {
            self.fields(&section.items, None);
            return;
        }
        if proper == LOG {
            return;
        }
        match self.kind.section(proper) {
            Some((_, SectionType::Fields(declared))) => {
                self.fields(&section.items, Some(declared));
            }
            Some((_, SectionType::Items(declared))) => self.items(&section.items, declared),
            None => {}
        }
    }

    /// Items, whose text is the first field's value.
    fn items(&mut self, items: &[Item], declared: &[(String, FieldType)]) {
        let [(first, ty), rest @ ..] = declared else {
            return;
        };
        for item in items {
            let line = self.line_of(item.span.start);
            let text = item.plain();
            if text.is_empty() {
                let why = format!("an item has no {first}");
                self.findings.error(line, "item", why);
            } else if let Some(why) = ty.misfit_text(&text) {
                self.findings
                    .error(line, "value", format!("{first}: {why}"));
            }
            self.fields(&item.items, Some(rest));
        }
    }

    fn times(&mut self) {
        let body = &self.record.body;
        for (from, to) in ORDERED {
            if let (Some(first), Some(item)) = (body.value(from), body.field(to))
                && backwards(&first, &item.value())
            {
                let line = self.line_of(item.span.start);
                let why = format!("{to} is before {from}");
                self.findings.error(line, "backwards", why);
            }
        }
    }
}

/// Whether a time comes before the one it should follow. Two dates, or two
/// times; a date and a time are compared by their days.
fn backwards(first: &str, then: &str) -> bool {
    match (minute(first), minute(then)) {
        (Some(a), Some(b)) => b < a,
        _ => match (first.get(..10), then.get(..10)) {
            (Some(a), Some(b)) if is_date(a) && is_date(b) => b < a,
            _ => false,
        },
    }
}
