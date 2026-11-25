//! What the addresses in a message lead to, told as the pages tell it of
//! themselves with the Open Graph protocol.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use famulen_gateway::{Gateway, PublicPage, Wanted};
use futures_util::future::BoxFuture;
use html5gum::{DefaultEmitter, Token, Tokenizer};
use sha2::Digest;
use tokio::sync::Semaphore;
use url::Url;

use crate::console_log::{Message, Who};
use crate::permissions::{Permission, Permissions};

/// The most pages shown for one message.
pub const MOST: usize = 3;
/// The most pages read for one message, as some tell nothing.
const MOST_READ: usize = 6;
/// How long what a page told, or that it told nothing, stands.
const KEPT: Duration = Duration::from_secs(24 * 60 * 60);
const KEPT_FAILED: Duration = Duration::from_secs(10 * 60);
/// How many pages are remembered.
const REMEMBERED: usize = 500;
/// How many pages are read at once.
const AT_ONCE: usize = 4;

pub(crate) const PAGE: Wanted = Wanted {
    types: &["text/html", "application/xhtml+xml"],
    most: 1024 * 1024,
    cut: true,
    hosts: &[],
};
/// Pictures alone: an SVG is a document.
const IMAGE: Wanted = Wanted {
    types: &[
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/avif",
    ],
    most: 5 * 1024 * 1024,
    cut: false,
    hosts: &[],
};

const TITLE_CHARS: usize = 200;
const DESCRIPTION_CHARS: usize = 300;

/// What a page tells of itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// As it stands in the message.
    pub url: String,
    pub site: String,
    pub title: String,
    pub description: String,
    /// What its picture is asked for by.
    pub image: Option<String>,
}

/// What was read of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Told {
    site: String,
    pub(crate) title: String,
    description: String,
    pub(crate) image: Option<Url>,
}

/// The name a picture is asked for by, so that only those a page named can be.
fn image_id(url: &Url) -> String {
    let hash = sha2::Sha256::digest(url.as_str());
    hash.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// The addresses in a message, in the order they come, each page once. Those shown
/// as code are not links.
pub fn links(text: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    let mut fence: Option<String> = None;
    for line in text.lines() {
        let start = line.trim_start();
        let mark: String = match start.chars().next() {
            Some(c @ ('`' | '~')) => start.chars().take_while(|x| *x == c).collect(),
            _ => String::new(),
        };
        match &fence {
            Some(open) => {
                if mark.starts_with(open.as_str()) {
                    fence = None;
                }
            }
            None if mark.len() >= 3 => fence = Some(mark),
            None => {
                for said in outside_code(line) {
                    for url in links_in(said) {
                        if !found.iter().any(|f| page_of(f) == page_of(&url)) {
                            found.push(url);
                        }
                    }
                }
            }
        }
    }
    found
}

/// The page an address is of, whatever part of it it points to.
fn page_of(url: &Url) -> Url {
    let mut page = url.clone();
    page.set_fragment(None);
    page
}

/// The parts of a line that are not between as many backticks as each other.
fn outside_code(line: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('`') {
        let ticks = rest[open..].len() - rest[open..].trim_start_matches('`').len();
        parts.push(&rest[..open]);
        let after = &rest[open + ticks..];
        rest = match closing(after, ticks) {
            Some(end) => &after[end..],
            None => after,
        };
    }
    parts.push(rest);
    parts
}

/// Where a run of just so many backticks ends.
fn closing(text: &str, ticks: usize) -> Option<usize> {
    let mut from = 0;
    while let Some(at) = text[from..].find('`').map(|i| from + i) {
        let run = text[at..].len() - text[at..].trim_start_matches('`').len();
        from = at + run;
        if run == ticks {
            return Some(from);
        }
    }
    None
}

/// Whether an address written in a text ends before `c`.
fn ends(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || "<>\"|{}\\^「」『』【】〈〉《》〔〕（）［］｛｝＜＞、。，．！？：；｜…“”".contains(c)
}

fn links_in(text: &str) -> Vec<Url> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("http").map(|i| from + i) {
        from = at + "http".len();
        let rest = &text[at..];
        let before = text[..at].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric())
            || !(rest.starts_with("http://") || rest.starts_with("https://"))
        {
            continue;
        }
        let end = rest.find(ends).unwrap_or(rest.len());
        let end = unopened(&rest[..end]).unwrap_or(end);
        from = at + end;
        if let Ok(url) = Url::parse(without_tail(&rest[..end], before)) {
            found.push(url);
        }
    }
    found
}

/// Where a bracket closes that none in the address opened, as a Markdown
/// link's does.
fn unopened(url: &str) -> Option<usize> {
    let mut open = 0;
    for (at, c) in url.char_indices() {
        match c {
            '[' => open += 1,
            ']' if open == 0 => return Some(at),
            ']' => open -= 1,
            _ => {}
        }
    }
    None
}

/// An address less what follows it in a sentence, and the mark it is put
/// within with the one `before` it.
fn without_tail(mut url: &str, before: Option<char>) -> &str {
    let mark = before.filter(|c| "*_~'".contains(*c));
    loop {
        let shorter = url.trim_end_matches(|c| ".,:;!?".contains(c) || Some(c) == mark);
        let closes = shorter.matches(')').count() > shorter.matches('(').count();
        let shorter = match shorter.strip_suffix(')') {
            Some(inner) if closes => inner,
            _ => shorter,
        };
        if shorter.len() == url.len() {
            return url;
        }
        url = shorter;
    }
}

/// A line of plain text, at most `most` characters.
fn line(text: &str, most: usize) -> String {
    let text = text
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    match text.char_indices().nth(most) {
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
        None => text,
    }
}

/// The encoding a page names, in its type or near its beginning.
fn decoded(page: &PublicPage) -> String {
    let head = String::from_utf8_lossy(&page.body[..page.body.len().min(1024)]);
    let label = [page.content_type.as_str(), &head]
        .into_iter()
        .find_map(|text| {
            let text = text.to_ascii_lowercase();
            let (_, rest) = text.split_once("charset=")?;
            let rest = rest.trim_start_matches(['"', '\'', ' ']);
            let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || "-_:".contains(c)));
            encoding_rs::Encoding::for_label(&rest.as_bytes()[..end.unwrap_or(rest.len())])
        })
        .unwrap_or(encoding_rs::UTF_8);
    label.decode(&page.body).0.into_owned()
}

/// What a page tells of itself; nothing when it does not name itself. Its
/// `<title>` does not do: a page that asks to log in has one too.
pub(crate) fn told(page: &PublicPage) -> Option<Told> {
    let html = decoded(page);
    let mut emitter = DefaultEmitter::default();
    emitter.naively_switch_states(true);
    let mut named: HashMap<String, String> = HashMap::new();
    for token in Tokenizer::new_with_emitter(html.as_str(), emitter) {
        let Ok(token) = token;
        match token {
            Token::StartTag(tag) if tag.name.as_slice() == b"meta" => {
                let attribute = |name: &str| {
                    let value = tag.attributes.get(name.as_bytes())?;
                    Some(String::from_utf8_lossy(value.as_slice()).into_owned())
                };
                let name = attribute("property").or_else(|| attribute("name"));
                if let (Some(name), Some(content)) = (name, attribute("content"))
                    && !content.trim().is_empty()
                {
                    let name = name.trim().to_ascii_lowercase();
                    named.entry(name).or_insert(content);
                }
            }
            Token::StartTag(tag) if tag.name.as_slice() == b"body" => break,
            Token::EndTag(tag) if tag.name.as_slice() == b"head" => break,
            _ => {}
        }
    }
    let text = |name: &str, most| named.get(name).map(|t| line(t, most));
    let title = text("og:title", TITLE_CHARS)?;
    Some(Told {
        title,
        site: text("og:site_name", TITLE_CHARS)
            .unwrap_or_else(|| page.url.host_str().unwrap_or_default().to_owned()),
        description: text("og:description", DESCRIPTION_CHARS).unwrap_or_default(),
        image: ["og:image", "og:image:url", "og:image:secure_url"]
            .iter()
            .find_map(|name| page.url.join(named.get(*name)?.trim()).ok()),
    })
}

/// How pages are read, so that a test can read its own.
type Read =
    Arc<dyn Fn(String, Wanted<'static>) -> BoxFuture<'static, Option<PublicPage>> + Send + Sync>;

struct Kept {
    until: Instant,
    told: Option<Told>,
}

#[derive(Clone)]
pub struct LinkPreviews {
    permissions: Permissions,
    read: Read,
    kept: Arc<Mutex<HashMap<Url, Kept>>>,
    reading: Arc<Semaphore>,
}

impl LinkPreviews {
    pub fn new(gateway: Gateway, permissions: Permissions) -> Self {
        Self::reading(
            permissions,
            Arc::new(move |url, wanted| {
                let gateway = gateway.clone();
                Box::pin(async move {
                    let read = gateway.read_public("console", url, wanted).await;
                    read.inspect_err(|error| tracing::debug!(%error, "no preview of a link"))
                        .ok()
                })
            }),
        )
    }

    fn reading(permissions: Permissions, read: Read) -> Self {
        Self {
            permissions,
            read,
            kept: Arc::default(),
            reading: Arc::new(Semaphore::new(AT_ONCE)),
        }
    }

    /// What the addresses in a message lead to, less those that tell nothing.
    pub async fn of(&self, message: &Message) -> anyhow::Result<Vec<Preview>> {
        if !self.may_read(message.who)? {
            return Ok(Vec::new());
        }
        let urls: Vec<Url> = links(&message.text).into_iter().take(MOST_READ).collect();
        let told =
            futures_util::future::join_all(urls.iter().map(|url| self.told(page_of(url)))).await;
        let previews = urls.into_iter().zip(told).filter_map(|(url, told)| {
            let told = told?;
            Some(Preview {
                url: url.into(),
                image: told.image.as_ref().map(image_id),
                site: told.site,
                title: told.title,
                description: told.description,
            })
        });
        Ok(previews.take(MOST).collect())
    }

    /// Reading what the secretary wrote lets it send what it knows to any
    /// site, as reading a page does, so it goes by what the owner let that do.
    fn may_read(&self, who: Who) -> anyhow::Result<bool> {
        Ok(who == Who::Owner
            || matches!(
                self.permissions.permission("WebFetch")?,
                Permission::Allow | Permission::Auto
            ))
    }

    async fn told(&self, url: Url) -> Option<Told> {
        // Asked again once it is this one's turn, as another may have read it.
        if let Some(known) = self.known(&url) {
            return known;
        }
        let _turn = self.reading.acquire().await.ok()?;
        if let Some(known) = self.known(&url) {
            return known;
        }
        let page = (self.read)(url.to_string(), PAGE).await;
        let told = page.as_ref().and_then(told);
        self.keep(url, told.clone());
        told
    }

    fn known(&self, url: &Url) -> Option<Option<Told>> {
        let kept = self.lock();
        let known = kept.get(url).filter(|k| k.until > Instant::now())?;
        Some(known.told.clone())
    }

    fn keep(&self, url: Url, told: Option<Told>) {
        let now = Instant::now();
        let mut kept = self.lock();
        if kept.len() >= REMEMBERED {
            kept.retain(|_, k| k.until > now);
        }
        if kept.len() >= REMEMBERED
            && let Some(first) = kept.iter().min_by_key(|(_, k)| k.until)
        {
            let first = first.0.clone();
            kept.remove(&first);
        }
        let lasts = if told.is_some() { KEPT } else { KEPT_FAILED };
        let until = now + lasts;
        kept.insert(url, Kept { until, told });
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Url, Kept>> {
        self.kept.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The picture of a page shown, by the name it was given.
    pub async fn image(&self, id: &str) -> Option<PublicPage> {
        let url = self
            .lock()
            .values()
            .filter_map(|k| k.told.as_ref()?.image.clone())
            .find(|url| image_id(url) == id)?;
        let _turn = self.reading.acquire().await.ok()?;
        (self.read)(url.into(), IMAGE).await
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn found(text: &str) -> Vec<String> {
        links(text).into_iter().map(String::from).collect()
    }

    fn page(content_type: &str, body: impl Into<Vec<u8>>) -> PublicPage {
        PublicPage {
            url: Url::parse("https://www.example.com/blog/1").unwrap(),
            content_type: content_type.into(),
            body: body.into(),
        }
    }

    fn html(body: &str) -> Option<Told> {
        told(&page("text/html; charset=utf-8", body))
    }

    #[test]
    fn links_are_found_in_sentences_and_marks() {
        for (text, url) in [
            ("https://example.com/a を見て", "https://example.com/a"),
            ("これ（https://example.com/a）。", "https://example.com/a"),
            ("見た：http://example.com/a。次", "http://example.com/a"),
            ("See https://example.com/a.", "https://example.com/a"),
            ("[名](https://example.com/a)", "https://example.com/a"),
            (
                "[w](https://example.com/a_(b))",
                "https://example.com/a_(b)",
            ),
            ("<https://example.com/a|名>", "https://example.com/a"),
            ("**https://example.com/a**", "https://example.com/a"),
            (
                "https://example.com/?a=1&b=2#part",
                "https://example.com/?a=1&b=2#part",
            ),
            (
                "[https://example.com/a](https://example.com/a)",
                "https://example.com/a",
            ),
            ("https://example.com/?a[]=1", "https://example.com/?a[]=1"),
            ("https://example.com/a_", "https://example.com/a_"),
            ("_https://example.com/a_", "https://example.com/a"),
            ("~~https://example.com/a~~", "https://example.com/a"),
            (
                "https://ja.wikipedia.org/wiki/手帳",
                "https://ja.wikipedia.org/wiki/%E6%89%8B%E5%B8%B3",
            ),
        ] {
            assert_eq!(found(text), [url], "{text}");
        }
    }

    #[test]
    fn code_and_other_schemes_are_no_links() {
        for text in [
            "`https://example.com`",
            "``https://example.com ` ``",
            "```\nhttps://example.com\n```",
            "~~~sh\ncurl https://example.com\n~~~",
            "ftp://example.com mailto:a@example.com xhttps://example.com http のこと",
            "https://",
        ] {
            assert!(found(text).is_empty(), "{text}");
        }
        assert_eq!(
            found("```\ncode\n```\nhttps://example.com/"),
            ["https://example.com/"]
        );
    }

    #[test]
    fn a_link_said_twice_is_one() {
        let text = "https://example.com/a https://example.com/b https://example.com/a#x";
        assert_eq!(
            found(text),
            ["https://example.com/a", "https://example.com/b"]
        );
    }

    #[test]
    fn a_page_is_told_as_it_tells_itself() {
        let told = html(
            r#"<!doctype html><html><head><title>別の題</title>
            <script>"<meta property='og:title' content='script'>"</script>
            <meta property="og:title" content="題 &amp; &lt;b&gt;名&lt;/b&gt;">
            <META NAME='og:description' CONTENT='一行目
              二行目'>
            <meta content="サイト" property="og:site_name"/>
            <meta property="og:image" content="/img/a.png">
            <meta property="og:title" content="二つ目">
            </head><body><meta property="og:title" content="body"></body>"#,
        )
        .unwrap();
        assert_eq!(told.title, "題 & <b>名</b>");
        assert_eq!(told.description, "一行目 二行目");
        assert_eq!(told.site, "サイト");
        assert_eq!(
            told.image.unwrap().as_str(),
            "https://www.example.com/img/a.png"
        );
    }

    #[test]
    fn a_page_that_does_not_name_itself_is_not_told() {
        for body in [
            "",
            "<title>ログイン</title>",
            r#"<meta property="og:title" content=" ">"#,
            r#"<!-- <meta property="og:title" content="x"> -->"#,
            r#"<meta property="og:title" content="cut"#,
        ] {
            assert_eq!(html(body), None, "{body}");
        }
    }

    #[test]
    fn a_site_without_a_name_is_told_by_its_host_and_long_words_are_cut() {
        let body = format!(
            r#"<meta property="og:title" content="t"><meta property="og:description" content="{}">"#,
            "長".repeat(1000)
        );
        let told = html(&body).unwrap();
        assert_eq!(told.site, "www.example.com");
        assert_eq!(told.description.chars().count(), DESCRIPTION_CHARS + 1);
        assert_eq!(told.image, None);
    }

    #[test]
    fn a_page_in_another_encoding_is_read() {
        let body = r#"<meta charset="Shift_JIS"><meta property="og:title" content="手帳">"#;
        let bytes = encoding_rs::SHIFT_JIS.encode(body).0.into_owned();
        assert_eq!(
            told(&page("text/html", bytes.clone())).unwrap().title,
            "手帳"
        );
        let bytes = encoding_rs::EUC_JP
            .encode(r#"<meta property="og:title" content="手帳">"#)
            .0
            .into_owned();
        let named = told(&page("text/html; charset=EUC-JP", bytes)).unwrap();
        assert_eq!(named.title, "手帳");
    }

    /// Reads pages of its own, and keeps what was asked for.
    fn previews(dir: &std::path::Path) -> (LinkPreviews, Arc<Mutex<Vec<String>>>) {
        let app = crate::runtime::testing::app(dir);
        let asked: Arc<Mutex<Vec<String>>> = Arc::default();
        let log = asked.clone();
        let read: Read = Arc::new(move |url: String, _| {
            log.lock().unwrap().push(url.clone());
            Box::pin(async move {
                (!url.contains("private")).then(|| {
                    page(
                        "text/html",
                        r#"<meta property="og:title" content="t"><meta property="og:image" content="/i.png">"#,
                    )
                })
            })
        });
        (LinkPreviews::reading(app.permissions.clone(), read), asked)
    }

    fn said(who: Who, text: &str) -> Message {
        Message {
            id: 1,
            at: 0,
            who,
            text: text.into(),
            meta: Value::Null,
        }
    }

    #[tokio::test]
    async fn what_a_page_told_or_that_it_told_nothing_is_kept_for_a_while() {
        let dir = crate::notebook::testing::TempDir::new("previews-kept");
        let (previews, asked) = previews(dir.path());
        let m = said(
            Who::Owner,
            "https://a.example/ https://a.example/private https://b.example/",
        );
        for _ in 0..2 {
            let shown = previews.of(&m).await.unwrap();
            let urls: Vec<_> = shown.iter().map(|p| p.url.as_str()).collect();
            assert_eq!(urls, ["https://a.example/", "https://b.example/"]);
        }
        assert_eq!(asked.lock().unwrap().len(), 3);

        for kept in previews.lock().values_mut() {
            assert_eq!(
                kept.until > Instant::now() + KEPT_FAILED,
                kept.told.is_some()
            );
            kept.until = Instant::now();
        }
        previews.of(&m).await.unwrap();
        assert_eq!(asked.lock().unwrap().len(), 6);
    }

    #[tokio::test]
    async fn what_is_remembered_does_not_grow_without_end() {
        let dir = crate::notebook::testing::TempDir::new("previews-most");
        let (previews, _) = previews(dir.path());
        for i in 0..REMEMBERED + 5 {
            previews.keep(
                Url::parse(&format!("https://example.com/{i}")).unwrap(),
                None,
            );
        }
        assert_eq!(previews.lock().len(), REMEMBERED);
    }

    #[tokio::test]
    async fn a_message_shows_only_so_many_pages() {
        let dir = crate::notebook::testing::TempDir::new("previews-few");
        let (previews, asked) = previews(dir.path());
        let text: String = (0..10)
            .map(|i| format!("https://example.com/{i} "))
            .collect();
        let shown = previews.of(&said(Who::Aide, &text)).await.unwrap();
        assert_eq!(shown.len(), MOST);
        assert_eq!(asked.lock().unwrap().len(), MOST_READ);
    }

    #[tokio::test]
    async fn pages_that_tell_nothing_leave_the_room_to_those_that_do() {
        let dir = crate::notebook::testing::TempDir::new("previews-room");
        let (previews, _) = previews(dir.path());
        let text = "https://a.example/private https://b.example/private https://c.example/#x";
        let shown = previews.of(&said(Who::Owner, text)).await.unwrap();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].url, "https://c.example/#x");
    }

    #[tokio::test]
    async fn what_the_secretary_wrote_is_read_only_while_it_may_read_pages_unasked() {
        let dir = crate::notebook::testing::TempDir::new("previews-may");
        let (previews, asked) = previews(dir.path());
        let m = |who| said(who, "https://example.com/");
        for permission in [Permission::Confirm, Permission::Forbid] {
            let scope = famulen_gateway::permissions::BUILTINS;
            previews
                .permissions
                .set_scoped("owner", scope, "WebFetch", Some(permission))
                .unwrap();
            assert!(previews.of(&m(Who::Aide)).await.unwrap().is_empty());
        }
        assert!(asked.lock().unwrap().is_empty());
        assert_eq!(previews.of(&m(Who::Owner)).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn only_a_picture_some_page_shown_named_is_read() {
        let dir = crate::notebook::testing::TempDir::new("previews-image");
        let (previews, asked) = previews(dir.path());
        let shown = previews
            .of(&said(Who::Owner, "https://example.com/"))
            .await
            .unwrap();
        let id = shown[0].image.clone().unwrap();
        assert!(previews.image(&id).await.is_some());
        assert_eq!(
            asked.lock().unwrap().last().unwrap(),
            "https://www.example.com/i.png"
        );
        assert!(previews.image("0000").await.is_none());
        assert_eq!(asked.lock().unwrap().len(), 2);
    }
}
