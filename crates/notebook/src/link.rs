//! Wiki links (`[[path]]`, `[[path|title]]`) in a text.

/// The target of a wiki link (`[[records/parties/p1|田中さん]]` gives
/// `records/parties/p1`), or the text itself when it is not one.
pub fn link_target(text: &str) -> &str {
    let text = text.trim();
    let Some(inner) = text.strip_prefix("[[").and_then(|t| t.strip_suffix("]]")) else {
        return text;
    };
    target_of(inner)
}

fn target_of(inner: &str) -> &str {
    inner.split('|').next().unwrap_or(inner).trim()
}

/// What stands between each `[[` and its `]]`, with both and without.
fn wiki_links(text: &str) -> impl Iterator<Item = (&str, &str)> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let start = at + text[at..].find("[[")?;
        let end = start + 2 + text[start + 2..].find("]]")?;
        at = end + 2;
        Some((&text[start..at], &text[start + 2..end]))
    })
}

/// Every wiki link target in a text, once each, in the order they appear.
/// Links shown inside code are not links.
pub fn links(text: &str) -> Vec<String> {
    let text = outside_code(text);
    let mut targets = Vec::new();
    for (_, inner) in wiki_links(&text) {
        let target = target_of(inner);
        if !target.is_empty() && !targets.iter().any(|t| t == target) {
            targets.push(target.to_owned());
        }
    }
    targets
}

/// A wiki link that carries a title.
pub(crate) struct TitledLink<'a> {
    /// The link as written.
    pub whole: &'a str,
    pub target: &'a str,
    pub title: &'a str,
}

/// Every `[[target|title]]` in a text.
pub(crate) fn titled_links(text: &str) -> impl Iterator<Item = TitledLink<'_>> {
    wiki_links(text).filter_map(|(whole, inner)| {
        let (target, title) = inner.split_once('|')?;
        Some(TitledLink {
            whole,
            target: target.trim(),
            title: title.trim(),
        })
    })
}

/// The text with fenced code blocks and code spans blanked out, since what
/// they hold is shown as written, not followed.
fn outside_code(text: &str) -> String {
    let mut out = String::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            let mut in_span = false;
            out.extend(line.chars().map(|c| {
                let tick = c == '`';
                in_span ^= tick;
                if tick || in_span { ' ' } else { c }
            }));
        }
        out.push('\n');
    }
    out
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
}
