//! The service definitions the gateway ships, checked against the services'
//! own answers kept beside them.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};

    use super::super::{Arrival, Gateway};
    use crate::connections::{Connection, Connections, Secret, Transport};
    use crate::definitions::{Definition, Definitions, Method, parse};
    use crate::permissions::{AutoCourse, Permission, Permissions, auto_course};
    use crate::store::Store;

    const SLACK_OAUTH: &str = include_str!("../../services/slack/examples/oauth.v2.access.json");
    const SLACK_EVENT: &str = include_str!("../../services/slack/examples/socket-events-api.json");
    const GOOGLE_TOKEN: &str = include_str!("../../services/google/examples/token.json");

    fn gateway(env: bool) -> Gateway {
        let store = Store::open_in_memory().unwrap();
        Gateway::new(
            store.clone(),
            Connections::new(store),
            reqwest::Client::new(),
            None,
        )
        .with_env(move |name| env.then(|| format!("value-of-{name}")))
    }

    #[test]
    fn every_bundled_definition_reads() {
        let defs = Definitions::load(None, &[]);
        let ids: Vec<&str> = defs.list().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "adroit",
                "bluesky",
                "discord",
                "github",
                "google",
                "kingoftime",
                "kintone",
                "rss",
                "slack"
            ]
        );
        assert_eq!(defs.broken().count(), 0);
    }

    #[test]
    fn every_argument_of_a_bundled_reading_tool_says_what_it_is_to_the_owner() {
        for def in Definitions::load(None, &[]).list() {
            let written = def
                .tools
                .iter()
                .filter(|t| t.method == Method::Get)
                .map(|t| (&t.name, &t.input));
            let commands = def
                .commands
                .iter()
                .filter(|t| t.read_only)
                .map(|t| (&t.name, &t.input));
            for (tool, input) in written.chain(commands) {
                let input = input.clone().unwrap_or_default();
                for (name, property) in input["properties"].as_object().into_iter().flatten() {
                    for key in ["title", "description"] {
                        assert!(
                            property[key].as_str().is_some_and(|t| !t.is_empty()),
                            "{} {} {name} has no {key}",
                            def.id,
                            tool
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_bundled_receivers_suggest_where_what_they_take_goes() {
        let defs = Definitions::load(None, &[]);
        let suggested = |def: &str, receiver: &str| {
            defs.get(def).unwrap().receive[receiver].destination.clone()
        };

        assert_eq!(suggested("slack", "events").as_deref(), Some("channel"));
        assert_eq!(suggested("google", "mail").as_deref(), Some("lookout"));
        assert_eq!(suggested("google", "calendar").as_deref(), Some("calendar"));
    }

    #[test]
    fn a_slack_install_answer_gives_the_bot_and_owner_tokens_and_the_workspace() {
        let g = gateway(true);
        let def = g.definitions().get("slack").cloned().unwrap();
        let oauth = def.login.as_ref().unwrap().oauth2.clone().unwrap();
        let mut secret = Secret::default();

        let values = g
            .take_tokens(
                &oauth,
                &serde_json::from_str(SLACK_OAUTH).unwrap(),
                &mut secret,
            )
            .unwrap();

        assert_eq!(secret.tokens["bot"], "xoxb-example-bot-token");
        assert_eq!(secret.tokens["user"], "xoxp-example-user-token");
        assert_eq!(values["team"], "T0ACME");
        assert_eq!(values["team_name"], "Acme");
        assert_eq!(values["user"], "U0OWNER");
        assert_eq!(values["bot_user"], "U0BOT");
        assert_eq!(values["app"], "A0123456789");
        assert_eq!(
            secret.tokens.keys().collect::<Vec<_>>(),
            ["bot", "user"],
            "only the tokens are sealed"
        );
        assert_eq!(oauth.same, ["team"]);
    }

    #[test]
    fn a_google_token_answer_gives_the_access_and_refresh_tokens() {
        let g = gateway(true);
        let def = g.definitions().get("google").cloned().unwrap();
        let oauth = def.login.as_ref().unwrap().oauth2.clone().unwrap();
        let mut secret = Secret::default();

        g.take_tokens(
            &oauth,
            &serde_json::from_str(GOOGLE_TOKEN).unwrap(),
            &mut secret,
        )
        .unwrap();

        assert_eq!(secret.tokens["access_token"], "ya29.example-access-token");
        assert_eq!(
            secret.refresh_token.as_deref(),
            Some("1//example-refresh-token")
        );
        assert!(secret.expires_at.is_some());
    }

    #[test]
    fn a_bluesky_session_answer_holds_the_token_and_the_account() {
        let def = Definitions::load(None, &[])
            .get("bluesky")
            .cloned()
            .unwrap();
        let session = def.login.unwrap().session.unwrap();
        let answer = crate::template::Context {
            doc: serde_json::from_str(include_str!(
                "../../services/bluesky/examples/createSession.json"
            ))
            .unwrap(),
            ..Default::default()
        };

        assert_eq!(
            answer.render_text(&session.tokens["access"]).as_deref(),
            Some("example-access-jwt")
        );
        assert_eq!(
            answer.render_text(&session.values["did"]).as_deref(),
            Some("did:plc:example")
        );
        assert_eq!(session.lifetime_secs().unwrap(), 3600);
    }

    #[test]
    fn a_github_token_answer_gives_the_token_the_mcp_server_takes() {
        let g = gateway(true);
        let def = g.definitions().get("github").cloned().unwrap();
        let oauth = def.login.as_ref().unwrap().oauth2.clone().unwrap();
        let mut secret = Secret::default();

        g.take_tokens(
            &oauth,
            &serde_json::from_str(include_str!(
                "../../services/github/examples/access_token.json"
            ))
            .unwrap(),
            &mut secret,
        )
        .unwrap();

        assert_eq!(secret.tokens["access_token"], "gho_example-access-token");
        assert_eq!(
            secret.refresh_token.as_deref(),
            Some("ghr_example-refresh-token")
        );
        assert!(secret.expires_at.is_some());
        let c = g.add_defined("github", BTreeMap::new(), None).unwrap();
        g.connections.set_secret(&c.id, &secret).unwrap();
        let c = g.connections.get(&c.id).unwrap().unwrap();
        let Some(crate::mcp::ServerConfig::Http { headers, .. }) = g.server_config(&c) else {
            panic!("an MCP server over HTTP")
        };
        assert_eq!(headers["Authorization"], "Bearer gho_example-access-token");
        assert!(headers["X-MCP-Toolsets"].contains("notifications"));
    }

    #[test]
    fn a_pressed_button_goes_to_the_workspace_it_was_pressed_in() {
        let g = gateway(true);
        let mut members = Vec::new();
        for team in ["T0OTHER", "T0ACME"] {
            let c = g
                .add_defined(
                    "slack",
                    BTreeMap::from([("team".into(), team.into())]),
                    None,
                )
                .unwrap();
            members.push((c.id, "events".to_owned()));
        }

        let pressed: Value = serde_json::from_str(include_str!(
            "../../services/slack/examples/socket-interactive.json"
        ))
        .unwrap();

        assert_eq!(
            g.route_event(&members, &pressed),
            Some(("slack-2".into(), "events".into()))
        );
    }

    #[test]
    fn a_slack_event_goes_to_the_workspace_it_came_from() {
        let g = gateway(true);
        for (id, team) in [("slack", "T0OTHER"), ("slack-2", "T0ACME")] {
            g.connections
                .add(
                    id,
                    id,
                    "slack",
                    Transport::Defined {
                        definition: "slack".into(),
                        values: BTreeMap::from([("team".into(), team.into())]),
                    },
                    Secret::default(),
                    "",
                )
                .unwrap();
        }
        let event: Value = serde_json::from_str(SLACK_EVENT).unwrap();

        let members = [
            ("slack".to_owned(), "events".to_owned()),
            ("slack-2".to_owned(), "events".to_owned()),
        ];
        let to = g.route_event(&members, &event);

        assert_eq!(to, Some(("slack-2".to_owned(), "events".to_owned())));
    }

    #[tokio::test]
    async fn a_calendar_change_with_the_accounts_token_is_handed_over() {
        let g = gateway(true);
        g.connections
            .add(
                "google",
                "Google",
                "google",
                Transport::Defined {
                    definition: "google".into(),
                    values: BTreeMap::from([("channel_token".into(), "t-123".into())]),
                },
                Secret::default(),
                "",
            )
            .unwrap();
        let got: Arc<Mutex<Vec<Arrival>>> = Arc::default();
        let sink = got.clone();
        g.register_destination("calendar", Arc::new(move |a| sink.lock().unwrap().push(a)));
        g.set_destination(crate::OWNER, "google", "calendar", Some("calendar"))
            .unwrap();
        let headers = |token: &str| {
            BTreeMap::from([
                ("X-Goog-Channel-Token".to_owned(), token.to_owned()),
                ("X-Goog-Resource-State".to_owned(), "exists".to_owned()),
            ])
        };

        let ok = g
            .receive_http("google", "calendar", headers("t-123"), "")
            .await
            .unwrap();
        let wrong = g
            .receive_http("google", "calendar", headers("t-999"), "")
            .await
            .unwrap();

        assert_eq!((ok.0, wrong.0), (200, 401));
        let got = got.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].headers["X-Goog-Resource-State"], "exists");
    }

    /// Every request the fake Google saw: method and path, the query and
    /// the body.
    type Seen = Arc<Mutex<Vec<(String, BTreeMap<String, String>, String)>>>;

    /// A fake Google that answers a deletion with 204 and no body, as
    /// Calendar does, and anything else with an empty event.
    async fn fake_google() -> (String, Seen) {
        use axum::extract::{Query, State};
        use axum::http::{Method, StatusCode, Uri};

        async fn record(
            State(seen): State<Seen>,
            method: Method,
            uri: Uri,
            Query(query): Query<BTreeMap<String, String>>,
            body: String,
        ) -> (StatusCode, String) {
            seen.lock()
                .unwrap()
                .push((format!("{method} {}", uri.path()), query, body));
            if method == Method::DELETE {
                (StatusCode::NO_CONTENT, String::new())
            } else {
                (StatusCode::OK, r#"{"kind":"calendar#event"}"#.to_owned())
            }
        }
        let seen: Seen = Arc::default();
        let app = axum::Router::new()
            .fallback(record)
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, seen)
    }

    /// The bundled Google definition pointed at `base`, with one account
    /// already logged in.
    fn google_at(base: &str) -> (Gateway, Connection, Definition) {
        let text = include_str!("../../services/google/google.toml")
            .replace("https://www.googleapis.com", base);
        let g = gateway(true).with_definitions(None, &[("google.toml".into(), text)]);
        let c = g
            .connections
            .add(
                "google",
                "Google",
                "google",
                Transport::Defined {
                    definition: "google".into(),
                    values: BTreeMap::new(),
                },
                Secret {
                    tokens: BTreeMap::from([("access_token".into(), "ya29.t".into())]),
                    ..Default::default()
                },
                "",
            )
            .unwrap();
        let def = g.definitions().get("google").cloned().unwrap();
        (g, c, def)
    }

    #[tokio::test]
    async fn the_google_event_tools_read_freely_and_change_only_after_confirming() {
        let (base, _) = fake_google().await;
        let (g, c, _) = google_at(&base);

        g.tools_of(&c.id).await.unwrap();

        let permissions = Permissions::new(g.store().clone());
        let default = |name: &str| permissions.default_of(&c.id, name).unwrap();
        let course = |name: &str| auto_course(permissions.marks_of(&c.id, name).unwrap(), name);
        assert_eq!(default("get_event"), Permission::Allow);
        for name in ["respond_to_event", "update_event", "delete_event"] {
            assert_eq!(default(name), Permission::Auto, "{name}");
            assert_eq!(course(name), AutoCourse::Ask, "{name}");
        }
    }

    #[tokio::test]
    async fn get_event_reads_one_event_of_a_calendar() {
        let (base, seen) = fake_google().await;
        let (g, c, def) = google_at(&base);

        let got = g
            .call_defined(
                &c,
                &def,
                "get_event",
                json!({ "calendar": "primary", "event": "e1" }),
            )
            .await
            .unwrap();

        assert!(!got.is_error);
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "GET /calendar/v3/calendars/primary/events/e1");
    }

    #[tokio::test]
    async fn respond_to_event_patches_the_whole_attendee_list_as_given() {
        let (base, seen) = fake_google().await;
        let (g, c, def) = google_at(&base);
        let attendees = json!([
            { "email": "boss@example.com", "organizer": true, "responseStatus": "accepted" },
            { "email": "owner@example.com", "self": true, "responseStatus": "declined" },
        ]);

        let got = g
            .call_defined(
                &c,
                &def,
                "respond_to_event",
                json!({
                    "calendar": "primary",
                    "event": "e1",
                    "attendees": attendees,
                    "send_updates": "all",
                }),
            )
            .await
            .unwrap();

        assert!(!got.is_error);
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "PATCH /calendar/v3/calendars/primary/events/e1");
        assert_eq!(seen[0].1["sendUpdates"], "all");
        let body: Value = serde_json::from_str(&seen[0].2).unwrap();
        assert_eq!(body, json!({ "attendees": attendees }));
    }

    #[tokio::test]
    async fn update_event_patches_only_the_fields_given() {
        let (base, seen) = fake_google().await;
        let (g, c, def) = google_at(&base);

        g.call_defined(
            &c,
            &def,
            "update_event",
            json!({
                "calendar": "primary",
                "event": "e1",
                "start": { "dateTime": "2026-11-24T10:00:00+09:00" },
                "send_updates": "none",
            }),
        )
        .await
        .unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "PATCH /calendar/v3/calendars/primary/events/e1");
        assert_eq!(seen[0].1["sendUpdates"], "none");
        let body: Value = serde_json::from_str(&seen[0].2).unwrap();
        assert_eq!(
            body,
            json!({ "start": { "dateTime": "2026-11-24T10:00:00+09:00" } })
        );
    }

    #[tokio::test]
    async fn delete_event_answered_with_no_content_is_a_success() {
        let (base, seen) = fake_google().await;
        let (g, c, def) = google_at(&base);

        let got = g
            .call_defined(
                &c,
                &def,
                "delete_event",
                json!({ "calendar": "primary", "event": "e1", "send_updates": "all" }),
            )
            .await
            .unwrap();

        assert!(!got.is_error, "204 is not a failure");
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "DELETE /calendar/v3/calendars/primary/events/e1");
        assert_eq!(seen[0].1["sendUpdates"], "all");
        assert_eq!(seen[0].2, "", "a deletion sends no body");
    }

    /// The bundled RSS definition with one connection to the feed at `url`.
    fn rss_at(url: &str) -> (Gateway, Connection) {
        let g = gateway(false);
        let c = g
            .add_defined("rss", BTreeMap::from([("url".into(), url.into())]), None)
            .unwrap();
        (g, c)
    }

    #[test]
    fn an_rss_definition_with_an_unknown_field_or_a_wrong_shape_is_refused() {
        let text = include_str!("../../services/rss/rss.toml");
        assert!(parse(text).is_ok());

        for (from, to, why) in [
            (
                "name = \"get_feed\"",
                "name = \"get_feed\"\nfeed = 1",
                "unknown field `feed`",
            ),
            (
                "url = { label",
                "url = { pattern = \"https\", label",
                "pattern",
            ),
            ("http = {", "http = { retry = 1,", "retry"),
            ("path = \"\"", "path = 1", "path"),
            ("same_host = true", "same_host = 1", "same_host"),
            ("opens = \"get_page\"", "opens = \"get_feed\"", "opens"),
            ("opens = \"get_page\"", "opens = \"nothing\"", "opens"),
            ("get_feed = \"allow\"", "get_feed = true", "permissions"),
        ] {
            let broken = text.replace(from, to);
            assert_ne!(broken, text, "{from} is not in the definition");
            let error = parse(&broken).unwrap_err().to_string();
            assert!(error.contains(why), "{error}");
        }
    }

    #[tokio::test]
    async fn the_rss_connection_only_reads_its_feed_and_pages_and_without_asking() {
        let (g, c) = rss_at("https://example.com/feed.atom");

        g.tools_of(&c.id).await.unwrap();

        let def = g.definitions().get("rss").cloned().unwrap();
        assert!(def.login.is_none());
        let tools: Vec<_> = def
            .tools
            .iter()
            .map(|t| (t.name.as_str(), t.method))
            .collect();
        assert_eq!(
            tools,
            [("get_feed", Method::Get), ("get_page", Method::Get)]
        );
        let permissions = Permissions::new(g.store().clone());
        for (tool, _) in tools {
            assert_eq!(
                permissions.default_of(&c.id, tool).unwrap(),
                Permission::Allow
            );
            assert_eq!(
                permissions.marks_of(&c.id, tool).unwrap().read_only,
                Some(true)
            );
        }
        assert_eq!(
            g.connections.get(&c.id).unwrap().unwrap().account,
            "https://example.com/feed.atom"
        );
    }

    #[tokio::test]
    async fn get_feed_reads_the_feeds_url_exactly_as_given() {
        let (base, seen) = fake_google().await;

        for (path, query) in [
            ("/releases.atom", vec![]),
            ("/feed/", vec![]),
            ("/rss", vec![("id", "1"), ("lang", "ja")]),
        ] {
            let pairs: Vec<_> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let url = match pairs.as_slice() {
                [] => format!("{base}{path}"),
                pairs => format!("{base}{path}?{}", pairs.join("&")),
            };
            let (g, c) = rss_at(&url);

            let result = g.call_as_owner(&c, "get_feed", json!({})).await.unwrap();

            assert!(!result.is_error);
            let (target, got, body) = seen.lock().unwrap().last().cloned().unwrap();
            assert_eq!(target, format!("GET {path}"));
            let query: BTreeMap<String, String> = query
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect();
            assert_eq!(got, query);
            assert_eq!(body, "");
        }
    }

    /// A site with a feed, whose pages may send a reader on to `elsewhere`.
    async fn site(elsewhere: String) -> (String, Arc<Mutex<Vec<String>>>) {
        use axum::http::{StatusCode, Uri, header};
        use axum::response::IntoResponse;

        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = seen.clone();
        let app = axum::Router::new().fallback(move |uri: Uri| {
            sink.lock().unwrap().push(uri.path().to_owned());
            let to = match uri.path() {
                "/away" => Some(format!("{elsewhere}/secret")),
                "/moved" => Some("/page".to_owned()),
                "/loop" => Some("/loop".to_owned()),
                _ => None,
            };
            let kind = match uri.path() {
                "/movie" => "video/mp4",
                _ => "text/html; charset=utf-8",
            };
            async move {
                match to {
                    Some(to) => (StatusCode::FOUND, [(header::LOCATION, to)]).into_response(),
                    None => ([(header::CONTENT_TYPE, kind)], "<p>page</p>").into_response(),
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, seen)
    }

    #[tokio::test]
    async fn get_page_reads_only_text_on_the_feeds_host_however_it_is_sent_on() {
        let (elsewhere, touched) = site(String::new()).await;
        let (base, _) = site(elsewhere.clone()).await;
        let (g, c) = rss_at(&format!("{base}/feed.xml"));
        let read = async |url: String| g.call_as_owner(&c, "get_page", json!({ "url": url })).await;

        for path in ["/page", "/moved"] {
            let page = read(format!("{base}{path}")).await.unwrap();
            assert!(!page.is_error);
            assert_eq!(page.text, "<p>page</p>");
        }
        for url in [
            format!("{elsewhere}/secret"),
            format!("{base}/away"),
            format!("{base}/loop"),
            format!("{base}/movie"),
            base.replace("http://127.0.0.1", "http://localhost"),
            base.replace("http://", "http://127.0.0.1@localhost:"),
            "file:///etc/passwd".to_owned(),
            "/page".to_owned(),
            String::new(),
        ] {
            assert!(read(url.clone()).await.is_err(), "{url}");
        }
        assert!(touched.lock().unwrap().is_empty());
    }

    #[test]
    fn a_feeds_link_is_opened_only_on_its_host() {
        let (g, c) = rss_at("http://example.com/feed.xml");
        let opens = |url: &str| g.opener(&c, "get_feed", url);

        assert_eq!(opens("http://example.com/a").as_deref(), Some("get_page"));
        assert_eq!(opens("https://example.com/a").as_deref(), Some("get_page"));
        for url in [
            "http://www.example.com/a",
            "http://example.com:8080/a",
            "http://example.com.evil.test/a",
            "http://example.com@evil.test/a",
            "http://127.0.0.1/a",
            "ftp://example.com/a",
            "/a",
        ] {
            assert_eq!(opens(url), None, "{url}");
        }
        assert_eq!(g.opener(&c, "get_page", "http://example.com/a"), None);
        let (g, c) = rss_at("https://example.com/feed.xml");
        assert_eq!(g.opener(&c, "get_feed", "http://example.com/a"), None);
    }

    #[test]
    fn without_the_google_app_a_login_points_at_its_setup() {
        let g = gateway(false);
        let def = g.definitions().get("google").cloned().unwrap();

        let message = g.app_values(&def).unwrap_err();

        assert!(message.contains("FAMULEN_GOOGLE_CLIENT_ID"), "{message}");
        assert!(
            message.contains("crates/gateway/services/google/README.md"),
            "{message}"
        );
    }
}
