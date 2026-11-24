//! Telling where a notebook strays from its rules, so whoever keeps it can
//! put it right. Checking changes nothing; fixing changes only what there is
//! one way to put right.

mod fix;
mod place;
mod record;
mod refuse;
mod relations;
mod secrets;

pub use fix::fix;
pub(crate) use place::markdown_files;
pub(crate) use refuse::refuse_new_errors;

use std::collections::BTreeMap;

use crate::link::{found, with_extension};
use crate::memory::{self, Layer};
use crate::records::{Ambiguous, Kind, Names, Records};
use crate::{MEMORY, Notebook, Page, README, RECORDS, SHAPE, slash, workgroups};
use place::{Place, all_files, place_of};
use relations::ByKey;

/// The most items core memory may hold.
pub const CORE_LIMIT: usize = 150;

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The tools cannot read it right, or it must not be there.
    Error,
    /// It reads, but strays from the rules.
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// One thing found wrong, and where.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    /// Relative to the notebook.
    pub path: String,
    /// From one.
    pub line: usize,
    pub rule: &'static str,
    pub severity: Severity,
    pub why: String,
}

/// What is found in one file.
struct Findings<'a> {
    path: &'a str,
    all: Vec<Finding>,
}

impl<'a> Findings<'a> {
    fn of(path: &'a str) -> Self {
        Self {
            path,
            all: Vec::new(),
        }
    }

    fn error(&mut self, line: usize, rule: &'static str, why: impl Into<String>) {
        self.add(line, rule, Severity::Error, why.into());
    }

    fn warning(&mut self, line: usize, rule: &'static str, why: impl Into<String>) {
        self.add(line, rule, Severity::Warning, why.into());
    }

    fn add(&mut self, line: usize, rule: &'static str, severity: Severity, why: String) {
        self.all.push(Finding {
            path: self.path.to_owned(),
            line,
            rule,
            severity,
            why,
        });
    }
}

/// Every way the notebook strays from its rules, by file.
pub fn check(notebook: &Notebook) -> anyhow::Result<Vec<Finding>> {
    let files = all_files(notebook)?;
    check_files(notebook, &files)
}

/// How these files, relative to the notebook, stray from its rules. What
/// they link to and what ids are taken is seen in the whole notebook.
pub fn check_files(notebook: &Notebook, files: &[String]) -> anyhow::Result<Vec<Finding>> {
    let known = Known::read(notebook)?;
    let mut all = Vec::new();
    for path in files {
        let mut findings = Findings::of(path);
        check_file(notebook, &known, &mut findings);
        all.append(&mut findings.all);
    }
    all.sort();
    all.dedup();
    Ok(all)
}

/// What a file is checked against: everything else in the notebook.
struct Known {
    kinds: BTreeMap<String, Result<Kind, String>>,
    /// Every record that can be read.
    records: ByKey,
    workgroups: Vec<String>,
    /// The files that hold each memory's or record's id.
    ids: BTreeMap<String, Vec<String>>,
    names: Names,
}

impl Known {
    fn read(notebook: &Notebook) -> anyhow::Result<Self> {
        let mut known = Self {
            kinds: BTreeMap::new(),
            records: ByKey::new(),
            workgroups: workgroups::list(notebook)
                .unwrap_or_default()
                .into_iter()
                .map(|w| w.id)
                .collect(),
            ids: BTreeMap::new(),
            names: Names::new(Records::new(notebook.clone())),
        };
        let records = Records::new(notebook.clone());
        for name in records.kind_names()? {
            let kind = records.kind(&name).map_err(|error| format!("{error:#}"));
            if kind.is_ok() {
                for record in records.list(&name)? {
                    let id = record.meta.text("id").unwrap_or_else(|| record.id.clone());
                    known.ids.entry(id).or_default().push(record.rel());
                    let key = (name.clone(), record.id.clone());
                    known.records.insert(key, record);
                }
            }
            known.kinds.insert(name, kind);
        }
        let dir = notebook.memory_dir();
        for rel in memory::entry_files(&dir)? {
            let Ok(doc) = memory::read_doc(&dir, &rel) else {
                continue;
            };
            let path = format!("{MEMORY}/{}", slash(&rel));
            for id in doc.entries().filter_map(|e| e.id()) {
                known
                    .ids
                    .entry(id.to_owned())
                    .or_default()
                    .push(path.clone());
            }
        }
        Ok(known)
    }

    /// The title of a record.
    fn title(&self, kind: &str, id: &str) -> Option<String> {
        self.records
            .get(&(kind.to_owned(), id.to_owned()))
            .map(|r| r.title())
    }

    fn id_twice(&self, id: &str, findings: &mut Findings) {
        let holders = self.ids.get(id).map(Vec::as_slice).unwrap_or_default();
        if holders.len() < 2 {
            return;
        }
        let others: Vec<&str> = holders
            .iter()
            .map(String::as_str)
            .filter(|h| *h != findings.path)
            .collect();
        let why = if others.is_empty() {
            format!("the id {id} is here twice")
        } else {
            format!("the id {id} is also in {}", others.join(", "))
        };
        findings.error(1, "id-twice", why);
    }

    fn unknown_workgroup(&self, id: &str, findings: &mut Findings) {
        if !self.workgroups.is_empty() && !self.workgroups.iter().any(|w| w == id) {
            findings.warning(1, "workgroup", format!("no workgroup {id}"));
        }
    }
}

fn check_file(notebook: &Notebook, known: &Known, findings: &mut Findings) {
    let path = findings.path;
    let place = place_of(path);
    if place == Place::Stray {
        findings.warning(1, "stray", "not in a place the notebook's shape has");
    }
    if !path.ends_with(".md") {
        return;
    }
    let text = match std::fs::read_to_string(notebook.root().join(path)) {
        Ok(text) => text,
        Err(error) => {
            findings.error(1, "unreadable", error.to_string());
            return;
        }
    };
    text_findings(&known.names, &text, findings);
    match place {
        Place::Readme => check_readme(known, &text, findings),
        Place::Record { kind, id } => {
            if let Some(Ok(declared)) = known.kinds.get(&kind) {
                record::check(declared, &id, &text, findings);
                if let Some(read) = known.records.get(&(kind, id)) {
                    relations::check(known, read, &text, findings);
                }
            }
        }
        Place::Memory(layer) => check_memory(notebook, known, layer, findings),
        Place::Workgroups => {
            if let Err(error) = workgroups::read(&text) {
                findings.error(1, "workgroups", format!("{error:#}"));
            }
        }
        Place::Other | Place::Stray => {}
    }
}

fn check_readme(known: &Known, text: &str, findings: &mut Findings) {
    let page = match Page::parse(text) {
        Ok(page) => page,
        Err(error) => {
            findings.error(1, "unreadable", format!("{error:#}"));
            return;
        }
    };
    match findings.path.split('/').collect::<Vec<_>>()[..] {
        [README] if page.text("notebook") != Some(SHAPE.to_string()) => {
            let why = format!("the README's frontmatter should say notebook: {SHAPE}");
            findings.error(1, "shape", why);
        }
        [RECORDS, kind, README] => {
            if let Some(Err(why)) = known.kinds.get(kind) {
                findings.error(1, "kind", why.as_str());
            }
        }
        _ => {}
    }
}

fn check_memory(notebook: &Notebook, known: &Known, layer: Layer, findings: &mut Findings) {
    let rel = &findings.path[MEMORY.len() + 1..];
    let doc = match memory::read_doc(&notebook.memory_dir(), rel) {
        Ok(doc) => doc,
        Err(error) => {
            findings.error(1, "unreadable", format!("{error:#}"));
            return;
        }
    };
    let items = doc.entries().count();
    if layer == Layer::Core && items > CORE_LIMIT {
        let why = format!("core holds {items} items, more than {CORE_LIMIT}");
        findings.warning(1, "core-length", why);
    }
    for entry in doc.entries() {
        if let Some(id) = entry.id() {
            known.id_twice(id, findings);
        }
        for workgroup in entry.workgroup() {
            known.unknown_workgroup(&workgroup, findings);
        }
    }
}

/// What is wrong with any text: links to nothing and secrets.
fn text_findings(names: &Names, text: &str, findings: &mut Findings) {
    for secret in secrets::secrets(text) {
        findings.error(1, "secret", format!("looks like a secret: {secret}"));
    }
    let mut seen: Vec<&str> = Vec::new();
    for link in found(text) {
        let target = link.target;
        let there = names.is_there(target);
        let written = with_extension(link.whole);
        if target.is_empty() || seen.contains(&target) || there && !written {
            continue;
        }
        seen.push(target);
        let line = line_at(text, link.at);
        match names.find(target, None) {
            Ok(Some(one)) => {
                let why = format!("{} is written {}", link.whole, one.link());
                findings.warning(line, "link-form", why);
            }
            Err(error) if error.is::<Ambiguous>() => {
                findings.error(line, "broken-link", format!("links to several: {error}"));
            }
            _ if there => {}
            _ => {
                let why = format!("links to nothing: [[{target}]]");
                findings.error(line, "broken-link", why);
            }
        }
    }
}

fn line_at(text: &str, at: usize) -> usize {
    text[..at].matches('\n').count() + 1
}

/// Whether a time is written with a `T` between its day and its minute.
fn has_t(text: &str) -> bool {
    text.as_bytes().get(10) == Some(&b'T')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::DETAILS;
    use crate::records::titled_link;
    use crate::testing::{me, notebook, record, write};

    const A: &str = "01kat3v8q4zj6m2x9r5c7w1hbn";
    const B: &str = "01kat3v8q4zj6m2x9r5c7w1hbz";

    fn rules(notebook: &Notebook) -> Vec<(&'static str, Severity, usize)> {
        check(notebook)
            .unwrap()
            .into_iter()
            .map(|f| (f.rule, f.severity, f.line))
            .collect()
    }

    use Severity::{Error, Warning};

    #[test]
    fn a_new_notebook_and_what_the_tools_write_have_nothing_wrong() {
        let (_tmp, nb) = notebook("check-clean");
        let r = Records::new(nb.clone());
        for kind in r.kind_names().unwrap() {
            let mut draft = r.draft(&kind, "題名: x").unwrap();
            draft.body.set_notes("- A: 1\n\n## 見出し").unwrap();
            if kind != "memos" {
                draft.body.set_in(DETAILS, "色", "青, 赤").unwrap();
                draft
                    .body
                    .log("2026-11-24 10:00", "owner", "足した")
                    .unwrap();
            }
            r.write(&me(), &draft, "足した").unwrap();
        }

        assert_eq!(check(&nb).unwrap(), []);
        assert_eq!(fix(&nb, &[]).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn a_file_in_no_place_of_the_shape_is_told() {
        let (_tmp, nb) = notebook("check-stray");
        write(nb.root(), "notes.md", "# x\n");
        write(nb.root(), "memory/long/other/x.md", "x\n");
        write(nb.root(), "records/tasks/sub/t1.md", "# x\n");
        write(nb.root(), ".notebook/whatever", "x");

        assert_eq!(rules(&nb), [("stray", Warning, 1); 3]);
    }

    #[test]
    fn metadata_that_does_not_fit_is_told() {
        let (_tmp, nb) = notebook("check-meta");
        write(
            nb.root(),
            "records/tasks/t1.md",
            "---\nid: t2\ncreated: yesterday\nmine: x\n---\n\n本文\n",
        );
        write(
            nb.root(),
            "memory/core.md",
            "# 心得\n\n- a {id: m1}\n- b {id: m1}\n",
        );

        let found: Vec<_> = check(&nb)
            .unwrap()
            .into_iter()
            .map(|f| (f.rule, f.severity))
            .collect();
        for expected in [
            ("id", Error),
            ("metadata", Error),
            ("metadata", Warning),
            ("title", Error),
            ("id-twice", Error),
        ] {
            assert!(found.contains(&expected), "{expected:?} in {found:?}");
        }
    }

    #[test]
    fn a_mark_of_doubt_without_a_day_and_a_move_to_no_record_are_told() {
        let (_tmp, nb) = notebook("check-marks");
        write(
            nb.root(),
            &format!("records/memos/{A}.md"),
            &format!("---\nid: {A}\nobsolete: soon\nmoved: elsewhere\n---\n\n# x\n"),
        );
        assert_eq!(rules(&nb), [("metadata", Error, 1), ("metadata", Error, 1)]);
    }

    #[test]
    fn fields_and_sections_that_do_not_fit_their_kind_are_told_with_their_lines() {
        let (_tmp, nb) = notebook("check-fields");
        record(
            &nb,
            "events",
            A,
            "# x\n\n- Status: someday\n- start: 2026-11-25T14:00\n- End: 2026-11-25 13:00\n- Mood: よい\n- End: 2026-11-26\n- ただの文\n\n## Log\n\n- x\n\n## Attendees\n\n段落\n\n- 田中\n  - Response: maybe\n",
        );

        let found = rules(&nb);

        for expected in [
            ("value", Error, 7),
            ("time-style", Warning, 8),
            ("name-style", Warning, 8),
            ("backwards", Error, 9),
            ("unknown-field", Warning, 10),
            ("twice", Error, 11),
            ("item", Error, 12),
            ("order", Warning, 18),
            ("item", Error, 20),
            ("value", Error, 23),
        ] {
            assert!(found.contains(&expected), "{expected:?} in {found:?}");
        }
        assert_eq!(found.len(), 10, "{found:?}");
    }

    #[test]
    fn links_to_nothing_to_another_kind_and_with_an_old_title_are_told() {
        let (_tmp, nb) = notebook("check-links");
        record(&nb, "tasks", A, "# 先\n");
        record(
            &nb,
            "tasks",
            B,
            &format!(
                "# 後\n\n- Parent: [[records/memos/x]]\n- Blocked by: [[records/tasks/{A}|前の題名]]\n\n## Notes\n\n[[records/tasks/{A}|あだ名]]\n"
            ),
        );

        assert_eq!(
            rules(&nb),
            [
                ("broken-link", Error, 7),
                ("value", Error, 7),
                ("link-title", Warning, 8)
            ]
        );
    }

    #[test]
    fn a_circle_is_told_and_a_write_that_would_make_one_is_refused() {
        let (_tmp, nb) = notebook("check-circle");
        let r = Records::new(nb.clone());
        record(
            &nb,
            "tasks",
            A,
            &format!("# 先\n\n- Blocked by: [[records/tasks/{B}|後]]\n"),
        );
        record(&nb, "tasks", B, "# 後\n");
        let mut b = r.get("tasks", B).unwrap().unwrap();
        b.body
            .set("Blocked by", vec![titled_link("tasks", A, "先")])
            .unwrap();

        assert!(
            r.write(&me(), &b, "x")
                .unwrap_err()
                .to_string()
                .contains("leads back")
        );

        std::fs::write(nb.root().join(b.rel()), b.render()).unwrap();
        assert_eq!(rules(&nb), [("circle", Error, 7), ("circle", Error, 7)]);
    }

    #[test]
    fn a_relation_written_on_both_sides_and_an_unknown_workgroup_are_told() {
        let (_tmp, nb) = notebook("check-both");
        let of = |id: &str| format!("\n## Relations\n\n- [[records/parties/{id}]]\n  - As: 夫婦\n");
        record(&nb, "parties", A, &format!("# 甲\n{}", of(B)));
        record(&nb, "parties", B, &format!("# 乙\n{}", of(A)));
        crate::workgroups::set(
            &nb,
            &me(),
            &crate::workgroups::Workgroup {
                id: "work".into(),
                name: "仕事".into(),
                color: "".into(),
            },
        )
        .unwrap();
        write(
            nb.root(),
            "memory/core.md",
            "# 心得\n\n- a {id: m1, workgroup: [home]}\n",
        );

        let found: Vec<_> = rules(&nb).into_iter().map(|(r, s, _)| (r, s)).collect();
        assert_eq!(
            found,
            [
                ("workgroup", Warning),
                ("both-sides", Warning),
                ("both-sides", Warning)
            ]
        );
    }

    #[test]
    fn a_memo_and_notes_are_checked_only_for_links_and_secrets() {
        let (_tmp, nb) = notebook("check-free");
        let free = "- A: 1\n- A: 2\n\n## Log\n\n段落\n";
        record(&nb, "memos", A, &format!("# メモ\n\n{free}"));
        record(
            &nb,
            "tasks",
            B,
            "# x\n\n## Notes\n\n- A: 1\n- A: 2\n\n### 見出し\n",
        );

        assert_eq!(rules(&nb), []);
    }

    #[test]
    fn a_declaration_or_workgroups_that_cannot_be_read_are_told() {
        let (_tmp, nb) = notebook("check-declarations");
        write(
            nb.root(),
            "records/things/README.md",
            "---\nfields:\n  A: nothing\n---\n",
        );
        write(
            nb.root(),
            "records/workgroups.md",
            "# x\n\n## Workgroups\n\n- a\n- a\n",
        );

        assert_eq!(rules(&nb), [("kind", Error, 1), ("workgroups", Error, 1)]);
    }

    #[test]
    fn core_longer_than_its_limit_is_told() {
        let (_tmp, nb) = notebook("check-core");
        let core: String = (1..=CORE_LIMIT + 1)
            .map(|n| format!("- rule {n} {{id: m{n}}}\n"))
            .collect();
        write(nb.root(), "memory/core.md", &format!("# 心得\n\n{core}"));

        assert_eq!(rules(&nb), [("core-length", Warning, 1)]);
    }

    #[test]
    fn a_file_that_cannot_be_read_and_a_readme_without_the_shape_are_told() {
        let (_tmp, nb) = notebook("check-unreadable");
        write(
            nb.root(),
            &format!("records/tasks/{A}.md"),
            "---\na: [\n---\n",
        );
        write(nb.root(), "README.md", "# 手帳\n");

        assert_eq!(rules(&nb), [("shape", Error, 1), ("unreadable", Error, 1)]);
    }

    #[test]
    fn named_files_alone_are_checked_against_the_whole_notebook() {
        let (_tmp, nb) = notebook("check-some");
        record(&nb, "tasks", A, "# 先\n\n- Status: x\n");
        let rel = record(
            &nb,
            "tasks",
            B,
            &format!("# 後\n\n- Parent: [[records/tasks/{A}|古い]]\n"),
        );

        let found = check_files(&nb, &[rel]).unwrap();

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rule, "link-title");
    }
}
