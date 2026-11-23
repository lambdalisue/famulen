//! Reading what the gateway hands over from Slack's Socket Mode, untouched:
//! which messages become inputs, which the lookout hears, and which buttons
//! were pressed.

use serde_json::Value;

use crate::input::{Input, Sender, Source};

/// What one Socket Mode frame asks of Famulen.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Envelope {
    pub input: Option<Input>,
    /// A message that is not for the secretary, for the lookout to screen.
    pub heard: Option<Heard>,
    /// Buttons pressed in a message Famulen posted.
    pub actions: Vec<Action>,
    /// A plain message someone wrote in a channel, whoever it is for.
    pub said: Option<Said>,
    /// Whether the input is a mention of the bot in a channel.
    pub mention: bool,
}

/// A plain message in a channel: what the channel weighs when it is
/// attending that channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    pub channel: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub user: String,
    pub text: String,
    pub by_owner: bool,
}

/// A message the secretary overheard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heard {
    pub channel: String,
    pub user: String,
    pub ts: String,
    pub text: String,
}

/// One button press. Who pressed it is checked by whoever acts on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// The workspace the press came from.
    pub workspace: String,
    pub user: String,
    pub action_id: String,
    pub value: String,
    /// What the message's fields held then, by block and action.
    pub fields: Value,
}

/// Who an event was delivered for. Slack sends the app one event however
/// many of its installations can see it, and names only one of them, so a
/// frame for the owner may still be about a conversation the bot is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delivered {
    /// To the bot, or a frame that does not say.
    Bot,
    /// On behalf of the owner, from the user events.
    Owner,
}

fn delivered(frame: &Value) -> Delivered {
    let list = frame
        .pointer("/payload/authorizations")
        .and_then(Value::as_array);
    match list {
        Some(list) if !list.is_empty() => {
            if list
                .iter()
                .any(|a| a.get("is_bot").and_then(Value::as_bool) == Some(true))
            {
                Delivered::Bot
            } else {
                Delivered::Owner
            }
        }
        _ => Delivered::Bot,
    }
}

/// Whether reading `frame` needs the channel of the bot's DM with the
/// owner: a direct message by the owner delivered on the owner's behalf
/// may be to the bot or to anyone else.
pub fn needs_bot_dm(frame: &Value, owner: &str) -> bool {
    let event = frame.pointer("/payload/event");
    let field = |key: &str| event.and_then(|e| e.get(key)).and_then(Value::as_str);
    delivered(frame) == Delivered::Owner
        && field("type") == Some("message")
        && field("channel_type") == Some("im")
        && !owner.is_empty()
        && field("user") == Some(owner)
}

/// Read one frame. Only the owner's direct messages to the bot and mentions
/// of the bot become inputs; bots, edits and other people never do.
/// `bot_dm` is the channel of the bot's DM with the owner, when known.
pub fn read_envelope(
    frame: &Value,
    workspace: &str,
    owner: &str,
    bot_dm: Option<&str>,
) -> Envelope {
    let kind = frame
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let event = frame.pointer("/payload/event");
    let by = delivered(frame);
    let input = match kind {
        "events_api" => event.and_then(|event| message(event, workspace, owner, by, bot_dm)),
        _ => None,
    };
    let heard = match (kind, &input) {
        ("events_api", None) => event.and_then(|event| overheard(event, owner, by)),
        _ => None,
    };
    let actions = match kind {
        "interactive" => frame
            .get("payload")
            .map(|payload| block_actions(payload, workspace))
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let said = match kind {
        "events_api" => event.and_then(|event| said_in_channel(event, owner)),
        _ => None,
    };
    let mention = input.is_some()
        && event.and_then(|e| e.get("type")).and_then(Value::as_str) == Some("app_mention");
    Envelope {
        input,
        heard,
        actions,
        said,
        mention,
    }
}

fn said_in_channel(event: &Value, owner: &str) -> Option<Said> {
    let field = |key: &str| event.get(key).and_then(Value::as_str);
    let plain = field("type")? == "message"
        && field("subtype").is_none()
        && field("bot_id").is_none()
        && matches!(field("channel_type"), Some("channel" | "group"));
    if !plain {
        return None;
    }
    let user = field("user")?;
    Some(Said {
        channel: field("channel")?.to_owned(),
        ts: field("ts")?.to_owned(),
        thread_ts: field("thread_ts").map(str::to_owned),
        user: user.to_owned(),
        text: field("text").unwrap_or_default().to_owned(),
        by_owner: !owner.is_empty() && user == owner,
    })
}

fn block_actions(payload: &Value, workspace: &str) -> Vec<Action> {
    let text = |pointer: &str| {
        payload
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    if text("/type").as_deref() != Some("block_actions") {
        return Vec::new();
    }
    let Some(user) = text("/user/id") else {
        return Vec::new();
    };
    payload
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|action| {
            Some(Action {
                workspace: workspace.to_owned(),
                user: user.clone(),
                action_id: action.get("action_id")?.as_str()?.to_owned(),
                value: action
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                fields: payload
                    .pointer("/state/values")
                    .cloned()
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// What the lookout may read: a plain message in a channel the bot is in, a
/// mention of the owner in a channel only the owner is in, or a direct
/// message from someone other than the owner. Bots, edits and joins never
/// are.
fn overheard(event: &Value, owner: &str, by: Delivered) -> Option<Heard> {
    let field = |key: &str| event.get(key).and_then(Value::as_str);
    if field("type")? != "message" || field("subtype").is_some() || field("bot_id").is_some() {
        return None;
    }
    let user = field("user")?;
    let watched = match (by, field("channel_type")) {
        (Delivered::Bot, Some("channel" | "group")) => true,
        (Delivered::Owner, Some("channel" | "group")) => {
            user != owner && mentions(field("text").unwrap_or_default(), owner)
        }
        (_, Some("im" | "mpim")) => user != owner,
        _ => false,
    };
    if !watched {
        return None;
    }
    Some(Heard {
        channel: field("channel")?.to_owned(),
        user: user.to_owned(),
        ts: field("ts")?.to_owned(),
        text: field("text").unwrap_or_default().to_owned(),
    })
}

pub(crate) fn mentions(text: &str, member: &str) -> bool {
    !member.is_empty()
        && (text.contains(&format!("<@{member}>")) || text.contains(&format!("<@{member}|")))
}

fn message(
    event: &Value,
    workspace: &str,
    owner: &str,
    by: Delivered,
    bot_dm: Option<&str>,
) -> Option<Input> {
    let field = |key: &str| event.get(key).and_then(Value::as_str);
    let addressed = match field("type")? {
        "message" => {
            field("channel_type") == Some("im")
                && (by == Delivered::Bot || (bot_dm.is_some() && field("channel") == bot_dm))
        }
        "app_mention" => true,
        _ => false,
    };
    let is_plain = field("subtype").is_none() && field("bot_id").is_none();
    let user = field("user")?;
    if !addressed || !is_plain || owner.is_empty() || user != owner {
        return None;
    }
    Some(Input {
        sender: Sender::Owner,
        source: Source::Slack {
            workspace: workspace.to_owned(),
            channel: field("channel")?.to_owned(),
            ts: field("ts")?.to_owned(),
            thread_ts: field("thread_ts").map(str::to_owned),
            user: user.to_owned(),
            unmentioned: false,
        },
        text: field("text").unwrap_or_default().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn frame(event: Value) -> Value {
        json!({ "envelope_id": "e1", "type": "events_api", "payload": { "event": event } })
    }

    #[test]
    fn a_channel_message_is_overheard_but_a_bot_post_is_not() {
        let heard = read_envelope(
            &frame(
                json!({ "type": "message", "channel_type": "channel", "channel": "C1", "user": "U2", "ts": "1.1", "text": "障害が出ています" }),
            ),
            "w1",
            "U1",
            None,
        );
        assert!(heard.input.is_none());
        assert_eq!(heard.heard.unwrap().text, "障害が出ています");
        let bot = read_envelope(
            &frame(
                json!({ "type": "message", "channel_type": "channel", "channel": "C1", "bot_id": "B1", "ts": "1.2", "text": "x" }),
            ),
            "w1",
            "U1",
            None,
        );
        assert!(bot.heard.is_none());
        let dm = read_envelope(
            &frame(
                json!({ "type": "message", "channel_type": "im", "channel": "D1", "user": "U1", "ts": "1.3", "text": "x" }),
            ),
            "w1",
            "U1",
            None,
        );
        assert!(dm.input.is_some() && dm.heard.is_none());
    }

    #[test]
    fn a_plain_channel_message_is_said_there_by_its_author() {
        let said = |event: Value| read_envelope(&frame(event), "w1", "U1", None).said;
        let owner = said(
            json!({ "type": "message", "channel_type": "channel", "channel": "C1", "user": "U1", "ts": "3.1", "thread_ts": "3.0", "text": "それで" }),
        )
        .unwrap();
        assert_eq!(
            owner,
            Said {
                channel: "C1".into(),
                ts: "3.1".into(),
                thread_ts: Some("3.0".into()),
                user: "U1".into(),
                text: "それで".into(),
                by_owner: true,
            }
        );
        let other = said(
            json!({ "type": "message", "channel_type": "group", "channel": "G1", "user": "U2", "ts": "3.2", "text": "どうぞ" }),
        )
        .unwrap();
        assert!(!other.by_owner);
        assert!(
            said(json!({ "type": "message", "channel_type": "channel", "channel": "C1", "bot_id": "B1", "ts": "3.3", "text": "x" })).is_none()
        );
        assert!(
            said(json!({ "type": "message", "subtype": "message_changed", "channel_type": "channel", "channel": "C1", "ts": "3.4" })).is_none()
        );
        assert!(said(dm("U1")).is_none(), "a DM is not said in a channel");
    }

    #[test]
    fn a_mention_says_so() {
        let mention = read_envelope(
            &frame(
                json!({ "type": "app_mention", "channel": "C1", "user": "U1", "ts": "2.2", "text": "<@B1> hi" }),
            ),
            "w1",
            "U1",
            None,
        );
        assert!(mention.mention);
        assert!(!read_envelope(&frame(dm("U1")), "w1", "U1", None).mention);
    }

    fn dm(user: &str) -> Value {
        json!({ "type": "message", "channel_type": "im", "channel": "D1", "user": user, "ts": "1.1", "text": "hi" })
    }

    #[test]
    fn a_direct_message_from_the_owner_becomes_an_input() {
        let input = read_envelope(&frame(dm("U1")), "w1", "U1", None)
            .input
            .unwrap();
        assert_eq!(input.sender, Sender::Owner);
        assert_eq!(input.text, "hi");
        assert_eq!(
            input.source,
            Source::Slack {
                workspace: "w1".into(),
                channel: "D1".into(),
                ts: "1.1".into(),
                thread_ts: None,
                user: "U1".into(),
                unmentioned: false,
            }
        );
    }

    #[test]
    fn a_reply_in_a_thread_keeps_the_thread() {
        let mut event = dm("U1");
        event["thread_ts"] = json!("1.0");
        let input = read_envelope(&frame(event), "w1", "U1", None)
            .input
            .unwrap();
        assert!(matches!(input.source, Source::Slack { thread_ts: Some(t), .. } if t == "1.0"));
    }

    #[test]
    fn a_mention_of_the_bot_in_a_channel_becomes_an_input() {
        let event = json!({ "type": "app_mention", "channel": "C1", "user": "U1", "ts": "2.2", "text": "<@B1> hi" });
        assert!(
            read_envelope(&frame(event), "w1", "U1", None)
                .input
                .is_some()
        );
    }

    #[test]
    fn a_plain_channel_message_is_not_an_input() {
        let event = json!({ "type": "message", "channel_type": "channel", "channel": "C1", "user": "U1", "ts": "3.3", "text": "hi" });
        assert_eq!(read_envelope(&frame(event), "w1", "U1", None).input, None);
    }

    #[test]
    fn someone_else_writing_directly_is_heard_but_not_answered() {
        let envelope = read_envelope(&frame(dm("U2")), "w1", "U1", None);
        assert_eq!(envelope.input, None);
        assert_eq!(envelope.heard.unwrap().user, "U2");
    }

    #[test]
    fn without_a_known_owner_nothing_becomes_an_input() {
        assert_eq!(read_envelope(&frame(dm("U1")), "w1", "", None).input, None);
    }

    #[test]
    fn a_bot_message_is_not_an_input() {
        let mut event = dm("U1");
        event["bot_id"] = json!("B1");
        assert_eq!(read_envelope(&frame(event), "w1", "U1", None).input, None);
    }

    #[test]
    fn an_edit_is_not_an_input() {
        let mut event = dm("U1");
        event["subtype"] = json!("message_changed");
        assert_eq!(read_envelope(&frame(event), "w1", "U1", None).input, None);
    }

    /// A frame Slack delivered on behalf of the owner (a user event), not
    /// the bot.
    fn for_the_owner(event: Value) -> Value {
        json!({
            "envelope_id": "e1",
            "type": "events_api",
            "payload": {
                "event": event,
                "authorizations": [{ "team_id": "T1", "user_id": "U1", "is_bot": false }],
            },
        })
    }

    #[test]
    fn a_frame_for_the_bot_reads_as_before() {
        let frame = json!({
            "type": "events_api",
            "payload": {
                "event": dm("U1"),
                "authorizations": [{ "team_id": "T1", "user_id": "B9", "is_bot": true }],
            },
        });
        assert!(read_envelope(&frame, "w1", "U1", None).input.is_some());
    }

    #[test]
    fn a_mention_of_the_owner_where_the_bot_is_not_is_heard() {
        let event = json!({ "type": "message", "channel_type": "channel", "channel": "C9", "user": "U2", "ts": "4.1", "text": "<@U1> レビューお願いします" });
        let envelope = read_envelope(&for_the_owner(event), "w1", "U1", None);
        assert_eq!(envelope.input, None);
        let heard = envelope.heard.unwrap();
        assert_eq!((heard.channel.as_str(), heard.user.as_str()), ("C9", "U2"));
        let named = json!({ "type": "message", "channel_type": "group", "channel": "G9", "user": "U2", "ts": "4.2", "text": "<@U1|alice> 見て" });
        assert!(
            read_envelope(&for_the_owner(named), "w1", "U1", None)
                .heard
                .is_some()
        );
    }

    #[test]
    fn other_talk_in_the_owners_channels_is_not_heard() {
        let event = json!({ "type": "message", "channel_type": "channel", "channel": "C9", "user": "U2", "ts": "4.3", "text": "<@U3> ランチ行こう" });
        let envelope = read_envelope(&for_the_owner(event), "w1", "U1", None);
        assert!(envelope.input.is_none() && envelope.heard.is_none());
    }

    #[test]
    fn someone_writing_to_the_owner_directly_is_heard() {
        let im = read_envelope(&for_the_owner(dm("U2")), "w1", "U1", None);
        assert_eq!(im.input, None);
        assert_eq!(im.heard.unwrap().user, "U2");
        let mpim = json!({ "type": "message", "channel_type": "mpim", "channel": "M1", "user": "U3", "ts": "4.4", "text": "相談です" });
        assert!(
            read_envelope(&for_the_owner(mpim), "w1", "U1", None)
                .heard
                .is_some()
        );
    }

    #[test]
    fn what_the_owner_writes_to_others_is_neither_an_input_nor_heard() {
        let to_a_colleague = json!({ "type": "message", "channel_type": "im", "channel": "D7", "user": "U1", "ts": "4.5", "text": "了解" });
        assert_eq!(
            read_envelope(&for_the_owner(to_a_colleague), "w1", "U1", Some("D1")),
            Envelope::default()
        );
        let in_a_channel = json!({ "type": "message", "channel_type": "channel", "channel": "C9", "user": "U1", "ts": "4.6", "text": "<@U1> メモ" });
        let envelope = read_envelope(&for_the_owner(in_a_channel), "w1", "U1", None);
        assert!(envelope.input.is_none() && envelope.heard.is_none());
    }

    #[test]
    fn the_owners_message_to_the_bot_delivered_for_the_owner_is_still_an_input() {
        let envelope = read_envelope(&for_the_owner(dm("U1")), "w1", "U1", Some("D1"));
        assert!(envelope.input.is_some());
        assert!(envelope.heard.is_none());
    }

    #[test]
    fn the_bots_dm_is_needed_only_to_tell_the_owners_own_direct_messages_apart() {
        assert!(needs_bot_dm(&for_the_owner(dm("U1")), "U1"));
        assert!(!needs_bot_dm(&for_the_owner(dm("U2")), "U1"));
        assert!(!needs_bot_dm(&frame(dm("U1")), "U1"));
    }

    #[test]
    fn hello_asks_nothing() {
        assert_eq!(
            read_envelope(&json!({ "type": "hello" }), "w1", "U1", None),
            Envelope::default()
        );
    }

    #[test]
    fn a_button_press_is_read_with_who_pressed_it_and_where() {
        let frame = json!({
            "envelope_id": "e2",
            "type": "interactive",
            "payload": {
                "type": "block_actions",
                "user": { "id": "U1" },
                "container": { "channel_id": "D1", "message_ts": "5.5" },
                "actions": [{ "action_id": "famulen.approval.approve", "value": "12" }],
            },
        });
        let envelope = read_envelope(&frame, "w1", "U1", None);
        assert_eq!(envelope.input, None);
        assert_eq!(
            envelope.actions,
            vec![Action {
                workspace: "w1".into(),
                user: "U1".into(),
                action_id: "famulen.approval.approve".into(),
                value: "12".into(),
                fields: Value::Null,
            }]
        );
    }

    #[test]
    fn a_press_by_someone_else_is_still_read_so_the_handler_can_ignore_it() {
        let frame = json!({
            "envelope_id": "e3",
            "type": "interactive",
            "payload": { "type": "block_actions", "user": { "id": "U2" }, "actions": [{ "action_id": "x", "value": "1" }] },
        });
        assert_eq!(
            read_envelope(&frame, "w1", "U1", None).actions[0].user,
            "U2"
        );
    }

    #[test]
    fn other_interactive_payloads_have_no_actions() {
        let frame = json!({ "envelope_id": "e4", "type": "interactive", "payload": { "type": "view_submission" } });
        assert!(read_envelope(&frame, "w1", "U1", None).actions.is_empty());
    }
}
