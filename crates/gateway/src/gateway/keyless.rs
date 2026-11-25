//! Connections anyone granted may add: those that need no secret, and reach
//! only the public internet.

use std::collections::BTreeMap;

use serde_json::Value;

use super::Gateway;
use crate::connections::{Connection, Transport};
use crate::definitions::Definition;

/// How long a name may be.
const MAX_NAME_CHARS: usize = 100;

/// A value as it is told from another: what a URL asks for, however it is
/// written.
fn same(value: &str) -> String {
    let value = value.trim();
    let Ok(mut url) = url::Url::parse(value) else {
        return value.to_owned();
    };
    if !matches!(url.scheme(), "http" | "https") {
        return value.to_owned();
    }
    url.set_fragment(None);
    let host = url.host_str().unwrap_or_default();
    let host = host.trim_end_matches('.').to_owned();
    // A host that cannot be set is left as it was written.
    let _ = url.set_host(Some(&host));
    url.into()
}

fn told_apart(values: &BTreeMap<String, String>) -> BTreeMap<&str, String> {
    values
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| (k.as_str(), same(v)))
        .collect()
}

impl Gateway {
    /// Add, as `actor`, a connection made from a definition that needs no
    /// secret. It reaches only the public internet, and there is one for the
    /// same values.
    pub fn add_keyless(
        &self,
        actor: impl AsRef<str>,
        definition: impl AsRef<str>,
        values: BTreeMap<String, String>,
        name: Option<&str>,
    ) -> anyhow::Result<Connection> {
        let _adding = crate::lock::lock(&self.adding);
        let (def, values) = self.check_keyless(definition, values, name)?;
        let name = name.map(str::trim).filter(|n| !n.is_empty());
        self.add_made_from(&def, values, name, actor.as_ref(), true)
    }

    /// Refuse what [`Self::add_keyless`] would not add; the definition and
    /// the values it would add.
    pub fn check_keyless(
        &self,
        definition: impl AsRef<str>,
        values: BTreeMap<String, String>,
        name: Option<&str>,
    ) -> anyhow::Result<(Definition, BTreeMap<String, String>)> {
        let definition = definition.as_ref().trim();
        let def = self.need_definition(definition)?;
        if let Some(why) = def.only_by_owner() {
            anyhow::bail!("only the owner adds a connection of {definition}: {why}");
        }
        let values: BTreeMap<String, String> = values
            .into_iter()
            .map(|(k, v)| (k, v.trim().to_owned()))
            .filter(|(_, v)| !v.is_empty())
            .collect();
        for (key, value) in &values {
            let field = def.fields.iter().find(|f| &f.name == key);
            let field =
                field.ok_or_else(|| anyhow::anyhow!("{definition} has no account field {key}"))?;
            anyhow::ensure!(
                !value.contains(char::is_control),
                "{} holds a control character",
                field.label
            );
        }
        let name = name.unwrap_or_default().trim();
        anyhow::ensure!(
            !name.contains(char::is_control) && name.chars().count() <= MAX_NAME_CHARS,
            "the name holds a control character or is longer than {MAX_NAME_CHARS} characters"
        );
        self.check_base(&def, &values)?;
        if let Some(there) = self.made_with(definition, &values)? {
            anyhow::bail!("it is already connected as {there}");
        }
        Ok((def, values))
    }

    /// Refuse values that lead the definition anywhere but to a public
    /// `https://` address.
    fn check_base(
        &self,
        def: &Definition,
        values: &BTreeMap<String, String>,
    ) -> anyhow::Result<()> {
        let missing = def
            .fields
            .iter()
            .find(|f| !f.optional && !values.contains_key(&f.name));
        if let Some(field) = missing {
            anyhow::bail!("{} is required", field.label);
        }
        let ctx = crate::template::Context {
            account: values.clone(),
            args: Value::Null,
            ..Default::default()
        };
        let http = def.transport.http.as_ref();
        let base = http
            .and_then(|h| ctx.render_text(&h.base))
            .ok_or_else(|| anyhow::anyhow!("a value of its address is missing"))?;
        self.public
            .check_secure(&base)
            // The URL may hold a password, and this is logged.
            .map_err(|why| anyhow::anyhow!("its address cannot be connected to: {why}"))
    }

    /// The connection already made from `definition` with the same values.
    fn made_with(
        &self,
        definition: &str,
        values: &BTreeMap<String, String>,
    ) -> anyhow::Result<Option<String>> {
        let wanted = told_apart(values);
        Ok(self.connections.list()?.into_iter().find_map(|c| {
            let Transport::Defined {
                definition: made_from,
                values,
                ..
            } = &c.transport
            else {
                return None;
            };
            (made_from == definition && told_apart(values) == wanted).then_some(c.id)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::Connections;
    use crate::store::Store;
    use axum::response::Redirect;
    use axum::{Router, routing::get};
    use serde_json::json;
    use std::net::SocketAddr;

    const JVN: &str = "https://jvn.jp/rss/jvn.rdf";

    fn gateway() -> Gateway {
        let store = Store::open_in_memory().unwrap();
        let connections = Connections::new(store.clone());
        Gateway::new(store, connections, reqwest::Client::new(), None)
    }

    fn url(value: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("url".to_owned(), value.to_owned())])
    }

    fn add(g: &Gateway, value: &str) -> anyhow::Result<Connection> {
        g.add_keyless("secretary", "rss", url(value), None)
    }

    #[test]
    fn a_connection_that_needs_no_secret_is_added_under_its_callers_name() {
        let g = gateway();
        let c = g
            .add_keyless("secretary", "rss", url(&format!(" {JVN} ")), Some(" JVN\n"))
            .unwrap();

        assert_eq!((c.name.as_str(), c.account.as_str()), ("JVN", JVN));
        assert_eq!(
            c.transport,
            Transport::Defined {
                definition: "rss".into(),
                values: url(JVN),
                public: true,
            }
        );
        let entry = &g.log().recent(1).unwrap()[0];
        assert_eq!(
            (entry.actor.as_str(), entry.tool.as_str()),
            ("secretary", "connection.add")
        );
        assert_eq!(
            g.add_keyless(
                "secretary",
                "rss",
                url("https://rustsec.org/feed.xml"),
                Some(" ")
            )
            .unwrap()
            .name,
            "RSS"
        );
    }

    #[test]
    fn of_the_bundled_definitions_only_rss_is_not_left_to_the_owner() {
        let g = gateway();
        let definitions = g.definitions();
        let open: Vec<_> = definitions
            .list()
            .filter(|d| d.only_by_owner().is_none())
            .map(|d| d.id.as_str())
            .collect();
        assert_eq!(open, ["rss"]);
        for def in definitions.list().filter(|d| d.id != "rss") {
            let refused = g.add_keyless("secretary", &def.id, url(JVN), None);
            let why = refused.unwrap_err().to_string();
            assert!(why.contains("only the owner"), "{}: {why}", def.id);
        }
        assert!(g.connections.list().unwrap().is_empty());
    }

    #[test]
    fn a_definition_that_needs_a_secret_or_reaches_further_is_left_to_the_owner() {
        let plain = r#"format = 1
id = "plain"
name = "Plain"
[transport]
http = { base = "https://plain.example" }
"#;
        let http = r#"http = { base = "https://plain.example" }"#;
        let parse = |text: &str| crate::definitions::parse(text).unwrap();
        assert_eq!(parse(plain).only_by_owner(), None);
        for (from, to) in [
            (
                "[transport]",
                "[login]\nkey = { label = \"k\", header = \"X\", value = \"{token}\" }\n[transport]",
            ),
            (
                "[transport]",
                "[account]\nk = { label = \"k\", optional = true, secret = true }\n[transport]",
            ),
            (
                "[transport]",
                "[account]\nk = { generate = true }\n[transport]",
            ),
            (
                http,
                "http = { base = \"https://plain.example\", openapi = \"{account.place}\" }",
            ),
            (http, "mcp_remote = { url = \"https://plain.example\" }"),
            (http, "mcp_local = { command = \"sh\" }"),
        ] {
            let text = plain.replace(from, to);
            assert_ne!(text, plain);
            assert!(parse(&text).only_by_owner().is_some(), "{text}");
        }
    }

    #[test]
    fn only_a_public_https_url_is_taken() {
        let g = gateway();
        for value in [
            "",
            "jvn.jp/rss/jvn.rdf",
            "http://jvn.jp/rss/jvn.rdf",
            "ftp://jvn.jp/",
            "file:///etc/passwd",
            "https://localhost/feed",
            "https://LOCALHOST./feed",
            "https://a.localhost/feed",
            "https://nas/feed",
            "https://nas.local/feed",
            "https://metadata.google.internal/",
            "https://127.0.0.1/feed",
            "https://2130706433/feed",
            "https://[::1]/feed",
            "https://10.0.0.1/feed",
            "https://192.168.1.1/feed",
            "https://169.254.169.254/latest/meta-data/",
            "https://100.100.100.100/feed",
            "https://jvn.jp:8443/feed",
            "https://me:hush@jvn.jp/feed",
            "https://jvn.jp/\nfeed",
        ] {
            assert!(add(&g, value).is_err(), "{value}");
        }
        let mut values = url(JVN);
        values.insert("token".into(), "hush".into());
        assert!(g.add_keyless("secretary", "rss", values, None).is_err());
        let refused = add(&g, "https://me:hush@jvn.jp/feed").unwrap_err();
        assert!(!refused.to_string().contains("hush"), "{refused}");
        for name in ["a\nb".to_owned(), "あ".repeat(101)] {
            assert!(
                g.add_keyless("secretary", "rss", url(JVN), Some(&name))
                    .is_err()
            );
        }
        assert!(
            g.add_keyless("secretary", "nothing", url(JVN), None)
                .is_err()
        );
        assert!(g.connections.list().unwrap().is_empty());
    }

    #[test]
    fn the_same_values_are_connected_once_however_they_are_written() {
        let g = gateway();
        let first = add(&g, JVN).unwrap();
        for value in [
            JVN,
            "https://JVN.jp/rss/jvn.rdf",
            "https://jvn.jp:443/rss/jvn.rdf",
            "https://jvn.jp./rss/jvn.rdf",
            "https://jvn.jp/rss/jvn.rdf#top",
        ] {
            let refused = add(&g, value).unwrap_err().to_string();
            assert!(refused.contains(&first.id), "{refused}");
        }
        g.add_defined("rss", url("https://rustsec.org/feed.xml"), None)
            .unwrap();
        assert!(add(&g, "https://rustsec.org/feed.xml").is_err());
        assert!(add(&g, "https://jvn.jp/rss/other.rdf").is_ok());
        assert_eq!(g.connections.list().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_definition_that_came_to_need_more_is_no_longer_reached() {
        let g = gateway();
        let c = add(&g, JVN).unwrap();
        let more = include_str!("../../services/rss/rss.toml").replace(
            "http = { base = \"{account.url}\" }",
            "http = { base = \"{account.url}\", openapi = \"/etc/hosts\" }",
        );
        let g = g.with_definitions(None, &[("rss.toml".into(), more)]);

        let why = g.tools_of(&c.id).await.unwrap_err().to_string();

        assert!(why.contains("only the owner"), "{why}");
    }

    async fn serve(router: Router) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        at
    }

    #[tokio::test]
    async fn where_it_leads_is_checked_each_time_it_is_called() {
        let inside = serve(Router::new().route("/", get(|| async { "inside" }))).await;
        let to = format!("http://{inside}/");
        let outside = serve(
            Router::new()
                .route("/feed", get(|| async { "feed" }))
                .route(
                    "/sjis",
                    get(|| async {
                        (
                            [("content-type", "text/xml; charset=Shift_JIS")],
                            b"\x93\xfa\x96\x7b".to_vec(),
                        )
                    }),
                )
                .route(
                    "/clear",
                    get(|| async { Redirect::temporary("http://jvn.jp/") }),
                )
                .route(
                    "/much",
                    get(|| async { vec![b'x'; crate::gateway::MAX_FILE_BYTES + 1] }),
                )
                .route(
                    "/away",
                    get(move || async move { Redirect::temporary(&to) }),
                ),
        )
        .await;
        let g = gateway().reading_also([outside]);
        let c = add(&g, JVN).unwrap();
        let read = async |tool: &str, value: String, args: Value| {
            let to = Transport::Defined {
                definition: "rss".into(),
                values: url(&value),
                public: true,
            };
            g.connections.set_transport(&c.id, &to).unwrap();
            g.call_as_owner(&c, tool, args).await
        };

        let feed = format!("http://{outside}/feed");
        assert_eq!(
            read("get_feed", feed, json!({})).await.unwrap().text,
            "feed"
        );
        let sjis = format!("http://{outside}/sjis");
        assert_eq!(
            read("get_feed", sjis, json!({})).await.unwrap().text,
            "日本"
        );
        for value in [
            format!("http://{inside}/"),
            format!("http://{outside}/away"),
            format!("http://{outside}/clear"),
            format!("http://{outside}/much"),
            "http://jvn.jp/rss/jvn.rdf".to_owned(),
            "https://localhost/".to_owned(),
        ] {
            assert!(
                read("get_feed", value.clone(), json!({})).await.is_err(),
                "{value}"
            );
            let page = json!({ "url": value.clone() });
            assert!(
                read("get_page", value.clone(), page).await.is_err(),
                "{value}"
            );
        }
    }
}
