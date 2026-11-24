//! What Famulen does with what the gateway hands over from the bundled
//! Google definition: a new-mail notice is read into items for the lookout,
//! and a calendar is read through the connection's tools. The gateway only
//! passes the notices along; reading them is Famulen's.

use base64::Engine;
use serde_json::{Value, json};

use crate::app::App;
use crate::connections::{Connection, Transport};
use crate::lookout::Item;

/// The bundled definition of a Google account.
pub const GOOGLE: &str = "google";
/// Its receiver of new-mail notices.
pub const MAIL: &str = "mail";
/// Its receiver of calendar changes.
pub const CALENDAR: &str = "calendar";

/// The status of a connection whose login is done.
const LOGGED_IN: &str = "つながっている";

/// How many pages of a listing are read at most.
const MAX_PAGES: usize = 20;

static MAIL_READING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn is_google(c: &Connection) -> bool {
    matches!(&c.transport, Transport::Defined { definition, .. } if definition == GOOGLE)
}

fn history_key(connection: &str) -> String {
    format!("mail.history.{connection}")
}

/// The history id a Pub/Sub message from Gmail tells of.
pub fn notified_history(payload: &Value) -> Option<u64> {
    let data = payload.pointer("/message/data")?.as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(data))
        .ok()?;
    let notice: Value = serde_json::from_slice(&bytes).ok()?;
    match &notice["historyId"] {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// The ids of the messages a history listing says were added.
pub fn added_messages(page: &Value) -> Vec<String> {
    page["history"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|h| h["messagesAdded"].as_array().cloned().unwrap_or_default())
        .filter_map(|m| m.pointer("/message/id")?.as_str().map(str::to_owned))
        .collect()
}

/// The id of the last record a history listing holds.
fn last_history(page: &Value) -> Option<u64> {
    page["history"]
        .as_array()?
        .iter()
        .filter_map(|h| match &h["id"] {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.parse().ok(),
            _ => None,
        })
        .max()
}

/// One message's headers and snippet as an item the lookout reads.
pub fn mail_item(message: &Value) -> Item {
    let header = |name: &str| {
        message
            .pointer("/payload/headers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|h| {
                h["name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .and_then(|h| h["value"].as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let subject = header("Subject");
    Item {
        title: if subject.is_empty() {
            "(件名なし)".to_owned()
        } else {
            subject
        },
        // Two mails of a shop may read the same; its id also names the one to
        // open.
        body: format!(
            "{ID}{}\nfrom: {}\n{}",
            message["id"].as_str().unwrap_or_default(),
            header("From"),
            message["snippet"].as_str().unwrap_or_default()
        ),
    }
}

/// What the line naming a mail in its item starts with.
const ID: &str = "id: ";

/// The id of the message an item is, when it is mail.
pub fn message_of(item: &Item) -> Option<&str> {
    let id = item.body.lines().next()?.strip_prefix(ID)?;
    (!id.is_empty()).then_some(id)
}

/// The labels Gmail puts on inbox mail by itself that tell mail apart.
const SYSTEM_LABELS: &[(&str, &str)] = &[
    ("IMPORTANT", "重要"),
    ("STARRED", "スター付き"),
    ("CATEGORY_PERSONAL", "メイン"),
    ("CATEGORY_SOCIAL", "ソーシャル"),
    ("CATEGORY_UPDATES", "新着"),
    ("CATEGORY_FORUMS", "フォーラム"),
    ("CATEGORY_PROMOTIONS", "プロモーション"),
];

impl App {
    /// Call one of a connection's tools and read the answer as JSON.
    pub(crate) async fn tool_json(
        &self,
        connection: &str,
        tool: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        let c = self
            .connections
            .get(connection)?
            .ok_or_else(|| anyhow::anyhow!("no connection {connection}"))?;
        let result = self.gateway.call_as_owner(&c, tool, args).await?;
        anyhow::ensure!(
            !result.is_error,
            "{tool} failed: {}",
            result.text.chars().take(200).collect::<String>()
        );
        Ok(serde_json::from_str(&result.text)?)
    }

    /// Read the mail a Gmail notice tells of and hand it to the lookout as
    /// heard from the connection. The first notice only marks where to read
    /// from; later ones read what was added since.
    pub async fn read_mail_notice(
        &self,
        connection: &str,
        payload: &Value,
    ) -> anyhow::Result<usize> {
        let Some(notified) = notified_history(payload) else {
            return Ok(0);
        };
        self.read_mail(connection, Some(notified)).await
    }

    /// Read the mail added since the last read, as a notice would. A notice
    /// is taken off the subscription before its mail is read, so mail whose
    /// read failed would otherwise wait for the next notice.
    pub async fn read_mail_since_last(&self, connection: &str) -> anyhow::Result<usize> {
        self.read_mail(connection, None).await
    }

    /// Read every Google account's mail since its last read.
    pub async fn catch_up_mail(&self) {
        let Ok(list) = self.connections.list() else {
            return;
        };
        for c in list
            .into_iter()
            .filter(|c| is_google(c) && c.status == LOGGED_IN)
        {
            if let Err(error) = self.read_mail_since_last(&c.id).await {
                tracing::warn!(connection = %c.id, %error, "could not catch up on mail");
            }
        }
    }

    async fn read_mail(&self, connection: &str, notified: Option<u64>) -> anyhow::Result<usize> {
        // A notice and a catch-up reading the same mailbox at once would both
        // read from the same place and hand the lookout the mail twice.
        let _reading = MAIL_READING.lock().await;
        let key = history_key(connection);
        let Some(since) = self.store.state(&key)? else {
            if let Some(notified) = notified {
                self.store.set_state(&key, notified.to_string())?;
            }
            return Ok(0);
        };
        let mut ids = Vec::new();
        let mut page: Option<String> = None;
        let mut read_to: u64 = since.parse().unwrap_or(0);
        for _ in 0..MAX_PAGES {
            let mut args = json!({ "since": since });
            if let Some(p) = &page {
                args["page"] = json!(p);
            }
            let answer = self.tool_json(connection, "list_history", args).await?;
            ids.extend(added_messages(&answer));
            read_to = read_to.max(last_history(&answer).unwrap_or(0));
            page = answer["nextPageToken"].as_str().map(str::to_owned);
            if page.is_none() {
                break;
            }
        }
        ids.dedup();
        let mut heard = 0;
        for id in &ids {
            let message = self
                .tool_json(connection, "get_message", json!({ "id": id }))
                .await?;
            let labels: Vec<String> = message["labelIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|l| l.as_str().map(str::to_owned))
                .collect();
            if self
                .lookout
                .hear_at(connection, &mail_item(&message), &labels)?
            {
                heard += 1;
            }
        }
        // Pages left unread are read from the last record read, so the next
        // notice picks them up instead of skipping to what it tells of.
        let cursor = if page.is_some() {
            read_to
        } else {
            read_to.max(notified.unwrap_or(0))
        };
        self.store.set_state(&key, cursor.to_string())?;
        Ok(heard)
    }

    /// The places a source reading what arrives on `connection` can be
    /// narrowed to: a Google account's labels, a Slack workspace's channels.
    pub async fn lookout_places(
        &self,
        connection: &str,
    ) -> anyhow::Result<Vec<crate::lookout::Place>> {
        match self.feed_of(connection)? {
            crate::lookout::Feed::Mail => self.mail_labels(connection).await,
            crate::lookout::Feed::Chat => self.chat_channels(connection).await,
            crate::lookout::Feed::Other => Ok(Vec::new()),
        }
    }

    async fn mail_labels(&self, connection: &str) -> anyhow::Result<Vec<crate::lookout::Place>> {
        let answer = self.tool_json(connection, "list_labels", json!({})).await?;
        let (mut own, mut system) = (Vec::new(), Vec::new());
        for label in answer["labels"].as_array().into_iter().flatten() {
            let (Some(id), Some(name)) = (label["id"].as_str(), label["name"].as_str()) else {
                continue;
            };
            let place = |name: &str| crate::lookout::Place {
                id: id.to_owned(),
                name: name.to_owned(),
            };
            if label["type"] == "user" {
                own.push(place(name));
            } else if let Some((_, name)) = SYSTEM_LABELS.iter().find(|(l, _)| *l == id) {
                system.push(place(name));
            }
        }
        own.sort_by(|a, b| a.name.cmp(&b.name));
        own.extend(system);
        Ok(own)
    }

    async fn chat_channels(&self, connection: &str) -> anyhow::Result<Vec<crate::lookout::Place>> {
        let mut channels = Vec::new();
        let mut cursor = String::new();
        for _ in 0..MAX_PAGES {
            let mut args = json!({});
            if !cursor.is_empty() {
                args["cursor"] = json!(cursor);
            }
            let answer = self.tool_json(connection, "list_channels", args).await?;
            for c in answer["channels"].as_array().into_iter().flatten() {
                if let (Some(id), Some(name)) = (c["id"].as_str(), c["name"].as_str()) {
                    channels.push(crate::lookout::Place {
                        id: id.to_owned(),
                        name: format!("#{name}"),
                    });
                }
            }
            cursor = answer
                .pointer("/response_metadata/next_cursor")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if cursor.is_empty() {
                break;
            }
        }
        channels.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(channels)
    }

    /// Every event of a Google calendar between `from` and `to`, as the
    /// pages its connection's `list_events` answered.
    pub async fn google_events(
        &self,
        connection: &str,
        calendar: &str,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Vec<String>> {
        let calendar: String = url::form_urlencoded::byte_serialize(calendar.as_bytes()).collect();
        let mut pages = Vec::new();
        let mut page: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut args =
                json!({ "calendar": calendar, "from": from.to_rfc3339(), "to": to.to_rfc3339() });
            if let Some(p) = &page {
                args["page"] = json!(p);
            }
            let answer = self.tool_json(connection, "list_events", args).await?;
            page = answer["nextPageToken"].as_str().map(str::to_owned);
            pages.push(answer.to_string());
            if page.is_none() {
                break;
            }
        }
        Ok(pages)
    }

    /// Ask again, before they lapse, for the notices of every Google
    /// account: new mail when its Pub/Sub topic is set, and calendar changes
    /// when the receivers' address and the account's token are set.
    pub async fn renew_google_watches(&self, receive_url: Option<&str>) {
        let now = chrono::Utc::now().timestamp_millis();
        let Ok(list) = self.connections.list() else {
            return;
        };
        // An account not logged in would only be refused; it is asked once its
        // login is done.
        for c in list
            .into_iter()
            .filter(|c| is_google(c) && c.status == LOGGED_IN)
        {
            let names = self.gateway.account_names(&c.id).unwrap_or_default();
            let set = |k: &str| names.contains(k);
            if set("topic") {
                self.renew(&c.id, "watch_mailbox", json!({}), now, |answer| {
                    if let Some(history) = answer["historyId"].as_str() {
                        let key = history_key(&c.id);
                        if self.store.state(&key).ok().flatten().is_none() {
                            let _ = self.store.set_state(&key, history);
                        }
                    }
                })
                .await;
            }
            if let Some(base) = receive_url.filter(|_| set("channel_token")) {
                let address = format!("{}/receive/{}/{CALENDAR}", base.trim_end_matches('/'), c.id);
                let channel = format!("famulen-{}-{now}", c.id);
                let args = json!({ "calendar": "primary", "channel": channel, "address": address });
                self.renew(&c.id, "watch_events", args, now, |_| {}).await;
            }
        }
    }

    /// Call a lookout-asking tool once its last answer's expiration is less
    /// than a day away, and keep the new expiration.
    async fn renew(
        &self,
        connection: &str,
        tool: &str,
        args: Value,
        now: i64,
        then: impl FnOnce(&Value),
    ) {
        const DAY_MS: i64 = 24 * 3600 * 1000;
        // A refused ask is tried again this much later, not every minute.
        const RETRY_MS: i64 = 3600 * 1000;
        let key = format!("renewal.expires.{connection}.{tool}");
        let expires: i64 = self
            .store
            .state(&key)
            .ok()
            .flatten()
            .and_then(|e| e.parse().ok())
            .unwrap_or(0);
        if expires - now > DAY_MS {
            return;
        }
        match self.tool_json(connection, tool, args).await {
            Ok(answer) => {
                let next = match &answer["expiration"] {
                    Value::String(s) => s.parse().unwrap_or(now + DAY_MS),
                    Value::Number(n) => n.as_i64().unwrap_or(now + DAY_MS),
                    _ => now + DAY_MS,
                };
                let _ = self.store.set_state(&key, next.to_string());
                then(&answer);
            }
            Err(error) => {
                tracing::warn!(%connection, %tool, %error, "could not ask for the notices again");
                let _ = self
                    .store
                    .set_state(&key, (now + DAY_MS - RETRY_MS).to_string());
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn a_gmail_notice_tells_its_history_id() {
        let data = base64::engine::general_purpose::STANDARD
            .encode(r#"{"emailAddress":"me@example.com","historyId":9876}"#);
        let payload = json!({ "ackId": "a1", "message": { "data": data } });
        assert_eq!(notified_history(&payload), Some(9876));
        assert_eq!(notified_history(&json!({ "message": {} })), None);
    }

    #[test]
    fn a_history_page_names_the_added_messages() {
        let page = json!({ "history": [
            { "messagesAdded": [{ "message": { "id": "m1" } }] },
            { "messages": [{ "id": "ignored" }] },
            { "messagesAdded": [{ "message": { "id": "m2" } }, { "message": { "id": "m3" } }] }
        ] });
        assert_eq!(added_messages(&page), ["m1", "m2", "m3"]);
        assert!(added_messages(&json!({})).is_empty());
    }

    /// Famulen whose Google definition reaches a local stand-in for Google,
    /// answering each path with the JSON given.
    pub(crate) async fn with_google(
        dir: &std::path::Path,
        answers: Vec<(&'static str, Value)>,
    ) -> (App, String) {
        let mut router = axum::Router::new();
        for (path, answer) in answers {
            router = router.route(
                path,
                axum::routing::get(move || {
                    let answer = answer.clone();
                    async move { answer.to_string() }
                }),
            );
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let services = dir.join("gateway/services");
        std::fs::create_dir_all(&services).unwrap();
        let bundled = include_str!("../../gateway/services/google/google.toml");
        std::fs::write(
            services.join("google.toml"),
            bundled.replace("https://www.googleapis.com", &base),
        )
        .unwrap();
        let app = crate::runtime::testing::app(dir);
        let c = app
            .gateway
            .add_defined(GOOGLE, Default::default(), None)
            .unwrap();
        (app, c.id)
    }

    fn notice(history: u64) -> Value {
        let data = base64::engine::general_purpose::STANDARD.encode(format!(
            r#"{{"emailAddress":"me@example.com","historyId":{history}}}"#
        ));
        json!({ "ackId": "a", "message": { "data": data } })
    }

    #[tokio::test]
    async fn a_mail_notice_is_read_through_the_tools_into_what_the_lookout_hears() {
        let dir = crate::notebook::testing::TempDir::new("arrivals-mail");
        let (app, id) = with_google(
            dir.path(),
            vec![
                (
                    "/gmail/v1/users/me/history",
                    json!({ "history": [{ "messagesAdded": [{ "message": { "id": "m1" } }] }], "historyId": "12" }),
                ),
                (
                    "/gmail/v1/users/me/messages/m1",
                    json!({ "snippet": "見積もりを送ります", "payload": { "headers": [{ "name": "Subject", "value": "見積もり" }, { "name": "From", "value": "sato@example.com" }] } }),
                ),
            ],
        )
        .await;
        app.lookout
            .add_source(
                "Gmail",
                &id,
                crate::lookout::ARRIVED,
                &json!({}),
                crate::lookout::Feed::Mail,
                Some(60),
            )
            .unwrap();

        let first = app.read_mail_notice(&id, &notice(10)).await.unwrap();
        let second = app.read_mail_notice(&id, &notice(12)).await.unwrap();

        assert_eq!(
            (first, second),
            (0, 1),
            "the first notice only marks where to start"
        );
        let heard = app.lookout.heard_on(&id).unwrap();
        assert_eq!(heard[0].title, "見積もり");
        assert!(heard[0].body.contains("sato@example.com"));
    }

    #[tokio::test]
    async fn mail_goes_to_the_source_of_its_label() {
        let dir = crate::notebook::testing::TempDir::new("arrivals-mail-label");
        let (app, id) = with_google(
            dir.path(),
            vec![
                (
                    "/gmail/v1/users/me/history",
                    json!({ "history": [{ "messagesAdded": [{ "message": { "id": "m1" } }] }], "historyId": "12" }),
                ),
                (
                    "/gmail/v1/users/me/messages/m1",
                    json!({ "labelIds": ["INBOX", "Label_1"], "payload": { "headers": [{ "name": "Subject", "value": "見積もり" }] } }),
                ),
            ],
        )
        .await;
        let add = |name: &str, args: Value| {
            let tool = crate::lookout::ARRIVED;
            app.lookout
                .add_source(name, &id, tool, &args, crate::lookout::Feed::Mail, None)
                .unwrap()
        };
        let rest = add("残り", json!({}));
        let work = add(
            "仕事",
            json!({ "places": [{ "id": "Label_1", "name": "仕事" }] }),
        );
        app.read_mail_notice(&id, &notice(10)).await.unwrap();
        app.read_mail_notice(&id, &notice(12)).await.unwrap();
        assert_eq!(app.lookout.take_heard(work).unwrap().len(), 1);
        assert!(app.lookout.take_heard(rest).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_google_accounts_places_are_its_labels() {
        let dir = crate::notebook::testing::TempDir::new("arrivals-labels");
        let (app, id) = with_google(
            dir.path(),
            vec![(
                "/gmail/v1/users/me/labels",
                json!({ "labels": [
                    { "id": "INBOX", "name": "INBOX", "type": "system" },
                    { "id": "IMPORTANT", "name": "IMPORTANT", "type": "system" },
                    { "id": "Label_1", "name": "仕事", "type": "user" },
                ] }),
            )],
        )
        .await;
        let places = app.lookout_places(&id).await.unwrap();
        let names: Vec<(&str, &str)> = places
            .iter()
            .map(|p| (p.id.as_str(), p.name.as_str()))
            .collect();
        assert_eq!(names, [("Label_1", "仕事"), ("IMPORTANT", "重要")]);
    }

    #[tokio::test]
    async fn mail_whose_notice_was_lost_is_read_when_the_mailbox_is_caught_up() {
        let dir = crate::notebook::testing::TempDir::new("arrivals-mail-catch-up");
        let (app, id) = with_google(
            dir.path(),
            vec![
                (
                    "/gmail/v1/users/me/history",
                    json!({ "history": [{ "id": "12", "messagesAdded": [{ "message": { "id": "m1" } }] }], "historyId": "12" }),
                ),
                (
                    "/gmail/v1/users/me/messages/m1",
                    json!({ "snippet": "届きました", "payload": { "headers": [{ "name": "Subject", "value": "請求書" }] } }),
                ),
            ],
        )
        .await;
        app.lookout
            .add_source(
                "Gmail",
                &id,
                crate::lookout::ARRIVED,
                &json!({}),
                crate::lookout::Feed::Mail,
                Some(60),
            )
            .unwrap();
        app.read_mail_notice(&id, &notice(10)).await.unwrap();

        let heard = app.read_mail_since_last(&id).await.unwrap();

        assert_eq!(heard, 1);
        assert_eq!(app.lookout.heard_on(&id).unwrap()[0].title, "請求書");
        assert_eq!(
            app.store.state(history_key(&id)).unwrap().as_deref(),
            Some("12")
        );
    }

    #[tokio::test]
    async fn a_mailbox_never_noticed_is_not_caught_up() {
        let dir = crate::notebook::testing::TempDir::new("arrivals-mail-unmarked");
        let (app, id) = with_google(dir.path(), vec![]).await;

        assert_eq!(app.read_mail_since_last(&id).await.unwrap(), 0);
        assert_eq!(app.store.state(history_key(&id)).unwrap(), None);
    }

    #[tokio::test]
    async fn a_history_longer_than_the_pages_read_is_taken_up_where_reading_stopped() {
        let dir = crate::notebook::testing::TempDir::new("arrivals-mail-long");
        let (app, id) = with_google(
            dir.path(),
            vec![(
                "/gmail/v1/users/me/history",
                json!({ "history": [{ "id": "11" }], "historyId": "99", "nextPageToken": "more" }),
            )],
        )
        .await;

        app.read_mail_notice(&id, &notice(10)).await.unwrap();
        app.read_mail_notice(&id, &notice(99)).await.unwrap();

        assert_eq!(
            app.store.state(history_key(&id)).unwrap().as_deref(),
            Some("11")
        );
    }

    #[tokio::test]
    async fn a_google_calendar_is_read_through_its_connections_tool() {
        let dir = crate::notebook::testing::TempDir::new("arrivals-calendar");
        let page = json!({ "items": [{ "id": "e1", "summary": "定例", "start": { "dateTime": "2026-11-23T10:00:00+09:00" }, "end": { "dateTime": "2026-11-23T11:00:00+09:00" } }] });
        let (app, id) = with_google(
            dir.path(),
            vec![("/calendar/v3/calendars/primary/events", page.clone())],
        )
        .await;

        let pages = app
            .google_events(&id, "primary", chrono::Utc::now(), chrono::Utc::now())
            .await
            .unwrap();

        assert_eq!(pages.len(), 1);
        let occurrences = crate::calendars::google_events(&pages, chrono_tz::Asia::Tokyo).unwrap();
        assert_eq!(occurrences.len(), 1);
    }

    #[test]
    fn a_message_becomes_an_item_with_its_subject_sender_and_snippet() {
        let message = json!({
            "id": "m1",
            "snippet": "明日の打ち合わせは",
            "payload": { "headers": [
                { "name": "From", "value": "Sato <sato@example.com>" },
                { "name": "subject", "value": "打ち合わせ" }
            ] }
        });
        let item = mail_item(&message);
        assert_eq!(item.title, "打ち合わせ");
        assert_eq!(
            item.body,
            "id: m1\nfrom: Sato <sato@example.com>\n明日の打ち合わせは"
        );
        assert_eq!(message_of(&item), Some("m1"));
    }
}
