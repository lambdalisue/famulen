//! Wiki links (`[[path]]`, `[[path|title]]`) in a text.

use std::ops::Range;

use ox_content_allocator::Allocator;
use ox_content_ast::{CodeBlock, InlineCode, Span, Visit};
use ox_content_parser::{Parser, ParserOptions};

use crate::records::record_of;

/// The target of a wiki link (`[[records/parties/p1|田中さん]]` gives
/// `records/parties/p1`), or the text itself when it is not one.
pub fn link_target(text: &str) -> &str {
    without_extension(written_target(text))
}

/// Whether a text is one wiki link and nothing else.
pub(crate) fn is_link(text: &str) -> bool {
    text.strip_prefix("[[")
        .and_then(|t| t.strip_suffix("]]"))
        .is_some_and(|inner| !inner.contains("[[") && !inner.contains("]]"))
}

/// Whether a link to a record is written with the `.md` of its file.
pub(crate) fn with_extension(text: &str) -> bool {
    written_target(text) != link_target(text)
}

fn written_target(text: &str) -> &str {
    let text = text.trim();
    match text.strip_prefix("[[").and_then(|t| t.strip_suffix("]]")) {
        Some(inner) => inner.split('|').next().unwrap_or(inner).trim(),
        None => text,
    }
}

/// A record is named by its path without the `.md` of its file.
fn without_extension(target: &str) -> &str {
    target
        .strip_suffix(".md")
        .filter(|path| record_of(path).is_some())
        .unwrap_or(target)
}

/// Where each wiki link of a text is, from its `[[` to its `]]`. A link stands
/// on one line.
fn spans(text: &str) -> impl Iterator<Item = Range<usize>> {
    let mut at = 0;
    std::iter::from_fn(move || {
        loop {
            let end = at + text[at..].find("]]")? + 2;
            let before = &text[at..end - 2];
            let start = before.rfind("[[").map(|n| at + n);
            at = end;
            if let Some(start) = start
                && !text[start..end].contains('\n')
            {
                return Some(start..end);
            }
        }
    })
}

/// One wiki link of a text.
pub(crate) struct Found<'a> {
    /// Where it starts.
    pub at: usize,
    /// The link as written.
    pub whole: &'a str,
    pub target: &'a str,
    pub title: Option<&'a str>,
}

/// Every wiki link of a text, but for those shown inside code.
pub(crate) fn found(text: &str) -> Vec<Found<'_>> {
    spans(&outside_code(text))
        .map(|s| {
            let inner = &text[s.start + 2..s.end - 2];
            Found {
                at: s.start,
                target: link_target(&text[s.clone()]),
                whole: &text[s],
                title: inner.split_once('|').map(|(_, title)| title.trim()),
            }
        })
        .collect()
}

/// A text with its wiki links replaced by what `with` gives for them. Links
/// shown inside code stay.
pub(crate) fn rewrite(
    text: &str,
    mut with: impl FnMut(&Found) -> anyhow::Result<Option<String>>,
) -> anyhow::Result<String> {
    let mut out = String::new();
    let mut at = 0;
    for link in found(text) {
        if let Some(new) = with(&link)? {
            out.push_str(&text[at..link.at]);
            out.push_str(&new);
            at = link.at + link.whole.len();
        }
    }
    out.push_str(&text[at..]);
    Ok(out)
}

/// Every wiki link target in a text, once each, in the order they appear.
/// Links shown inside code are not links.
pub fn links(text: &str) -> Vec<String> {
    let mut targets: Vec<String> = Vec::new();
    for link in found(text) {
        if !link.target.is_empty() && !targets.iter().any(|t| t == link.target) {
            targets.push(link.target.to_owned());
        }
    }
    targets
}

/// The text with code blocks and code spans blanked out, since what they hold
/// is shown as written, not followed. Every byte stays where it was.
fn outside_code(text: &str) -> String {
    let allocator = Allocator::new();
    let mut code = Code(Vec::new());
    if let Ok(document) = Parser::with_options(&allocator, text, ParserOptions::gfm()).parse() {
        code.visit_document(&document);
    }
    let mut out = text.as_bytes().to_vec();
    for range in code.0 {
        if text.get(range.clone()).is_none() {
            continue;
        }
        for byte in &mut out[range] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Where the code of a text is.
struct Code(Vec<Range<usize>>);

impl Code {
    fn add(&mut self, span: Span) {
        self.0.push(span.start as usize..span.end as usize);
    }
}

impl<'a> Visit<'a> for Code {
    fn visit_code_block(&mut self, code: &CodeBlock<'a>) {
        self.add(code.span);
    }

    fn visit_inline_code(&mut self, code: &InlineCode<'a>) {
        self.add(code.span);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wiki_link_gives_its_target() {
        assert_eq!(
            link_target("[[records/parties/p1|田中さん]]"),
            "records/parties/p1"
        );
        assert_eq!(link_target("[[records/parties/p1]]"), "records/parties/p1");
        assert_eq!(link_target("p1"), "p1");
    }

    #[test]
    fn links_are_every_wiki_link_target_once() {
        assert_eq!(
            links(
                "[[records/parties/p1|田中さん]] と [[records/parties/p2]]、また [[records/parties/p1]]"
            ),
            ["records/parties/p1", "records/parties/p2"]
        );
        assert!(links("no [[unclosed link").is_empty());
        assert!(links("`[[records/x]]` and\n```\n[[records/y]]\n```\n").is_empty());
    }

    #[test]
    fn a_link_inside_code_however_it_is_fenced_is_not_one() {
        for code in [
            "``[[x]]``",
            "`` a ` [[x]] ` b ``",
            "`a\nb [[x]]`",
            "````\n```\n[[x]]\n````",
            "~~~\n``` \n[[x]]\n~~~",
            "```\n[[x]]\n``` rs\n[[x]]\n```",
            "文\n\n    [[x]]\n",
            "> 引用 `a\n> [[x]]`",
            "| 日 |\n|---|\n| `[[x]]` |",
        ] {
            assert!(links(code).is_empty(), "{code}");
        }
        for text in [
            "a ` [[x]]",
            "`` [[x]] `",
            "\\`[[x]]\\`",
            "a ` b\n\n[[x]] ` c",
            "- a ` b\n- [[x]] ` c",
            "```a``` [[x]]",
            "- a\n\n    [[x]]\n",
        ] {
            assert_eq!(links(text), ["x"], "{text}");
        }
    }

    #[test]
    fn a_record_is_the_same_target_with_the_extension_of_its_file() {
        assert_eq!(
            links("[[records/tasks/a.md]] [[records/tasks/a|題名]] [[memory/core.md]]"),
            ["records/tasks/a", "memory/core.md"]
        );
    }

    #[test]
    fn rewriting_replaces_links_outside_code_and_leaves_every_other_byte() {
        let text = "頭 `[[あ]]` [[い|名]]\n~~~\n[[い]]\n~~~\n[[う]] if [[ a\nb ]] 尾";

        let new = rewrite(text, |l| {
            Ok((l.target == "い").then(|| format!("<{}>", l.title.unwrap_or_default())))
        })
        .unwrap();

        assert_eq!(
            new,
            "頭 `[[あ]]` <名>\n~~~\n[[い]]\n~~~\n[[う]] if [[ a\nb ]] 尾"
        );
        assert_eq!(links(text), ["い", "う"]);
    }
}
