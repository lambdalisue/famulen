//! Writing a body: each edit replaces only the span it concerns.

use std::ops::Range;

use anyhow::Context as _;

use super::{Body, Item, LOG, NOTES, same, split_commas};

/// What to write as a field's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    One(String),
    Many(Vec<String>),
}

impl Value {
    pub fn is_empty(&self) -> bool {
        match self {
            Value::One(text) => text.trim().is_empty(),
            Value::Many(items) => items.is_empty(),
        }
    }

    fn render(&self, name: &str, indent: &str) -> String {
        let inner = format!("{indent}  ");
        match self {
            Value::One(text) => {
                let mut lines = text.trim().lines();
                let first = lines.next().unwrap_or_default();
                let mut out = format!("{indent}- {name}: {first}").trim_end().to_owned();
                out.push('\n');
                for line in lines {
                    push_indented(&mut out, line, &inner);
                }
                out
            }
            Value::Many(items) => {
                let one_line = items
                    .iter()
                    .all(|i| split_commas(i).len() == 1 && !i.contains('\n'));
                if one_line {
                    return format!("{indent}- {name}: {}\n", items.join(", "));
                }
                let mut out = format!("{indent}- {name}:\n");
                for item in items {
                    out.push_str(&render_text(item, &inner));
                }
                out
            }
        }
    }
}

impl From<&str> for Value {
    fn from(text: &str) -> Self {
        Value::One(text.to_owned())
    }
}

impl From<String> for Value {
    fn from(text: String) -> Self {
        Value::One(text)
    }
}

impl From<Vec<String>> for Value {
    fn from(items: Vec<String>) -> Self {
        Value::Many(items)
    }
}

/// An item to add to a section of items.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewItem {
    pub text: String,
    pub fields: Vec<(String, Value)>,
}

impl NewItem {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            fields: Vec::new(),
        }
    }

    /// Add a field, unless its value is empty.
    pub fn with(mut self, name: &str, value: impl Into<Value>) -> Self {
        let value = value.into();
        if !value.is_empty() {
            self.fields.push((name.to_owned(), value));
        }
        self
    }

    fn render(&self) -> String {
        let mut out = render_text(&self.text, "");
        for (name, value) in &self.fields {
            out.push_str(&value.render(name, "  "));
        }
        out
    }
}

impl Body {
    pub fn set_title(&mut self, title: &str) -> anyhow::Result<()> {
        let title = one_line(title);
        anyhow::ensure!(!title.is_empty(), "a record needs a title");
        let heading = format!("# {title}\n");
        match &self.title {
            Some((_, span)) => self.splice(span.clone(), &heading),
            None => self.splice(0..0, &format!("{heading}\n")),
        }
    }

    /// Write a field under the title, or remove it when its value is empty.
    pub fn set(&mut self, name: &str, value: impl Into<Value>) -> anyhow::Result<()> {
        let value = value.into();
        if let Some(span) = self.field(name).map(|i| i.span.clone()) {
            return self.put(span, &value, name, "");
        }
        if value.is_empty() {
            return Ok(());
        }
        let line = value.render(name, "");
        if let Some(last) = self.fields.last() {
            return self.insert_line(last.span.end, &line);
        }
        let (_, title) = self.title.as_ref().context("a record needs a title")?;
        let at = title.end;
        self.insert_block(at, &line)
    }

    /// Write a field of a section, or remove it when its value is empty. A
    /// section left without anything goes with it.
    pub fn set_in(
        &mut self,
        section: &str,
        name: &str,
        value: impl Into<Value>,
    ) -> anyhow::Result<()> {
        let value = value.into();
        let item = self
            .items(section)
            .into_iter()
            .find(|i| i.is_field(name))
            .map(|i| i.span.clone());
        if let Some(span) = item {
            self.put(span, &value, name, "")?;
            return self.drop_if_empty(section);
        }
        if value.is_empty() {
            return Ok(());
        }
        self.add_lines(section, &value.render(name, ""))
    }

    /// Add an item to a section of items.
    pub fn add_item(&mut self, section: &str, item: &NewItem) -> anyhow::Result<()> {
        self.add_lines(section, &item.render())
    }

    pub fn remove_item(&mut self, section: &str, index: usize) -> anyhow::Result<()> {
        let span = self.item_at(section, index)?.span.clone();
        self.splice(span, "")?;
        self.drop_if_empty(section)
    }

    /// Replace an item of a section, with whatever was nested in it.
    pub fn replace_item(
        &mut self,
        section: &str,
        index: usize,
        item: &NewItem,
    ) -> anyhow::Result<()> {
        let span = self.item_at(section, index)?.span.clone();
        self.splice(span, &item.render())
    }

    /// Write a field nested in an item, or remove it when its value is
    /// empty.
    pub fn set_item_field(
        &mut self,
        section: &str,
        index: usize,
        name: &str,
        value: impl Into<Value>,
    ) -> anyhow::Result<()> {
        let value = value.into();
        let item = self.item_at(section, index)?;
        if let Some(nested) = item.nested(name) {
            let (span, indent) = (nested.span.clone(), self.indent_at(nested.span.start));
            return self.put(span, &value, name, &indent);
        }
        if value.is_empty() {
            return Ok(());
        }
        let (at, indent) = match item.items.last() {
            Some(last) => (last.span.end, self.indent_at(last.span.start)),
            None => (item.span.end, "  ".to_owned()),
        };
        self.insert_line(at, &value.render(name, &indent))
    }

    /// Replace the free text. Text that would read as a section of the
    /// record is refused, as is text that would swallow what follows it.
    pub fn set_notes(&mut self, notes: &str) -> anyhow::Result<()> {
        let notes = notes.replace("\r\n", "\n");
        let notes = notes.trim();
        if self.layout.free {
            let preamble = self.preamble.clone();
            self.splice(preamble, "")?;
            if notes.is_empty() {
                return Ok(());
            }
            let end = self.text.len();
            return self.insert_block(end, &format!("{notes}\n"));
        }
        let before: Vec<String> = self.other_sections();
        let mut next = self.clone();
        while let Some(span) = next.section(NOTES).map(|s| s.heading.start..s.content.end) {
            next.splice(span, "")?;
        }
        let preamble = next.preamble.clone();
        if !next.text[preamble.clone()].trim().is_empty() {
            let gap = if next.sections.is_empty() { "" } else { "\n" };
            next.splice(preamble, gap)?;
        }
        if !notes.is_empty() {
            next.add_section(NOTES, &format!("{notes}\n"))?;
        }
        anyhow::ensure!(
            next.other_sections() == before && next.notes() == notes,
            "the notes would read as another part of the record: a \"## \" heading there may not \
             be named after a section ({}), and a code block has to be closed",
            self.layout.all_sections().collect::<Vec<_>>().join(", ")
        );
        *self = next;
        Ok(())
    }

    /// Add to the log what `who` did.
    pub fn log(&mut self, at: &str, who: &str, what: &str) -> anyhow::Result<()> {
        let mut lines = what.trim().lines();
        let mut line = format!("- {at} {who}: {}\n", lines.next().unwrap_or_default());
        for more in lines {
            push_indented(&mut line, more, "  ");
        }
        self.add_lines(LOG, &line)
    }

    fn other_sections(&self) -> Vec<String> {
        self.sections
            .iter()
            .filter(|s| !same(&s.name, NOTES))
            .map(|s| s.name.clone())
            .collect()
    }

    fn item_at(&self, section: &str, index: usize) -> anyhow::Result<&Item> {
        self.items(section)
            .get(index)
            .copied()
            .with_context(|| format!("no item {index} in {section}"))
    }

    fn indent_at(&self, at: usize) -> String {
        self.text[at..]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }

    fn put(
        &mut self,
        span: Range<usize>,
        value: &Value,
        name: &str,
        indent: &str,
    ) -> anyhow::Result<()> {
        if value.is_empty() {
            return self.splice(span, "");
        }
        self.splice(span, &value.render(name, indent))
    }

    /// Drop a section that holds nothing.
    fn drop_if_empty(&mut self, name: &str) -> anyhow::Result<()> {
        let span = self.section(name).and_then(|s| {
            self.text[s.content.clone()]
                .trim()
                .is_empty()
                .then_some(s.heading.start..s.content.end)
        });
        match span {
            Some(span) => self.splice(span, ""),
            None => Ok(()),
        }
    }

    /// Add lines of a list to a section, after its last item.
    fn add_lines(&mut self, section: &str, lines: &str) -> anyhow::Result<()> {
        let Some(found) = self.section(section) else {
            return self.add_section(section, lines);
        };
        match found.items.last() {
            Some(last) => self.insert_line(last.span.end, lines),
            None => {
                let at = found.heading.end;
                self.insert_block(at, lines)
            }
        }
    }

    /// Add a section in its place in the order.
    fn add_section(&mut self, name: &str, content: &str) -> anyhow::Result<()> {
        let rank = self
            .layout
            .rank(name)
            .with_context(|| format!("no section {name}"))?;
        let at = self
            .sections
            .iter()
            .find(|s| self.layout.rank(&s.name).is_some_and(|r| r > rank))
            .map_or(self.text.len(), |s| s.heading.start);
        self.insert_block(at, &format!("## {name}\n\n{content}"))
    }

    /// Insert a whole line at a line's start.
    fn insert_line(&mut self, at: usize, line: &str) -> anyhow::Result<()> {
        let lead = if at > 0 && !self.text[..at].ends_with('\n') {
            "\n"
        } else {
            ""
        };
        self.splice(at..at, &format!("{lead}{line}"))
    }

    /// Insert a block with one blank line around it.
    fn insert_block(&mut self, at: usize, block: &str) -> anyhow::Result<()> {
        let before = &self.text[..at];
        let after = &self.text[at..];
        let lead = if before.trim().is_empty() || before.ends_with("\n\n") {
            ""
        } else if before.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        let tail = if after.trim().is_empty() || after.starts_with('\n') {
            ""
        } else {
            "\n"
        };
        self.splice(at..at, &format!("{lead}{block}{tail}"))
    }

    fn splice(&mut self, span: Range<usize>, with: &str) -> anyhow::Result<()> {
        let mut text = self.text.clone();
        text.replace_range(span, with);
        *self = Self::parse(text, &self.layout)?;
        Ok(())
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn push_indented(out: &mut String, line: &str, indent: &str) {
    if !line.trim().is_empty() {
        out.push_str(indent);
        out.push_str(line);
    }
    out.push('\n');
}

/// One list item holding `text`.
fn render_text(text: &str, indent: &str) -> String {
    let mut lines = text.trim().lines();
    let first = lines.next().unwrap_or_default();
    let mut out = format!("{indent}- {}", escape(first)).trim_end().to_owned();
    out.push('\n');
    let inner = format!("{indent}  ");
    for line in lines {
        push_indented(&mut out, line, &inner);
    }
    out
}

/// Keep text at the start of an item from reading as Markdown structure.
fn escape(text: &str) -> String {
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let ordered = digits > 0 && text[digits..].starts_with(['.', ')']);
    if ordered {
        return format!("{}\\{}", &text[..digits], &text[digits..]);
    }
    let task = ["[ ] ", "[x] ", "[X] "].iter().any(|t| text.starts_with(t));
    if task || text.starts_with(['-', '+', '*', '#', '>', '`', '~', '=', '_', '\\']) {
        format!("\\{text}")
    } else {
        text.to_owned()
    }
}

/// The text [`escape`] was given.
pub(super) fn unescape(text: &str) -> String {
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && (text[digits..].starts_with("\\.") || text[digits..].starts_with("\\)")) {
        return format!("{}{}", &text[..digits], &text[digits + 1..]);
    }
    match text.strip_prefix('\\') {
        Some(rest)
            if rest.starts_with(['-', '+', '*', '#', '>', '`', '~', '=', '_', '\\', '[']) =>
        {
            rest.to_owned()
        }
        _ => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::tests::{EVENT, body};

    #[test]
    fn changing_one_field_leaves_every_other_line_as_it_was() {
        let mut b = body(EVENT);

        b.set("Start", "2026-11-26 09:00").unwrap();

        assert_eq!(
            b.text(),
            EVENT.replace("start: 2026-11-25 14:00", "Start: 2026-11-26 09:00")
        );
        b.set("start", "").unwrap();
        assert_eq!(b.text(), EVENT.replace("- start: 2026-11-25 14:00\n", ""));
    }

    #[test]
    fn fields_are_written_under_the_title_and_values_with_commas_as_a_nested_list() {
        let mut b = body("# t\n");

        b.set("Status", "open").unwrap();
        b.set("Aliases", vec!["a".to_owned(), "b, c".to_owned()])
            .unwrap();
        b.set("As", vec!["部下".to_owned(), "友人".to_owned()])
            .unwrap();

        assert_eq!(
            b.text(),
            "# t\n\n- Status: open\n- Aliases:\n  - a\n  - b, c\n- As: 部下, 友人\n"
        );
        assert_eq!(b.values("Aliases"), ["a", "b, c"]);
        assert_eq!(b.values("As"), ["部下", "友人"]);
    }

    #[test]
    fn a_value_of_several_lines_is_read_back() {
        let mut b = body("# t\n");

        b.set("Reason", "一つ目\n\n- 箇条\n- 書き").unwrap();
        b.set("Next", "x").unwrap();

        assert_eq!(
            b.value("Reason").as_deref(),
            Some("一つ目\n\n- 箇条\n- 書き")
        );
        assert_eq!(b.value("Next").as_deref(), Some("x"));
    }

    #[test]
    fn sections_are_added_in_their_order_whatever_is_there() {
        let mut b = body("# t\n\n## Notes\n\nn\n");

        b.log("2026-11-24 10:00", "owner", "した").unwrap();
        b.set_in("Details", "色", "青").unwrap();
        b.set_in("Acquired", "On", "2026-03-01").unwrap();
        b.add_item("Attendees", &NewItem::new("田中").with("Email", "a@b"))
            .unwrap();
        b.set("Status", "open").unwrap();

        assert_eq!(
            b.text(),
            "# t\n\n- Status: open\n\n## Attendees\n\n- 田中\n  - Email: a@b\n\n## Acquired\n\n- On: 2026-03-01\n\n## Details\n\n- 色: 青\n\n## Notes\n\nn\n\n## Log\n\n- 2026-11-24 10:00 owner: した\n"
        );
    }

    #[test]
    fn an_item_can_be_changed_replaced_and_removed_and_an_empty_section_goes() {
        let mut b = body(EVENT);

        b.set_item_field("Attendees", 0, "Response", "declined")
            .unwrap();
        b.set_item_field("Attendees", 1, "Email", "s@example.com")
            .unwrap();
        assert_eq!(
            b.text(),
            EVENT
                .replace("accepted", "declined")
                .replace("- 佐藤\n", "- 佐藤\n  - Email: s@example.com\n")
        );
        b.remove_item("Attendees", 0).unwrap();
        b.replace_item("Attendees", 0, &NewItem::new("鈴木"))
            .unwrap();
        assert_eq!(b.items("Attendees")[0].plain(), "鈴木");
        b.remove_item("Attendees", 0).unwrap();
        assert!(b.section("Attendees").is_none());
        assert_eq!(b.items("Details")[0].field(), Some(("係", "佐藤")));
    }

    #[test]
    fn notes_keep_their_own_headings_and_the_log_does_not_touch_them() {
        let mut b = body("# t\n\n- Status: open\n\n前書き\n\n## Log\n\n- x\n");
        assert_eq!(b.notes(), "前書き");

        b.set_notes("## 背景\n\n- A: 1\n\n```\n## Log\n```")
            .unwrap();
        b.log("2026-11-24 10:00", "owner", "y").unwrap();

        assert_eq!(b.notes(), "## 背景\n\n- A: 1\n\n```\n## Log\n```");
        assert_eq!(b.items("Log").len(), 2);
        assert_eq!(b.value("Status").as_deref(), Some("open"));
        b.set_notes("").unwrap();
        assert_eq!(b.notes(), "");
        assert_eq!(
            b.text(),
            "# t\n\n- Status: open\n\n## Log\n\n- x\n- 2026-11-24 10:00 owner: y\n"
        );
    }

    #[test]
    fn notes_that_would_read_as_another_part_are_refused_and_change_nothing() {
        let mut b = body("# t\n\n## Log\n\n- x\n");
        let before = b.clone();

        assert!(b.set_notes("## Log\n\n- 偽").is_err());
        assert!(b.set_notes("## attendees\n\n- 偽").is_err());
        assert!(b.set_notes("```\n閉じない").is_err());
        assert_eq!(b, before);
    }

    #[test]
    fn notes_that_start_with_a_list_do_not_become_fields() {
        let mut b = body("# t\n");

        b.set_notes("- A: 1\n- B: 2").unwrap();

        assert!(b.fields().is_empty());
        assert_eq!(b.notes(), "- A: 1\n- B: 2");
    }

    #[test]
    fn text_that_would_be_structure_is_kept_as_an_items_text() {
        let mut b = body("# t\n");
        for text in ["- a", "# b", "1. c", "\\d", "[ ] e", "> f"] {
            b.add_item("Attendees", &NewItem::new(text).with("Email", "x"))
                .unwrap();
        }

        let texts: Vec<String> = b.items("Attendees").iter().map(|i| i.plain()).collect();
        assert_eq!(texts, ["- a", "# b", "1. c", "\\d", "[ ] e", "> f"]);
        assert!(b.items("Attendees").iter().all(|i| i.items.len() == 1));
    }

    #[test]
    fn the_title_is_replaced_or_added() {
        let mut b = body("本文\n");
        b.set_title("題\n名").unwrap();
        assert_eq!(b.text(), "# 題 名\n\n本文\n");
        b.set_title("別").unwrap();
        assert_eq!(b.text(), "# 別\n\n本文\n");
        assert!(b.set_title(" ").is_err());
    }
}
