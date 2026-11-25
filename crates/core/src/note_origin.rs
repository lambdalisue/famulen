//! Where a note the lookout wrote came from. The note itself keeps only which
//! message that was; what the message said is told from what the lookout
//! read, which stays out of the notebook.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::json;

use crate::app::App;
use crate::arrivals;
use crate::connections::Connection;
use crate::lookout::{Feed, Item, Outcome, shown_text};
use crate::people;

/// How much of what was read is shown.
const EXCERPT_CHARS: usize = 160;
/// How long one answer of the service is waited for.
const PATIENCE: Duration = Duration::from_secs(4);

/// The message a note came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Mail {
        connection: String,
        id: String,
    },
    Slack {
        connection: String,
        channel: String,
        ts: String,
    },
}

/// What the line before a chat message's text says of it.
struct ChatHead<'a> {
    channel: &'a str,
    user: &'a str,
    ts: &'a str,
    text: &'a str,
}

fn chat_head(item: &Item) -> Option<ChatHead<'_>> {
    let (line, text) = item.body.split_once('\n')?;
    let mut parts = line.split(" · ");
    Some(ChatHead {
        channel: parts.next()?.strip_prefix("channel: ")?,
        user: parts.next()?.strip_prefix("from: ")?,
        ts: parts.next()?.strip_prefix("ts: ")?,
        text,
    })
}

/// Whether a part stands in an origin as it is.
fn is_plain(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

impl Origin {
    /// The attribute of a note that holds its origin.
    pub const ATTR: &str = "origin";

    /// The message an item read through `connection` is.
    pub fn of(feed: Feed, connection: &str, item: &Item) -> Option<Self> {
        let origin = match feed {
            Feed::Mail => Self::Mail {
                connection: connection.to_owned(),
                id: arrivals::message_of(item)?.to_owned(),
            },
            Feed::Chat => {
                let head = chat_head(item)?;
                Self::Slack {
                    connection: connection.to_owned(),
                    channel: head.channel.to_owned(),
                    ts: head.ts.to_owned(),
                }
            }
            Feed::Other => return None,
        };
        // Only what reads back as the same message names it.
        (Self::parse(&origin.to_string()).as_ref() == Some(&origin)).then_some(origin)
    }

    pub fn parse(text: &str) -> Option<Self> {
        let parts: Vec<&str> = text.split('/').collect();
        if !parts.iter().all(|p| is_plain(p)) {
            return None;
        }
        match parts[..] {
            ["mail", connection, id] => Some(Self::Mail {
                connection: connection.into(),
                id: id.into(),
            }),
            ["slack", connection, channel, ts] => Some(Self::Slack {
                connection: connection.into(),
                channel: channel.into(),
                ts: ts.into(),
            }),
            _ => None,
        }
    }

    fn connection(&self) -> &str {
        match self {
            Self::Mail { connection, .. } | Self::Slack { connection, .. } => connection,
        }
    }

    /// What the lookout's items of it start with, and how their first line
    /// ends.
    fn marks(&self) -> (String, String) {
        match self {
            Self::Mail { id, .. } => (format!("id: {id}\n"), String::new()),
            Self::Slack { channel, ts, .. } => (
                format!("channel: {channel} · from: "),
                format!(" · ts: {ts}\n"),
            ),
        }
    }
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mail { connection, id } => write!(f, "mail/{connection}/{id}"),
            Self::Slack {
                connection,
                channel,
                ts,
            } => write!(f, "slack/{connection}/{channel}/{ts}"),
        }
    }
}

/// What can still be told of where a note came from. Whatever is not known
/// is empty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Told {
    pub origin: Option<Origin>,
    /// The connection's account, or its name.
    pub account: String,
    /// The channel, by its name where Slack tells it.
    pub channel: String,
    pub from: String,
    pub subject: String,
    /// When it was written, in seconds.
    pub at: Option<i64>,
    pub url: String,
    pub excerpt: String,
    /// Where the lookout sent it, and why.
    pub outcome: Option<Outcome>,
    pub reason: String,
}

/// The start of a text on one line, without the characters that only fill up
/// the preview of mail.
fn excerpt(text: &str) -> String {
    let filler = |c: char| matches!(c, '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}');
    let shown: String = text.chars().filter(|c| !filler(*c)).collect();
    let words = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut cut: String = words.chars().take(EXCERPT_CHARS).collect();
    if words.chars().count() > EXCERPT_CHARS {
        cut.push('…');
    }
    cut
}

/// What a mail's snippet shows; Gmail gives it escaped as HTML.
fn mail_excerpt(text: &str) -> String {
    excerpt(&shown_text(text, EXCERPT_CHARS * 4).unwrap_or_default())
}

/// A Slack message with its links and channels as what they show.
fn unmarked(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('<') {
        out.push_str(before);
        let Some((mark, after)) = after.split_once('>') else {
            rest = after;
            continue;
        };
        match mark.split_once('|').filter(|(_, label)| !label.is_empty()) {
            Some((target, label)) => {
                if target.starts_with('#') {
                    out.push('#');
                }
                out.push_str(label);
            }
            None => out.push_str(mark.trim_end_matches('|')),
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// What a Slack message shows, its members by their names.
fn chat_excerpt(text: &str, names: &BTreeMap<String, String>) -> String {
    let ids: Vec<&str> = names.keys().map(String::as_str).collect();
    let text = unmarked(&people::with_names(text, &ids, |id| names.get(id).cloned()))
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    excerpt(&text)
}

/// Where a mail opens in Gmail, in the account it is in.
fn mail_url(account: &str, id: &str) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    if account.contains('@') {
        query.append_pair("authuser", account);
    }
    format!("https://mail.google.com/mail/?{}#all/{id}", query.finish())
}

impl App {
    /// What can still be told of where a note with this origin came from.
    pub async fn told_of(&self, origin: Origin) -> anyhow::Result<Told> {
        let connection = self.connections.get(origin.connection())?;
        let (start, end) = origin.marks();
        let seen = self.lookout.seen(origin.connection(), &start, &end)?;
        let mut told = Told {
            account: connection
                .as_ref()
                .map(crate::channel::account_name)
                .unwrap_or_default(),
            ..Told::default()
        };
        if let Some(seen) = &seen {
            told.outcome = Some(seen.outcome);
            told.reason.clone_from(&seen.reason);
        }
        match &origin {
            Origin::Mail { id, .. } => {
                let account = connection.as_ref().map_or("", |c| c.account.as_str());
                told.url = mail_url(account, id);
                if let Some(seen) = &seen {
                    let head = arrivals::mail_head(&seen.item);
                    told.from = head.from.to_owned();
                    told.subject.clone_from(&seen.item.title);
                    told.at = head.sent.or(Some(seen.at));
                    told.excerpt = mail_excerpt(head.text);
                }
            }
            Origin::Slack { channel, ts, .. } => {
                told.channel.clone_from(channel);
                told.at = ts.split('.').next().and_then(|s| s.parse().ok());
                let head = seen.as_ref().and_then(|s| chat_head(&s.item));
                let names = match &connection {
                    Some(c) if crate::channel::is_slack(c) => {
                        let members = head.as_ref().map(members_of).unwrap_or_default();
                        let (names, name, url) = tokio::join!(
                            self.slack_names_of(&c.id, &members),
                            self.channel_name(c, channel),
                            self.permalink(c, channel, ts),
                        );
                        if let Some(name) = name {
                            told.channel = name;
                        }
                        told.url = url.unwrap_or_default();
                        names
                    }
                    _ => BTreeMap::new(),
                };
                if let Some(head) = head {
                    let name = names.get(head.user).map_or(head.user, String::as_str);
                    told.from = name.to_owned();
                    told.excerpt = chat_excerpt(head.text, &names);
                }
            }
        }
        told.origin = Some(origin);
        Ok(told)
    }

    /// What can still be told of a source as a memory writes it: an origin, or
    /// `slack:<channel>/<ts>`, which leaves open which workspace it is in.
    pub async fn told_of_source(&self, source: &str) -> anyhow::Result<Told> {
        if let Some(origin) = Origin::parse(source) {
            return self.told_of(origin).await;
        }
        let Some((channel, ts)) = source
            .strip_prefix("slack:")
            .and_then(|rest| rest.split_once('/'))
            .filter(|(channel, ts)| is_plain(channel) && is_plain(ts))
        else {
            return Ok(Told::default());
        };
        let asked = self
            .connections
            .list()?
            .into_iter()
            .filter(crate::channel::is_slack)
            .map(|c| async move {
                let url = self.permalink(&c, channel, ts).await;
                url.map(|_| c.id)
            });
        let Some(connection) = futures_util::future::join_all(asked)
            .await
            .into_iter()
            .flatten()
            .next()
        else {
            return Ok(Told::default());
        };
        let (channel, ts) = (channel.to_owned(), ts.to_owned());
        self.told_of(Origin::Slack {
            connection,
            channel,
            ts,
        })
        .await
    }

    /// The text at `pointer` of what a tool answers in time.
    async fn asked(
        &self,
        c: &Connection,
        tool: &str,
        args: serde_json::Value,
        pointer: &str,
    ) -> Option<String> {
        let asked = tokio::time::timeout(PATIENCE, self.tool_json(&c.id, tool, args)).await;
        match asked {
            Ok(Ok(answer)) => Some(answer.pointer(pointer)?.as_str()?.to_owned()),
            Ok(Err(error)) => {
                tracing::debug!(connection = %c.id, %tool, %error, "Slack did not tell of a message");
                None
            }
            Err(_) => None,
        }
    }

    pub(crate) async fn channel_name(&self, c: &Connection, channel: &str) -> Option<String> {
        // Slack names neither, and the owner's token may not ask of them.
        if channel.starts_with('D') {
            return Some("DM".into());
        }
        let args = json!({ "channel": channel });
        let name = self.asked(c, "channel_info", args, "/channel/name").await?;
        Some(if name.starts_with("mpdm-") {
            "グループの DM".into()
        } else {
            format!("#{name}")
        })
    }

    async fn permalink(&self, c: &Connection, channel: &str, ts: &str) -> Option<String> {
        let args = json!({ "channel": channel, "ts": ts });
        self.asked(c, "get_permalink", args, "/permalink")
            .await
            .filter(|url| url.starts_with("https://"))
    }
}

/// The members who wrote a message and are named in it, each once.
fn members_of(head: &ChatHead<'_>) -> Vec<String> {
    let named = head
        .text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| people::is_member(word));
    let mut members: Vec<String> = Vec::new();
    for member in std::iter::once(head.user).chain(named) {
        if !members.iter().any(|m| m == member) {
            members.push(member.to_owned());
        }
    }
    members
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookout::{ARRIVED, Screened, Urgency, Verdict};
    use crate::notebook::testing::TempDir;

    fn item(title: &str, body: &str) -> Item {
        Item {
            title: title.into(),
            body: body.into(),
        }
    }

    async fn read(app: &App, connection: &str, feed: Feed, item: Item) {
        app.lookout
            .add_source("s", connection, ARRIVED, &json!({}), feed, None)
            .unwrap();
        let source = app.lookout.sources().unwrap()[0].id.to_string();
        app.lookout
            .screen(&source, vec![item], |_| {
                std::future::ready(Ok(Some(Verdict {
                    items: vec![Screened {
                        related: false,
                        urgency: Urgency::Later,
                    }],
                    groups: vec![],
                })))
            })
            .await
            .unwrap();
    }

    #[test]
    fn an_origin_reads_back_as_the_same_message_or_is_none() {
        let mail = item("t", "id: 1a21\nfrom: a\nx");
        let chat = item("t", "channel: C1 · from: U1 · ts: 1.0\nx");
        for (feed, item, written) in [
            (Feed::Mail, &mail, "mail/google-2/1a21"),
            (Feed::Chat, &chat, "slack/google-2/C1/1.0"),
        ] {
            let origin = Origin::of(feed, "google-2", item).unwrap();
            assert_eq!(origin.to_string(), written);
            assert_eq!(Origin::parse(written), Some(origin));
        }
        assert_eq!(Origin::of(Feed::Other, "c", &mail), None);
        assert_eq!(Origin::of(Feed::Chat, "c", &mail), None);
        let slash = item("t", "id: a/b\nfrom: a\nx");
        assert_eq!(Origin::of(Feed::Mail, "c", &slash), None);
        assert_eq!(Origin::parse("mail/c"), None);
    }

    #[tokio::test]
    async fn a_mails_note_tells_its_sender_subject_link_and_a_short_excerpt() {
        let dir = TempDir::new("origin-mail");
        let app = crate::runtime::testing::app(dir.path());
        let body = format!(
            "id: 1a21\nfrom: 田中 <t@example.com>\nsent: 2026-11-25T09:22:24+00:00\nautomated: yes\nR&amp;D\u{34f} \u{200c}{}",
            "長".repeat(500)
        );
        read(&app, "g", Feed::Mail, item("件名", &body)).await;

        let origin = Origin::parse("mail/g/1a21").unwrap();
        let told = app.told_of(origin).await.unwrap();

        assert_eq!(told.from, "田中 <t@example.com>");
        assert_eq!(told.subject, "件名");
        assert_eq!(told.at, Some(1_795_598_544));
        assert_eq!(told.url, "https://mail.google.com/mail/?#all/1a21");
        assert!(told.excerpt.starts_with("R&D 長"), "{}", told.excerpt);
        assert_eq!(told.excerpt.chars().count(), EXCERPT_CHARS + 1);
        assert_eq!(told.outcome, Some(Outcome::Unrelated));
    }

    #[test]
    fn a_mail_opens_in_the_account_it_is_in() {
        assert_eq!(
            mail_url("me+a@example.com", "x"),
            "https://mail.google.com/mail/?authuser=me%2Ba%40example.com#all/x"
        );
    }

    #[tokio::test]
    async fn a_chats_note_tells_its_channel_writer_link_and_excerpt_with_names_for_ids() {
        let dir = TempDir::new("origin-chat");
        let (app, connection, _) = crate::slack::names::tests::with_slack(
            dir.path(),
            &[("U089L8QJU2Y", "倉谷匠"), ("U0TANAKA1", "田中")],
        )
        .await;
        let body = "channel: C1 · from: U089L8QJU2Y · ts: 1795598544.5\n<@U0TANAKA1> a &lt; b <https://x.example|資料> <#C2|dev> <https://y.example>";
        read(&app, &connection, Feed::Chat, item("t", body)).await;

        let origin = Origin::parse(&format!("slack/{connection}/C1/1795598544.5")).unwrap();
        let told = app.told_of(origin).await.unwrap();

        assert_eq!(told.from, "倉谷匠");
        assert_eq!(told.channel, "#general");
        assert_eq!(told.at, Some(1_795_598_544));
        assert_eq!(told.url, "https://acme.slack.com/archives/C1/p1");
        assert_eq!(
            told.excerpt,
            "田中（U0TANAKA1） a < b 資料 #dev https://y.example"
        );
    }

    #[tokio::test]
    async fn a_source_that_names_no_workspace_is_found_in_the_one_that_knows_it() {
        let dir = TempDir::new("origin-source");
        let (app, _, _) = crate::slack::names::tests::with_slack(dir.path(), &[]).await;

        let told = app.told_of_source("slack:C1/1795598544.5").await.unwrap();
        assert_eq!(told.url, "https://acme.slack.com/archives/C1/p1");
        assert_eq!(told.channel, "#general");
        assert_eq!(told.at, Some(1_795_598_544));

        for unknown in ["slack:プロフィール", "AI との会話", ""] {
            assert_eq!(app.told_of_source(unknown).await.unwrap(), Told::default());
        }
    }

    #[tokio::test]
    async fn what_another_connection_read_of_the_same_message_is_not_told() {
        let dir = TempDir::new("origin-other");
        let app = crate::runtime::testing::app(dir.path());
        read(
            &app,
            "other",
            Feed::Mail,
            item("件名", "id: 1a21\nfrom: a\nx"),
        )
        .await;

        let origin = Origin::parse("mail/g/1a21").unwrap();

        assert_eq!(app.told_of(origin).await.unwrap().subject, "");
    }

    #[tokio::test]
    async fn what_the_lookout_did_not_read_here_is_told_by_its_link_alone() {
        let dir = TempDir::new("origin-none");
        let app = crate::runtime::testing::app(dir.path());
        let other = item("他", "id: 1a21b\nfrom: a\nid: 1a21\nx");
        read(&app, "g", Feed::Mail, other).await;

        let origin = Origin::parse("mail/g/1a21").unwrap();
        let told = app.told_of(origin).await.unwrap();

        assert_eq!((told.from.as_str(), told.excerpt.as_str()), ("", ""));
        assert_eq!((told.at, told.outcome), (None, None));
        assert!(told.url.ends_with("#all/1a21"));
    }
}
