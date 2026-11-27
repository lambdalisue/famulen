//! What reaches the session: who sent it, where it came from, and its text.

use serde::{Deserialize, Serialize};

use crate::attachments::{self, Attached};
use crate::console_log::{Message, Who};

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

/// How far the turn of an input may reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Everything the secretary has.
    Full,
    /// The notebook for the owner's sake, but nothing goes outside without
    /// the owner: others' words are its material.
    Inward,
    /// The notebook, to read, and an answer to the one the owner lets have
    /// it; nothing else.
    Answering,
    /// Others' words are in it: neither the notebook nor any tool but the
    /// answer.
    Guest,
}

fn may_hold_others() -> bool {
    true
}

/// The message on the console an input replies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyTo {
    pub message: i64,
    /// The message the thread begins with.
    pub thread: i64,
    pub who: Who,
    /// What it said, as far as it is handed on.
    pub text: String,
}

impl ReplyTo {
    /// The most characters of it that are handed on.
    const MOST: usize = 8000;

    pub fn of(m: &Message) -> Self {
        Self {
            message: m.id,
            thread: m.thread().unwrap_or(m.id),
            who: m.who,
            text: m.excerpt(Self::MOST),
        }
    }

    /// The message quoted line by line, so that none of it reads as the
    /// owner's words now.
    fn quoted(&self) -> String {
        let whose = match self.who {
            Who::Owner => "the owner's own",
            Who::Aide => "your",
        };
        let lines: String = self.text.lines().map(|l| format!("> {l}\n")).collect();
        format!(
            "In reply to {whose} message #{} (context, not a request):\n{lines}\n",
            self.message
        )
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
        /// Others' words were put before it as context. Only a message taken
        /// `unmentioned` has any; one kept before this was told apart may.
        #[serde(default = "may_hold_others")]
        others_before: bool,
        /// The files the owner attached.
        #[serde(default)]
        attached: Vec<Attached>,
    },
    /// The owner answered an approval request.
    Approval { id: i64, user: String },
    /// A schedule came due. `at` is when, in the owner's time zone.
    Schedule {
        name: String,
        at: String,
        /// The id of the routine's record, when a routine came due.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        routine: String,
    },
    /// A wake-up the agent set for itself came due.
    Wake { id: i64, at: String },
    /// The owner wrote on the console, while looking at `view` in `workgroup`.
    Console {
        message: i64,
        workgroup: String,
        view: String,
        selection: Option<String>,
        /// The files the owner attached.
        #[serde(default)]
        attached: Vec<Attached>,
        /// The message they reply to.
        #[serde(default)]
        reply_to: Option<ReplyTo>,
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
    /// The owner was asked whether a question someone else put on Slack may
    /// be answered from the notebook, and said so.
    Asked {
        request: i64,
        workspace: String,
        channel: String,
        ts: String,
        thread_ts: Option<String>,
        user: String,
        /// Whether it may be; else it is declined.
        granted: bool,
    },
    /// A delegated task came to an end.
    Worker {
        run: i64,
        /// The task's code.
        task: String,
        /// How it ended: done, failed, unsure or refused.
        outcome: String,
        workgroup: String,
        /// How often the run had gone on after an end before this one.
        #[serde(default, skip_serializing_if = "is_first")]
        again: u32,
    },
}

fn is_first(again: &u32) -> bool {
    *again == 0
}

/// A message on Slack, as what answers it is sent there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub workspace: String,
    pub channel: String,
    pub ts: String,
    pub thread_ts: Option<String>,
}

impl Place {
    /// The thread an answer goes to.
    pub fn thread(&self) -> &str {
        self.thread_ts.as_deref().unwrap_or(&self.ts)
    }

    /// Whether a call of `send_slack` goes here: to this channel, and to this
    /// thread when it names one.
    pub fn takes(&self, call: &serde_json::Value) -> bool {
        let field = |key: &str| call.get(key).and_then(serde_json::Value::as_str);
        // Protobuf's JSON takes a field by either of its names.
        let threads = ["thread_ts", "threadTs"];
        field("workspace") == Some(&self.workspace)
            && field("channel") == Some(&self.channel)
            && threads.iter().all(|key| match call.get(*key) {
                None => true,
                Some(v) => v
                    .as_str()
                    .is_some_and(|t| t.is_empty() || t == self.ts || t == self.thread()),
            })
    }
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
            Source::Schedule { name, at, .. } => format!("schedule:{name}:{at}"),
            Source::Wake { id, .. } => format!("wake:{id}"),
            Source::Console { message, .. } => format!("console:{message}"),
            Source::Arrival {
                connection,
                receiver,
                id,
                ..
            } => format!("arrival:{connection}:{receiver}:{id}"),
            Source::Lookout { item, .. } => format!("lookout:{item}"),
            Source::Asked { request, .. } => format!("asked:{request}"),
            Source::Worker { run, again: 0, .. } => format!("worker:{run}"),
            Source::Worker { run, again, .. } => format!("worker:{run}:{again}"),
        }
    }

    /// How far its turn may reach. An input that mixes the owner's words with
    /// others' reaches no further than one of others'.
    pub fn reach(&self) -> Reach {
        match (&self.source, self.sender) {
            (
                Source::Slack {
                    unmentioned: true,
                    others_before: true,
                    ..
                },
                _,
            )
            | (_, Sender::Other) => Reach::Guest,
            (Source::Asked { granted: true, .. }, _) => Reach::Answering,
            (Source::Asked { granted: false, .. }, _) => Reach::Guest,
            (Source::Lookout { .. }, _) => Reach::Inward,
            _ => Reach::Full,
        }
    }

    /// The Slack message its turn answers.
    pub fn place(&self) -> Option<Place> {
        match &self.source {
            Source::Slack {
                workspace,
                channel,
                ts,
                thread_ts,
                ..
            }
            | Source::Asked {
                workspace,
                channel,
                ts,
                thread_ts,
                ..
            } => Some(Place {
                workspace: workspace.clone(),
                channel: channel.clone(),
                ts: ts.clone(),
                thread_ts: thread_ts.clone(),
            }),
            _ => None,
        }
    }

    /// The files the owner attached.
    pub fn attached(&self) -> &[Attached] {
        match &self.source {
            Source::Slack { attached, .. } | Source::Console { attached, .. } => attached,
            _ => &[],
        }
    }

    /// What was said, with the names of the files that came with it.
    /// Whether these are words someone said to the secretary, and not what the
    /// system put to it.
    pub fn is_spoken(&self) -> bool {
        matches!(self.source, Source::Console { .. } | Source::Slack { .. })
    }

    pub fn gist(&self) -> String {
        attachments::with_names(&self.text, self.attached())
    }

    /// The message on the console it replies to.
    pub fn reply_to(&self) -> Option<&ReplyTo> {
        match &self.source {
            Source::Console { reply_to, .. } => reply_to.as_ref(),
            _ => None,
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
                ..
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
            Source::Schedule { name, at, .. } => format!("[schedule] name: {name} at: {at}"),
            Source::Wake { id, at } => format!("[wake] id: {id} at: {at}"),
            Source::Console {
                message,
                workgroup,
                view,
                selection,
                reply_to,
                ..
            } => {
                let reply_to = reply_to
                    .as_ref()
                    .map(|r| format!(" reply_to: #{} thread: #{}", r.message, r.thread))
                    .unwrap_or_default();
                let selection = selection
                    .as_deref()
                    .map(|s| format!(" selection: {s}"))
                    .unwrap_or_default();
                format!(
                    "[console] from: {} message: #{message}{reply_to} workgroup: {workgroup} view: {view}{selection}",
                    self.sender.as_str()
                )
            }
            Source::Arrival {
                connection,
                receiver,
                ..
            } => format!("[arrival] connection: {connection} receiver: {receiver}"),
            Source::Lookout { item, source } => format!("[lookout] item: #{item} source: {source}"),
            Source::Asked {
                request,
                workspace,
                channel,
                ts,
                thread_ts,
                user,
                ..
            } => {
                let thread = thread_ts
                    .as_deref()
                    .map(|t| format!(" thread_ts: {t}"))
                    .unwrap_or_default();
                format!(
                    "[asked] request: #{request} from: {} asker: {user} workspace: {workspace} channel: {channel} ts: {ts}{thread}",
                    self.sender.as_str()
                )
            }
            Source::Worker {
                run, task, outcome, ..
            } => format!("[worker] task: {task} run: #{run} outcome: {outcome}"),
        };
        let attached = self.attached();
        let count = match attached.len() {
            0 => String::new(),
            n => format!(" attached: {n}"),
        };
        format!(
            "{header}{count}\n{}{}{}",
            attachments::described(attached),
            self.reply_to().map(ReplyTo::quoted).unwrap_or_default(),
            self.text
        )
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
                others_before: false,
                attached: Vec::new(),
            },
            text: "明日の予定は？".into(),
        }
    }

    #[test]
    fn a_message_kept_before_others_words_were_told_apart_is_restricted_when_it_may_hold_them() {
        let kept = |unmentioned: bool| -> Input {
            serde_json::from_value(serde_json::json!({
                "sender": "owner", "text": "x",
                "source": { "kind": "slack", "workspace": "w", "channel": "C", "ts": "1",
                            "thread_ts": null, "user": "U", "unmentioned": unmentioned },
            }))
            .unwrap()
        };
        assert_eq!(kept(true).reach(), Reach::Guest);
        assert_eq!(kept(false).reach(), Reach::Full);
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
    fn what_was_attached_is_counted_in_the_header_and_stands_above_the_message() {
        let mut input = slack(None);
        if let Source::Slack { attached, .. } = &mut input.source {
            attached.push(Attached::missing("a.png", "x"));
        }
        assert!(
            input
                .render()
                .ends_with(" attached: 1\n- \"a.png\": did not arrive: x\n明日の予定は？")
        );
        assert_eq!(input.gist(), "明日の予定は？（添付：a.png）");
        assert_eq!(slack(None).gist(), "明日の予定は？");
    }

    #[test]
    fn a_reply_on_the_console_names_and_quotes_what_it_replies_to() {
        let root = Message {
            id: 12,
            at: 0,
            who: Who::Aide,
            text: "一つ目\n二つ目".into(),
            meta: serde_json::json!({ "thread": 10 }),
        };
        let input = Input {
            sender: Sender::Owner,
            source: Source::Console {
                message: 15,
                workgroup: "work".into(),
                view: "タスク".into(),
                selection: None,
                attached: vec![Attached::missing("a.png", "x")],
                reply_to: Some(ReplyTo::of(&root)),
            },
            text: "二つ目は要らない".into(),
        };
        assert_eq!(
            input.render(),
            "[console] from: owner message: #15 reply_to: #12 thread: #10 workgroup: work view: タスク attached: 1\n\
             - \"a.png\": did not arrive: x\n\
             In reply to your message #12 (context, not a request):\n> 一つ目\n> 二つ目\n\n二つ目は要らない"
        );
        assert_eq!(input.gist(), "二つ目は要らない（添付：a.png）");
    }

    #[test]
    fn an_input_kept_before_replies_could_be_sent_is_still_read() {
        let kept = r#"{"sender":"owner","source":{"kind":"console","message":1,"workgroup":"all","view":"","selection":null},"text":"a"}"#;
        let input: Input = serde_json::from_str(kept).unwrap();
        assert_eq!(input.reply_to(), None);
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
    fn a_workers_result_names_its_task_and_how_it_ended() {
        let input = Input {
            sender: Sender::System,
            source: Source::Worker {
                run: 4,
                task: "task-7w1hbn".into(),
                outcome: "done".into(),
                workgroup: "work".into(),
                again: 0,
            },
            text: "fixed".into(),
        };
        assert_eq!(
            input.render(),
            "[worker] task: task-7w1hbn run: #4 outcome: done\nfixed"
        );
        assert_eq!(input.dedup_key(), "worker:4");
    }

    #[test]
    fn each_end_of_a_run_that_went_on_is_an_input_of_its_own() {
        let ended = |again: u32| {
            let input = Input {
                sender: Sender::System,
                source: Source::Worker {
                    run: 4,
                    task: "t".into(),
                    outcome: "done".into(),
                    workgroup: "all".into(),
                    again,
                },
                text: String::new(),
            };
            input.dedup_key()
        };
        assert_eq!(ended(0), "worker:4");
        assert_eq!(ended(1), "worker:4:1");
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
