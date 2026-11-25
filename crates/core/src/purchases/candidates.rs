//! What in a mail may be the picture of a thing, or the page that sells it.

use std::collections::BTreeSet;

use html5gum::{DefaultEmitter, StartTag, Token, Tokenizer};
use url::{Url, form_urlencoded};

use crate::link_previews;

/// The most of each that one mail gives.
const MOST: usize = 60;
/// The most of each, and of the mail, that is gone through for them.
const MOST_SEEN: usize = 300;
const MOST_HTML: usize = 2 * 1024 * 1024;
/// How much of the text around one is told, and gone through for that.
const BEFORE: usize = 80;
const AFTER: usize = 160;
const AROUND: usize = 2000;
/// A picture shown smaller than this is a mark or a pixel that tells the mail
/// was opened.
const SMALLEST_SHOWN: f64 = 40.0;
const LONGEST_SHOWN: f64 = 3.0;
/// A word of an address as long as this, of letters and digits, names the
/// reader and not the page.
const TOKEN_CHARS: usize = 20;
/// How much of a name stands in the words that are of the same thing.
const ALIKE: f64 = 0.6;

const PICTURES: &[&str] = &["jpg", "jpeg", "png", "webp"];
const NO_PICTURE: &[&str] = &[
    "logo", "banner", "bnr", "icon", "icons", "spacer", "pixel", "blank", "button", "btn",
    "header", "footer", "beacon", "tracking", "noimage", "noimg", "sns", "rating", "stars",
];
/// What a page does rather than shows, or who it is shown to.
const NO_PAGE: &[&str] = &[
    "unsubscribe",
    "optout",
    "login",
    "signin",
    "logout",
    "register",
    "cancel",
    "order",
    "orders",
    "cart",
    "basket",
    "checkout",
    "mypage",
    "account",
    "member",
    "members",
    "review",
    "reviews",
    "track",
    "tracking",
    "confirm",
    "verify",
    "auth",
    "token",
    "click",
    "redirect",
    "password",
    "return",
    "returns",
    "payment",
    "help",
    "contact",
    "privacy",
    "survey",
    "coupon",
];

/// A picture or a link, with the words it stands among.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Candidate {
    pub url: Url,
    pub words: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Candidates {
    pub images: Vec<Candidate>,
    pub pages: Vec<Candidate>,
    /// How many there were before those that are none were left out.
    pub seen: Seen,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Seen {
    pub images: usize,
    pub links: usize,
}

impl Candidates {
    pub fn are_none(&self) -> bool {
        self.images.is_empty() && self.pages.is_empty()
    }

    pub fn only(mut self, keep: impl Fn(&Candidate) -> bool) -> Self {
        self.images.retain(&keep);
        self.pages.retain(&keep);
        self
    }

    pub fn of_html(html: &str) -> Self {
        let read = Read::of(&html[..html.floor_char_boundary(MOST_HTML)]);
        let mut all = Self {
            seen: Seen {
                images: read.seen_images,
                links: read.links.len(),
            },
            ..Self::default()
        };
        for image in read.images.iter().take(MOST_SEEN) {
            let linked: Vec<&str> = read
                .links
                .iter()
                .take(MOST_SEEN)
                .filter(|l| image.link.is_some() && l.href == read.href(image.link))
                .map(|l| read.said(l.from, l.to))
                .collect();
            let words = line(&format!(
                "{} | {} | {}",
                image.alt,
                linked.join(" "),
                read.around(image.at, image.at)
            ));
            push(&mut all.images, image.url.clone(), words);
        }
        for link in read.links.iter().take(MOST_SEEN) {
            if let Some(url) = page_url(&link.href) {
                let words = line(&format!(
                    "{} | {}",
                    read.said(link.from, link.to),
                    read.around(link.from, link.to)
                ));
                push(&mut all.pages, url, words);
            }
        }
        all
    }

    pub fn of_text(text: &str) -> Self {
        let mut all = Self::default();
        let lines: Vec<&str> = text.lines().take(MOST_SEEN * 10).collect();
        for (i, said) in lines.iter().enumerate() {
            for url in link_previews::links(said).into_iter().take(MOST) {
                all.seen.links += 1;
                if let Some(url) = page_url(url.as_str()) {
                    let words = line(&lines[i.saturating_sub(3)..=i].join(" "));
                    push(&mut all.pages, url, words);
                }
            }
        }
        all
    }
}

/// Once each, with all the words it stands among.
fn push(to: &mut Vec<Candidate>, url: Url, words: String) {
    let room = to.len() < MOST;
    match to.iter_mut().find(|c| c.url == url) {
        Some(known) if known.words.contains(&words) => {}
        Some(known) => known.words = format!("{} | {words}", known.words),
        None if room => to.push(Candidate { url, words }),
        None => {}
    }
}

fn line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Image {
    url: Url,
    alt: String,
    at: usize,
    link: Option<usize>,
}

struct Link {
    href: String,
    from: usize,
    to: usize,
}

/// A mail read through once. Places in it are bytes of its text.
#[derive(Default)]
struct Read {
    text: String,
    images: Vec<Image>,
    links: Vec<Link>,
    seen_images: usize,
}

impl Read {
    fn of(html: &str) -> Self {
        let mut emitter = DefaultEmitter::default();
        emitter.naively_switch_states(true);
        let mut read = Self::default();
        let mut open: Option<usize> = None;
        let mut unshown = false;
        for token in Tokenizer::new_with_emitter(html, emitter) {
            let Ok(token) = token;
            match token {
                Token::StartTag(tag) => match tag.name.as_slice() {
                    b"style" | b"script" | b"title" => unshown = true,
                    b"a" => {
                        open = attribute(&tag, "href").map(|href| {
                            let at = read.text.len();
                            read.links.push(Link {
                                href,
                                from: at,
                                to: at,
                            });
                            read.links.len() - 1
                        });
                    }
                    b"img" => {
                        read.seen_images += 1;
                        if let Some(url) = shown_picture(&tag) {
                            read.images.push(Image {
                                url,
                                alt: attribute(&tag, "alt").unwrap_or_default(),
                                at: read.text.len(),
                                link: open,
                            });
                        }
                    }
                    _ => read.text.push(' '),
                },
                Token::EndTag(tag) => match tag.name.as_slice() {
                    b"style" | b"script" | b"title" => unshown = false,
                    b"a" => {
                        if let Some(link) = open.take() {
                            read.links[link].to = read.text.len();
                        }
                    }
                    _ => read.text.push(' '),
                },
                Token::String(said) if !unshown => {
                    read.text.push_str(&String::from_utf8_lossy(&said.value));
                }
                _ => {}
            }
        }
        read
    }

    fn href(&self, link: Option<usize>) -> &str {
        link.map(|l| self.links[l].href.as_str())
            .unwrap_or_default()
    }

    fn said(&self, from: usize, to: usize) -> &str {
        &self.text[from..to]
    }

    /// The words just before `from` and just after `to`, spaces left out so
    /// that how a mail is laid out does not count.
    fn around(&self, from: usize, to: usize) -> String {
        let near: String = self.text[..from].chars().rev().take(AROUND).collect();
        let near: String = near.chars().rev().collect();
        let before = line(&near);
        let skip = before.chars().count().saturating_sub(BEFORE);
        let before: String = before.chars().skip(skip).collect();
        let near: String = self.text[to..].chars().take(AROUND).collect();
        let after: String = line(&near).chars().take(AFTER).collect();
        format!("{before} | {after}")
    }
}

fn attribute(tag: &StartTag<()>, name: &str) -> Option<String> {
    let value = tag.attributes.get(name.as_bytes())?;
    Some(String::from_utf8_lossy(value.as_slice()).into_owned())
}

/// Where a picture is, unless what shows it tells it is no thing's.
fn shown_picture(tag: &StartTag<()>) -> Option<Url> {
    let style = attribute(tag, "style").unwrap_or_default();
    let style: String = style.to_ascii_lowercase().split_whitespace().collect();
    if style.contains("display:none") || style.contains("visibility:hidden") {
        return None;
    }
    let side = |name: &str| {
        let value = attribute(tag, name)?;
        value.trim().trim_end_matches("px").parse::<f64>().ok()
    };
    let (width, height) = (side("width"), side("height"));
    let small = |side: Option<f64>| side.is_some_and(|s| s < SMALLEST_SHOWN);
    if small(width) || small(height) {
        return None;
    }
    if let (Some(w), Some(h)) = (width, height)
        && (w / h > LONGEST_SHOWN || h / w > LONGEST_SHOWN)
    {
        return None;
    }
    picture_url(&attribute(tag, "src")?)
}

fn web(url: &str) -> Option<Url> {
    let mut url = Url::parse(url.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_scheme("https").ok()?;
    url.set_fragment(None);
    Some(url)
}

/// The words of a path, read as the server reads them.
fn path_words(url: &Url) -> Vec<String> {
    let path = format!("={}", url.path().replace('&', "/"));
    let read: String = form_urlencoded::parse(path.as_bytes())
        .map(|(_, path)| path.to_ascii_lowercase())
        .collect();
    read.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A picture's address without what it may carry of its reader.
fn picture_url(src: &str) -> Option<Url> {
    let mut url = web(src)?;
    url.set_query(None);
    let ends = url.path().rsplit_once('.')?.1.to_ascii_lowercase();
    let words = path_words(&url);
    (PICTURES.contains(&ends.as_str()) && !words.iter().any(|w| NO_PICTURE.contains(&w.as_str())))
        .then_some(url)
}

/// The address of a page that merely shows something: what may act for the
/// owner or tell who they are is not asked for.
fn page_url(href: &str) -> Option<Url> {
    let mut url = web(href)?;
    // A shop leads to its pages by way of a page that counts.
    let within = url
        .query_pairs()
        .find_map(|(_, value)| web(&value).filter(|_| value.starts_with("http")));
    if let Some(within) = within {
        url = within;
    }
    url.set_query(None);
    url.set_fragment(None);
    let words = path_words(&url);
    let token = |w: &String| {
        w.len() >= TOKEN_CHARS
            && w.contains(|c: char| c.is_ascii_digit())
            && w.contains(|c: char| c.is_ascii_alphabetic())
    };
    let no_page = words.iter().any(|w| NO_PAGE.contains(&w.as_str()));
    if words.is_empty() || no_page || words.iter().any(token) {
        return None;
    }
    Some(url)
}

/// The same picture where the shop has it larger.
pub(super) fn larger(url: &Url) -> Option<Url> {
    let host = url.host_str()?;
    if !["media-amazon.com", "ssl-images-amazon.com"]
        .iter()
        .any(|h| host.ends_with(h))
    {
        return None;
    }
    let (folder, file) = url.path().rsplit_once('/')?;
    let (name, ends) = (file.split('.').next()?, file.rsplit('.').next()?);
    let mut to = url.clone();
    to.set_path(&format!("{folder}/{name}._SL500_.{ends}"));
    Some(to)
}

/// Letters and digits alone, written one way.
fn letters(text: &str) -> Vec<char> {
    text.chars()
        .map(|c| match c {
            '！'..='～' => char::from_u32(c as u32 - 0xfee0).unwrap_or(c),
            c => c,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn numbers(text: &[char]) -> BTreeSet<String> {
    let text: String = text.iter().collect();
    text.split(|c: char| !c.is_ascii_digit())
        .filter(|n| !n.is_empty())
        .map(|n| n.trim_start_matches('0').to_owned())
        .collect()
}

fn pairs(text: &[char]) -> BTreeSet<(char, char)> {
    let text: Vec<char> = text
        .iter()
        .copied()
        .filter(|c| c.is_alphanumeric())
        .collect();
    text.windows(2).map(|w| (w[0], w[1])).collect()
}

/// Whether words tell of the thing a name is of: they hold its every number,
/// as of a volume or a model, and most of the rest.
pub(super) fn agrees(name: &str, words: &str) -> bool {
    let (name, words) = (letters(name), letters(words));
    let (of_name, of_words) = (pairs(&name), pairs(&words));
    if of_name.is_empty() || !numbers(&name).is_subset(&numbers(&words)) {
        return false;
    }
    let both = of_name.intersection(&of_words).count();
    both as f64 >= of_name.len() as f64 * ALIKE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(of: &[Candidate]) -> Vec<&str> {
        of.iter().map(|c| c.url.as_str()).collect()
    }

    #[test]
    fn a_thing_s_picture_is_told_with_the_words_it_stands_among() {
        let all = Candidates::of_html(
            r#"<html><head><title>題</title><style>p{}</style></head><body><table><tr>
            <td><a href="https://shop.example/p/1"><img src="http://img.example/a/1.jpg?u=abc" width="115" alt=""></a></td>
            <td><a href="https://shop.example/p/1">暗号の本 3</a><br>1,200 円</td></tr></table></body></html>"#,
        );
        assert_eq!(urls(&all.images), ["https://img.example/a/1.jpg"]);
        assert!(all.images[0].words.contains("暗号の本 3"));
        assert!(!all.images[0].words.contains("p{}") && !all.images[0].words.contains('題'));
        assert_eq!(urls(&all.pages), ["https://shop.example/p/1"]);
    }

    #[test]
    fn marks_pixels_banners_and_what_is_not_shown_are_no_pictures_of_things() {
        for img in [
            r#"<img src="https://a.example/x.jpg" width="1" height="1">"#,
            r#"<img src="https://a.example/x.jpg" width="20px">"#,
            r#"<img src="https://a.example/x.jpg" width="600" height="100">"#,
            r#"<img src="https://a.example/x.jpg" style="Display: none">"#,
            r#"<img src="https://a.example/img/logo.png">"#,
            r#"<img src="https://a.example/img/top_banner.jpg">"#,
            r#"<img src="https://a.example/open?u=1">"#,
            r#"<img src="https://a.example/x.gif">"#,
            r#"<img src="cid:1.jpg"><img src="data:image/png;base64,AA"><img>"#,
        ] {
            assert!(Candidates::of_html(img).are_none(), "{img}");
        }
    }

    #[test]
    fn a_picture_shown_twice_is_one() {
        let img = r#"<img src="https://a.example/x.jpg" alt="甲"> "#;
        assert_eq!(Candidates::of_html(&img.repeat(2)).images.len(), 1);
    }

    #[test]
    fn a_page_is_asked_for_without_what_its_address_tells_of_the_reader() {
        for (href, page) in [
            (
                "http://shop.example/p/1?mail=a%40b.example#top",
                "https://shop.example/p/1",
            ),
            (
                "https://shop.example/gp/r.html?C=1&U=https%3A%2F%2Fshop.example%2Fdp%2FB01%2Fref%3Dx%3Fu%3D1&H=2",
                "https://shop.example/dp/B01/ref=x",
            ),
            (
                "https://shop.example/%E6%9A%97%E5%8F%B7%E3%81%AE%E6%9C%AC/1",
                "https://shop.example/%E6%9A%97%E5%8F%B7%E3%81%AE%E6%9C%AC/1",
            ),
            (
                "https://shop.example/product/100000001003891234/",
                "https://shop.example/product/100000001003891234/",
            ),
        ] {
            assert_eq!(page_url(href).map(String::from).as_deref(), Some(page));
        }
    }

    #[test]
    fn a_page_that_acts_or_tells_who_reads_is_not_asked_for() {
        for href in [
            "https://shop.example/",
            "https://shop.example/unsubscribe/1",
            "https://shop.example/my/order/123",
            "https://shop.example/%75nsubscribe/1",
            "https://shop.example/%E6%9C%AC/a1b2c3d4e5f6a7b8c9d0e1f2",
            "https://shop.example/cancel.html?id=1",
            "https://c.example/ls/a1b2c3d4e5f6a7b8c9d0e1f2",
            "https://c.example/r?to=https%3A%2F%2Fshop.example%2Flogin",
            "mailto:a@example.com",
            "javascript:void(0)",
        ] {
            assert_eq!(page_url(href), None, "{href}");
        }
    }

    #[test]
    fn a_plain_mail_gives_the_pages_under_the_names() {
        let all = Candidates::of_text(
            "・暗号の本\nhttps://shop.example/p/1\n\n注文 https://shop.example/order/1",
        );
        assert_eq!(urls(&all.pages), ["https://shop.example/p/1"]);
        assert!(all.pages[0].words.contains("暗号の本"));
    }

    #[test]
    fn a_mail_gives_only_so_many() {
        let html: String = (0..200)
            .map(|i| format!(r#"<img src="https://a.example/{i}.jpg">"#))
            .collect();
        assert_eq!(Candidates::of_html(&html).images.len(), MOST);
    }

    #[test]
    fn a_name_agrees_with_words_of_the_same_thing_and_no_other_volume_or_model() {
        assert!(agrees(
            "メイドインアビス 10",
            "メイドインアビス（１０） (バンブーコミックス)"
        ));
        assert!(agrees(
            "Kindle Paperwhite",
            "【2021】 KINDLE paperwhite ブラック"
        ));
        assert!(!agrees("メイドインアビス 1", "メイドインアビス 10"));
        assert!(!agrees("イヤホン WH-1000", "イヤホン WH-2000"));
        assert!(!agrees("暗号の本", "電動ドライバー"));
        assert!(!agrees("", "電動ドライバー"));
    }

    #[test]
    fn amazon_has_its_pictures_larger() {
        let url =
            Url::parse("https://m.media-amazon.com/images/I/41ab._SCLZZZ__SX115_.jpg").unwrap();
        assert_eq!(
            larger(&url).unwrap().as_str(),
            "https://m.media-amazon.com/images/I/41ab._SL500_.jpg"
        );
        assert_eq!(
            larger(&Url::parse("https://a.example/1.jpg").unwrap()),
            None
        );
    }
}
