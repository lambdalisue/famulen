//! A memory file: its items, and the rest of it kept as it was.

use std::path::Path;

use anyhow::Context as _;

use crate::Page;
use crate::entry::{Attr, Entry, attr_of};

/// One line of a memory file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Entry(Entry),
    Other(String),
}

/// A memory file: its items, and every other line kept as it was. A
/// long-term memory file holds one item, whose fields are the frontmatter;
/// its prose is kept as it was.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Doc {
    lines: Vec<Line>,
    long: Option<Page>,
}

impl Doc {
    pub fn parse(text: &str) -> Self {
        let lines = text
            .split('\n')
            .map(|line| match Entry::parse(line) {
                Some(entry) => Line::Entry(entry),
                None => Line::Other(line.to_string()),
            })
            .collect();
        Doc { lines, long: None }
    }

    /// Read a long-term memory file: one item from its frontmatter.
    pub(crate) fn parse_long(text: &str) -> anyhow::Result<Self> {
        let page = Page::parse(text)?;
        Ok(Doc {
            lines: long_entry(&page).map(Line::Entry).into_iter().collect(),
            long: Some(page),
        })
    }

    /// A long-term memory file with nothing in it yet.
    pub fn new_long() -> Self {
        Doc {
            lines: Vec::new(),
            long: Some(Page::new()),
        }
    }

    /// Read a long-term memory file; a missing one holds nothing yet.
    pub(crate) fn load_long(path: &Path) -> anyhow::Result<Self> {
        match read(path)? {
            Some(text) => {
                Doc::parse_long(&text).with_context(|| format!("reading {}", path.display()))
            }
            None => Ok(Doc::new_long()),
        }
    }

    pub(crate) fn is_long(&self) -> bool {
        self.long.is_some()
    }

    /// The prose of a long-term memory file.
    pub fn long_text(&self) -> Option<&str> {
        self.long.as_ref().map(Page::body)
    }

    /// Replace the prose of a long-term memory file.
    pub fn set_long_text(&mut self, text: &str) -> anyhow::Result<()> {
        let page = self
            .long
            .as_mut()
            .context("only a long-term memory file has prose of its own")?;
        page.set_body(format!("{}\n", text.trim()));
        Ok(())
    }

    /// Read a file; a missing one is an empty document.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        Ok(read(path)?
            .map(|text| Doc::parse(&text))
            .unwrap_or_default())
    }

    /// Write the file. A long-term memory file left without its item goes.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if self.long.is_some() && self.lines.is_empty() {
            return match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            };
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.render()).with_context(|| format!("writing {}", path.display()))
    }

    pub fn render(&self) -> String {
        if let Some(page) = &self.long {
            return render_long(page, self.entries().next());
        }
        self.lines
            .iter()
            .map(|line| match line {
                Line::Entry(entry) => entry.render(),
                Line::Other(text) => text.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.lines.iter().filter_map(|line| match line {
            Line::Entry(entry) => Some(entry),
            Line::Other(_) => None,
        })
    }

    pub fn find(&self, id: &str) -> Option<&Entry> {
        self.entries().find(|e| e.id() == Some(id))
    }

    /// Change the item with `id` in place; `None` when there is none.
    pub fn update<T>(&mut self, id: &str, f: impl FnOnce(&mut Entry) -> T) -> Option<T> {
        let entry = self.lines.iter_mut().find_map(|line| match line {
            Line::Entry(entry) if entry.id() == Some(id) => Some(entry),
            _ => None,
        })?;
        Some(f(entry))
    }

    pub fn remove(&mut self, id: &str) -> Option<Entry> {
        let at = self
            .lines
            .iter()
            .position(|line| matches!(line, Line::Entry(e) if e.id() == Some(id)))?;
        match self.lines.remove(at) {
            Line::Entry(entry) => Some(entry),
            Line::Other(_) => unreachable!("position matched an entry"),
        }
    }

    /// Add an item at the end, after a blank line when it follows prose or a
    /// heading, and keep the file ending in a newline. A long-term memory
    /// file takes one item only.
    pub fn push(&mut self, entry: Entry) -> anyhow::Result<()> {
        if self.long.is_some() {
            anyhow::ensure!(
                self.lines.is_empty(),
                "a long-term memory file holds one memory; give the new one a file of its own"
            );
            self.lines.push(Line::Entry(entry));
            return Ok(());
        }
        let trailing_newline = matches!(self.lines.last(), Some(Line::Other(s)) if s.is_empty());
        let mut at = if trailing_newline {
            self.lines.len() - 1
        } else {
            self.lines.len()
        };
        if at > 0 && matches!(&self.lines[at - 1], Line::Other(s) if !s.is_empty()) {
            self.lines.insert(at, Line::Other(String::new()));
            at += 1;
        }
        self.lines.insert(at, Line::Entry(entry));
        if !trailing_newline {
            self.lines.push(Line::Other(String::new()));
        }
        Ok(())
    }
}

/// A file's text, or `None` when there is no file.
fn read(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// The field of a long-term memory that is its item's text.
const SUMMARY: &str = "summary";

/// The item a long-term memory file holds: its summary as the text, and every
/// plain field of the frontmatter as an attribute.
fn long_entry(page: &Page) -> Option<Entry> {
    let text = page.text(SUMMARY).or_else(|| {
        page.body()
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_owned)
    })?;
    let attrs = page
        .fields()
        .filter(|(k, _)| *k != SUMMARY)
        .filter_map(|(k, v)| Some((k.to_owned(), attr_of(v)?)))
        .collect();
    Some(Entry::with_attrs(&text, attrs))
}

fn render_long(page: &Page, entry: Option<&Entry>) -> String {
    let Some(entry) = entry else {
        return String::new();
    };
    let mut page = page.clone();
    if let Some(was) = long_entry(&page) {
        for (key, _) in was.attrs() {
            if !entry.attrs().iter().any(|(k, _)| k == key) {
                page.remove(key);
            }
        }
    }
    if let Some(id) = entry.id() {
        page.set("id", id);
    }
    page.set(SUMMARY, entry.text());
    for (key, value) in entry.attrs() {
        match value {
            Attr::List(items) => page.set_list(key, items),
            Attr::Text(t) => match t.parse::<u64>() {
                Ok(n) => page.set(key, n),
                Err(_) => page.set(key, t.as_str()),
            },
        }
    }
    if page.body().trim().is_empty() {
        page.set_body(format!("{}\n", entry.text()));
    }
    page.render()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Src;
    use crate::testing::TempDir;

    #[test]
    fn a_document_round_trips_with_its_non_item_lines() {
        let text = "# 田中さん\n\n> note\n- 甘いものが好き {id: m1, party: c1}\n  continued line\n- plain item\n";

        let doc = Doc::parse(text);

        assert_eq!(doc.render(), text);
        assert_eq!(doc.entries().count(), 2);
    }

    #[test]
    fn a_document_finds_updates_and_removes_items_by_id() {
        let mut doc = Doc::parse("# x\n\n- a {id: m1}\n- b {id: m2}\n");

        doc.update("m2", |e| e.set("ev", "1")).unwrap().unwrap();
        let removed = doc.remove("m1").unwrap();

        assert_eq!(removed.text(), "a");
        assert_eq!(doc.find("m2").unwrap().ev(), Some(1));
        assert!(doc.find("m1").is_none());
        assert!(doc.update("missing", |_| ()).is_none());
        assert_eq!(doc.render(), "# x\n\n- b {id: m2, ev: 1}\n");
    }

    #[test]
    fn pushing_under_a_heading_leaves_a_blank_line_and_a_final_newline() {
        let mut doc = Doc::parse("# 心得\n");

        doc.push(Entry::new("a").with("id", "m1").unwrap()).unwrap();
        doc.push(Entry::new("b").with("id", "m2").unwrap()).unwrap();

        assert_eq!(doc.render(), "# 心得\n\n- a {id: m1}\n- b {id: m2}\n");
    }

    #[test]
    fn pushing_into_an_empty_document_writes_one_line() {
        let mut doc = Doc::parse("");

        doc.push(Entry::new("a")).unwrap();

        assert_eq!(doc.render(), "- a\n");
    }

    #[test]
    fn a_long_term_file_is_one_item_from_its_frontmatter() {
        let text = "---\nid: m1\nsummary: 返事は短く、要点を先に\nsrc: said\nfrom: 2026-09-03\nev: 2\nworkgroup:\n- work\n- personal\nmood: calm\n---\n\n返事は短く、要点を先に書く。長い説明は求められてから。\n";

        let doc = Doc::parse_long(text).unwrap();

        let entry = doc.entries().next().unwrap();
        assert_eq!(doc.entries().count(), 1);
        assert_eq!(entry.text(), "返事は短く、要点を先に");
        assert_eq!(entry.id(), Some("m1"));
        assert_eq!(entry.src(), Some(Src::Said));
        assert_eq!(entry.ev(), Some(2));
        assert_eq!(entry.workgroup(), vec!["work", "personal"]);
        assert_eq!(entry.get("mood"), Some("calm"));
        assert_eq!(Doc::parse_long(&doc.render()).unwrap(), doc);
    }

    #[test]
    fn editing_a_long_term_item_keeps_its_prose_and_unknown_fields() {
        let text = "---\nid: m1\nsummary: 朝が早い\nsrc: inferred\nev: 1\nnested:\n  a: 1\n---\n\n山田さんは朝が早い。7 時には返事が来る。\n";
        let mut doc = Doc::parse_long(text).unwrap();

        doc.update("m1", |e| {
            e.set("ev", "2").unwrap();
            e.remove_attr("src");
        })
        .unwrap();

        let again = Page::parse(&doc.render()).unwrap();
        assert_eq!(again.text("ev").as_deref(), Some("2"));
        assert_eq!(again.text("src"), None);
        assert!(again.value("nested").is_some());
        assert_eq!(again.body(), "\n山田さんは朝が早い。7 時には返事が来る。\n");
    }

    #[test]
    fn a_long_term_file_takes_one_item_and_goes_when_it_is_taken_out() {
        let dir = TempDir::new("long-doc");
        let path = dir.path().join("long/semantic/tea.md");
        let mut doc = Doc::load_long(&path).unwrap();

        doc.push(Entry::new("紅茶が好き").with("id", "m3").unwrap())
            .unwrap();
        assert!(doc.push(Entry::new("二つ目")).is_err());
        doc.save(&path).unwrap();
        let saved = Page::load(&path).unwrap();
        assert_eq!(saved.text("id").as_deref(), Some("m3"));
        assert_eq!(saved.text("summary").as_deref(), Some("紅茶が好き"));
        assert_eq!(saved.body(), "\n紅茶が好き\n");

        let mut doc = Doc::load_long(&path).unwrap();
        doc.remove("m3").unwrap();
        doc.save(&path).unwrap();
        assert!(!path.exists());
    }
}
