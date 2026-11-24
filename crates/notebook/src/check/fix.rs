//! Putting right what there is one way to put right.

use std::ops::Range;

use super::place::{Place, all_files, place_of};
use super::{Known, has_t};
use crate::Notebook;
use crate::body::{Body, Item, LOG, NOTES, Section, same};
use crate::link::rewrite as rewrite_links;
use crate::records::{
    FieldType, Kind, Record, SectionType, Unsure, minute, record_of, titled_link,
};

/// Put right what there is one way to put right in these files, or in every
/// file when none is named. Returns the files changed, without committing
/// them.
pub fn fix(notebook: &Notebook, files: &[String]) -> anyhow::Result<Vec<String>> {
    let files = if files.is_empty() {
        all_files(notebook)?
    } else {
        files.to_vec()
    };
    let known = Known::read(notebook)?;
    let mut changed = Vec::new();
    for path in files {
        let full = notebook.root().join(&path);
        let Ok(text) = std::fs::read_to_string(&full) else {
            continue;
        };
        let new = match place_of(&path) {
            Place::Record { kind, id } => {
                let Some(Ok(declared)) = known.kinds.get(&kind) else {
                    continue;
                };
                let Ok(record) = Record::parse(declared, &id, &text) else {
                    continue;
                };
                let mut new = known.names.record(declared, &record, "", Unsure::Leave)?;
                if !declared.layout.free && new.body.title().is_some() {
                    let body = fixed(&known, declared, &new)?;
                    new.body = Body::parse(body, &declared.layout)?;
                }
                if new == record {
                    continue;
                }
                new.render()
            }
            Place::Memory(_) => known.names.memory(&text, "", Unsure::Leave)?,
            _ => continue,
        };
        if new != text {
            std::fs::write(&full, new)?;
            changed.push(path);
        }
    }
    Ok(changed)
}

/// The body of a record put right: its fields and sections in their order,
/// each written as it is declared.
fn fixed(known: &Known, kind: &Kind, record: &Record) -> anyhow::Result<String> {
    let body = &record.body;
    let text = body.text();
    let rank = |s: &Section| kind.layout.rank(&s.name);
    let mut sections: Vec<(&Section, String)> = body
        .sections()
        .iter()
        .map(|s| (s, fixed_section(known, kind, text, s)))
        .collect();
    let head = fixed_head(known, kind, body);
    if sections.is_sorted_by_key(|(s, _)| rank(s)) {
        // Nothing moves, so the blank lines between stay as they are.
        let mut out = head;
        for (section, fixed) in &sections {
            out.push_str(fixed);
            let raw = &text[section.content.clone()];
            out.push_str(&raw[raw.trim_end().len()..]);
        }
        return Ok(out);
    }
    sections.sort_by_key(|(s, _)| rank(s));
    let mut out = format!("{}\n", head.trim_end());
    for (_, fixed) in sections {
        out.push_str(&format!("\n{fixed}\n"));
    }
    Ok(out)
}

/// What stands before the first section.
fn fixed_head(known: &Known, kind: &Kind, body: &Body) -> String {
    let text = body.text();
    let end = body
        .sections()
        .first()
        .map_or(text.len(), |s| s.heading.start);
    let mut head = text[..end].to_owned();
    if let [first, ..] = body.fields()
        && let [.., last] = body.fields()
    {
        let mut items: Vec<&Item> = body.fields().iter().collect();
        items.sort_by_key(|i| {
            i.field()
                .and_then(|(n, _)| kind.fields.iter().position(|(d, _)| same(d, n)))
                .unwrap_or(usize::MAX)
        });
        let lines: String = items
            .iter()
            .map(|i| fixed_field(known, text, i, &kind.fields))
            .collect();
        head.replace_range(first.span.start..last.span.end, &lines);
    }
    head
}

/// A section, without the blank lines after it.
fn fixed_section(known: &Known, kind: &Kind, text: &str, section: &Section) -> String {
    let proper = kind.layout.proper(&section.name);
    let span = section.content.clone();
    let field =
        |item: &Item, declared: &[(String, FieldType)]| fixed_field(known, text, item, declared);
    let content = if proper == NOTES || proper == LOG {
        text[span].to_owned()
    } else {
        rewrite(
            text,
            span,
            &section.items,
            str::to_owned,
            |item| match kind.section(proper) {
                Some((_, SectionType::Items(declared))) => rewrite(
                    text,
                    item.span.clone(),
                    &item.items,
                    |between| fixed_titles(known, between),
                    |nested| field(nested, declared.get(1..).unwrap_or_default()),
                ),
                Some((_, SectionType::Fields(declared))) => field(item, declared),
                None => field(item, &[]),
            },
        )
    };
    format!("## {proper}\n{content}").trim_end().to_owned()
}

/// Rewrite each item in a span, and what stands between them.
fn rewrite(
    text: &str,
    span: Range<usize>,
    items: &[Item],
    between: impl Fn(&str) -> String,
    each: impl Fn(&Item) -> String,
) -> String {
    let mut out = String::new();
    let mut at = span.start;
    for item in items {
        out.push_str(&between(&text[at..item.span.start]));
        out.push_str(&each(item));
        at = item.span.end;
    }
    out.push_str(&between(&text[at..span.end]));
    out
}

fn fixed_field(known: &Known, text: &str, item: &Item, declared: &[(String, FieldType)]) -> String {
    let mut raw = fixed_titles(known, &text[item.span.clone()]);
    let Some((name, _)) = item.field() else {
        return raw;
    };
    let Some((proper, ty)) = declared.iter().find(|(n, _)| same(n, name)) else {
        return raw;
    };
    if proper != name {
        raw = raw.replacen(name, proper, 1);
    }
    let value = item.value();
    if *ty == FieldType::DateOrMinute && has_t(&value) && minute(&value).is_some() {
        raw = raw.replacen(&value, &value.replacen('T', " ", 1), 1);
    }
    if !raw.ends_with('\n') {
        raw.push('\n');
    }
    raw
}

fn fixed_titles(known: &Known, text: &str) -> String {
    rewrite_links(text, |link| {
        Ok(record_of(link.target)
            .filter(|_| link.title.is_some())
            .and_then(|(kind, id)| Some(titled_link(kind, id, &known.title(kind, id)?))))
    })
    .unwrap_or_else(|_| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{notebook, record};

    const A: &str = "01kat3v8q4zj6m2x9r5c7w1hbn";
    const B: &str = "01kat3v8q4zj6m2x9r5c7w1hbz";

    #[test]
    fn fixing_puts_right_only_what_has_one_way_and_a_second_time_changes_nothing() {
        let (_tmp, nb) = notebook("check-fix");
        record(&nb, "tasks", A, "# 先\n");
        let notes = "## Notes\n\n```\n- due: 2026-11-28T17:00\n```\n\n[[records/tasks/X|古い]]\n";
        let rel = record(
            &nb,
            "tasks",
            B,
            &format!(
                "# 後\n\n- due: 2026-11-28T17:00\n- Mood: よい\n- Blocked by: [[records/tasks/{A}|古い]]\n- status: someday\n\n## log\n\n- x\n\n{notes}"
            ),
        );

        assert_eq!(
            fix(&nb, std::slice::from_ref(&rel)).unwrap(),
            std::slice::from_ref(&rel)
        );

        assert_eq!(
            std::fs::read_to_string(nb.root().join(&rel)).unwrap(),
            format!(
                "---\nid: {B}\n---\n\n# 後\n\n- Status: someday\n- Due: 2026-11-28 17:00\n- Blocked by: [[records/tasks/{A}|先]]\n- Mood: よい\n\n{notes}\n## Log\n\n- x\n"
            )
        );
        assert_eq!(fix(&nb, &[]).unwrap(), Vec::<String>::new());
    }
}
