//! What is wrong with a record beside the notebook's others.

use std::collections::{BTreeMap, BTreeSet};

use super::{Findings, Known, line_at};
use crate::body::{LOG, NOTES, same};
use crate::link::found;
use crate::link_target;
use crate::records::{Record, record_of, titled_link};

/// The records of a notebook, by kind and id.
pub(super) type ByKey = BTreeMap<(String, String), Record>;

/// The fields that may not lead back to the record they are in.
pub(super) const ACYCLIC: [&str; 2] = ["Parent", "Blocked by"];

pub(super) fn check(known: &Known, record: &Record, text: &str, findings: &mut Findings) {
    known.id_twice(&record.id, findings);
    for workgroup in record.workgroups() {
        known.unknown_workgroup(&workgroup, findings);
    }
    let line_of = |part: &str| text.find(part).map_or(1, |at| line_at(text, at));
    for link in found(text) {
        let (Some((kind, id)), Some(title)) = (record_of(link.target), link.title) else {
            continue;
        };
        // A name given on purpose in free text, such as a nickname, is not
        // stale.
        if let Some(now) = known.title(kind, id)
            && titled_link(kind, id, title) != titled_link(kind, id, &now)
            && is_structured(record, link.whole)
        {
            let why = format!("{title} is now {now}");
            findings.warning(line_of(link.whole), "link-title", why);
        }
    }
    let key = (record.kind.clone(), record.id.clone());
    for field in ACYCLIC {
        if leads_to_itself(&known.records, &key, field) {
            let why = format!("{field} leads back to the task itself");
            findings.error(line_of(field), "circle", why);
        }
    }
    let mine = record.target();
    for target in related(record) {
        if target != mine
            && let Some(other) = record_of(&target)
                .and_then(|(kind, id)| known.records.get(&(kind.to_owned(), id.to_owned())))
            && related(other).contains(&mine)
        {
            let why = format!("{} says it too", other.title());
            findings.warning(line_of(&target), "both-sides", why);
        }
    }
}

/// What a record says it is related to, as link targets.
fn related(record: &Record) -> Vec<String> {
    let relations = record.body.items("Relations");
    record
        .body
        .values("Related")
        .into_iter()
        .chain(relations.into_iter().map(|i| i.plain()))
        .map(|v| link_target(&v).to_owned())
        .collect()
}

pub(super) fn leads_to_itself(all: &ByKey, start: &(String, String), field: &str) -> bool {
    let mut seen = BTreeSet::new();
    let mut next = vec![start.clone()];
    while let Some(at) = next.pop() {
        let Some(record) = all.get(&at) else {
            continue;
        };
        for value in record.body.values(field) {
            let Some((kind, id)) = record_of(link_target(&value)) else {
                continue;
            };
            let to = (kind.to_owned(), id.to_owned());
            if to == *start {
                return true;
            }
            if seen.insert(to.clone()) {
                next.push(to);
            }
        }
    }
    false
}

/// Whether a link stands in the fields or sections, where a title is the
/// record's own, and not in free text.
fn is_structured(record: &Record, link: &str) -> bool {
    let body = &record.body;
    body.fields().iter().any(|f| f.value().contains(link))
        || body.sections().iter().any(|s| {
            !same(&s.name, NOTES)
                && !same(&s.name, LOG)
                && body.text()[s.content.clone()].contains(link)
        })
}
