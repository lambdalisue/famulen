//! Which record a text names, and links brought to the one form they are kept
//! in: `[[records/<kind>/<id>|title]]`. Whoever writes a link may name its
//! record by code, title or id instead.

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::ops::Range;

use super::titled_link;
use super::{Ambiguous, CODE_LENGTH, FieldType, Kind, Record, Records, SectionType, record_of};
use crate::body::{Item, LOG, NOTES, Value, same};
use crate::link::{Found, found, rewrite, with_extension};
use crate::{Entry, link_target, page};

/// A record, as a text can name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub kind: String,
    pub id: String,
    pub title: String,
    /// The word its kind's codes start with.
    word: String,
}

impl Named {
    pub fn link(&self) -> String {
        titled_link(&self.kind, &self.id, &self.title)
    }

    fn has_code(&self, code: &str) -> bool {
        code.rsplit_once('-').is_some_and(|(word, end)| {
            word == self.word && end.len() >= CODE_LENGTH && self.id.ends_with(end)
        })
    }
}

/// What to do with a link that could be several records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unsure {
    Refuse,
    Leave,
}

/// Every record of a notebook by what it is called, read when first asked.
pub struct Names {
    records: Records,
    all: OnceCell<Vec<Named>>,
}

impl Names {
    pub fn new(records: Records) -> Self {
        Self {
            records,
            all: OnceCell::new(),
        }
    }

    fn all(&self) -> anyhow::Result<&[Named]> {
        if let Some(all) = self.all.get() {
            return Ok(all);
        }
        let mut all = Vec::new();
        for kind in self.records.kind_names()? {
            let Ok(declared) = self.records.kind(&kind) else {
                continue;
            };
            all.extend(self.records.list(&kind)?.into_iter().map(|r| Named {
                title: r.title(),
                word: declared.code.clone(),
                kind: r.kind,
                id: r.id,
            }));
        }
        Ok(self.all.get_or_init(|| all))
    }

    /// The record of any kind, or of `kind`, that a path, an id, a code or a
    /// title names. One that fits several is an [`Ambiguous`] error.
    pub fn find(&self, text: impl AsRef<str>, kind: Option<&str>) -> anyhow::Result<Option<Named>> {
        let text = link_target(text.as_ref());
        let path = record_of(text);
        let name = path.map_or(text, |(_, id)| id);
        let lower = name.to_lowercase();
        if lower.is_empty() {
            return Ok(None);
        }
        let of_kind =
            |n: &&Named| kind.is_none_or(|k| k == n.kind) && path.is_none_or(|(k, _)| k == n.kind);
        let pick = |fits: &dyn Fn(&Named) -> bool| -> anyhow::Result<Vec<&Named>> {
            Ok(self
                .all()?
                .iter()
                .filter(of_kind)
                .filter(|n| fits(n))
                .collect())
        };
        let mut hits = pick(&|n| n.id == lower || n.has_code(&lower))?;
        if hits.is_empty() && path.is_none() {
            hits = pick(&|n| n.title.to_lowercase() == lower)?;
        }
        match hits[..] {
            [] => Ok(None),
            [one] => Ok(Some(one.clone())),
            _ => Err(Ambiguous {
                text: text.to_owned(),
                candidates: hits
                    .iter()
                    .map(|n| {
                        let code = self.records.code(&n.kind, &n.id)?;
                        Ok((code, n.title.clone()))
                    })
                    .collect::<anyhow::Result<_>>()?,
            }
            .into()),
        }
    }

    /// Whether a link target is a file or directory of the notebook.
    pub(crate) fn is_there(&self, target: &str) -> bool {
        let root = self.records.notebook().root();
        root.join(format!("{target}.md")).is_file() || root.join(target).exists()
    }

    /// A link in its kept form, when it is not in it and names one record.
    /// Free text may call a record what it likes; elsewhere a link carries
    /// the record's title.
    fn kept(
        &self,
        whole: &str,
        title: Option<&str>,
        free: bool,
        kind: Option<&str>,
        unsure: Unsure,
    ) -> anyhow::Result<Option<String>> {
        let target = link_target(whole);
        let title = title.filter(|t| !t.is_empty());
        if let Some((k, id)) = record_of(target)
            && self.records.path(k, id).is_file()
        {
            // Without a title a link is still whole, so it is only given one
            // as it is written.
            let named = match unsure {
                Unsure::Refuse => title.is_some(),
                Unsure::Leave => true,
            };
            if named && (free || title.is_none()) && !with_extension(whole) {
                return Ok(None);
            }
            let Some(record) = self.records.get(k, id).ok().flatten() else {
                return Ok(None);
            };
            let now = match title {
                Some(title) if free => titled_link(k, id, title),
                _ => record.link(),
            };
            let written = titled_link(k, id, title.unwrap_or_default());
            let same = title.is_some() && written == now && !with_extension(whole);
            return Ok((!same).then_some(now));
        }
        if self.is_there(target) {
            return Ok(None);
        }
        let found = match self.find(target, kind) {
            Err(error) if unsure == Unsure::Leave && error.is::<Ambiguous>() => None,
            found => found?,
        };
        Ok(found.map(|n| match title {
            Some(title) if free => titled_link(&n.kind, &n.id, title),
            _ => n.link(),
        }))
    }

    /// A text of free prose with its links in their kept form. The links of
    /// `before` stay as they are.
    pub fn canonical_text(
        &self,
        text: impl AsRef<str>,
        before: impl AsRef<str>,
    ) -> anyhow::Result<String> {
        self.text(text.as_ref(), before.as_ref(), Unsure::Refuse, |_| true)
    }

    /// The text of a memory file with its links in their kept form. The links
    /// of `before` stay as they are, as does one that would break the YAML it
    /// stands in.
    pub fn canonical_memory(
        &self,
        text: impl AsRef<str>,
        before: impl AsRef<str>,
    ) -> anyhow::Result<String> {
        self.memory(text.as_ref(), before.as_ref(), Unsure::Refuse)
    }

    pub(crate) fn memory(
        &self,
        text: &str,
        before: &str,
        unsure: Unsure,
    ) -> anyhow::Result<String> {
        let yaml = yaml_of(text);
        let mut before = wholes(before);
        rewrite(text, |link: &Found| {
            if stood(&mut before, link.whole) {
                return Ok(None);
            }
            let new = self.kept(link.whole, link.title, true, None, unsure)?;
            let in_yaml = yaml.iter().any(|y| y.contains(&link.at));
            Ok(new.filter(|new| !in_yaml || !new.contains(['"', '\'', '\\', ':', '#'])))
        })
    }

    fn text(
        &self,
        text: &str,
        before: &str,
        unsure: Unsure,
        is_free: impl Fn(usize) -> bool,
    ) -> anyhow::Result<String> {
        let mut before = wholes(before);
        rewrite(text, |link: &Found| {
            if stood(&mut before, link.whole) {
                return Ok(None);
            }
            self.kept(link.whole, link.title, is_free(link.at), None, unsure)
        })
    }

    /// Bring the links of a record to their kept form. The links of `before`,
    /// the record as it was, stay as they are.
    pub(crate) fn record(
        &self,
        kind: &Kind,
        record: &Record,
        before: &str,
        unsure: Unsure,
    ) -> anyhow::Result<Record> {
        let mut record = record.clone();
        self.typed(kind, &mut record, before, unsure)?;
        let body = &record.body;
        let free: Vec<_> = if kind.layout.free {
            let start = body.fields().last().map_or(0, |f| f.span.end);
            std::iter::once(start..body.text().len()).collect()
        } else {
            body.sections()
                .iter()
                .filter(|s| same(&s.name, NOTES) || same(&s.name, LOG))
                .map(|s| s.content.clone())
                .chain([body.preamble()])
                .collect()
        };
        // A title is a name, not a place for links.
        let (title, rest) = body.text().split_at(body.title_end());
        let rest = self.text(rest, before, unsure, |at| {
            free.iter().any(|f| f.contains(&(at + title.len())))
        })?;
        let text = format!("{title}{rest}");
        if text != record.body.text() {
            record.body = crate::body::Body::parse(text, &kind.layout)?;
        }
        Ok(record)
    }

    /// Fields declared as links, whose values may be a bare code, title or id.
    fn typed(
        &self,
        kind: &Kind,
        record: &mut Record,
        before: &str,
        unsure: Unsure,
    ) -> anyhow::Result<()> {
        let old = Record::parse(kind, &record.id, before).ok().map(|r| r.body);
        let old_items = |name: &str| old.iter().flat_map(|b| b.items(name)).collect::<Vec<_>>();
        let value = |item: &Item, ty: &FieldType, was: Vec<String>| {
            self.typed_value(item, ty, &was, unsure)
        };
        for (name, ty) in &kind.fields {
            let was = old.iter().filter_map(|b| b.field(name)).flat_map(values_of);
            if let Some(new) = record
                .body
                .field(name)
                .map(|i| value(i, ty, was.collect()))
                .transpose()?
                && let Some(new) = new
            {
                record.body.set(name, new)?;
            }
        }
        for (name, ty) in &kind.sections {
            match ty {
                SectionType::Fields(fields) => {
                    for (field, ty) in fields {
                        let was = old_items(name)
                            .into_iter()
                            .filter(|i| i.is_field(field))
                            .flat_map(values_of);
                        let new = record
                            .body
                            .items(name)
                            .into_iter()
                            .find(|i| i.is_field(field))
                            .map(|i| value(i, ty, was.collect()))
                            .transpose()?
                            .flatten();
                        if let Some(new) = new {
                            record.body.set_in(name, field, new)?;
                        }
                    }
                }
                SectionType::Items(fields) => {
                    self.typed_items(name, fields, record, &old_items(name), unsure)?;
                }
            }
        }
        Ok(())
    }

    fn typed_items(
        &self,
        section: &str,
        fields: &[(String, FieldType)],
        record: &mut Record,
        old: &[&Item],
        unsure: Unsure,
    ) -> anyhow::Result<()> {
        let Some(((_, first), nested)) = fields.split_first() else {
            return Ok(());
        };
        let items: Vec<Item> = record.body.items(section).into_iter().cloned().collect();
        for (at, item) in items.iter().enumerate() {
            for (field, ty) in nested {
                let was: Vec<_> = old
                    .iter()
                    .filter_map(|i| i.nested(field))
                    .flat_map(values_of)
                    .collect();
                if let Some(new) = item
                    .nested(field)
                    .map(|i| self.typed_value(i, ty, &was, unsure))
                    .transpose()?
                    .flatten()
                {
                    record.body.set_item_field(section, at, field, new)?;
                }
            }
            let FieldType::Link(of) = first else {
                continue;
            };
            let was: Vec<_> = old.iter().map(|i| i.plain()).collect();
            let Some(new) = self.named_value(&item.plain(), of.as_deref(), &was, unsure)? else {
                continue;
            };
            record.body.set_item_text(section, at, &new)?;
        }
        Ok(())
    }

    fn typed_value(
        &self,
        item: &Item,
        ty: &FieldType,
        was: &[String],
        unsure: Unsure,
    ) -> anyhow::Result<Option<Value>> {
        match ty {
            FieldType::Link(of) => Ok(self
                .named_value(&item.value(), of.as_deref(), was, unsure)?
                .map(Value::One)),
            FieldType::Links(of) => {
                let mut changed = false;
                let values = item
                    .values()
                    .into_iter()
                    .map(|v| {
                        let new = self.named_value(&v, of.as_deref(), was, unsure)?;
                        changed |= new.is_some();
                        Ok(new.unwrap_or(v))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                Ok(changed.then_some(Value::Many(values)))
            }
            _ => Ok(None),
        }
    }

    fn named_value(
        &self,
        text: &str,
        kind: Option<&str>,
        was: &[String],
        unsure: Unsure,
    ) -> anyhow::Result<Option<String>> {
        let text = text.trim();
        if text.is_empty() || text.starts_with("[[") && was.iter().any(|w| w == text) {
            return Ok(None);
        }
        let title = found(text).first().and_then(|l| l.title);
        self.kept(text, title, false, kind, unsure)
    }
}

/// How often a text holds each link.
fn wholes(text: &str) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for link in found(text) {
        *counts.entry(link.whole).or_default() += 1;
    }
    counts
}

/// Whether a link is one of those that stood `before`, each taken once.
fn stood(before: &mut BTreeMap<&str, usize>, whole: &str) -> bool {
    match before.get_mut(whole) {
        Some(left) if *left > 0 => {
            *left -= 1;
            true
        }
        _ => false,
    }
}

/// A field's value as it is written, and as the list it may be.
fn values_of(item: &Item) -> Vec<String> {
    let mut values = item.values();
    values.push(item.value().trim().to_owned());
    values
}

/// Where a memory file holds YAML: its frontmatter, and the attributes of its
/// items.
fn yaml_of(text: &str) -> Vec<Range<usize>> {
    let mut yaml = Vec::new();
    let mut at = 0;
    if let Some((_, body)) = page::split(text) {
        at = text.len() - body.len();
        yaml.push(0..at);
    }
    for line in text[at..].split_inclusive('\n') {
        if let Some(entry) = Entry::parse(line.trim_end())
            && !entry.attrs().is_empty()
        {
            let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
            yaml.push(at + indent + "- ".len() + entry.text().len()..at + line.len());
        }
        at += line.len();
    }
    yaml
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{NewItem, RELATED};
    use crate::check::{Severity, check, fix};
    use crate::testing::{me, notebook, record, write};

    const A: &str = "01kat3v8q4zj6m2x9r5c7w1hbn";
    const B: &str = "01kat3v8q4zj6m2x9r5c7w1hbz";
    const C: &str = "01kat3v8q4zj6m2x9r5c7w1hcz";

    #[test]
    fn a_link_by_code_title_or_id_is_written_with_the_id_and_the_title_of_now() {
        let (_tmp, nb) = notebook("names-write");
        let r = Records::new(nb.clone());
        record(&nb, "tasks", A, "# 見積もり\n");
        record(&nb, "parties", B, "# 田中\n");
        let mut memo = r.draft("memos", "メモ").unwrap();
        memo.body
            .set_notes(&format!(
                "[[TASK-7w1hbn]] [[見積もり]] [[{B}|たなっち]] [[records/parties/{B}]] [[memory/core]] `[[見積もり]]`"
            ))
            .unwrap();

        r.write(&me(), &memo, "足した").unwrap();

        let task = format!("[[records/tasks/{A}|見積もり]]");
        assert_eq!(
            r.get("memos", &memo.id).unwrap().unwrap().notes(),
            format!(
                "{task} {task} [[records/parties/{B}|たなっち]] [[records/parties/{B}|田中]] [[memory/core]] `[[見積もり]]`"
            )
        );
        assert_eq!(check(&nb).unwrap(), []);
    }

    #[test]
    fn every_kind_takes_related_links_to_any_kind_by_bare_names_too() {
        let (tmp, nb) = notebook("names-related");
        std::fs::create_dir_all(tmp.path().join("records/machines")).unwrap();
        let r = Records::new(nb.clone());
        record(&nb, "tasks", A, "# 見積もり, 二つ目\n");
        record(&nb, "parties", B, "# 田中\n");
        let wanted = [
            format!("[[records/tasks/{A}|見積もり, 二つ目]]"),
            format!("[[records/parties/{B}|田中]]"),
        ];
        for kind in r.kind_names().unwrap() {
            let mut draft = r.draft(&kind, "題名").unwrap();
            draft
                .body
                .set(RELATED, vec!["task-7w1hbn".to_owned(), "田中".to_owned()])
                .unwrap();
            r.write(&me(), &draft, "足した").unwrap();

            let read = r.get(&kind, &draft.id).unwrap().unwrap();
            assert_eq!(read.body.values(RELATED), wanted, "{kind}");
        }
        assert_eq!(check(&nb).unwrap(), []);
        let mut bad = r.draft("memos", "x").unwrap();
        bad.body.set(RELATED, "ない記録").unwrap();
        assert!(r.write(&me(), &bad, "足した").is_err());
    }

    #[test]
    fn a_field_for_one_kind_looks_only_there_and_items_are_named_too() {
        let (_tmp, nb) = notebook("names-kind");
        let r = Records::new(nb.clone());
        record(&nb, "tasks", A, "# 山田\n");
        record(&nb, "parties", B, "# 山田\n");
        let mut person = r.draft("parties", "花子").unwrap();
        person
            .body
            .add_item(
                "Member of",
                &NewItem::new("山田").with("Role", "部長, 兼務"),
            )
            .unwrap();
        r.write(&me(), &person, "足した").unwrap();
        let mut task = r.draft("tasks", "x").unwrap();
        task.body.set("Parent", "[[山田]]").unwrap();
        r.write(&me(), &task, "足した").unwrap();

        let person = r.get("parties", &person.id).unwrap().unwrap();
        let item = person.body.items("Member of")[0];
        assert_eq!(item.plain(), format!("[[records/parties/{B}|山田]]"));
        assert_eq!(item.nested("Role").unwrap().value(), "部長, 兼務");
        let task = r.get("tasks", &task.id).unwrap().unwrap();
        assert_eq!(
            task.body.value("Parent"),
            Some(format!("[[records/tasks/{A}|山田]]"))
        );
    }

    #[test]
    fn a_link_in_a_field_of_an_item_may_be_bare_too() {
        let (_tmp, nb) = notebook("names-items");
        write(
            nb.root(),
            "records/logs/README.md",
            "---\ncode: log\nsections:\n  Entries:\n    - Name: text\n      Task: link:tasks\n      Others: links\n---\n\n# logs\n",
        );
        let r = Records::new(nb.clone());
        record(&nb, "tasks", A, "# 見積もり, 二つ目\n");
        let mut log = r.draft("logs", "x").unwrap();
        log.body
            .add_item(
                "Entries",
                &NewItem::new("一つ")
                    .with("Task", "task-7w1hbn")
                    .with("Others", vec!["task-7w1hbn".to_owned()]),
            )
            .unwrap();

        r.write(&me(), &log, "足した").unwrap();

        let log = r.get("logs", &log.id).unwrap().unwrap();
        let item = log.body.items("Entries")[0];
        let link = format!("[[records/tasks/{A}|見積もり, 二つ目]]");
        assert_eq!(item.text, "一つ");
        assert_eq!(item.nested("Task").unwrap().value(), link);
        assert_eq!(item.nested("Others").unwrap().values(), [link]);
    }

    #[test]
    fn a_link_with_the_extension_of_its_file_is_kept_without_it() {
        let (_tmp, nb) = notebook("names-extension");
        let r = Records::new(nb.clone());
        record(&nb, "tasks", A, "# 見積もり\n");
        let mut memo = r.draft("memos", "x").unwrap();
        memo.body
            .set(RELATED, format!("[[records/tasks/{A}.md]]"))
            .unwrap();
        memo.body
            .set_notes(&format!("[[records/tasks/{A}.md|あれ]]"))
            .unwrap();

        r.write(&me(), &memo, "足した").unwrap();

        let memo = r.get("memos", &memo.id).unwrap().unwrap();
        assert_eq!(
            memo.body.values(RELATED),
            [format!("[[records/tasks/{A}|見積もり]]")]
        );
        assert_eq!(memo.notes(), format!("[[records/tasks/{A}|あれ]]"));

        let rel = record(
            &nb,
            "memos",
            C,
            &format!("# y\n\n[[records/tasks/{A}.md]] [[records/tasks/README.md]]\n"),
        );
        assert_eq!(check(&nb).unwrap()[0].rule, "link-form");
        fix(&nb, &[]).unwrap();
        assert!(
            std::fs::read_to_string(nb.root().join(rel))
                .unwrap()
                .contains(&format!("[[records/tasks/{A}|見積もり]]"))
        );
        assert_eq!(check(&nb).unwrap(), []);
    }

    #[test]
    fn what_else_an_item_holds_stays_when_its_link_is_brought_right() {
        let (_tmp, nb) = notebook("names-rest");
        record(&nb, "parties", A, "# 山田\n");
        let rel = record(
            &nb,
            "parties",
            B,
            &format!(
                "# 花子\n\n## Member of\n\n- [[records/parties/{A}|古い]]\n  - Role: 部長\n\n  入った経緯。\n"
            ),
        );

        fix(&nb, &[]).unwrap();

        let text = std::fs::read_to_string(nb.root().join(rel)).unwrap();
        assert!(
            text.ends_with(&format!(
                "- [[records/parties/{A}|山田]]\n  - Role: 部長\n\n  入った経緯。\n"
            )),
            "{text}"
        );
    }

    #[test]
    fn a_name_that_fits_several_is_refused_with_them_unless_it_stood_before() {
        let (_tmp, nb) = notebook("names-several");
        let r = Records::new(nb.clone());
        record(&nb, "tasks", A, "# 同じ\n");
        record(&nb, "memos", B, "# 同じ\n");
        let mut memo = r.draft("memos", "x").unwrap();
        memo.body.set_notes("[[同じ]]").unwrap();

        let error = r.write(&me(), &memo, "足した").unwrap_err();
        let ambiguous = error.downcast_ref::<Ambiguous>().unwrap();
        assert_eq!(
            ambiguous.candidates,
            [
                ("memo-7w1hbz".to_owned(), "同じ".to_owned()),
                ("task-7w1hbn".to_owned(), "同じ".to_owned())
            ]
        );

        record(&nb, "memos", C, "# y\n\n[[同じ]]\n");
        let mut old = r.get("memos", C).unwrap().unwrap();
        old.body.set_notes("[[同じ]] と続き").unwrap();
        r.write(&me(), &old, "直した").unwrap();
    }

    #[test]
    fn a_name_that_fits_several_is_refused_where_it_is_added_though_it_stands_elsewhere() {
        let (_tmp, nb) = notebook("names-added");
        let r = Records::new(nb.clone());
        record(&nb, "tasks", A, "# 同じ\n");
        record(&nb, "memos", B, "# 同じ\n");
        record(&nb, "memos", C, "# y\n\n[[同じ]]\n");
        let old = r.get("memos", C).unwrap().unwrap();

        let mut linked = old.clone();
        linked.body.set(RELATED, "[[同じ]]").unwrap();
        assert!(r.write(&me(), &linked, "足した").is_err());
        let mut twice = old;
        twice.body.set_notes("[[同じ]] [[同じ]]").unwrap();
        assert!(r.write(&me(), &twice, "足した").is_err());
    }

    #[test]
    fn a_path_names_a_record_of_its_own_kind_only() {
        let (_tmp, nb) = notebook("names-path");
        record(&nb, "tasks", A, "# 見積もり\n");
        let names = Names::new(Records::new(nb));

        assert!(
            names
                .find(format!("[[records/parties/{A}]]"), None)
                .unwrap()
                .is_none()
        );
        assert!(
            names
                .find("records/parties/task-7w1hbn", None)
                .unwrap()
                .is_none()
        );
        assert!(
            names
                .find("records/tasks/task-7w1hbn", None)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn a_memory_keeps_a_link_whose_title_its_yaml_could_not_hold() {
        let (_tmp, nb) = notebook("names-yaml");
        record(&nb, "tasks", A, "# 「\"急\"」: 見積もり\n");
        record(&nb, "tasks", B, "# 下調べ\n");
        let names = Names::new(Records::new(nb));
        let (odd, plain) = (
            format!("[[records/tasks/{A}|「\"急\"」: 見積もり]]"),
            format!("[[records/tasks/{B}|下調べ]]"),
        );

        let new = names
            .canonical_memory(
                "---\nsummary: \"[[task-7w1hbn]] [[task-7w1hbz]]\"\n---\n\n- [[task-7w1hbn]] {id: m1, reason: \"[[task-7w1hbn]] [[task-7w1hbz]]\"}\n",
                "",
            )
            .unwrap();

        assert_eq!(
            new,
            format!(
                "---\nsummary: \"[[task-7w1hbn]] {plain}\"\n---\n\n- {odd} {{id: m1, reason: \"[[task-7w1hbn]] {plain}\"}}\n"
            )
        );
    }

    #[test]
    fn checking_tells_links_to_bring_right_and_fixing_brings_those_that_name_one_record() {
        let (_tmp, nb) = notebook("names-fix");
        record(&nb, "tasks", A, "# 見積もり\n");
        record(&nb, "parties", B, "# 同じ\n");
        record(
            &nb,
            "memos",
            C,
            "# 同じ\n\n[[task-7w1hbn|あれ]] [[同じ]] [[ない]]\n",
        );
        write(
            nb.root(),
            "memory/long/semantic/x.md",
            "---\nid: m1\nsummary: x\n---\n\n[[見積もり]]\n",
        );
        let found = || -> Vec<_> {
            check(&nb)
                .unwrap()
                .into_iter()
                .map(|f| (f.rule, f.severity))
                .collect()
        };
        assert_eq!(
            found(),
            [
                ("link-form", Severity::Warning),
                ("broken-link", Severity::Error),
                ("broken-link", Severity::Error),
                ("link-form", Severity::Warning)
            ]
        );

        assert_eq!(fix(&nb, &[]).unwrap().len(), 2);

        let read = |rel: &str| std::fs::read_to_string(nb.root().join(rel)).unwrap();
        assert!(
            read(&format!("records/memos/{C}.md"))
                .contains(&format!("[[records/tasks/{A}|あれ]] [[同じ]] [[ない]]"))
        );
        assert!(
            read("memory/long/semantic/x.md").contains(&format!("[[records/tasks/{A}|見積もり]]"))
        );
        assert_eq!(found(), [("broken-link", Severity::Error); 2]);
        assert_eq!(fix(&nb, &[]).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn links_to_a_record_are_found_in_prose_fields_sections_and_memories() {
        let (_tmp, nb) = notebook("names-backlinks");
        let r = Records::new(nb.clone());
        let to = format!("[[records/tasks/{A}|先]]");
        record(&nb, "tasks", A, &format!("# 先\n\n## Notes\n\n{to}\n"));
        record(
            &nb,
            "memos",
            B,
            &format!("# メモ\n\n- Related: {to}\n\n本文 {to}\n"),
        );
        record(
            &nb,
            "parties",
            C,
            &format!(
                "# 田中\n\n## Details\n\n- 頼み: {to}\n\n## Log\n\n- 2026-11-24 10:00 owner: {to}\n"
            ),
        );
        write(
            nb.root(),
            "memory/core.md",
            &format!("# 心得\n\n- {to} を先に {{id: m1}}\n"),
        );
        write(
            nb.root(),
            "memory/forgotten/2026-11.md",
            &format!("# 2026-11\n\n- {to} {{id: m2}}\n"),
        );

        let found = crate::index::Index::open(&nb)
            .unwrap()
            .backlinks("tasks", A)
            .unwrap();

        let from: Vec<_> = found
            .records
            .iter()
            .map(|b| (b.kind.as_str(), b.from.as_str()))
            .collect();
        assert_eq!(
            from,
            [
                ("memos", "Related"),
                ("memos", ""),
                ("parties", "Details"),
                ("parties", "Log")
            ]
        );
        assert_eq!(r.backlinks("tasks", A).unwrap(), found.records);
        assert_eq!(found.memories.len(), 1);
        assert_eq!(found.memories[0].path, "memory/core.md");
        write(
            nb.root(),
            &format!("records/memos/{B}.md"),
            "---\n---\n\n# メモ\n",
        );
        assert_eq!(r.backlinks("tasks", A).unwrap().len(), 2);
    }
}
