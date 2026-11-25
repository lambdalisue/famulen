//! What Famulen does with what the gateway hands over from the bundled
//! Google definition, and how it reads the account's mail for the lookout and
//! its calendars through the connection's tools.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::app::App;
use crate::connections::{Connection, Transport};
use crate::lookout::Item;

/// The bundled definition of a Google account.
pub const GOOGLE: &str = "google";
/// Its receiver of calendar changes.
pub const CALENDAR: &str = "calendar";

/// The status of a connection whose login is done.
const LOGGED_IN: &str = "つながっている";

/// How many pages of a listing are read at most.
const MAX_PAGES: usize = 20;

/// How many mails the owner sent are read to find one person among whom they
/// went to.
const SENT_READ: usize = 5;

static MAIL_READING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn is_google(c: &Connection) -> bool {
    matches!(&c.transport, Transport::Defined { definition, .. } if definition == GOOGLE)
}

fn history_key(connection: &str) -> String {
    format!("mail.history.{connection}")
}

/// A history id, which Gmail writes as a number or as text.
fn history_id(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// A tool call the service answered with an error.
#[derive(Debug)]
struct Refused {
    tool: String,
    text: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text: String = self.text.chars().take(200).collect();
        write!(f, "{} failed: {text}", self.tool)
    }
}

impl std::error::Error for Refused {}

/// Whether Google answered that what was asked for is not there (any more).
pub(crate) fn is_gone(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<Refused>()
        .and_then(|r| serde_json::from_str::<Value>(&r.text).ok())
        .is_some_and(|answer| answer["error"]["code"] == 404)
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
        .filter_map(|h| history_id(&h["id"]))
        .max()
}

/// A message's header, or nothing when it has none of that name.
fn header<'a>(message: &'a Value, name: &str) -> Option<&'a str> {
    message["payload"]["headers"].as_array()?.iter().find(|h| {
        h["name"]
            .as_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    })?["value"]
        .as_str()
}

/// The senders' names of mail no person writes, without the marks between
/// their words.
const AUTOMATED_SENDERS: &[&str] = &[
    "mailerdaemon",
    "postmaster",
    "bounce",
    "bounces",
    "calendarnotification",
    "notification",
    "notifications",
    "newsletter",
    "newsletters",
    "mailmag",
    "mailmagazine",
    "magazine",
    "marketing",
    "promotion",
    "promotions",
];

/// The services that send mail to many at once, by their domains.
const SENDING_SERVICES: &[&str] = &[
    "amazonses.com",
    "cuenote.jp",
    "exacttarget.com",
    "hubspotemail.net",
    "mailchimpapp.net",
    "mailgun.org",
    "mcdlv.net",
    "mcsv.net",
    "rsgsv.net",
    "sendgrid.net",
];

/// Who sends calendar invitations in the name of the person who invites.
const INVITATIONS: &str = "calendar-notification@google.com";

/// The address of a From or the like.
fn address(value: &str) -> &str {
    match value.rsplit_once('<') {
        Some((_, address)) => address.trim_end().trim_end_matches('>'),
        None => value,
    }
    .trim()
}

/// Whether a text is one address and nothing a search reads otherwise.
fn is_address(text: &str) -> bool {
    text.split_once('@').is_some_and(|(name, domain)| {
        !name.is_empty() && domain.contains('.') && !domain.contains('@')
    }) && text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "@.+-_%'".contains(c))
}

/// Whether an address sends mail no person writes.
fn sends_automatically(value: &str) -> bool {
    let address = address(value).to_lowercase();
    let (name, domain) = address.rsplit_once('@').unwrap_or((&address, ""));
    let name: String = name.chars().filter(|c| !"-_.".contains(*c)).collect();
    name.contains("noreply")
        || name.contains("donotreply")
        || AUTOMATED_SENDERS.contains(&name.as_str())
        || SENDING_SERVICES
            .iter()
            .any(|s| domain == *s || domain.ends_with(&format!(".{s}")))
}

/// Whether mail goes out to many or by itself: advertising, newsletters,
/// lists and notices.
fn is_bulk(message: &Value) -> bool {
    let has = |name: &str| header(message, name).is_some();
    let is = |name: &str, words: &[&str]| {
        header(message, name).is_some_and(|v| {
            let v = v.trim();
            words.iter().any(|w| v.eq_ignore_ascii_case(w))
        })
    };
    let labelled = message["labelIds"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|l| {
            l.as_str()
                .is_some_and(|l| ["CATEGORY_PROMOTIONS", "CATEGORY_SOCIAL", "SPAM"].contains(&l))
        });
    let invitation =
        header(message, "Sender").is_some_and(|s| address(s).eq_ignore_ascii_case(INVITATIONS));
    // Only mail sent by itself comes back to an address of its own.
    let comes_back = header(message, "Return-Path").is_some_and(|path| {
        address(path).to_lowercase().contains("bounce") || sends_automatically(path)
    });
    let by_itself =
        header(message, "Auto-Submitted").is_some_and(|v| !v.trim().eq_ignore_ascii_case("no"));
    labelled
        || has("List-Unsubscribe")
        || has("List-Id")
        || is("Precedence", &["bulk", "list", "junk"])
        || header(message, "From").is_some_and(sends_automatically)
        || !invitation && (comes_back || by_itself)
}

/// One message's headers and snippet as an item the lookout reads.
pub fn mail_item(message: &Value) -> Item {
    let header = |name: &str| header(message, name).unwrap_or_default().to_owned();
    let subject = header("Subject");
    let sent = message["internalDate"]
        .as_str()
        .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms.parse().ok()?))
        .map(|at| format!("{SENT}{}\n", at.to_rfc3339()))
        .unwrap_or_default();
    let automated = if is_bulk(message) {
        format!("{AUTOMATED}\n")
    } else {
        String::new()
    };
    Item {
        title: if subject.is_empty() {
            "(件名なし)".to_owned()
        } else {
            subject
        },
        // Two mails of a shop may read the same; its id also names the one to
        // open.
        body: format!(
            "{ID}{}\n{FROM}{}\n{sent}{automated}{}",
            message["id"].as_str().unwrap_or_default(),
            header("From"),
            message["snippet"].as_str().unwrap_or_default()
        ),
    }
}

/// What the line naming a mail in its item starts with.
const ID: &str = "id: ";
const FROM: &str = "from: ";
const SENT: &str = "sent: ";
/// The line of mail that goes out to many or by itself.
const AUTOMATED: &str = "automated: yes";

/// The id of the message an item is, when it is mail.
pub fn message_of(item: &Item) -> Option<&str> {
    let id = item.body.lines().next()?.strip_prefix(ID)?;
    (!id.is_empty()).then_some(id)
}

/// What the lines before an item's text say of mail.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MailHead<'a> {
    pub from: &'a str,
    /// When it was sent, in seconds.
    pub sent: Option<i64>,
    marked: bool,
    /// What is left of it: what the mail itself says.
    pub text: &'a str,
}

impl MailHead<'_> {
    /// Whether the mail goes out to many or by itself.
    pub fn is_automated(&self) -> bool {
        self.marked || sends_automatically(self.from)
    }
}

/// Read the lines before the text of an item that is mail. Each is read only
/// in its place, since the text is the sender's to write.
pub fn mail_head(item: &Item) -> MailHead<'_> {
    fn take<'a>(rest: &mut &'a str, mark: &str) -> Option<&'a str> {
        let (line, after) = rest.split_once('\n').unwrap_or((*rest, ""));
        let value = line.strip_prefix(mark)?;
        *rest = after;
        Some(value)
    }
    let mut rest = item.body.as_str();
    take(&mut rest, ID);
    let from = take(&mut rest, FROM).unwrap_or_default();
    let sent = take(&mut rest, SENT)
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.timestamp());
    let marked = take(&mut rest, AUTOMATED).is_some();
    MailHead {
        from,
        sent,
        marked,
        text: rest,
    }
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
        if result.is_error {
            return Err(Refused {
                tool: tool.to_owned(),
                text: result.text,
            }
            .into());
        }
        Ok(serde_json::from_str(&result.text)?)
    }

    /// Which of `addresses` the owner wrote mail to from a Google account.
    /// What cannot be asked counts as not written to.
    pub async fn written_to(
        &self,
        connection: &str,
        addresses: &BTreeSet<String>,
    ) -> BTreeSet<String> {
        let mut written_to = BTreeSet::new();
        // Mail between the owner's own accounts is no correspondence.
        let own: BTreeSet<String> = self
            .connections
            .list()
            .unwrap_or_default()
            .iter()
            .map(|c| c.account.to_lowercase())
            .collect();
        for address in addresses {
            if own.contains(address) || !is_address(address) {
                continue;
            }
            match self.wrote_to(connection, address).await {
                Ok(true) => {
                    written_to.insert(address.clone());
                }
                Ok(false) => {}
                Err(error) => {
                    // The rest would most likely wait for the same failure.
                    tracing::warn!(%connection, %error, "could not look for mail the owner sent");
                    break;
                }
            }
        }
        written_to
    }

    async fn wrote_to(&self, connection: &str, address: &str) -> anyhow::Result<bool> {
        // Gmail finds those in Cc, and addresses alike, by `to` too.
        let args = json!({ "query": format!("in:sent to:\"{address}\""), "max": SENT_READ });
        let answer = self.tool_json(connection, "search_messages", args).await?;
        for found in answer["messages"].as_array().into_iter().flatten() {
            let args = json!({ "id": found["id"] });
            let message = self.tool_json(connection, "get_message", args).await?;
            let to = header(&message, "To").unwrap_or_default();
            if to
                .split(',')
                .any(|one| self::address(one).eq_ignore_ascii_case(address))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Read every Google account's mail since its last read.
    pub async fn read_every_mail(&self) {
        let Ok(list) = self.connections.list() else {
            return;
        };
        for c in list
            .into_iter()
            .filter(|c| is_google(c) && c.status == LOGGED_IN)
        {
            if let Err(error) = self.read_mail(&c.id).await {
                tracing::warn!(connection = %c.id, %error, "could not read the mail");
            }
        }
    }

    /// Hand the mail added since the last read to the lookout as heard from
    /// the connection. The first read only marks where to read from.
    pub async fn read_mail(&self, connection: &str) -> anyhow::Result<usize> {
        // Two reads of the same mailbox at once, as when a login ends, would
        // read from the same place and hand the lookout the mail twice.
        let _reading = MAIL_READING.lock().await;
        let key = history_key(connection);
        let Some(since) = self.store.state(&key)? else {
            self.mark_mail(connection).await?;
            return Ok(0);
        };
        let mut ids = Vec::new();
        let mut page: Option<String> = None;
        let mut read_to: u64 = since.parse().unwrap_or(0);
        let mut latest = None;
        for _ in 0..MAX_PAGES {
            let mut args = json!({ "since": since });
            if let Some(p) = &page {
                args["page"] = json!(p);
            }
            let answer = match self.tool_json(connection, "list_history", args).await {
                Err(error) if is_gone(&error) => {
                    // Gmail keeps the history for about a week, so after a
                    // longer stop there is nothing left to read on from.
                    tracing::warn!(%connection, "the mail's history is gone; reading from now on");
                    self.mark_mail(connection).await?;
                    return Ok(0);
                }
                other => other?,
            };
            ids.extend(added_messages(&answer));
            read_to = read_to.max(last_history(&answer).unwrap_or(0));
            // Mail that came while the pages were read is after the first
            // page's id, so it is in the next read whatever page it fell on.
            latest = latest.or_else(|| history_id(&answer["historyId"]));
            page = answer["nextPageToken"].as_str().map(str::to_owned);
            if page.is_none() {
                break;
            }
        }
        ids.dedup();
        let mut heard = 0;
        for id in &ids {
            let message = match self
                .tool_json(connection, "get_message", json!({ "id": id }))
                .await
            {
                // Deleted before it was read.
                Err(error) if is_gone(&error) => continue,
                other => other?,
            };
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
        // Pages left unread are read from the last record read next time.
        let cursor = match latest {
            Some(latest) if page.is_none() => read_to.max(latest),
            _ => read_to,
        };
        self.store.set_state(&key, cursor.to_string())?;
        Ok(heard)
    }

    /// Mark the mailbox as read up to now.
    async fn mark_mail(&self, connection: &str) -> anyhow::Result<()> {
        let profile = self.tool_json(connection, "get_profile", json!({})).await?;
        let now = history_id(&profile["historyId"])
            .ok_or_else(|| anyhow::anyhow!("the mailbox's profile has no history id"))?;
        self.store
            .set_state(history_key(connection), now.to_string())
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
    /// account's calendar changes, when the receivers are reached from outside.
    pub async fn renew_google_watches(&self, receive_url: Option<&str>) {
        let now = chrono::Utc::now().timestamp_millis();
        let Some(base) = receive_url else {
            return;
        };
        let Ok(list) = self.connections.list() else {
            return;
        };
        // An account not logged in would only be refused; it is asked once its
        // login is done.
        for c in list
            .into_iter()
            .filter(|c| is_google(c) && c.status == LOGGED_IN)
        {
            let address = format!("{}/receive/{}/{CALENDAR}", base.trim_end_matches('/'), c.id);
            let channel = format!("famulen-{}-{now}", c.id);
            let args = json!({ "calendar": "primary", "channel": channel, "address": address });
            self.renew(&c.id, "watch_events", args, now).await;
        }
    }

    /// Call a notice-asking tool once its last answer's expiration is less
    /// than a day away, and keep the new expiration.
    async fn renew(&self, connection: &str, tool: &str, args: Value, now: i64) {
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
                    let status = answer["error"]["code"].as_u64().unwrap_or(200) as u16;
                    async move {
                        (
                            axum::http::StatusCode::from_u16(status).unwrap(),
                            answer.to_string(),
                        )
                    }
                }),
            );
        }
        serving(dir, router).await
    }

    /// Famulen with a Google account in which a search for `sent` finds a mail
    /// that went to `to`, and any other finds none.
    pub(crate) async fn with_mailbox(
        dir: &std::path::Path,
        sent: &'static str,
        to: &'static str,
    ) -> (App, String) {
        use axum::extract::Query;
        let search = move |Query(query): Query<std::collections::BTreeMap<String, String>>| {
            let found = query.get("q").map(String::as_str) == Some(sent);
            async move {
                if found {
                    json!({ "messages": [{ "id": "s1" }] }).to_string()
                } else {
                    json!({ "resultSizeEstimate": 0 }).to_string()
                }
            }
        };
        let message = json!({ "payload": { "headers": [{ "name": "To", "value": to }] } });
        let message = message.to_string();
        let router = axum::Router::new()
            .route("/gmail/v1/users/me/messages", axum::routing::get(search))
            .route(
                "/gmail/v1/users/me/messages/s1",
                axum::routing::get(move || async move { message }),
            );
        serving(dir, router).await
    }

    async fn serving(dir: &std::path::Path, router: axum::Router) -> (App, String) {
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

    fn mailbox_at(history: &str) -> (&'static str, Value) {
        (
            "/gmail/v1/users/me/profile",
            json!({ "emailAddress": "me@example.com", "historyId": history }),
        )
    }

    fn gone() -> Value {
        json!({ "error": { "code": 404, "message": "Requested entity was not found." } })
    }

    #[tokio::test]
    async fn mail_added_since_the_last_read_is_read_through_the_tools_into_what_the_lookout_hears()
    {
        let dir = crate::notebook::testing::TempDir::new("arrivals-mail");
        let (app, id) = with_google(
            dir.path(),
            vec![
                mailbox_at("10"),
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

        let first = app.read_mail(&id).await.unwrap();
        let second = app.read_mail(&id).await.unwrap();

        assert_eq!(
            (first, second),
            (0, 1),
            "the first read only marks where to start"
        );
        let heard = app.lookout.heard_on(&id).unwrap();
        assert_eq!(heard[0].title, "見積もり");
        assert!(heard[0].body.contains("sato@example.com"));
        assert_eq!(
            app.store.state(history_key(&id)).unwrap().as_deref(),
            Some("12"),
            "the next read is from the mailbox's latest record"
        );
    }

    #[tokio::test]
    async fn mail_goes_to_the_source_of_its_label() {
        let dir = crate::notebook::testing::TempDir::new("arrivals-mail-label");
        let (app, id) = with_google(
            dir.path(),
            vec![
                mailbox_at("10"),
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
        app.read_mail(&id).await.unwrap();
        app.read_mail(&id).await.unwrap();
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
    async fn a_message_deleted_before_it_was_read_does_not_hold_up_the_rest() {
        let dir = crate::notebook::testing::TempDir::new("arrivals-mail-deleted");
        let (app, id) = with_google(
            dir.path(),
            vec![
                mailbox_at("10"),
                (
                    "/gmail/v1/users/me/history",
                    json!({ "history": [{ "id": "12", "messagesAdded": [{ "message": { "id": "m1" } }, { "message": { "id": "m2" } }] }], "historyId": "12" }),
                ),
                ("/gmail/v1/users/me/messages/m1", gone()),
                (
                    "/gmail/v1/users/me/messages/m2",
                    json!({ "payload": { "headers": [{ "name": "Subject", "value": "請求書" }] } }),
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

        app.read_mail(&id).await.unwrap();

        assert_eq!(app.read_mail(&id).await.unwrap(), 1);
        assert_eq!(app.lookout.heard_on(&id).unwrap()[0].title, "請求書");
    }

    #[tokio::test]
    async fn a_mailbox_whose_history_is_gone_is_read_from_now_on() {
        let dir = crate::notebook::testing::TempDir::new("arrivals-mail-expired");
        let (app, id) = with_google(
            dir.path(),
            vec![mailbox_at("500"), ("/gmail/v1/users/me/history", gone())],
        )
        .await;
        app.store.set_state(history_key(&id), "10").unwrap();

        assert_eq!(app.read_mail(&id).await.unwrap(), 0);
        assert_eq!(
            app.store.state(history_key(&id)).unwrap().as_deref(),
            Some("500")
        );
    }

    #[tokio::test]
    async fn a_mailbox_that_cannot_be_read_is_taken_up_from_the_same_place() {
        let dir = crate::notebook::testing::TempDir::new("arrivals-mail-failed");
        let (app, id) = with_google(
            dir.path(),
            vec![(
                "/gmail/v1/users/me/history",
                json!({ "error": { "code": 500 } }),
            )],
        )
        .await;
        app.store.set_state(history_key(&id), "10").unwrap();

        assert!(app.read_mail(&id).await.is_err());
        assert_eq!(
            app.store.state(history_key(&id)).unwrap().as_deref(),
            Some("10")
        );
    }

    #[tokio::test]
    async fn a_history_longer_than_the_pages_read_is_taken_up_where_reading_stopped() {
        let dir = crate::notebook::testing::TempDir::new("arrivals-mail-long");
        let (app, id) = with_google(
            dir.path(),
            vec![
                mailbox_at("10"),
                (
                    "/gmail/v1/users/me/history",
                    json!({ "history": [{ "id": "11" }], "historyId": "99", "nextPageToken": "more" }),
                ),
            ],
        )
        .await;

        app.read_mail(&id).await.unwrap();
        app.read_mail(&id).await.unwrap();

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

    #[tokio::test]
    async fn only_whom_mail_was_addressed_to_was_written_to() {
        let addresses = BTreeSet::from(["sato@example.com".to_owned()]);
        for (to, written) in [
            ("\"Sato, T\" <Sato@example.com>, mori@example.com", true),
            ("sato.taro@example.com, jiro.sato@example.com", false),
            ("", false),
        ] {
            let dir = crate::notebook::testing::TempDir::new("arrivals-written");
            let (app, id) = with_mailbox(dir.path(), r#"in:sent to:"sato@example.com""#, to).await;
            let found = app.written_to(&id, &addresses).await;
            assert_eq!(found.len() == 1, written, "{to}");
        }
    }

    #[test]
    fn only_a_plain_address_is_looked_for() {
        assert!(is_address("a.b+c@example.co.jp"));
        for text in [
            "",
            "a",
            "a@b",
            "@b.c",
            "a@b.c@d.e",
            "a@b.c\" OR x",
            "a@b.c d",
        ] {
            assert!(!is_address(text), "{text}");
        }
    }

    fn mail(headers: &[(&str, &str)], labels: &[&str]) -> Item {
        let headers: Vec<Value> = headers
            .iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect();
        mail_item(&json!({
            "id": "m1",
            "internalDate": "1795598544000",
            "labelIds": labels,
            "snippet": "本文",
            "payload": { "headers": headers }
        }))
    }

    #[test]
    fn mail_to_many_or_by_itself_is_told_by_its_headers_sender_or_label() {
        let person = ("From", "Sato <sato@example.com>");
        for headers in [
            &[person, ("List-Unsubscribe", "<https://a.example/u>")][..],
            &[person, ("List-Id", "<dev.example.com>")],
            &[person, ("Precedence", "Bulk")],
            &[person, ("Auto-Submitted", "auto-generated")],
            &[person, ("Return-Path", "<bounces+1-a=b@em1.example.com>")],
            &[person, ("Return-Path", "<3z4@rtc.bounces.google.com>")],
            &[
                person,
                ("Return-Path", "<0101@ap-northeast-1.amazonses.com>"),
            ],
            &[
                ("From", "Google Calendar <calendar-notification@google.com>"),
                ("Sender", "Google <calendar-notification@google.com>"),
            ],
            &[("From", "Shop <no_reply@example.com>")],
            &[("From", "LinkedIn <updates-noreply@linkedin.com>")],
            &[("From", "Discord <Notifications@discord.com>")],
        ] {
            assert!(mail_head(&mail(headers, &[])).is_automated(), "{headers:?}");
        }
        assert!(mail_head(&mail(&[person], &["CATEGORY_PROMOTIONS"])).is_automated());
    }

    #[test]
    fn mail_a_person_wrote_and_an_invitation_in_their_name_are_not_automated() {
        let person = ("From", "Sato <sato@example.com>");
        for headers in [
            &[person, ("Return-Path", "<sato@example.com>")][..],
            &[person, ("Auto-Submitted", "no")],
            &[
                person,
                ("Sender", "Google <calendar-notification@google.com>"),
                ("Auto-Submitted", "auto-generated"),
                ("Return-Path", "<3a@calendar-server.bounces.google.com>"),
            ],
            // A service's name within another's.
            &[("From", "a@notsendgrid.net")],
            &[("From", "marketing.sato@example.com")],
        ] {
            assert!(
                !mail_head(&mail(headers, &["INBOX"])).is_automated(),
                "{headers:?}"
            );
        }
    }

    #[test]
    fn the_head_of_mail_is_read_apart_from_what_it_says() {
        let item = mail(&[("From", "a@example.com"), ("List-Id", "x")], &[]);
        let head = mail_head(&item);
        assert_eq!(head.from, "a@example.com");
        assert_eq!(head.sent, Some(1_795_598_544));
        assert_eq!(head.text, "本文");

        let before = Item {
            title: "t".into(),
            body: "id: m1\nfrom: a@example.com\n本文\nautomated: yes".into(),
        };
        let head = mail_head(&before);
        assert_eq!((head.sent, head.is_automated()), (None, false));
        assert_eq!(head.text, "本文\nautomated: yes");
    }
}
