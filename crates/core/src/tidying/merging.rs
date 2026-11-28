//! Putting records of one thing into one: what the others say goes to the one
//! that stays, what it says otherwise to its log, and links follow it.

use famulen_notebook::body::{RELATED, same};
use famulen_notebook::link_target;
use famulen_notebook::records::{FieldType, record_of};
use rusqlite::params;

use super::holdings::how_many;
use crate::memory::{Change, Op};
use crate::notebook::Writer;
use crate::records::{
    ADDED_BY, CONTRACTS, Field, HOLDINGS, Held, LAST_SEEN, PARTIES, Patch, QUANTITY, Record,
    SECRETARY, SectionContent, TASKS, bare, counted, details, fields, last_seen, log_lines,
    sections, with_quantity,
};
use crate::store::Store;

/// How the log line of a merge begins; the ids it names follow the colon.
pub(super) const MERGED: &str = "夢で重なっていた記録をまとめた";

pub(super) fn word(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Add what `into` does not say, and the values of a list it lacks. What it
/// says otherwise is given back.
fn fill(into: &mut Vec<Field>, from: Vec<Field>, listed: &dyn Fn(&str) -> bool) -> Vec<Field> {
    let mut otherwise = Vec::new();
    for field in from {
        match into.iter_mut().find(|f| same(&f.name, &field.name)) {
            Some(said) if listed(&field.name) => {
                let mut values = said.values.clone();
                union(&mut values, field.values);
                said.value = values.join(", ");
                said.values = values;
            }
            Some(said) if word(&said.value) != word(&field.value) => otherwise.push(field),
            Some(_) => {}
            None => into.push(field),
        }
    }
    otherwise
}

fn union(into: &mut Vec<String>, from: Vec<String>) {
    for item in from {
        if !into.contains(&item) {
            into.push(item);
        }
    }
}

/// The metadata a merge combines, or that is about the record itself.
const COMBINED: [&str; 6] = ["id", "created", "updated", "labels", "workgroup", "source"];

/// The metadata of `other` that does not reach the one that stays.
fn left_behind<'a>(
    other: &'a Record,
    kept: &'a Record,
    carried: &'a dyn Fn(&str) -> Option<String>,
) -> impl Iterator<Item = String> + 'a {
    other.meta.fields().filter_map(|(name, value)| {
        let text = other.meta.text(name);
        if COMBINED.contains(&name)
            || kept.meta.value(name) == Some(value)
            || (["url", "google"].contains(&name) && carried(name) == text)
        {
            return None;
        }
        let text = text.or_else(|| serde_json::to_string(value).ok())?;
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        Some(format!("{name}: {text}"))
    })
}

/// Let the party that stays go by `name` too, unless it already does.
fn add_alias(top: &mut Vec<Field>, kept: &Record, name: &str) {
    let mut names = std::iter::once(kept.title()).chain(
        top.iter()
            .filter(|f| same(&f.name, "Aliases"))
            .flat_map(|f| f.values.clone()),
    );
    if names.any(|n| bare(&n) == bare(name)) {
        return;
    }
    match top.iter_mut().find(|f| same(&f.name, "Aliases")) {
        Some(aliases) => {
            aliases.values.push(name.to_owned());
            aliases.value = aliases.values.join(", ");
        }
        None => top.push(Field {
            values: vec![name.to_owned()],
            ..Field::new("Aliases", name)
        }),
    }
}

/// Whether a task of these waits for another of them, however far along
/// the parents and the tasks it is blocked by: one would come to wait for
/// itself.
fn waits_among(held: &Held<'_>, alike: &[Record]) -> anyhow::Result<bool> {
    let ids: Vec<&str> = alike.iter().map(|r| r.id.as_str()).collect();
    for start in alike {
        let mut seen = vec![start.id.clone()];
        let mut next = vec![start.clone()];
        while let Some(task) = next.pop() {
            let waited = ["Parent", "Blocked by"]
                .into_iter()
                .flat_map(|name| task.body.values(name));
            for value in waited {
                let Some((TASKS, id)) = record_of(link_target(&value)) else {
                    continue;
                };
                if ids.contains(&id) && id != start.id {
                    return Ok(true);
                }
                if seen.iter().any(|s| s == id) {
                    continue;
                }
                seen.push(id.to_owned());
                next.extend(held.records().get(TASKS, id)?);
            }
        }
    }
    Ok(false)
}

/// The records to merge with the one that stays first: the first, unless it
/// is a party that may come to rest and another is not. Who added a party is
/// not the secretary's to change, so the one kept from rest stays.
pub(super) fn kept_first(mut alike: Vec<Record>) -> Vec<Record> {
    let may_rest = |r: &Record| {
        r.kind == PARTIES
            && r.body.value(ADDED_BY).as_deref() == Some(SECRETARY)
            && r.body.value("Status").is_none_or(|s| s.is_empty())
    };
    if let Some(at) = alike.iter().position(|r| !may_rest(r)) {
        alike[..=at].rotate_right(1);
    }
    alike
}

/// Put what the others say into the first, which stays, and remove them. All
/// are of one kind, ordered by `kept_first`.
pub(super) fn merge(
    store: &Store,
    held: &Held<'_>,
    alike: &[Record],
    reason: String,
) -> anyhow::Result<Change> {
    let (kept, gone) = alike.split_first().expect("a group is not empty");
    let kind = kept.kind.as_str();
    anyhow::ensure!(
        gone.iter().all(|r| r.kind == kind && r.id != kept.id),
        "only records of one kind are merged, each once"
    );
    anyhow::ensure!(
        kind != TASKS || !waits_among(held, alike)?,
        "one of them waits for another, so they are not one"
    );
    if kind == PARTIES {
        // Checked before the links move to the one that stays.
        let ids: Vec<&str> = alike.iter().map(|r| r.id.as_str()).collect();
        if let Some(taken) = held.records().party_called(kept, gone, &ids)? {
            anyhow::bail!(
                "another party is called a name these go by: {} ({})",
                taken.title(),
                taken.id
            );
        }
    }
    {
        // Tried before the links move, so that a refusal leaves every record
        // and every link as it was. Moving links changes no more than links.
        let (patch, _) = put_together(store, held, alike)?;
        let gone_ids: Vec<&str> = gone.iter().map(|r| r.id.as_str()).collect();
        held.may_change_to(Writer::Secretary, kind, &kept.id, patch, &gone_ids)?;
    }
    // Links follow before what they say is put together: read before, a link
    // one of them has to another would come back pointing at a record gone.
    for other in gone {
        held.relink(Writer::Secretary, kind, &other.id, &kept.id)?;
    }
    let alike = alike
        .iter()
        .map(|r| held.records().need(kind, &r.id))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let (kept, gone) = alike.split_first().expect("a group is not empty");
    let gone_ids: Vec<&str> = gone.iter().map(|r| r.id.as_str()).collect();
    let (patch, many) = put_together(store, held, &alike)?;
    // Changed before any of the others is removed, so that a refusal leaves
    // them as they were.
    let merged = held.change_besides(Writer::Secretary, kind, &kept.id, patch, &gone_ids)?;
    if kind == CONTRACTS {
        for other in gone {
            crate::contracts::put_wrote_together(store, kept, &merged, &other.id)?;
        }
    }
    let from = gone.iter().flat_map(|r| [r.id.clone(), r.rel()]);
    crate::disclosure::carry_record(store, from, &kept.id)?;
    // Moved before the removal, so that stopping midway leaves nothing that a
    // second pass cannot finish.
    for other in gone {
        move_what_is_kept_beside(store, kind, &other.id, &kept.id)?;
        held.remove(Writer::Secretary, kind, &other.id)?;
    }
    let text = match many {
        Some(many) => format!("{}（数量 {many}）", kept.title()),
        None => kept.title(),
    };
    Ok(Change {
        op: Op::Edited,
        id: kept.id.clone(),
        file: kept.rel(),
        text,
        previous: None,
        from_file: None,
        replaces: None,
        reason: Some(reason),
        basis: Vec::new(),
    })
}

/// What the others of `alike` say, put into the first. How many there are of
/// a thing.
fn put_together(
    store: &Store,
    held: &Held<'_>,
    alike: &[Record],
) -> anyhow::Result<(Patch, Option<u32>)> {
    let (kept, gone) = alike.split_first().expect("a group is not empty");
    let kind = kept.kind.as_str();
    let holding = kind == HOLDINGS;
    let many = if holding {
        Some(how_many(store, alike)?)
    } else {
        None
    };
    let def = held.records().kinds()?.into_iter().find(|k| k.name == kind);
    let listed = |name: &str| {
        def.as_ref()
            .and_then(|k| k.field(name))
            .is_some_and(|(_, ty)| matches!(ty, FieldType::List | FieldType::Links(_)))
            || same(name, RELATED)
    };
    let no_list = |_: &str| false;
    let mut top = fields(kept);
    let mut parts: Vec<SectionContent> = sections(kept);
    let mut owners = details(kept);
    let mut labels = kept.labels();
    let mut workgroups = kept.workgroups();
    let mut notes = kept.notes().trim().to_owned();
    let mut log = Vec::new();
    let meta = |name: &str| alike.iter().find_map(|r| r.meta.text(name));
    let party = kind == PARTIES;
    // Who added a party is the owner's to set, and the day last seen is the
    // latest of all: neither is filled in from the others.
    let own = |f: &Field| {
        party
            && (same(&f.name, LAST_SEEN)
                || same(&f.name, ADDED_BY) && kept.body.value(ADDED_BY).is_none())
    };
    // A day the owner wrote ahead stays with the one it was written on: the
    // secretary may not set a day that has not come.
    let come = chrono::Local::now().date_naive() + chrono::Duration::days(1);
    let ahead = |r: &Record| last_seen(r).is_some_and(|on| on > come);
    for other in gone {
        let (theirs, said): (Vec<Field>, Vec<Field>) = fields(other).into_iter().partition(own);
        let mut otherwise: Vec<String> = fill(&mut top, said, &listed)
            .into_iter()
            .chain(
                theirs
                    .into_iter()
                    .filter(|f| same(&f.name, ADDED_BY) || ahead(other)),
            )
            .filter(|f| !(holding && same(&f.name, "Status")))
            .map(|f| format!("{}: {}", f.name, f.value))
            .collect();
        if party {
            add_alias(&mut top, kept, &other.title());
        } else if word(&other.title()) != word(&kept.title()) {
            otherwise.push(format!("名前: {}", other.title()));
        }
        for section in sections(other) {
            match parts.iter_mut().find(|s| same(&s.name, &section.name)) {
                Some(part) => {
                    otherwise.extend(
                        fill(&mut part.fields, section.fields, &no_list)
                            .into_iter()
                            .map(|f| format!("{} の {}: {}", section.name, f.name, f.value)),
                    );
                    for item in section.items {
                        if !part.items.contains(&item) {
                            part.items.push(item);
                        }
                    }
                }
                None => parts.push(section),
            }
        }
        let (counts, said): (Vec<Field>, Vec<Field>) = details(other)
            .into_iter()
            .partition(|d| holding && d.name == QUANTITY);
        otherwise.extend(
            counts
                .into_iter()
                .filter(|d| counted(&d.value).is_none())
                .map(|d| format!("{}: {}", d.name, d.value)),
        );
        otherwise.extend(
            fill(&mut owners, said, &no_list)
                .into_iter()
                .map(|f| format!("{}: {}", f.name, f.value)),
        );
        otherwise.extend(left_behind(other, kept, &meta));
        union(&mut labels, other.labels());
        union(&mut workgroups, other.workgroups());
        let said = other.notes();
        let said = said.trim();
        if !notes.contains(said) {
            notes = format!("{notes}\n\n{said}").trim().to_owned();
        }
        log.extend(
            log_lines(other)
                .into_iter()
                .map(|l| format!("{} の経過：{} {}", other.id, l.at, l.text)),
        );
        log.extend(
            otherwise
                .into_iter()
                .map(|said| format!("{} では {said}", other.id)),
        );
    }
    if let Some(seen) = alike
        .iter()
        .filter(|r| r.id == kept.id || !ahead(r))
        .filter_map(last_seen)
        .max()
        .filter(|_| party)
    {
        top.retain(|f| !same(&f.name, LAST_SEEN));
        top.push(Field::new(LAST_SEEN, seen.to_string()));
    }
    if holding
        && alike
            .iter()
            .any(|r| r.body.value("Status").as_deref() == Some("owned"))
    {
        top.retain(|f| !same(&f.name, "Status"));
        top.push(Field::new("Status", "owned"));
    }
    let gone_ids: Vec<&str> = gone.iter().map(|r| r.id.as_str()).collect();
    log.push(match many {
        Some(many) => format!("{MERGED}（数量 {many}）：{}", gone_ids.join(", ")),
        None => format!("{MERGED}：{}", gone_ids.join(", ")),
    });
    let details = match many {
        Some(many) => with_quantity(owners, many),
        None => owners,
    };
    // Links to the others now point at the one that stays; it does not link
    // to itself.
    let itself = kept.target();
    for field in &mut top {
        if field.values.iter().any(|v| link_target(v) == itself) {
            field.values.retain(|v| link_target(v) != itself);
            field.value = field.values.join(", ");
        }
    }
    let patch = Patch {
        fields: top,
        sections: parts,
        details: (details != crate::records::details(kept)).then_some(details),
        notes: (notes != kept.notes().trim()).then_some(notes),
        labels: Some(labels),
        workgroups: Some(workgroups),
        url: meta("url"),
        google: meta("google"),
        log,
        ..Patch::default()
    };
    let free = def.as_ref().is_some_and(|k| k.layout.free);
    let patch = if free {
        Patch {
            sections: Vec::new(),
            details: None,
            ..patch
        }
    } else {
        patch
    };
    Ok((patch, many))
}

/// What mail and pictures know of a thing goes to the one that stays. Where
/// that one has its own, the other's is dropped: left, a mail read again would
/// take it for a thing the owner removed.
fn move_what_is_kept_beside(store: &Store, kind: &str, from: &str, to: &str) -> anyhow::Result<()> {
    let conn = store.conn();
    if kind == HOLDINGS {
        // Empty bytes say that no picture was found, which gives way to one.
        conn.execute(
            "DELETE FROM holding_pictures WHERE holding = ?2 AND length(bytes) = 0
             AND EXISTS (SELECT 1 FROM holding_pictures WHERE holding = ?1 AND length(bytes) > 0)",
            params![from, to],
        )?;
        for table in [
            "purchases",
            "purchase_settled",
            "holding_pictures",
            "holding_looks",
        ] {
            conn.execute(
                &format!("UPDATE OR IGNORE {table} SET holding = ?2 WHERE holding = ?1"),
                params![from, to],
            )?;
            conn.execute(&format!("DELETE FROM {table} WHERE holding = ?1"), [from])?;
        }
    }
    if kind == CONTRACTS {
        conn.execute(
            "UPDATE OR IGNORE contracts SET contract = ?2 WHERE contract = ?1",
            params![from, to],
        )?;
        conn.execute("DELETE FROM contracts WHERE contract = ?1", [from])?;
        conn.execute(
            "UPDATE contract_events SET contract = ?2 WHERE contract = ?1",
            params![from, to],
        )?;
    }
    if kind == TASKS {
        // A routine waits on the task it made last; left on the one merged
        // away, it would take that as finished and make another.
        conn.execute(
            "UPDATE routine_runs SET task = ?2 WHERE task = ?1",
            params![from, to],
        )?;
    }
    conn.execute(
        "DELETE FROM record_looks WHERE kind = ?1 AND id = ?2",
        params![kind, from],
    )?;
    Ok(())
}
