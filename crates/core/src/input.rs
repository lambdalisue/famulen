//! What reaches the session: who sent it, where it came from, and its text.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sender {
    Owner,
    Other,
    /// famulen itself: schedules and wake-ups.
    System,
}

impl Sender {
    pub fn as_str(self) -> &'static str {
        match self {
            Sender::Owner => "owner",
            Sender::Other => "other",
            Sender::System => "system",
        }
    }
}

/// Where an input came from, with what the model needs to answer there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    Slack {
        /// The connection of the workspace it came in.
        workspace: String,
        channel: String,
        ts: String,
        thread_ts: Option<String>,
        user: String,
        /// Taken without a mention, only because the channel is attended:
        /// it may not be for the secretary at all.
        #[serde(default)]
        unmentioned: bool,
    },
    /// The owner answered an approval request.
    Approval { id: i64, user: String },
    /// A schedule came due. `at` is when, in the owner's time zone.
    Schedule { name: String, at: String },
    /// A wake-up the agent set for itself came due.
    Wake { id: i64, at: String },
    /// The owner wrote on the console, while looking at `view` in `workgroup`.
    Console {
        message: i64,
        workgroup: String,
        view: String,
        selection: Option<String>,
    },
    /// A connected service told the secretary something itself.
    Arrival {
        connection: String,
        receiver: String,
        /// Tells the same thing told again from another.
        id: String,
        /// What it is about, such as a session.
        #[serde(default)]
        about: String,
    },
    /// The lookout passed something on.
    Lookout { item: i64, source: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    pub sender: Sender,
    pub source: Source,
    pub text: String,
}

impl Input {
    /// Identifies a delivery from the outside, so a resent event is kept once.
    pub fn dedup_key(&self) -> String {
        match &self.source {
            Source::Slack {
                workspace,
                channel,
                ts,
                ..
            } => format!("slack:{workspace}:{channel}:{ts}"),
            Source::Approval { id, .. } => format!("approval:{id}"),
            Source::Schedule { name, at } => format!("schedule:{name}:{at}"),
            Source::Wake { id, .. } => format!("wake:{id}"),
            Source::Console { message, .. } => format!("console:{message}"),
            Source::Arrival {
                connection,
                receiver,
                id,
                ..
            } => format!("arrival:{connection}:{receiver}:{id}"),
            Source::Lookout { item, .. } => format!("lookout:{item}"),
        }
    }

    /// The text the model reads: a header line saying where this came from
    /// and how to answer there, then the message itself.
    pub fn render(&self) -> String {
        let header = match &self.source {
            Source::Slack {
                workspace,
                channel,
                ts,
                thread_ts,
                user,
                unmentioned,
            } => {
                let thread = thread_ts
                    .as_deref()
                    .map(|t| format!(" thread_ts: {t}"))
                    .unwrap_or_default();
                let unmentioned = if *unmentioned { " unmentioned" } else { "" };
                format!(
                    "[slack] workspace: {workspace} from: {} ({user}) channel: {channel} ts: {ts}{thread}{unmentioned}",
                    self.sender.as_str()
                )
            }
            Source::Approval { id, user } => {
                format!(
                    "[approval] from: {} ({user}) request: #{id}",
                    self.sender.as_str()
                )
            }
            Source::Schedule { name, at } => format!("[schedule] name: {name} at: {at}"),
            Source::Wake { id, at } => format!("[wake] id: {id} at: {at}"),
            Source::Console {
                message,
                workgroup,
                view,
                selection,
            } => {
                let selection = selection
                    .as_deref()
                    .map(|s| format!(" selection: {s}"))
                    .unwrap_or_default();
                format!(
                    "[console] from: {} message: #{message} workgroup: {workgroup} view: {view}{selection}",
                    self.sender.as_str()
                )
            }
            Source::Arrival {
                connection,
                receiver,
                ..
            } => format!("[arrival] connection: {connection} receiver: {receiver}"),
            Source::Lookout { item, source } => format!("[lookout] item: #{item} source: {source}"),
        };
        format!("{header}\n{}", self.text)
    }

    /// [`Input::render`], with the time it is delivered added to the header,
    /// so the model always knows the owner's local time.
    pub fn render_at(&self, now: impl std::fmt::Display) -> String {
        let rendered = self.render();
        let (header, body) = rendered.split_once('\n').unwrap_or((&rendered, ""));
        format!("{header} now: {now}\n{body}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slack(thread_ts: Option<&str>) -> Input {
        Input {
            sender: Sender::Owner,
            source: Source::Slack {
                workspace: "w1".into(),
                channel: "D1".into(),
                ts: "1700.2".into(),
                thread_ts: thread_ts.map(Into::into),
                user: "U1".into(),
                unmentioned: false,
            },
            text: "明日の予定は？".into(),
        }
    }

    #[test]
    fn a_top_level_message_names_no_thread() {
        assert_eq!(
            slack(None).render(),
            "[slack] workspace: w1 from: owner (U1) channel: D1 ts: 1700.2\n明日の予定は？"
        );
    }

    #[test]
    fn a_reply_in_a_thread_is_answered_in_that_thread() {
        assert!(slack(Some("1700.1")).render().starts_with(
            "[slack] workspace: w1 from: owner (U1) channel: D1 ts: 1700.2 thread_ts: 1700.1\n"
        ));
    }

    #[test]
    fn words_taken_only_because_the_channel_is_attended_say_so() {
        let mut input = slack(None);
        if let Source::Slack { unmentioned, .. } = &mut input.source {
            *unmentioned = true;
        }
        assert_eq!(
            input.render(),
            "[slack] workspace: w1 from: owner (U1) channel: D1 ts: 1700.2 unmentioned\n明日の予定は？"
        );
        assert_eq!(input.dedup_key(), slack(None).dedup_key());
    }

    #[test]
    fn an_approval_names_its_request() {
        let input = Input {
            sender: Sender::Owner,
            source: Source::Approval {
                id: 3,
                user: "U1".into(),
            },
            text: "approved".into(),
        };
        assert_eq!(
            input.render(),
            "[approval] from: owner (U1) request: #3\napproved"
        );
        assert_eq!(input.dedup_key(), "approval:3");
    }

    #[test]
    fn the_delivery_time_is_added_to_the_header() {
        assert_eq!(
            slack(None).render_at("2026-11-21T09:00:00+09:00"),
            "[slack] workspace: w1 from: owner (U1) channel: D1 ts: 1700.2 now: 2026-11-21T09:00:00+09:00\n明日の予定は？"
        );
    }

    #[test]
    fn the_same_slack_message_has_the_same_key() {
        assert_eq!(slack(None).dedup_key(), "slack:w1:D1:1700.2");
        assert_eq!(slack(None).dedup_key(), slack(Some("1700.1")).dedup_key());
    }
}
