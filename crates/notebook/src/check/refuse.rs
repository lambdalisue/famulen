//! Refusing a write that would make a record wrong.

use std::collections::BTreeSet;

use super::relations::{ACYCLIC, ByKey, leads_to_itself};
use super::{Findings, Severity, record, text_findings};
use crate::records::{Names, Record, Records};

/// Refuse a record's new text when it is wrong where the text it replaces
/// was not.
pub(crate) fn refuse_new_errors(
    records: &Records,
    written: &Record,
    text: &str,
) -> anyhow::Result<()> {
    let kind = records.kind(&written.kind)?;
    let rel = written.rel();
    let names = Names::new(records.clone());
    let wrong = |text: &str| -> BTreeSet<(&'static str, String)> {
        let mut findings = Findings::of(&rel);
        record::check(&kind, &written.id, text, &mut findings);
        text_findings(&names, text, &mut findings);
        findings
            .all
            .into_iter()
            .filter(|f| f.severity == Severity::Error)
            .map(|f| (f.rule, f.why))
            .collect()
    };
    let before = std::fs::read_to_string(records.path(&written.kind, &written.id))
        .map(|text| wrong(&text))
        .unwrap_or_default();
    let new: Vec<String> = wrong(text)
        .difference(&before)
        .map(|(rule, why)| format!("{rule}: {why}"))
        .collect();
    anyhow::ensure!(new.is_empty(), "{rel} would be wrong: {}", new.join("; "));
    if kind.name == "tasks" {
        refuse_new_circles(records, written)?;
    }
    Ok(())
}

fn refuse_new_circles(records: &Records, written: &Record) -> anyhow::Result<()> {
    if ACYCLIC.iter().all(|f| written.body.field(f).is_none()) {
        return Ok(());
    }
    let mut all: ByKey = records
        .list(&written.kind)?
        .into_iter()
        .map(|r| ((r.kind.clone(), r.id.clone()), r))
        .collect();
    let key = (written.kind.clone(), written.id.clone());
    let before = ACYCLIC.map(|field| leads_to_itself(&all, &key, field));
    all.insert(key.clone(), written.clone());
    for (field, before) in ACYCLIC.into_iter().zip(before) {
        anyhow::ensure!(
            before || !leads_to_itself(&all, &key, field),
            "{} would be wrong: {field} leads back to the task itself",
            written.rel()
        );
    }
    Ok(())
}
