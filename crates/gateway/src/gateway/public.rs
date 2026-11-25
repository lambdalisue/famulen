//! Pages of the public web, read as anyone may read them: with no account,
//! and never from this machine or the network it is in.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use url::{Host, Url};

use super::Gateway;
use crate::log::{LogEntry, digest};

/// What the log names the public web by.
const WEB: &str = "web";
const TIMEOUT: Duration = Duration::from_secs(6);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_REDIRECTS: usize = 4;
/// Nothing of the owner's is told.
const AGENT: &str = "Mozilla/5.0 (compatible; Famulen link preview)";

/// What to read of the public web.
#[derive(Debug, Clone, Copy)]
pub struct Wanted<'a> {
    /// The media types taken, without their parameters.
    pub types: &'a [&'a str],
    /// How many bytes of the body are read.
    pub most: usize,
    /// Whether the beginning of a longer body does.
    pub cut: bool,
    /// The hosts asked, redirects included; any when there are none.
    pub hosts: &'a [&'a str],
}

/// What was read.
#[derive(Debug, Clone)]
pub struct PublicPage {
    /// Where it was in the end.
    pub url: Url,
    /// As the server named it, parameters included.
    pub content_type: String,
    pub body: Vec<u8>,
}

impl PublicPage {
    /// The media type, without its parameters.
    pub fn media_type(&self) -> String {
        media_type(&self.content_type)
    }
}

fn media_type(content_type: &str) -> String {
    let name = content_type.split(';').next().unwrap_or_default();
    name.trim().to_ascii_lowercase()
}

/// Whether an address is on the public internet, so that what answers there
/// is neither this machine, its network nor the cloud it runs in.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

/// Every block IANA keeps for a special purpose is left out.
fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a, b) == (169, 254)
        || (a, b) == (192, 168)
        || (a == 100 && (64..128).contains(&b))
        || (a == 172 && (16..32).contains(&b))
        || (a == 198 && (18..20).contains(&b))
        || [
            (192, 0, 0),
            (192, 0, 2),
            (192, 31, 196),
            (192, 52, 193),
            (192, 88, 99),
            (192, 175, 48),
            (198, 51, 100),
            (203, 0, 113),
        ]
        .contains(&(a, b, c)))
}

/// Only what is handed out for global use, less the blocks of it that carry
/// an IPv4 address within or are kept for a special purpose.
fn is_public_v6(ip: Ipv6Addr) -> bool {
    let [a, b, ..] = ip.segments();
    (0x2000..0x4000).contains(&a)
        && !(a == 0x2001 && b < 0x200)
        && !(a == 0x2001 && b == 0xdb8)
        && a != 0x2002
        && !(a == 0x2620 && b == 0x4f)
        && !(a == 0x3fff && b < 0x1000)
        && a != 0x3ffe
}

/// Whether two addresses are in one network as a home or an office has it. A
/// public address of the owner's own network is no more open to anyone than
/// a private one, when a firewall stands before it.
fn same_network(a: IpAddr, b: IpAddr) -> bool {
    match (a, b) {
        (IpAddr::V4(a), IpAddr::V4(b)) => a.octets()[..3] == b.octets()[..3],
        (IpAddr::V6(a), IpAddr::V6(b)) => a.segments()[..3] == b.segments()[..3],
        _ => false,
    }
}

/// Whether what answers at `ip` is open to anyone and outside the network this
/// machine reaches it from.
fn may_reach(ip: IpAddr) -> bool {
    is_public(ip) && !own_address_toward(ip).is_some_and(|own| same_network(own, ip))
}

/// The address this machine would reach `ip` from. Nothing is sent.
fn own_address_toward(ip: IpAddr) -> Option<IpAddr> {
    let any: IpAddr = match ip {
        IpAddr::V4(_) => Ipv4Addr::UNSPECIFIED.into(),
        IpAddr::V6(_) => Ipv6Addr::UNSPECIFIED.into(),
    };
    let socket = std::net::UdpSocket::bind((any, 0)).ok()?;
    socket.connect((ip, 443)).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Why a name was not resolved.
#[derive(Debug)]
struct NoPublicAddress;

const NO_PUBLIC_ADDRESS: &str = "the name has no public address";

impl std::fmt::Display for NoPublicAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(NO_PUBLIC_ADDRESS)
    }
}

impl std::error::Error for NoPublicAddress {}

#[derive(Debug, Clone, Default)]
struct Reach {
    /// Set by tests alone, whose servers are on this machine.
    also: Vec<SocketAddr>,
}

impl Reach {
    /// Why `url` is not asked for. A name is checked once it is resolved.
    fn check(&self, url: &Url) -> Result<(), &'static str> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err("not a page of the web");
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("the address carries a name or a password");
        }
        let ip = match url.host() {
            Some(Host::Domain(_)) => None,
            Some(Host::Ipv4(ip)) => Some(IpAddr::V4(ip)),
            Some(Host::Ipv6(ip)) => Some(IpAddr::V6(ip)),
            None => return Err("the address names no host"),
        };
        if let (Some(ip), Some(port)) = (ip, url.port())
            && self.also.contains(&SocketAddr::new(ip, port))
        {
            return Ok(());
        }
        // What a service of the owner's own listens at is rarely one of these.
        if url.port().is_some() {
            return Err("not a port of the web");
        }
        match ip {
            Some(ip) if !may_reach(ip) => Err("not a public address"),
            _ => Ok(()),
        }
    }
}

/// Resolves a name to its public addresses alone. The connection is made to
/// what is given here, so a name cannot lead elsewhere once it was checked.
struct PublicNames;

impl Resolve for PublicNames {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let found: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 0))
                .await?
                .filter(|at| may_reach(at.ip()))
                .collect();
            if found.is_empty() {
                return Err(NoPublicAddress.into());
            }
            Ok(Box::new(found.into_iter()) as Addrs)
        })
    }
}

#[derive(Clone)]
pub(super) struct Public {
    http: reqwest::Client,
    reach: Reach,
}

impl Default for Public {
    fn default() -> Self {
        Self {
            http: reqwest::Client::builder()
                // Each step of a redirect is checked before it is taken.
                .redirect(reqwest::redirect::Policy::none())
                // A proxy would resolve the name itself, unchecked.
                .no_proxy()
                .dns_resolver(Arc::new(PublicNames))
                .connect_timeout(CONNECT_TIMEOUT)
                .user_agent(AGENT)
                .build()
                .expect("a plain HTTP client builds"),
            reach: Reach::default(),
        }
    }
}

/// Why nothing was read.
enum Unread {
    /// The gateway did not ask there.
    Refused(&'static str),
    Failed(String),
}

impl From<reqwest::Error> for Unread {
    fn from(error: reqwest::Error) -> Self {
        let mut source = std::error::Error::source(&error);
        while let Some(why) = source {
            if why.is::<NoPublicAddress>() {
                return Self::Refused(NO_PUBLIC_ADDRESS);
            }
            source = why.source();
        }
        // The log keeps no address.
        Self::Failed(error.without_url().to_string())
    }
}

impl Public {
    async fn read(&self, url: &str, wanted: Wanted<'_>) -> Result<PublicPage, Unread> {
        let mut at = Url::parse(url).map_err(|_| Unread::Refused("not an address"))?;
        for _ in 0..=MAX_REDIRECTS {
            self.reach.check(&at).map_err(Unread::Refused)?;
            if !wanted.hosts.is_empty() && !at.host_str().is_some_and(|h| wanted.hosts.contains(&h))
            {
                return Err(Unread::Refused("not a host asked for"));
            }
            let response = self
                .http
                .get(at.clone())
                .header(ACCEPT, wanted.types.join(", "))
                .send()
                .await?;
            let header = |name| {
                let value = response.headers().get(name);
                value.and_then(|v| v.to_str().ok()).unwrap_or_default()
            };
            if response.status().is_redirection() {
                at = Some(header(LOCATION))
                    .filter(|to| !to.is_empty())
                    .and_then(|to| at.join(to).ok())
                    .ok_or_else(|| Unread::Failed("redirected nowhere".into()))?;
                continue;
            }
            if !response.status().is_success() {
                return Err(Unread::Failed(response.status().as_u16().to_string()));
            }
            let content_type = header(CONTENT_TYPE).to_owned();
            if !wanted.types.contains(&media_type(&content_type).as_str()) {
                return Err(Unread::Failed("not of a type asked for".into()));
            }
            let body = take(response, wanted).await?;
            return Ok(PublicPage {
                url: at,
                content_type,
                body,
            });
        }
        Err(Unread::Failed("redirected too often".into()))
    }
}

async fn take(mut response: reqwest::Response, wanted: Wanted<'_>) -> Result<Vec<u8>, Unread> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let room = wanted.most - body.len();
        if chunk.len() <= room {
            body.extend_from_slice(&chunk);
            continue;
        }
        if !wanted.cut {
            return Err(Unread::Failed("too large".into()));
        }
        body.extend_from_slice(&chunk[..room]);
        break;
    }
    Ok(body)
}

impl Gateway {
    /// Read what is at `url` as anyone on the public internet may.
    pub async fn read_public(
        &self,
        actor: impl AsRef<str>,
        url: impl AsRef<str>,
        wanted: Wanted<'_>,
    ) -> anyhow::Result<PublicPage> {
        let started = Instant::now();
        let read = tokio::time::timeout(TIMEOUT, self.public.read(url.as_ref(), wanted))
            .await
            .unwrap_or_else(|_| Err(Unread::Failed("timed out".into())));
        let mut entry = LogEntry {
            actor: actor.as_ref().to_owned(),
            connection: WEB.into(),
            tool: "web.read".into(),
            decision: "allow".into(),
            ms: started.elapsed().as_millis() as i64,
            ..LogEntry::default()
        };
        match &read {
            Ok(page) => {
                entry.outcome = "200".into();
                entry.digest = digest(&page.body);
            }
            Err(Unread::Refused(why)) => {
                entry.decision = "deny".into();
                entry.reason = (*why).into();
            }
            Err(Unread::Failed(why)) => entry.outcome = format!("error: {why}"),
        }
        self.log().write(&entry)?;
        read.map_err(|unread| match unread {
            Unread::Refused(why) => anyhow::anyhow!(why),
            Unread::Failed(why) => anyhow::anyhow!(why),
        })
    }

    /// Read from a server of a test's own, on this machine, too.
    #[cfg(test)]
    fn reading_also(mut self, at: impl IntoIterator<Item = SocketAddr>) -> Self {
        self.public.reach.also.extend(at);
        self
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::response::{IntoResponse, Redirect};
    use axum::{Router, routing::get};

    use super::*;
    use crate::connections::Connections;
    use crate::store::Store;

    const PAGE: Wanted = Wanted {
        types: &["text/html"],
        most: 10,
        cut: true,
        hosts: &[],
    };

    fn gateway() -> Gateway {
        let store = Store::open_in_memory().unwrap();
        let connections = Connections::new(store.clone());
        Gateway::new(store, connections, reqwest::Client::new(), None)
    }

    async fn serve(router: Router) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        at
    }

    fn html(body: &'static str) -> Router {
        Router::new().route(
            "/",
            get(move || async move { ([(CONTENT_TYPE, "text/html; charset=utf-8")], body) }),
        )
    }

    #[test]
    fn only_addresses_of_the_public_internet_are_public() {
        for ip in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "100.127.255.255",
            "127.0.0.1",
            "127.9.9.9",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.192",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:8.8.8.8",
            "64:ff9b::a9fe:a9fe",
            "fc00::1",
            "fd7a:115c:a1e0::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2001::1",
            "2001:db8::1",
            "2002:7f00:1::",
            "3fff::1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "1.1.1.1",
            "100.63.255.255",
            "100.128.0.1",
            "172.32.0.1",
            "198.20.0.1",
            "2606:4700::1111",
            "2001:4860:4860::8888",
        ] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn this_machine_and_its_network_are_refused_however_they_are_written() {
        let gateway = gateway();
        for url in [
            "http://127.0.0.1/",
            "http://127.1/",
            "http://2130706433/",
            "http://0x7f.0.0.1/",
            "http://0177.0.0.1/",
            "http://127.0.0.1./",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://169.254.169.254/latest/meta-data/",
            "http://192.168.1.1/",
            "http://100.100.100.100/",
            "http://1.1.1.1:7790/",
            "https://example.com:8443/",
            "http://user:secret@example.com/",
            "file:///etc/passwd",
            "ftp://example.com/",
            "not an address",
        ] {
            assert!(
                gateway.read_public("owner", url, PAGE).await.is_err(),
                "{url}"
            );
            let last = &gateway.log().recent(1).unwrap()[0];
            assert_eq!(last.decision, "deny", "{url}");
        }
    }

    #[tokio::test]
    async fn a_name_that_leads_to_this_machine_is_not_resolved() {
        let found = PublicNames.resolve("localhost".parse().unwrap()).await;
        assert!(found.is_err());
        // The name passes the check of the address, and is stopped here.
        let gateway = gateway();
        let refused = gateway
            .read_public("owner", "http://localhost/", PAGE)
            .await;
        assert!(refused.is_err());
        let last = &gateway.log().recent(1).unwrap()[0];
        assert_eq!(
            (&*last.decision, &*last.reason),
            ("deny", NO_PUBLIC_ADDRESS)
        );
    }

    #[test]
    fn a_public_address_next_to_this_machines_is_of_its_network() {
        let same = |a: &str, b: &str| same_network(a.parse().unwrap(), b.parse().unwrap());
        assert!(same("2400:1:2:3::5", "2400:1:2:4::6"));
        assert!(same("8.8.8.8", "8.8.8.1"));
        assert!(!same("2400:1:2:3::5", "2400:1:3::5"));
        assert!(!same("8.8.8.8", "8.8.4.4"));
        assert!(!same("8.8.8.8", "2400::1"));
    }

    #[tokio::test]
    async fn each_redirect_is_checked_before_it_is_taken() {
        let asked = Arc::new(AtomicUsize::new(0));
        let count = asked.clone();
        let inside = serve(Router::new().route(
            "/",
            get(move || async move {
                count.fetch_add(1, Ordering::SeqCst);
                ([(CONTENT_TYPE, "text/html")], "inside").into_response()
            }),
        ))
        .await;
        let to = format!("http://{inside}/");
        let outside =
            serve(Router::new().route("/", get(move || async move { Redirect::temporary(&to) })))
                .await;
        let from = format!("http://{outside}/");

        let refused = gateway().reading_also([outside]);
        assert!(refused.read_public("owner", &from, PAGE).await.is_err());
        assert_eq!(asked.load(Ordering::SeqCst), 0);

        let let_in = gateway().reading_also([outside, inside]);
        let page = let_in.read_public("owner", &from, PAGE).await.unwrap();
        assert_eq!(page.body, b"inside");
        assert_eq!(page.url.as_str(), format!("http://{inside}/"));
    }

    #[tokio::test]
    async fn only_the_hosts_asked_for_are_read_redirects_included() {
        let at = serve(html("page").route(
            "/away",
            get(|| async { Redirect::temporary("http://elsewhere.example/") }),
        ))
        .await;
        let gateway = gateway().reading_also([at]);
        let here = Wanted {
            hosts: &["127.0.0.1"],
            ..PAGE
        };
        let read =
            |path, wanted| gateway.read_public("famulen", format!("http://{at}{path}"), wanted);
        assert!(read("/", here).await.is_ok());
        assert!(read("/away", here).await.is_err());
        assert_eq!(gateway.log().recent(1).unwrap()[0].decision, "deny");
        let there = Wanted {
            hosts: &["elsewhere.example"],
            ..PAGE
        };
        assert!(read("/", there).await.is_err());
        assert_eq!(gateway.log().recent(1).unwrap()[0].decision, "deny");
    }

    #[tokio::test]
    async fn redirects_that_never_end_are_left() {
        let at = serve(Router::new().route("/", get(|| async { Redirect::temporary("/") }))).await;
        let gateway = gateway().reading_also([at]);
        let read = gateway
            .read_public("owner", format!("http://{at}/"), PAGE)
            .await;
        assert_eq!(read.unwrap_err().to_string(), "redirected too often");
    }

    #[tokio::test]
    async fn a_body_is_read_up_to_its_most_and_refused_when_all_of_it_is_needed() {
        let at = serve(html("0123456789abcdef")).await;
        let gateway = gateway().reading_also([at]);
        let url = format!("http://{at}/");
        let page = gateway.read_public("owner", &url, PAGE).await.unwrap();
        assert_eq!(page.body, b"0123456789");
        let whole = Wanted { cut: false, ..PAGE };
        assert!(gateway.read_public("owner", &url, whole).await.is_err());
    }

    #[tokio::test]
    async fn another_type_and_a_failure_are_not_read() {
        let at = serve(
            html("page")
                .route(
                    "/pdf",
                    get(|| async { ([(CONTENT_TYPE, "application/pdf")], "x") }),
                )
                .route(
                    "/private",
                    get(|| async {
                        (
                            reqwest::StatusCode::NOT_FOUND,
                            [(CONTENT_TYPE, "text/html")],
                            "x",
                        )
                    }),
                ),
        )
        .await;
        let gateway = gateway().reading_also([at]);
        for path in ["pdf", "private", "nowhere"] {
            let url = format!("http://{at}/{path}");
            assert!(gateway.read_public("owner", url, PAGE).await.is_err());
        }
    }

    #[tokio::test]
    async fn a_page_read_is_logged_without_where_it_is_or_what_it_says() {
        let at = serve(html("hush")).await;
        let gateway = gateway().reading_also([at]);
        let url = format!("http://{at}/?token=hush");
        gateway.read_public("owner", url, PAGE).await.unwrap();
        let [entry] = &gateway.log().recent(5).unwrap()[..] else {
            panic!("not one entry");
        };
        assert_eq!((&*entry.connection, &*entry.outcome), ("web", "200"));
        assert!(!serde_json::to_string(entry).unwrap().contains("hush"));
        assert!(!serde_json::to_string(entry).unwrap().contains("127.0.0.1"));
    }
}
