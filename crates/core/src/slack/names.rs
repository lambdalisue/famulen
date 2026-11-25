//! The names Slack shows for its members.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::app::App;

/// How long a name is kept before Slack is asked again.
const KEPT: Duration = Duration::from_secs(60 * 60);
/// How long Slack is not asked again for a member it does not know.
const MISSED: Duration = Duration::from_secs(10 * 60);
/// How long Slack is not asked again for a member it could not be asked for.
const FAILED: Duration = Duration::from_secs(60);
/// How long one member's name is waited for.
const EACH: Duration = Duration::from_secs(4);
/// How long one batch's names are waited for.
const PATIENCE: Duration = Duration::from_secs(10);

/// The name a Slack `users.info` answer shows for a user, and the handle
/// typed after @: Slack shows the display name, or the real name when that
/// is empty.
pub fn shown(answer: &Value) -> (String, String) {
    let user = &answer["user"];
    let text = |v: &Value| v.as_str().unwrap_or_default().trim().to_owned();
    let shown = [
        &user["profile"]["display_name"],
        &user["profile"]["real_name"],
        &user["real_name"],
        &user["name"],
    ]
    .into_iter()
    .map(text)
    .find(|s| !s.is_empty())
    .unwrap_or_default();
    (shown, text(&user["name"]))
}

/// What Slack answered for a member.
#[derive(Debug, Clone, PartialEq)]
enum Answer {
    Named(String),
    Unknown,
    Failed,
}

impl Answer {
    fn lasts(&self) -> Duration {
        match self {
            Self::Named(_) => KEPT,
            Self::Unknown => MISSED,
            Self::Failed => FAILED,
        }
    }
}

type Asked = BTreeMap<(String, String), (Instant, Answer)>;

/// The names Slack gave lately, by connection and member. Clones share them.
#[derive(Clone, Default)]
pub struct Names(Arc<Mutex<Asked>>);

impl Names {
    fn kept(&self, connection: &str, member: &str, now: Instant) -> Option<Answer> {
        let mut asked = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        asked.retain(|_, (at, answer)| now.saturating_duration_since(*at) < answer.lasts());
        asked
            .get(&(connection.to_owned(), member.to_owned()))
            .map(|(_, answer)| answer.clone())
    }

    fn keep(&self, connection: &str, member: &str, answer: Answer, now: Instant) {
        let mut asked = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        asked.insert((connection.to_owned(), member.to_owned()), (now, answer));
    }
}

impl App {
    /// The names Slack shows for `members` of a workspace, without those it
    /// gives none for.
    pub async fn slack_names_of(
        &self,
        connection: &str,
        members: &[String],
    ) -> BTreeMap<String, String> {
        let mut names = BTreeMap::new();
        let ask = async {
            for member in members {
                let answer = match self.slack_names.kept(connection, member, Instant::now()) {
                    Some(answer) => answer,
                    None => {
                        let asked = self.ask_slack_name(connection, member);
                        let answer = tokio::time::timeout(EACH, asked)
                            .await
                            .unwrap_or(Answer::Failed);
                        let now = Instant::now();
                        self.slack_names
                            .keep(connection, member, answer.clone(), now);
                        answer
                    }
                };
                if let Answer::Named(name) = answer {
                    names.insert(member.clone(), name);
                }
            }
        };
        if tokio::time::timeout(PATIENCE, ask).await.is_err() {
            tracing::warn!(%connection, "Slack did not give its members' names in time");
        }
        names
    }

    async fn ask_slack_name(&self, connection: &str, member: &str) -> Answer {
        let asked = self.tool_json(connection, "user_info", json!({ "user": member }));
        match asked.await.map(|answer| shown(&answer).0) {
            Ok(name) if name.is_empty() => Answer::Unknown,
            Ok(name) => Answer::Named(name),
            Err(error) => {
                tracing::warn!(%connection, %member, %error, "could not read a member's name");
                let why = error.to_string();
                let unknown = ["user_not_found", "user_not_visible"];
                if unknown.iter().any(|u| why.contains(u)) {
                    Answer::Unknown
                } else {
                    Answer::Failed
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::notebook::testing::TempDir;

    /// Famulen whose Slack definition reaches a local stand-in for Slack that
    /// knows the members given, with the members it was asked for.
    pub(crate) async fn with_slack(
        dir: &std::path::Path,
        members: &'static [(&'static str, &'static str)],
    ) -> (App, String, Arc<Mutex<Vec<String>>>) {
        use axum::extract::Query;
        let asked: Arc<Mutex<Vec<String>>> = Arc::default();
        let seen = asked.clone();
        let info = move |Query(query): Query<BTreeMap<String, String>>| {
            let user = query["user"].clone();
            seen.lock().unwrap().push(user.clone());
            let answer = match members.iter().find(|(id, _)| *id == user) {
                Some((_, name)) => json!({ "ok": true, "user": { "name": "handle",
                    "profile": { "display_name": name, "phone": "090-0000-0000" } } }),
                None => json!({ "ok": false, "error": "user_not_found" }),
            };
            async move { answer.to_string() }
        };
        let router = axum::Router::new().route("/users.info", axum::routing::get(info));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let services = dir.join("gateway/services");
        std::fs::create_dir_all(&services).unwrap();
        let bundled = include_str!("../../../gateway/services/slack/slack.toml");
        std::fs::write(
            services.join("slack.toml"),
            bundled.replace("https://slack.com/api", &base),
        )
        .unwrap();
        let app = crate::runtime::testing::app(dir);
        let c = app
            .gateway
            .add_defined("slack", Default::default(), None)
            .unwrap();
        (app, c.id, asked)
    }

    #[test]
    fn a_user_is_shown_by_the_display_name_and_mentioned_by_the_handle() {
        let answer = json!({ "user": { "name": "fam", "real_name": "Famulen",
            "profile": { "display_name": "ファム", "real_name": "Famulen" } } });
        assert_eq!(shown(&answer), ("ファム".into(), "fam".into()));
    }

    #[test]
    fn a_user_without_a_display_name_is_shown_by_the_real_name() {
        let answer = json!({ "user": { "name": "fam",
            "profile": { "display_name": "", "real_name": "Famulen" } } });
        assert_eq!(shown(&answer), ("Famulen".into(), "fam".into()));
    }

    #[tokio::test]
    async fn slack_is_asked_once_for_a_member_whether_or_not_it_gives_a_name() {
        let dir = TempDir::new("slack-names");
        let (app, connection, asked) = with_slack(dir.path(), &[("U1", "倉谷匠")]).await;
        let members = ["U1".to_owned(), "U2".to_owned()];

        for _ in 0..2 {
            let names = app.slack_names_of(&connection, &members).await;
            assert_eq!(names, BTreeMap::from([("U1".into(), "倉谷匠".into())]));
        }

        assert_eq!(*asked.lock().unwrap(), members);
    }

    #[tokio::test]
    async fn a_member_slack_could_not_be_asked_for_is_asked_for_again_soon() {
        let dir = TempDir::new("slack-names-failed");
        let app = crate::runtime::testing::app(dir.path());

        let names = app.slack_names_of("gone", &["U1".to_owned()]).await;

        assert!(names.is_empty());
        let kept = app.slack_names.kept("gone", "U1", Instant::now());
        assert_eq!(kept, Some(Answer::Failed));
    }

    #[test]
    fn a_name_is_kept_for_a_while_an_unknown_member_for_less_and_a_failure_shortly() {
        let names = Names::default();
        let now = Instant::now();
        let named = Answer::Named("倉谷匠".into());
        names.keep("slack", "U1", named.clone(), now);
        names.keep("slack", "U2", Answer::Unknown, now);
        names.keep("slack", "U3", Answer::Failed, now);

        assert_eq!(names.kept("slack-2", "U1", now), None);
        assert_eq!(names.kept("slack", "U3", now), Some(Answer::Failed));
        assert_eq!(names.kept("slack", "U3", now + FAILED), None);
        let unknown = names.kept("slack", "U2", now + FAILED);
        assert_eq!(unknown, Some(Answer::Unknown));
        assert_eq!(names.kept("slack", "U2", now + MISSED), None);
        assert_eq!(names.kept("slack", "U1", now + MISSED), Some(named));
        assert_eq!(names.kept("slack", "U1", now + KEPT), None);
    }
}
