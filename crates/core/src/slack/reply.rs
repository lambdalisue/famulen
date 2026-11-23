//! What the secretary says on Slack: in the thread of the message it
//! answers, in Slack's own format.

use crate::input::Source;
use crate::slack::mrkdwn::to_mrkdwn;
use crate::slack::presence;
use crate::slack::web::{Outgoing, Slack};
use crate::store::Store;

/// What was posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    pub ts: String,
    /// As Slack was given it.
    pub text: String,
}

/// The Slack message the running turn answers, when it is in `channel`.
struct Answered {
    input: i64,
    thread: String,
    /// Whether the message was shown as being worked on.
    shown: bool,
}

fn answered(store: &Store, workspace: &str, channel: &str) -> anyhow::Result<Option<Answered>> {
    let Some(row) = store.running_input()? else {
        return Ok(None);
    };
    let Source::Slack {
        workspace: from,
        channel: at,
        ts,
        thread_ts,
        unmentioned,
        ..
    } = row.input.source
    else {
        return Ok(None);
    };
    Ok((from == workspace && at == channel).then(|| Answered {
        input: row.id,
        thread: thread_ts.unwrap_or(ts),
        shown: !unmentioned,
    }))
}

/// Post `text`, Markdown, to `channel` of `workspace`: in `thread` when one
/// is named, else in the thread of the message being answered there. What showed that it is being worked on goes away:
/// Slack takes it away with a reply in its thread, and this elsewhere.
pub async fn send<S: Slack>(
    slack: &S,
    store: &Store,
    workspace: &str,
    channel: &str,
    text: &str,
    thread: Option<String>,
) -> anyhow::Result<Posted> {
    let answered = answered(store, workspace, channel)?;
    let thread = thread
        .filter(|t| !t.is_empty())
        .or_else(|| answered.as_ref().map(|a| a.thread.clone()));
    let text = to_mrkdwn(text);
    let ts = slack
        .post(
            channel,
            Outgoing {
                text: text.clone(),
                blocks: None,
                thread_ts: thread.clone(),
            },
        )
        .await?;
    if let Some(answered) = answered {
        // The message is out; failing here would have it said twice.
        if let Err(error) = presence::answered(store, answered.input) {
            tracing::warn!(%error, "could not note that the message was answered");
        }
        if answered.shown
            && thread.as_ref() != Some(&answered.thread)
            && let Err(error) = slack.set_status(channel, &answered.thread, "").await
        {
            tracing::debug!(%error, "could not take the status away");
        }
    }
    Ok(Posted { ts, text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Input, Sender};
    use crate::slack::fake::FakeSlack;

    /// A store whose running turn answers a message on Slack.
    fn answering(thread_ts: Option<&str>, unmentioned: bool) -> Store {
        let store = Store::open_in_memory().unwrap();
        let input = Input {
            sender: Sender::Owner,
            source: Source::Slack {
                workspace: "w1".into(),
                channel: "D1".into(),
                ts: "2.0".into(),
                thread_ts: thread_ts.map(Into::into),
                user: "U1".into(),
                unmentioned,
            },
            text: "hi".into(),
        };
        let id = store.enqueue(&input).unwrap().unwrap();
        store.start_turn(id, "all").unwrap();
        store
    }

    async fn sent(store: &Store, to: (&str, &str), thread: Option<&str>) -> FakeSlack {
        let slack = FakeSlack::default();
        send(
            &&slack,
            store,
            to.0,
            to.1,
            "これは**大事**です",
            thread.map(Into::into),
        )
        .await
        .unwrap();
        slack
    }

    #[tokio::test]
    async fn a_reply_goes_to_the_thread_of_the_message_unless_another_is_named() {
        let here = ("w1", "D1");
        for (message_in, to, named, expected) in [
            (None, here, None, Some("2.0")),
            (None, here, Some(""), Some("2.0")),
            (Some("1.0"), here, None, Some("1.0")),
            (Some("1.0"), here, Some("5.0"), Some("5.0")),
            (Some("1.0"), ("w1", "C9"), None, None),
            (Some("1.0"), ("w2", "D1"), None, None),
        ] {
            let slack = sent(&answering(message_in, false), to, named).await;
            assert_eq!(
                slack.posts()[0].1.thread_ts.as_deref(),
                expected,
                "{message_in:?} {to:?} {named:?}"
            );
        }
    }

    #[tokio::test]
    async fn what_answers_no_message_starts_no_thread() {
        let store = Store::open_in_memory().unwrap();
        let slack = sent(&store, ("w1", "D1"), None).await;
        assert_eq!(slack.posts()[0].1.thread_ts, None);
    }

    #[tokio::test]
    async fn a_reply_is_put_into_slacks_format() {
        let slack = sent(&answering(None, false), ("w1", "D1"), None).await;
        assert_eq!(slack.posts()[0].1.text, "これは *大事* です");
    }

    #[tokio::test]
    async fn the_status_is_taken_away_when_the_reply_is_outside_its_thread() {
        let here = ("w1", "D1");
        let inside = sent(&answering(None, false), here, None).await;
        assert!(inside.statuses().is_empty(), "Slack takes it away");

        let outside = sent(&answering(None, false), here, Some("1.0")).await;
        assert_eq!(
            outside.statuses(),
            [("D1".into(), "2.0".into(), String::new())]
        );

        let never_shown = sent(&answering(None, true), here, Some("1.0")).await;
        assert!(never_shown.statuses().is_empty());
    }

    #[tokio::test]
    async fn a_reply_that_is_out_does_not_fail_over_its_status() {
        let slack = FakeSlack {
            no_status: true,
            ..Default::default()
        };
        let store = answering(None, false);
        let sent = send(&&slack, &store, "w1", "D1", "a", Some("1.0".into())).await;
        assert!(sent.is_ok());
    }
}
