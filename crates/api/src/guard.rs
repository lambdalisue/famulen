//! Which requests the served API takes. It has no login, so a page of another
//! site must not reach it: neither by calling it across sites, nor through a
//! name of its own that it made resolve to this machine.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// The names the API answers to: this machine's, the address it listens at,
/// and those the owner gave.
#[derive(Debug, Clone)]
pub struct Hosts(Arc<BTreeSet<String>>);

impl Hosts {
    pub fn new(listen: SocketAddr, given: &[String]) -> Self {
        let mut names: BTreeSet<String> =
            ["localhost", "127.0.0.1", "[::1]"].map(String::from).into();
        // Every address is no name to reach it by.
        if !listen.ip().is_unspecified() {
            names.insert(match listen.ip() {
                std::net::IpAddr::V4(ip) => ip.to_string(),
                std::net::IpAddr::V6(ip) => format!("[{ip}]"),
            });
        }
        names.extend(given.iter().map(|name| name.to_lowercase()));
        Self(Arc::new(names))
    }

    /// Why a request is refused; nothing when it may go on.
    fn refusal(&self, uri: &Uri, headers: &HeaderMap) -> Option<(StatusCode, &'static str)> {
        let unknown = (StatusCode::MISDIRECTED_REQUEST, "unknown host");
        // HTTP/2 names the host in the URI instead of a header.
        let host = match headers.get(header::HOST) {
            Some(host) => host.to_str().ok(),
            None => uri.authority().map(|a| a.as_str()),
        };
        let Some(host) = host.map(str::to_lowercase) else {
            return Some(unknown);
        };
        if headers.get_all(header::HOST).iter().count() > 1 || !self.0.contains(name_of(&host)) {
            return Some(unknown);
        }
        // Only a browser says where a call comes from, and then truthfully.
        let mut origins = headers.get_all(header::ORIGIN).iter();
        let foreign = origins.any(|origin| {
            let origin = origin.to_str().unwrap_or_default().to_lowercase();
            origin
                .strip_prefix("http://")
                .or(origin.strip_prefix("https://"))
                != Some(host.as_str())
        });
        foreign.then_some((
            StatusCode::FORBIDDEN,
            "calls must not come from another site's pages",
        ))
    }
}

/// `host` without its port.
fn name_of(host: &str) -> &str {
    match host.find(']') {
        Some(end) if host.starts_with('[') => &host[..=end],
        _ => host.split(':').next().unwrap_or(host),
    }
}

/// `router`, taking only what `hosts` lets through.
pub fn guarded(router: axum::Router, hosts: Hosts) -> axum::Router {
    router.layer(axum::middleware::from_fn_with_state(hosts, check))
}

async fn check(State(hosts): State<Hosts>, request: Request, next: Next) -> Response {
    match hosts.refusal(request.uri(), request.headers()) {
        Some(refused) => refused.into_response(),
        None => next.run(request).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    async fn answer(listen: &str, given: &[&str], headers: &[(&str, &str)]) -> u16 {
        let given: Vec<String> = given.iter().map(|g| (*g).to_owned()).collect();
        let router = axum::Router::new().route("/health", axum::routing::get(|| async { "OK" }));
        let mut request = Request::builder().uri("/health");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        guarded(router, Hosts::new(listen.parse().unwrap(), &given))
            .oneshot(request.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    const LISTEN: &str = "127.0.0.1:7790";

    #[tokio::test]
    async fn this_machine_the_listening_address_and_given_names_are_answered() {
        for host in [
            "localhost:7790",
            "LOCALHOST",
            "127.0.0.1:7790",
            "[::1]:7790",
        ] {
            assert_eq!(answer(LISTEN, &[], &[("host", host)]).await, 200, "{host}");
        }
        for (listen, host) in [
            ("100.64.0.1:7790", "100.64.0.1:7790"),
            ("[fd00::1]:7790", "[fd00::1]:7790"),
        ] {
            assert_eq!(answer(listen, &[], &[("host", host)]).await, 200, "{host}");
        }
        let given = &["Backend"];
        assert_eq!(
            answer("0.0.0.0:7790", given, &[("host", "backend:7790")]).await,
            200
        );
    }

    #[tokio::test]
    async fn another_name_is_refused_wherever_it_leads() {
        for host in [
            "evil.example",
            "evil.example:7790",
            "localhost.evil.example",
            "0.0.0.0:7790",
            "[::1",
            "",
        ] {
            assert_eq!(
                answer("0.0.0.0:7790", &[], &[("host", host)]).await,
                421,
                "{host}"
            );
        }
        assert_eq!(answer(LISTEN, &[], &[]).await, 421);
        let twice = [("host", "localhost"), ("host", "evil.example")];
        assert_eq!(answer(LISTEN, &[], &twice).await, 421);
    }

    #[tokio::test]
    async fn another_sites_page_is_refused_and_one_of_its_own_is_not() {
        let from = async |origin| {
            answer(
                LISTEN,
                &[],
                &[("host", "127.0.0.1:7790"), ("origin", origin)],
            )
            .await
        };
        assert_eq!(from("http://127.0.0.1:7790").await, 200);
        for origin in [
            "http://evil.example",
            "http://127.0.0.1:3000",
            "null",
            "",
            "127.0.0.1:7790",
        ] {
            assert_eq!(from(origin).await, 403, "{origin}");
        }
    }
}
