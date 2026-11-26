//! Whether a page of the web is one anyone on the internet may read.

use std::collections::HashMap;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use url::{Host, Url};

use super::sources::Looks;
use crate::gateway::{Gateway, Wanted, is_local_name, is_public};

/// Ends of names that the internet does not hand out, besides those the
/// gateway knows.
const UNDELEGATED: &[&str] = &[
    ".corp",
    ".example",
    ".home",
    ".intranet",
    ".invalid",
    ".lan",
    ".private",
    ".test",
];

/// Hosts whose links carry their own key in the path: whoever has the link
/// reads it, and only they.
const KEYED_LINKS: &[&str] = &[
    "1drv.ms",
    "box.com",
    "docs.google.com",
    "drive.google.com",
    "dropbox.com",
    "figma.com",
    "files.slack.com",
    "gist.github.com",
    "hackmd.io",
    "icloud.com",
    "notion.site",
    "notion.so",
    "onedrive.live.com",
    "pastebin.com",
    "sharepoint.com",
    "slack-files.com",
    "we.tl",
    "wetransfer.com",
];

/// The shortest run of letters and digits mixed that is taken for a key in a
/// link's path.
const SHORTEST_KEY: usize = 16;

/// Whether a part of a link's path looks like a key: a long run of letters
/// and digits mixed, or a UUID.
fn looks_like_a_key(segment: &str) -> bool {
    let mixed = |part: &str| {
        part.chars().count() >= SHORTEST_KEY
            && part.chars().all(|c| c.is_ascii_alphanumeric())
            && part.chars().any(|c| c.is_ascii_digit())
            && part.chars().any(|c| c.is_ascii_alphabetic())
    };
    let groups: Vec<&str> = segment.split('-').collect();
    let uuid = groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12])
        && groups
            .iter()
            .all(|g| g.chars().all(|c| c.is_ascii_hexdigit()));
    uuid || segment.split(['-', '_', '.']).any(mixed)
}

/// Words in the name of a query's field that carry a key.
const KEYS: &[&str] = &[
    "auth",
    "code",
    "credential",
    "jwt",
    "key",
    "pass",
    "pwd",
    "secret",
    "session",
    "sid",
    "sig",
    "ticket",
    "token",
];

/// How long a name is waited for. It is asked while the model's message is
/// read, so it must not hold that up for long.
const NAME_PATIENCE: Duration = Duration::from_secs(2);

/// Whether `url` names a page that anyone may ask for as it is: no account and
/// no key in it, on a name the internet hands out or a public address.
pub(super) fn opens_to_anyone(url: &str) -> bool {
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    let keyed = url.query_pairs().any(|(name, _)| {
        let name = name.to_ascii_lowercase();
        KEYS.iter().any(|key| name.contains(key))
    });
    let host = match url.host() {
        Some(Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            let under = |host: &&str| name == *host || name.ends_with(&format!(".{host}"));
            !is_local_name(&name)
                && !UNDELEGATED.iter().any(|end| name.ends_with(end))
                && !KEYED_LINKS.iter().any(under)
        }
        Some(Host::Ipv4(ip)) => is_public(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => is_public(IpAddr::V6(ip)),
        None => false,
    };
    let keyed = keyed
        || url
            .path_segments()
            .is_some_and(|mut segments| segments.any(looks_like_a_key));
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && !keyed
        && host
}

/// Whether the page at `url` is read from the public internet: it opens to
/// anyone, and its name leads only to public addresses now.
pub(super) fn is_public_page(url: &str) -> bool {
    if !opens_to_anyone(url) {
        return false;
    }
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    match url.host() {
        Some(Host::Domain(name)) => {
            let at = format!("{name}:{}", url.port_or_known_default().unwrap_or(443));
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let found: Option<Vec<IpAddr>> = at
                    .to_socket_addrs()
                    .ok()
                    .map(|addrs| addrs.map(|a| a.ip()).collect());
                let _ = tx.send(found);
            });
            match rx.recv_timeout(NAME_PATIENCE) {
                Ok(Some(found)) => !found.is_empty() && found.into_iter().all(is_public),
                _ => false,
            }
        }
        _ => true,
    }
}

/// How long what was told of a repository holds. A repository made private
/// stops being taken for public within it.
const KEPT_FOR: Duration = Duration::from_secs(15 * 60);

/// Asks GitHub of a repository as anyone may: with no account.
pub(crate) struct Anonymous<'a>(pub(crate) &'a Gateway);

static TOLD: Mutex<Option<HashMap<String, (bool, Instant)>>> = Mutex::new(None);

const REPOSITORY: Wanted = Wanted {
    types: &["application/json"],
    most: 256 * 1024,
    cut: false,
    hosts: &["api.github.com"],
};

impl Looks for Anonymous<'_> {
    fn repository<'a>(&'a self, full_name: &'a str) -> BoxFuture<'a, Option<bool>> {
        async move {
            let kept = TOLD.lock().ok().and_then(|told| {
                let (public, at) = *told.as_ref()?.get(full_name)?;
                (at.elapsed() < KEPT_FOR).then_some(public)
            });
            if kept.is_some() {
                return kept;
            }
            let url = format!("https://api.github.com/repos/{full_name}");
            let page = match self.0.read_public("famulen", &url, REPOSITORY).await {
                Ok(page) => page,
                Err(error) => {
                    tracing::debug!(%error, %full_name, "could not tell whether a repository is public");
                    return None;
                }
            };
            let read: serde_json::Value = serde_json::from_slice(&page.body).ok()?;
            let public = read["private"] == false && read["visibility"] == "public";
            if let Ok(mut told) = TOLD.lock() {
                told.get_or_insert_with(HashMap::new)
                    .insert(full_name.to_owned(), (public, Instant::now()));
            }
            Some(public)
        }
        .boxed()
    }
}
