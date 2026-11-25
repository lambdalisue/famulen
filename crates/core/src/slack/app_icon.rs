//! The Slack app's own icon, in the sidebar and the lists: set to the
//! secretary's portrait as the owner chooses it.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::looks;
use crate::app::App;
use crate::channel::DEFINITION;
use crate::connections::Connection;
use crate::persona::Gender;

/// The account field that lets a connection change its app.
const TOKEN_FIELD: &str = "config_refresh";

/// How long the choice of a portrait waits to say what went wrong.
const PATIENCE: Duration = Duration::from_secs(10);

/// How long one app may take before the next portrait is not kept waiting.
const LIMIT: Duration = Duration::from_secs(60);

/// Held while icons are set, so the last portrait chosen is the one left.
pub type AppIcons = Arc<tokio::sync::Mutex<()>>;

/// A Slack app the connected workspaces share, and whether its icon follows
/// the secretary's portrait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackApp {
    pub id: String,
    pub workspaces: Vec<String>,
    pub follows: bool,
}

/// What the owner copied instead of a refresh token, if it is not one.
fn not_a_refresh_token(text: &str) -> Option<&'static str> {
    if text.starts_with("xoxe.") || text.starts_with("xoxp-") {
        Some("それは Access Token。下に並ぶ Refresh Token（xoxe- で始まる）を写す")
    } else if text.starts_with("xapp-") {
        Some(
            "それは App-Level Token。アプリの一覧の下で作った Refresh Token（xoxe- で始まる）を写す",
        )
    } else if !text.starts_with("xoxe-") || text.contains(char::is_whitespace) {
        Some("Refresh Token は xoxe- で始まる。全部を写して貼る")
    } else {
        None
    }
}

/// What the owner can do about why Slack refused.
fn remedy(why: &str) -> Option<&'static str> {
    if ["invalid_refresh_token", "token_expired", "token_revoked"]
        .iter()
        .any(|e| why.contains(e))
    {
        Some("トークンが古いか、もう使われている。作り直して貼る")
    } else if [
        "access_denied",
        "no_permission",
        "invalid_app",
        "team_access_not_granted",
    ]
    .iter()
    .any(|e| why.contains(e))
    {
        Some(
            "別のワークスペースのトークンかもしれない。このアプリを作ったワークスペースで作り直して貼る",
        )
    } else {
        None
    }
}

impl App {
    /// The Slack apps of the connected workspaces.
    pub fn slack_apps(&self) -> anyhow::Result<Vec<SlackApp>> {
        let mut apps: Vec<SlackApp> = Vec::new();
        for (id, c, follows) in self.slack_app_connections()? {
            let name = if c.account.is_empty() {
                c.name
            } else {
                c.account
            };
            match apps.iter_mut().find(|a| a.id == id) {
                Some(app) => {
                    app.workspaces.push(name);
                    app.follows |= follows;
                }
                None => apps.push(SlackApp {
                    id,
                    workspaces: vec![name],
                    follows,
                }),
            }
        }
        Ok(apps)
    }

    /// Keep what lets Famulen change the Slack app `id`, as `actor` pasted
    /// it, and set the app's icon now; what went wrong with the icon.
    pub async fn let_slack_app_follow(
        &self,
        actor: &str,
        id: &str,
        token: &str,
    ) -> anyhow::Result<Vec<String>> {
        let token = token.trim();
        if let Some(why) = not_a_refresh_token(token) {
            anyhow::bail!("{why}");
        }
        let of_app: Vec<_> = self
            .slack_app_connections()?
            .into_iter()
            .filter(|(app, ..)| app == id)
            .collect();
        // Put where one already is, so an old one is not left beside it.
        let (_, connection, _) = of_app
            .iter()
            .find(|(.., follows)| *follows)
            .or_else(|| of_app.first())
            .ok_or_else(|| anyhow::anyhow!("no Slack app {id} is connected"))?;
        self.gateway
            .set_account_value(actor, &connection.id, TOKEN_FIELD, token)
            .await?;
        // Waited for until Slack answers: told that it is not done yet, the owner
        // would paste again a token already spent.
        Ok(self.set_slack_app_icons_within(LIMIT * 2, Some(id)).await)
    }

    /// The Slack connections whose app is known, with the app and whether
    /// each holds what lets Famulen change it.
    fn slack_app_connections(&self) -> anyhow::Result<Vec<(String, Connection, bool)>> {
        let mut found = Vec::new();
        for c in self.connections.list()? {
            if c.kind != DEFINITION {
                continue;
            }
            if let Some(app) = self.gateway.account_values(&c.id)?.remove("app") {
                let follows = self.gateway.account_names(&c.id)?.contains(TOKEN_FIELD);
                found.push((app, c, follows));
            }
        }
        Ok(found)
    }

    /// Set the icon of every Slack app a connection may change to the
    /// secretary's portrait; what went wrong, for the owner.
    pub async fn set_slack_app_icons(&self) -> Vec<String> {
        self.set_slack_app_icons_within(PATIENCE, None).await
    }

    async fn set_slack_app_icons_within(
        &self,
        patience: Duration,
        only: Option<&str>,
    ) -> Vec<String> {
        let only = only.map(str::to_owned);
        // Apart from the request: a token already traded for must be kept
        // even when whoever asked is gone.
        let app = self.clone();
        let mut work = tokio::spawn(async move {
            let troubles = app
                .slack_app_icon_troubles(only.as_deref())
                .await
                .unwrap_or_else(|error| {
                    vec![format!(
                        "Slack のアプリのアイコンを替えられなかった（{error}）"
                    )]
                });
            for trouble in &troubles {
                tracing::warn!(%trouble, "the Slack app's icon was not set");
                app.journal.record("setting", trouble);
            }
            troubles
        });
        match tokio::time::timeout(patience, &mut work).await {
            Ok(done) => done.unwrap_or_else(|error| vec![error.to_string()]),
            Err(_) => vec!["Slack のアプリのアイコンは、まだ替わっていない".into()],
        }
    }

    async fn slack_app_icon_troubles(&self, only: Option<&str>) -> anyhow::Result<Vec<String>> {
        let _one = self.slack_app_icons.lock().await;
        let icon = looks::url(Gender::load(&self.store)?);
        let mut set = BTreeSet::new();
        let mut failed = Vec::new();
        for c in self.connections.list()? {
            if c.kind != DEFINITION || !self.gateway.account_names(&c.id)?.contains(TOKEN_FIELD) {
                continue;
            }
            let app_id = self.gateway.account_values(&c.id)?.remove("app");
            if only.is_some() && app_id.as_deref() != only {
                continue;
            }
            // Workspaces share an app, whose icon is one.
            if app_id.as_ref().is_some_and(|app| set.contains(app)) {
                continue;
            }
            match self.set_slack_app_icon(&c, app_id.as_deref(), &icon).await {
                Ok(()) => set.extend(app_id),
                Err(error) => failed.push((app_id, c, error)),
            }
        }
        Ok(failed
            .into_iter()
            .filter(|(app, ..)| app.as_ref().is_none_or(|app| !set.contains(app)))
            .map(|(_, c, error)| {
                let whose = if c.account.is_empty() {
                    c.id
                } else {
                    c.account
                };
                let why = error.to_string();
                let remedy = remedy(&why).map(|r| format!("。{r}")).unwrap_or_default();
                format!("{whose} の Slack のアプリのアイコンを替えられなかった（{why}）{remedy}")
            })
            .collect())
    }

    async fn set_slack_app_icon(
        &self,
        connection: &Connection,
        app: Option<&str>,
        icon: &str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(app.is_some(), "アプリの ID が分からない。ログインし直す");
        let result = tokio::time::timeout(
            LIMIT,
            self.gateway
                .call_as_owner(connection, "set_app_icon", json!({ "icon": icon })),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Slack が答えない"))??;
        if result.is_error {
            let answer: Value = serde_json::from_str(&result.text).unwrap_or(Value::Null);
            anyhow::bail!("{}", answer["error"].as_str().unwrap_or("no answer"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::notebook::testing::TempDir;

    type Seen = Arc<Mutex<Vec<(String, String, String)>>>;

    /// Famulen whose Slack takes every trade, and refuses the icon for the
    /// token got with the refresh token `bad`.
    async fn famulen(dir: &TempDir) -> (App, Seen) {
        async fn answer(
            axum::extract::State(seen): axum::extract::State<Seen>,
            axum::extract::Path(method): axum::extract::Path<String>,
            headers: axum::http::HeaderMap,
            body: String,
        ) -> String {
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            let answer = match method.as_str() {
                "tooling.tokens.rotate" => {
                    let spent = body.trim_start_matches("refresh_token=");
                    json!({ "ok": true, "token": format!("for-{spent}"), "refresh_token": format!("{spent}+") })
                }
                _ if auth.ends_with("for-bad") => json!({ "ok": false, "error": "access_denied" }),
                _ => json!({ "ok": true }),
            };
            seen.lock().unwrap().push((method, auth, body));
            answer.to_string()
        }
        let seen: Seen = Arc::default();
        let router = axum::Router::new()
            .route("/api/{method}", axum::routing::post(answer))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let services = dir.path().join("gateway/services");
        std::fs::create_dir_all(&services).unwrap();
        std::fs::write(
            services.join("slack.toml"),
            include_str!("../../../gateway/services/slack/slack.toml")
                .replace("https://slack.com/api", &format!("{base}/api")),
        )
        .unwrap();
        (crate::runtime::testing::app(dir.path()), seen)
    }

    fn workspace(app: &App, values: &[(&str, &str)]) {
        let values = values
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        app.gateway.add_defined(DEFINITION, values, None).unwrap();
    }

    fn icons_set(seen: &Seen) -> Vec<(String, Value)> {
        let seen = seen.lock().unwrap();
        seen.iter()
            .filter(|s| s.0 == "apps.icon.set")
            .map(|s| (s.1.clone(), serde_json::from_str(&s.2).unwrap()))
            .collect()
    }

    #[tokio::test]
    async fn each_app_gets_the_portraits_icon_once_and_a_connection_without_a_token_is_left() {
        let dir = TempDir::new("app-icon-each");
        let (app, seen) = famulen(&dir).await;
        workspace(&app, &[("app", "A1")]);
        workspace(&app, &[("app", "A1"), ("config_refresh", "one")]);
        workspace(&app, &[("app", "A1"), ("config_refresh", "two")]);
        workspace(&app, &[("app", "A2"), ("config_refresh", "three")]);
        workspace(&app, &[("app", "A3")]);
        Gender::Female.save(&app.store).unwrap();

        let troubles = app.set_slack_app_icons().await;

        assert_eq!(troubles, Vec::<String>::new());
        let url = "https://raw.githubusercontent.com/lambdalisue/famulen/main/brand/female/avatar-512.png";
        assert_eq!(
            icons_set(&seen),
            [
                (
                    "Bearer for-one".into(),
                    json!({ "app_id": "A1", "url": url })
                ),
                (
                    "Bearer for-three".into(),
                    json!({ "app_id": "A2", "url": url })
                ),
            ]
        );
    }

    #[tokio::test]
    async fn without_any_token_slack_is_not_called() {
        let dir = TempDir::new("app-icon-none");
        let (app, seen) = famulen(&dir).await;
        workspace(&app, &[("app", "A1")]);

        assert!(app.set_slack_app_icons().await.is_empty());

        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refusal_is_told_without_the_token_and_the_other_apps_go_on() {
        let dir = TempDir::new("app-icon-refused");
        let (app, seen) = famulen(&dir).await;
        workspace(
            &app,
            &[
                ("app", "A1"),
                ("team_name", "Acme"),
                ("config_refresh", "bad"),
            ],
        );
        workspace(&app, &[("app", "A2"), ("config_refresh", "good")]);
        workspace(&app, &[("config_refresh", "old")]);

        let troubles = app.set_slack_app_icons().await;

        assert_eq!(troubles.len(), 2, "{troubles:?}");
        assert!(troubles[0].contains("Acme") && troubles[0].contains("access_denied"));
        assert!(troubles[1].contains("ログインし直す"));
        assert!(!troubles.concat().contains("bad"));
        assert_eq!(icons_set(&seen).len(), 2);
    }

    #[tokio::test]
    async fn another_connections_token_stands_in_for_one_the_app_refuses() {
        let dir = TempDir::new("app-icon-stands-in");
        let (app, seen) = famulen(&dir).await;
        workspace(&app, &[("app", "A1"), ("config_refresh", "bad")]);
        workspace(&app, &[("app", "A1"), ("config_refresh", "good")]);

        assert!(app.set_slack_app_icons().await.is_empty());

        assert_eq!(icons_set(&seen).len(), 2);
    }

    #[tokio::test]
    async fn a_pasted_token_is_kept_once_for_the_app_and_its_icon_is_set_at_once() {
        let dir = TempDir::new("app-icon-paste");
        let (app, seen) = famulen(&dir).await;
        workspace(&app, &[("app", "A1"), ("team_name", "Acme")]);
        workspace(&app, &[("app", "A1"), ("team_name", "Beta")]);
        let shown = |follows| SlackApp {
            id: "A1".into(),
            workspaces: vec!["Acme".into(), "Beta".into()],
            follows,
        };
        assert_eq!(app.slack_apps().unwrap(), [shown(false)]);

        for token in [" xoxe-1-one\n", "xoxe-1-two"] {
            let troubles = app.let_slack_app_follow("owner", "A1", token).await;
            assert_eq!(troubles.unwrap(), Vec::<String>::new());
        }

        assert_eq!(app.slack_apps().unwrap(), [shown(true)]);
        let held = |id| app.gateway.account_names(id).unwrap().contains(TOKEN_FIELD);
        assert_eq!((held("slack"), held("slack-2")), (true, false));
        let set: Vec<String> = icons_set(&seen).into_iter().map(|s| s.0).collect();
        assert_eq!(set, ["Bearer for-xoxe-1-one", "Bearer for-xoxe-1-two"]);
    }

    #[tokio::test]
    async fn pasting_for_one_app_leaves_the_other_apps_alone() {
        let dir = TempDir::new("app-icon-only");
        let (app, seen) = famulen(&dir).await;
        workspace(&app, &[("app", "A1")]);
        workspace(&app, &[("app", "A2"), ("config_refresh", "bad")]);

        let troubles = app.let_slack_app_follow("owner", "A1", "xoxe-1-a").await;

        assert_eq!(troubles.unwrap(), Vec::<String>::new());
        assert_eq!(icons_set(&seen).len(), 1);
    }

    #[tokio::test]
    async fn what_is_not_a_refresh_token_is_named_and_kept_nowhere() {
        let dir = TempDir::new("app-icon-mistaken");
        let (app, seen) = famulen(&dir).await;
        workspace(&app, &[("app", "A1")]);

        for (pasted, says) in [
            ("xoxe.xoxp-1-a", "Access Token"),
            ("xapp-1-a", "App-Level Token"),
            ("xoxe-1-a xoxe-1-b", "全部を写して"),
            ("", "xoxe-"),
        ] {
            let error = app.let_slack_app_follow("owner", "A1", pasted).await;
            let error = error.unwrap_err().to_string();
            assert!(error.contains(says), "{pasted}: {error}");
        }
        let unknown = app.let_slack_app_follow("owner", "A9", "xoxe-1-a").await;

        assert!(unknown.is_err());
        assert!(!app.slack_apps().unwrap()[0].follows);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_refusal_the_owner_can_mend_says_how() {
        for why in [
            "Slack gave no config token: invalid_refresh_token",
            "access_denied",
        ] {
            assert!(remedy(why).is_some_and(|r| r.contains("作り直して貼る")));
        }
        assert_eq!(remedy("app_edit_disabled"), None);
    }
}
