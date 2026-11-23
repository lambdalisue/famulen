//! Reading the entries of an RSS or Atom feed, and the pages they link to.

use feed_rs::model::{Entry, Link};

use super::{Item, More, Reading};

const TITLE_CHARS: usize = 120;
const ABSTRACT_CHARS: usize = 300;
const TEXT_CHARS: usize = 4000;

/// The feed's entries, or none when `text` is not a feed.
pub fn entries(text: &str) -> Option<Vec<Reading>> {
    let text = text.trim_start_matches(|c: char| c == '\u{feff}' || c.is_whitespace());
    if !text.starts_with('<') {
        return None;
    }
    // The text is already decoded, while the parser would decode it again by
    // the encoding the declaration names.
    let xml = match text.strip_prefix("<?xml ") {
        Some(rest) => rest.split_once("?>")?.1,
        None => text,
    };
    let feed = feed_rs::parser::parse(xml.as_bytes()).ok()?;
    Some(feed.entries.into_iter().map(reading).collect())
}

fn reading(entry: Entry) -> Reading {
    let summary = entry.summary.map(|s| plain(&s.content));
    let content = entry.content.and_then(|c| c.body).map(|html| plain(&html));
    let full = content
        .filter(|c| !c.is_empty())
        .or_else(|| summary.clone())
        .unwrap_or_default();
    let about = summary.filter(|s| !s.is_empty()).unwrap_or(full.clone());
    let about = cut(&about, ABSTRACT_CHARS);
    let title = entry
        .title
        .map(|t| plain(&t.content))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| full.clone());
    let link = page(&entry.links).map(|l| l.href.clone());
    let head: Vec<String> = [
        link.clone(),
        entry.updated.or(entry.published).map(|t| t.to_rfc3339()),
    ]
    .into_iter()
    .flatten()
    .collect();
    let with_head = |text: String| {
        head.iter()
            .cloned()
            .chain([text])
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let more = full.chars().count() > about.chars().count();
    Reading {
        item: Item {
            title: cut(&title, TITLE_CHARS),
            body: with_head(about),
        },
        more: if more {
            Some(More::Text(with_head(cut(&full, TEXT_CHARS))))
        } else {
            link.map(More::Page)
        },
    }
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// The link to the entry's page, among links to the feed itself and files.
fn page(links: &[Link]) -> Option<&Link> {
    links
        .iter()
        .find(|l| l.rel.as_deref().is_none_or(|r| r == "alternate"))
        .or_else(|| links.first())
}

/// The words of a page's article, or none when `html` is not a page.
pub fn page_text(html: &str) -> Option<String> {
    if !html.trim_start().starts_with('<') {
        return None;
    }
    let mut html = html.to_owned();
    for name in [
        "script", "style", "noscript", "template", "svg", "head", "nav", "header", "footer",
        "aside", "form",
    ] {
        html = without(&html, name);
    }
    let article = ["article", "main"]
        .iter()
        .find_map(|name| {
            let (start, end) = element(&html, name)?;
            Some(&html[start..end])
        })
        .or_else(|| Some(&html[opening(&html, "h1")?..]))
        .unwrap_or(&html);
    Some(cut(&plain(article), TEXT_CHARS)).filter(|text| !text.is_empty())
}

/// `html` without any element called `name`.
fn without(html: &str, name: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some((start, end)) = element(rest, name) {
        out.push_str(&rest[..start]);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Where the first element called `name` starts and ends; one that is never
/// closed goes on to the end.
fn element(html: &str, name: &str) -> Option<(usize, usize)> {
    let start = opening(html, name)?;
    let close = format!("</{name}");
    let end = html[start..]
        .to_ascii_lowercase()
        .find(&close)
        .and_then(|at| Some(start + at + html[start + at..].find('>')? + 1))
        .unwrap_or(html.len());
    Some((start, end))
}

fn opening(html: &str, name: &str) -> Option<usize> {
    let open = format!("<{name}");
    let lower = html.to_ascii_lowercase();
    lower.match_indices(&open).map(|(at, _)| at).find(|at| {
        lower[at + open.len()..].starts_with(|c: char| c == '>' || c == '/' || c.is_whitespace())
    })
}

/// The words of HTML, without its tags.
fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let tag = after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!');
        match after.find('>').filter(|_| tag) {
            Some(end) => {
                out.push(' ');
                rest = &after[end + 1..];
            }
            None => {
                out.push('<');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    // `&amp;` comes last, so that what it escapes is not read as an entity.
    for (entity, c) in [
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&apos;", "'"),
        ("&nbsp;", " "),
        ("&amp;", "&"),
    ] {
        out = out.replace(entity, c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::lookout::items_from;
    use crate::mcp::ToolResult;

    pub(in crate::lookout) fn answer(text: &str) -> ToolResult {
        ToolResult {
            content: vec![],
            structured_content: None,
            is_error: false,
            text: text.into(),
        }
    }

    pub(in crate::lookout) const RSS: &str = r#"<?xml version="1.0" encoding="Shift_JIS"?>
<rss version="2.0"><channel><title>知らせ</title><link>https://example.com</link>
<item><title>A &amp; B の脆弱性</title><link>https://example.com/1</link>
<pubDate>Mon, 23 Nov 2026 00:00:00 GMT</pubDate>
<description><![CDATA[<p>遠隔から<b>実行</b>できる</p><p>3 &lt; 4</p>]]></description></item>
<item><title>二つ目</title><link>https://example.com/2</link></item>
</channel></rss>"#;

    #[test]
    fn each_entry_of_a_feed_is_read_by_its_title_and_abstract_first() {
        let text = "本".repeat(5000);
        let feed = format!(
            r#"<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/"><channel><title>t</title>
<item><title>長い</title><link>https://example.com/1</link><description>要約</description>
<content:encoded>{text}</content:encoded></item>
<item><description>{text}</description></item></channel></rss>"#
        );

        let readings = items_from(&answer(&feed));

        assert_eq!(readings[0].item.title, "長い");
        assert_eq!(readings[0].item.body, "https://example.com/1\n要約");
        assert_eq!(
            readings[0].more,
            Some(More::Text(format!(
                "https://example.com/1\n{}",
                "本".repeat(4000)
            )))
        );
        assert_eq!(
            readings[1].item.title,
            "本".repeat(120),
            "named by its text"
        );
        assert_eq!(readings[1].item.body, "本".repeat(300));
        assert!(matches!(readings[1].more, Some(More::Text(_))));
    }

    #[test]
    fn an_entry_with_no_more_than_its_abstract_has_its_page_to_open() {
        let readings = items_from(&answer(RSS));

        assert_eq!(
            readings,
            [
                Reading {
                    item: Item {
                        title: "A & B の脆弱性".into(),
                        body: "https://example.com/1\n2026-11-23T00:00:00+00:00\n遠隔から 実行 できる 3 < 4".into(),
                    },
                    more: Some(More::Page("https://example.com/1".into())),
                },
                Reading {
                    item: Item {
                        title: "二つ目".into(),
                        body: "https://example.com/2".into(),
                    },
                    more: Some(More::Page("https://example.com/2".into())),
                },
            ]
        );
    }

    #[test]
    fn atom_and_rdf_feeds_have_entries_too() {
        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>f</title><id>f</id>
<updated>2026-11-23T00:00:00Z</updated>
<entry><title>e</title><id>1</id><link rel="self" href="https://example.com/self"/><link href="https://example.com/e"/>
<published>2026-11-22T00:00:00Z</published><updated>2026-11-23T00:00:00Z</updated>
<content type="html">&lt;p&gt;本文&lt;/p&gt;</content></entry></feed>"#;
        let rdf = r#"<rdf:RDF xmlns="http://purl.org/rss/1.0/" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<channel rdf:about="https://example.com"><title>c</title><link>https://example.com</link><description>d</description></channel>
<item rdf:about="https://example.com/r"><title>r</title><link>https://example.com/r</link><description>要約</description></item></rdf:RDF>"#;

        assert_eq!(
            items_from(&answer(atom))[0].item.body,
            "https://example.com/e\n2026-11-23T00:00:00+00:00\n本文"
        );
        assert_eq!(
            items_from(&answer(rdf))[0].item.body,
            "https://example.com/r\n要約"
        );
    }

    #[test]
    fn a_feed_without_entries_has_no_items_and_xml_that_is_no_feed_is_one() {
        let empty = "\u{feff}<rss version=\"2.0\"><channel><title>t</title></channel></rss>";

        assert!(items_from(&answer(empty)).is_empty());
        assert_eq!(items_from(&answer("<html><p>hi</p></html>")).len(), 1);
        assert_eq!(items_from(&answer("<rss version=\"2.0\"><chan")).len(), 1);
    }

    #[test]
    fn a_pages_text_is_its_article_without_what_surrounds_it() {
        let page = |html: &str| page_text(html);

        assert_eq!(
            page("<!DOCTYPE html><html><head><title>t</title><style>a{}</style></head><body><nav>menu</nav><main><h1>題</h1><script>if (a < b) {}</script><p>本文</p></main><footer>f</footer></body></html>").as_deref(),
            Some("題 本文")
        );
        assert_eq!(
            page("<html><body><div>menu</div><H1 class=\"a\">題</H1><p>本文</p></body></html>")
                .as_deref(),
            Some("題 本文")
        );
        assert_eq!(
            page(&format!("<p>{}</p>", "あ".repeat(5000)))
                .unwrap()
                .chars()
                .count(),
            4000
        );
        assert_eq!(page("%PDF-1.7 binary"), None);
        assert_eq!(page("<html><script>only"), None);
    }
}
