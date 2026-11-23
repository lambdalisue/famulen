//! A record's body as Markdown structure: the title, the fields in the list
//! under it, the sections, the notes and the log. It is read from the syntax
//! tree, and an edit replaces only the span it concerns, so the rest of what
//! someone wrote by hand stays as they wrote it.

mod edit;

use edit::unescape;
pub use edit::{NewItem, Value};

use std::ops::Range;

use ox_content_allocator::Allocator;
use ox_content_ast::{List, ListItem, Node, Span};
use ox_content_parser::{Parser, ParserOptions};

use crate::link::is_link;

pub const DETAILS: &str = "Details";
pub const NOTES: &str = "Notes";
pub const LOG: &str = "Log";

/// The field every kind has: the records it has to do with. Under the title
/// of a free body, it is the one thing that is not text.
pub const RELATED: &str = "Related";

/// The sections every kind with a structured body may have, after its own.
pub const COMMON_SECTIONS: [&str; 3] = [DETAILS, NOTES, LOG];

/// One item of a list, with what is nested in it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Item {
    /// The item's first paragraph, as written.
    pub text: String,
    /// The list nested in it.
    pub items: Vec<Item>,
    /// Everything after the first paragraph, without its indent.
    pub rest: String,
    /// Where it is in the body.
    pub span: Range<usize>,
}

impl Item {
    /// The item read as `Name: value`.
    pub fn field(&self) -> Option<(&str, &str)> {
        let (name, value) = self.text.split_once(':')?;
        let name = name.trim();
        let spaced = value.is_empty() || value.starts_with(char::is_whitespace);
        (spaced && !name.is_empty() && !name.contains(['[', '\n'])).then(|| (name, value.trim()))
    }

    pub fn is_field(&self, name: &str) -> bool {
        self.field().is_some_and(|(n, _)| same(n, name))
    }

    /// The value of the item read as a field, with what continues it.
    pub fn value(&self) -> String {
        let first = self.field().map(|(_, v)| v).unwrap_or(&self.text);
        match (first.is_empty(), self.rest.is_empty()) {
            (_, true) => first.to_owned(),
            (true, false) => self.rest.clone(),
            (false, false) => format!("{first}\n\n{}", self.rest),
        }
    }

    /// The value of the item read as a field that holds several: the nested
    /// items, or what is separated by commas.
    pub fn values(&self) -> Vec<String> {
        if !self.items.is_empty() {
            return self.items.iter().map(Item::plain).collect();
        }
        split_commas(self.field().map(|(_, v)| v).unwrap_or_default())
    }

    /// The text of an item that is not a field.
    pub fn plain(&self) -> String {
        unescape(&self.text)
    }

    pub fn nested(&self, name: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.is_field(name))
    }
}

/// A `##` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Its name as written.
    pub name: String,
    pub heading: Range<usize>,
    /// From the heading's end to the next section.
    pub content: Range<usize>,
    /// The items of its lists, for a section of fields or items.
    pub items: Vec<Item>,
    /// What else it holds, for a section of fields or items.
    pub stray: Vec<Range<usize>>,
}

/// How a kind's body is laid out.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Layout {
    /// The whole body after the title is free text.
    pub free: bool,
    /// The sections the kind declares, in their order.
    pub sections: Vec<String>,
}

impl Layout {
    /// The sections a body may have, in their order.
    pub(crate) fn all_sections(&self) -> impl Iterator<Item = &str> {
        self.sections
            .iter()
            .map(String::as_str)
            .chain(COMMON_SECTIONS)
    }

    /// A section's place in the order, when it is a section of this kind.
    pub(crate) fn rank(&self, name: &str) -> Option<usize> {
        self.all_sections().position(|s| same(s, name))
    }

    /// A section's name as it should be written.
    pub(crate) fn proper<'a>(&'a self, name: &'a str) -> &'a str {
        self.all_sections().find(|s| same(s, name)).unwrap_or(name)
    }
}

/// A body read as structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    text: String,
    layout: Layout,
    title: Option<(String, Range<usize>)>,
    fields: Vec<Item>,
    fields_span: Option<Range<usize>>,
    /// What stands between the fields and the first section.
    preamble: Range<usize>,
    sections: Vec<Section>,
}

impl Body {
    pub fn parse(text: impl Into<String>, layout: &Layout) -> anyhow::Result<Self> {
        // Positions are read back as byte offsets, which a lone carriage
        // return would put off by one inside nested lists.
        let text = text.into().replace("\r\n", "\n");
        let allocator = Allocator::new();
        let document = Parser::with_options(&allocator, &text, ParserOptions::gfm())
            .parse()
            .map_err(|e| anyhow::anyhow!("reading the Markdown: {e:?}"))?;
        let nodes: Vec<&Node> = document.children.iter().collect();

        let mut title = None;
        let mut at = 0;
        let first = nodes.first().copied();
        if let Some(Node::Heading(h)) = first
            && h.depth == 1
        {
            title = Some((heading_text(&text, h.span), range(h.span)));
            at = 1;
        }
        let mut fields = Vec::new();
        let mut fields_span = None;
        if !layout.free
            && title.is_some()
            && let Some(Node::List(list)) = nodes.get(at).copied()
            && !list.ordered
        {
            fields = items(&text, list);
            fields_span = Some(range(list.span));
            at += 1;
        }
        if layout.free
            && title.is_some()
            && let Some(Node::List(list)) = nodes.get(at).copied()
            && !list.ordered
            && let Some(first) = list.children.iter().next()
            && let related = item(&text, first)
            && is_related(&related)
        {
            fields_span = Some(related.span.clone());
            fields = vec![related];
        }
        let preamble_start = fields_span
            .as_ref()
            .or(title.as_ref().map(|(_, r)| r))
            .map_or(0, |r| r.end);

        let mut sections: Vec<Section> = Vec::new();
        if !layout.free {
            for node in &nodes[at..] {
                if let Node::Heading(h) = node
                    && h.depth == 2
                    && layout.rank(&heading_text(&text, h.span)).is_some()
                {
                    let heading = range(h.span);
                    if let Some(last) = sections.last_mut() {
                        last.content.end = heading.start;
                    }
                    sections.push(Section {
                        name: heading_text(&text, h.span),
                        content: heading.end..text.len(),
                        heading,
                        items: Vec::new(),
                        stray: Vec::new(),
                    });
                    continue;
                }
                let Some(section) = sections.last_mut() else {
                    continue;
                };
                match node {
                    Node::List(list) if !list.ordered => {
                        section.items.extend(items(&text, list));
                    }
                    other => section.stray.extend(span_of(other).map(range)),
                }
            }
        }
        let preamble_end = sections.first().map_or(text.len(), |s| s.heading.start);
        Ok(Self {
            layout: layout.clone(),
            title,
            fields,
            fields_span,
            preamble: preamble_start.min(preamble_end)..preamble_end,
            sections,
            text,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_ref().map(|(t, _)| t.as_str())
    }

    pub fn fields(&self) -> &[Item] {
        &self.fields
    }

    pub fn field(&self, name: &str) -> Option<&Item> {
        self.fields.iter().find(|i| i.is_field(name))
    }

    /// A field's value, when it has one.
    pub fn value(&self, name: &str) -> Option<String> {
        self.field(name)
            .map(Item::value)
            .filter(|v| !v.trim().is_empty())
    }

    pub fn values(&self, name: &str) -> Vec<String> {
        self.field(name).map(Item::values).unwrap_or_default()
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| same(&s.name, name))
    }

    /// The items of every section of that name.
    pub fn items(&self, section: &str) -> Vec<&Item> {
        self.sections
            .iter()
            .filter(|s| same(&s.name, section))
            .flat_map(|s| &s.items)
            .collect()
    }

    /// The free text: what stands before the first section, and the notes.
    pub fn notes(&self) -> String {
        let mut parts = vec![self.text[self.preamble.clone()].trim()];
        parts.extend(
            self.sections
                .iter()
                .filter(|s| same(&s.name, NOTES))
                .map(|s| self.text[s.content.clone()].trim()),
        );
        parts.retain(|p| !p.is_empty());
        parts.join("\n\n")
    }

    /// Where what stands between the fields and the first section is.
    pub(crate) fn preamble(&self) -> Range<usize> {
        self.preamble.clone()
    }

    pub(crate) fn title_end(&self) -> usize {
        self.title.as_ref().map_or(0, |(_, span)| span.end)
    }

    /// The line, from one, that a place in the body is on.
    pub fn line_of(&self, at: usize) -> usize {
        self.text[..at.min(self.text.len())].matches('\n').count() + 1
    }
}

/// Whether two names of a field or section are the same.
pub fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Whether an item is the `Related` of a free body: one that holds only
/// links, so that text of the owner's that merely starts with the word stays
/// text.
fn is_related(item: &Item) -> bool {
    let values = item.values();
    let only_links = if item.items.is_empty() {
        item.rest.is_empty()
    } else {
        item.field().is_some_and(|(_, value)| value.is_empty())
            && item.items.iter().all(|i| i.rest.is_empty())
            && item.rest.lines().count() == item.items.len()
    };
    item.is_field(RELATED) && !values.is_empty() && only_links && values.iter().all(|v| is_link(v))
}

/// Split at commas that are not inside a wiki link.
pub fn split_commas(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut depth = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '[' if chars.peek() == Some(&'[') => {
                depth += 1;
                part.push(c);
                part.extend(chars.next());
                continue;
            }
            ']' if chars.peek() == Some(&']') && depth > 0 => {
                depth -= 1;
                part.push(c);
                part.extend(chars.next());
                continue;
            }
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut part));
                continue;
            }
            _ => {}
        }
        part.push(c);
    }
    parts.push(part);
    parts
        .into_iter()
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect()
}

fn range(span: Span) -> Range<usize> {
    span.start as usize..span.end as usize
}

fn heading_text(text: &str, span: Span) -> String {
    let raw = text[range(span)].trim();
    let line = raw.lines().next().unwrap_or_default().trim();
    match line.strip_prefix('#') {
        Some(rest) => {
            let rest = rest.trim_start_matches('#').trim();
            let closed = rest.trim_end_matches('#');
            if closed.ends_with(char::is_whitespace) {
                closed.trim().to_owned()
            } else {
                rest.to_owned()
            }
        }
        None => line.to_owned(),
    }
}

fn items(text: &str, list: &List) -> Vec<Item> {
    list.children.iter().map(|i| item(text, i)).collect()
}

fn item(text: &str, item: &ListItem) -> Item {
    let span = range(item.span);
    let mut first = String::new();
    let mut rest_from = None;
    let mut nested = Vec::new();
    for (n, child) in item.children.iter().enumerate() {
        if n == 0
            && let Node::Paragraph(p) = child
        {
            first = text[range(p.span)]
                .lines()
                .map(str::trim)
                .collect::<Vec<_>>()
                .join("\n");
            continue;
        }
        if rest_from.is_none() {
            rest_from = span_of(child).map(|s| s.start as usize);
        }
        if let Node::List(list) = child
            && !list.ordered
        {
            nested.extend(items(text, list));
        }
    }
    let rest = rest_from.map_or_else(String::new, |from| dedent(&text[from..span.end]));
    Item {
        text: first.trim().to_owned(),
        items: nested,
        rest,
        span,
    }
}

fn dedent(text: &str) -> String {
    let indent = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start_matches(' ').len())
        .min()
        .unwrap_or_default();
    text.lines()
        .map(|l| l.get(indent..).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn span_of(node: &Node) -> Option<Span> {
    Some(match node {
        Node::Paragraph(n) => n.span,
        Node::Heading(n) => n.span,
        Node::ThematicBreak(n) => n.span,
        Node::BlockQuote(n) => n.span,
        Node::List(n) => n.span,
        Node::CodeBlock(n) => n.span,
        Node::MathBlock(n) => n.span,
        Node::Html(n) => n.span,
        Node::Table(n) => n.span,
        Node::DefinitionList(n) => n.span,
        Node::Definition(n) => n.span,
        Node::FootnoteDefinition(n) => n.span,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn layout() -> Layout {
        Layout {
            free: false,
            sections: vec!["Attendees".into(), "Acquired".into()],
        }
    }

    pub(super) fn body(text: &str) -> Body {
        Body::parse(text, &layout()).unwrap()
    }

    pub(super) const EVENT: &str = "# 定例\n\n- Status: confirmed\n- start: 2026-11-25 14:00\n- Map: <https://maps.example.com/?q=a,b>\n- Aliases: たなっち, [[records/parties/x|田中, 太郎]]\n\n## Attendees\n\n- [[records/parties/a|田中]]\n  - Email: tanaka@example.com\n  - Response: accepted\n- 佐藤\n\n## Details\n\n- 係: 佐藤\n\n## Notes\n\n本文\n\n## 背景\n\n続き\n\n## Log\n\n- 2026-11-24 10:00 secretary: 足した\n";

    #[test]
    fn a_body_is_read_as_its_title_fields_sections_notes_and_log() {
        let b = body(EVENT);

        assert_eq!(b.title(), Some("定例"));
        assert_eq!(b.value("Status").as_deref(), Some("confirmed"));
        assert_eq!(b.value("Start").as_deref(), Some("2026-11-25 14:00"));
        assert_eq!(
            b.values("aliases"),
            ["たなっち", "[[records/parties/x|田中, 太郎]]"]
        );
        let attendees = b.items("Attendees");
        assert_eq!(attendees[0].plain(), "[[records/parties/a|田中]]");
        assert_eq!(attendees[0].nested("Response").unwrap().value(), "accepted");
        assert_eq!(attendees[1].plain(), "佐藤");
        assert_eq!(b.items("Details")[0].field(), Some(("係", "佐藤")));
        assert_eq!(b.notes(), "本文\n\n## 背景\n\n続き");
        assert_eq!(b.items("Log").len(), 1);
    }

    #[test]
    fn what_looks_like_structure_in_a_code_block_or_a_quote_is_not() {
        let b = body("# t\n\n## Notes\n\n```\n## Log\n- a: b\n```\n\n> ## Log\n\n## Log\n\n- x\n");

        assert_eq!(b.sections().len(), 2);
        assert_eq!(b.items("Log").len(), 1);
        assert!(body("# t\n\n```\n- A: 1\n```\n").field("A").is_none());
    }

    #[test]
    fn a_free_body_has_no_fields_or_sections() {
        let b = Body::parse(
            "# メモ\n\n- A: 1\n\n## Notes\n\nx\n",
            &Layout {
                free: true,
                ..Layout::default()
            },
        )
        .unwrap();

        assert_eq!(b.title(), Some("メモ"));
        assert!(b.fields().is_empty() && b.sections().is_empty());
        assert_eq!(b.notes(), "- A: 1\n\n## Notes\n\nx");
        let mut b = b;
        b.set_notes("## Log\n\n別").unwrap();
        assert_eq!(b.text(), "# メモ\n\n## Log\n\n別\n");
    }

    #[test]
    fn the_related_of_a_free_body_is_read_and_written_apart_from_its_text() {
        let free = Layout {
            free: true,
            ..Layout::default()
        };
        let mut b = Body::parse("# メモ\n\n- a\n- b\n", &free).unwrap();

        b.set(
            RELATED,
            vec!["memo-7w1hbn".to_owned(), "[[x|甲]]".to_owned()],
        )
        .unwrap();

        assert_eq!(
            b.text(),
            "# メモ\n\n- Related: [[memo-7w1hbn]], [[x|甲]]\n\n- a\n- b\n"
        );
        assert_eq!(b.values(RELATED), ["[[memo-7w1hbn]]", "[[x|甲]]"]);
        assert_eq!(b.notes(), "- a\n- b");
        b.set_notes("別\n\n続き").unwrap();
        assert_eq!(b.values(RELATED).len(), 2);
        assert_eq!(b.notes(), "別\n\n続き");
        b.set(RELATED, "").unwrap();
        assert_eq!(b.text().trim_end(), "# メモ\n\n別\n\n続き".trim_end());
        assert!(b.set("Status", "x").is_err());
        assert!(b.set_notes("- Related: [[x]]").is_err());

        for notes in [
            "- Related: [[x]] を確認して [[y]]",
            "- Related: [[x]]\n\n  続き",
            "- Related:\n  - [[x]]\n  - [[y]]\n\n  続き",
        ] {
            let b = Body::parse(format!("# メモ\n\n{notes}\n"), &free).unwrap();
            assert_eq!(b.notes(), notes);
        }
        let listed = Body::parse("# メモ\n\n- Related:\n  - [[x]]\n  - [[y]]\n", &free).unwrap();
        assert_eq!(listed.values(RELATED), ["[[x]]", "[[y]]"]);

        let b = Body::parse("# メモ\n\n- Related: ただの文\n", &free).unwrap();
        assert_eq!(b.notes(), "- Related: ただの文");
    }

    #[test]
    fn windows_line_ends_and_a_missing_last_one_are_read() {
        let mut b = body("# t\r\n\r\n- A: 1\r\n- B: 2");
        assert_eq!(b.value("A").as_deref(), Some("1"));

        b.set("C", "3").unwrap();

        assert_eq!(b.text(), "# t\n\n- A: 1\n- B: 2\n- C: 3\n");
        assert_eq!(b.line_of(b.field("B").unwrap().span.start), 4);
    }
}
