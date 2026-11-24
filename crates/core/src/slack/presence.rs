//! Showing on Slack that what the owner said arrived and is being worked on.

use std::time::{Duration, Instant};

use crate::input::{Sender, Source};
use crate::slack::web::Slack;
use crate::store::{Status, Store, now};

pub const WORKING: &str = "考えています…";
/// Slack takes the status away two minutes after it was set.
const SET_AGAIN: Duration = Duration::from_secs(90);
/// What arrives next waits no longer than this for Slack.
const GIVE_UP_EXPECTING: Duration = Duration::from_secs(5);
const LOOK_AGAIN: Duration = Duration::from_secs(1);
/// The input answered last. One turn runs at a time, so one is enough.
const ANSWERED_KEY: &str = "slack.answered";
/// A turn held this long shows nothing more.
const GIVE_UP: Duration = Duration::from_secs(60 * 60);
/// A message this long after the last one starts a conversation.
const CONVERSATION_SECS: i64 = 60 * 60;

/// The message to show it at.
#[derive(Debug, Clone)]
pub struct At {
    pub workspace: String,
    pub channel: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    /// It was said without a mention, in an attended channel.
    pub unmentioned: bool,
    /// When Famulen received it.
    pub received: Instant,
}

/// Show that input `id` is being worked on until it is answered or its turn
/// ends. Where Slack
/// shows no status, the first message of a conversation gets a reaction,
/// unless it was said without a mention.
pub async fn show_working<S: Slack>(
    slack: &S,
    store: &Store,
    at: &At,
    id: i64,
) -> anyhow::Result<()> {
    let first = starts_conversation(store, at)?;
    let thread = at.thread_ts.as_deref().unwrap_or(&at.ts);
    if let Err(error) = slack.set_status(&at.channel, thread, WORKING).await {
        tracing::debug!(%error, "Slack shows no status here");
        if first && !at.unmentioned {
            slack.react(&at.channel, &at.ts, "eyes").await?;
        }
        return Ok(());
    }
    tracing::info!(
        workspace = %at.workspace,
        since_received_ms = at.received.elapsed().as_millis() as u64,
        since_said_ms = since_said_ms(&at.ts, chrono::Utc::now()),
        "showed on Slack that a message is being worked on"
    );
    let started = Instant::now();
    let mut set = started;
    while started.elapsed() < GIVE_UP && in_hand(store, id)? && !is_answered(store, id)? {
        tokio::time::sleep(LOOK_AGAIN).await;
        if set.elapsed() >= SET_AGAIN && !is_answered(store, id)? {
            set = Instant::now();
            slack.set_status(&at.channel, thread, WORKING).await?;
        }
    }
    // A reply takes it away by itself, but one that came while the status was
    // on its way to Slack took away nothing.
    slack.set_status(&at.channel, thread, "").await
}

/// Show a message that may not be for the secretary as worked on while it is
/// weighed. Returns whether it has to be withdrawn when it is not. A thread
/// that shows an input already is left alone: withdrawing would take that
/// away.
pub async fn expect<S: Slack>(
    slack: &S,
    store: &Store,
    workspace: &str,
    channel: &str,
    thread: &str,
) -> anyhow::Result<bool> {
    if shows_an_input(store, workspace, channel, thread)? {
        return Ok(false);
    }
    match tokio::time::timeout(
        GIVE_UP_EXPECTING,
        slack.set_status(channel, thread, WORKING),
    )
    .await
    {
        Ok(Ok(())) => Ok(true),
        Ok(Err(error)) => Err(error),
        // It may have reached Slack all the same.
        Err(_) => Ok(true),
    }
}

/// The message `expect` showed was not for the secretary.
pub async fn withdraw<S: Slack>(slack: &S, channel: &str, thread: &str) {
    // Awaited, not spawned: a status taken away late would be that of the
    // next message in the thread.
    match tokio::time::timeout(GIVE_UP_EXPECTING, slack.set_status(channel, thread, "")).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::debug!(%error, "could not take the status away"),
        Err(_) => tracing::debug!("taking the status away took too long"),
    }
}

fn shows_an_input(
    store: &Store,
    workspace: &str,
    channel: &str,
    thread: &str,
) -> anyhow::Result<bool> {
    for row in store.in_hand()? {
        let Source::Slack {
            workspace: from,
            channel: at,
            ts,
            thread_ts,
            ..
        } = &row.input.source
        else {
            continue;
        };
        if row.input.sender == Sender::Owner
            && from == workspace
            && at == channel
            && thread_ts.as_ref().unwrap_or(ts) == thread
            && !is_answered(store, row.id)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The secretary replied to input `id`: it is shown no longer.
pub fn answered(store: &Store, id: i64) -> anyhow::Result<()> {
    store.set_state(ANSWERED_KEY, id.to_string())
}

fn is_answered(store: &Store, id: i64) -> anyhow::Result<bool> {
    Ok(store.state(ANSWERED_KEY)?.as_deref() == Some(id.to_string().as_str()))
}

fn in_hand(store: &Store, id: i64) -> anyhow::Result<bool> {
    Ok(store
        .get(id)?
        .is_some_and(|row| matches!(row.status, Status::Pending | Status::Running)))
}

/// How long ago, by Slack's clock, the message `ts` was said.
fn since_said_ms(ts: &str, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
    let said = (ts.parse::<f64>().ok()? * 1000.0).round() as i64;
    Some((now.timestamp_millis() - said).max(0))
}

/// Whether nothing was said there lately. Marks that something was now.
fn starts_conversation(store: &Store, at: &At) -> anyhow::Result<bool> {
    let key = format!(
        "channel.spoken.{}.{}.{}",
        at.workspace,
        at.channel,
        at.thread_ts.as_deref().unwrap_or_default()
    );
    let before = store.state(&key)?.and_then(|at| at.parse::<i64>().ok());
    store.set_state(&key, now().to_string())?;
    Ok(before.is_none_or(|before| now() - before > CONVERSATION_SECS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Input;
    use crate::slack::fake::FakeSlack;

    fn taken(store: &Store, ts: &str) -> (At, i64) {
        let input = Input {
            sender: Sender::Owner,
            source: Source::Slack {
                workspace: "w".into(),
                channel: "D1".into(),
                ts: ts.into(),
                thread_ts: None,
                user: "U1".into(),
                unmentioned: false,
            },
            text: "hi".into(),
        };
        let id = store.enqueue(&input).unwrap().unwrap();
        let at = At {
            workspace: "w".into(),
            channel: "D1".into(),
            ts: ts.into(),
            thread_ts: None,
            unmentioned: false,
            received: Instant::now(),
        };
        (at, id)
    }

    #[tokio::test]
    async fn the_status_is_shown_until_the_turn_ends() {
        let store = Store::open_in_memory().unwrap();
        let (at, id) = taken(&store, "1.0");
        let slack = FakeSlack::default();

        let shown = async {
            show_working(&&slack, &store, &at, id).await.unwrap();
        };
        let turn = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            store.start_turn(id, "all").unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert_eq!(
                slack.statuses(),
                [("D1".into(), "1.0".into(), WORKING.to_owned())]
            );
            store.end_turn().unwrap();
        };
        tokio::join!(shown, turn);

        let said: Vec<String> = slack.statuses().into_iter().map(|s| s.2).collect();
        assert_eq!(said, [WORKING, ""]);
        assert!(slack.reactions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_status_is_not_shown_again_once_the_message_is_answered() {
        let store = Store::open_in_memory().unwrap();
        let (at, id) = taken(&store, "1.0");
        store.start_turn(id, "all").unwrap();
        let slack = FakeSlack::default();

        let client = &slack;
        let shown = show_working(&client, &store, &at, id);
        let reply = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            answered(&store, id).unwrap();
        };
        let (shown, ()) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(shown, reply)
        })
        .await
        .expect("the turn goes on, but nothing is shown of it");
        shown.unwrap();

        let said: Vec<String> = slack.statuses().into_iter().map(|s| s.2).collect();
        assert_eq!(
            said,
            [WORKING, ""],
            "a status still on its way when the reply came is taken away"
        );
    }

    #[tokio::test]
    async fn a_reply_that_overtakes_the_status_still_leaves_none() {
        let store = Store::open_in_memory().unwrap();
        let (at, id) = taken(&store, "1.0");
        store.start_turn(id, "all").unwrap();
        answered(&store, id).unwrap();
        let slack = FakeSlack::default();

        show_working(&&slack, &store, &at, id).await.unwrap();

        let said: Vec<String> = slack.statuses().into_iter().map(|s| s.2).collect();
        assert_eq!(said, [WORKING, ""]);
    }

    #[test]
    fn how_long_ago_a_message_was_said_is_read_from_its_ts() {
        let now = chrono::DateTime::from_timestamp_millis(1_700_000_002_500).unwrap();
        for (ts, since) in [
            ("1700000000.000000", Some(2500)),
            ("1700000002.250000", Some(250)),
            ("1700000009.000000", Some(0)),
            ("x", None),
        ] {
            assert_eq!(since_said_ms(ts, now), since, "{ts}");
        }
    }

    #[tokio::test]
    async fn where_no_status_is_shown_only_the_first_message_is_marked() {
        let store = Store::open_in_memory().unwrap();
        let slack = FakeSlack {
            no_status: true,
            ..Default::default()
        };
        for ts in ["1.0", "2.0"] {
            let (at, id) = taken(&store, ts);
            show_working(&&slack, &store, &at, id).await.unwrap();
        }
        assert_eq!(
            *slack.reactions.lock().unwrap(),
            [("D1".into(), "1.0".into(), "eyes".into())]
        );
    }
}
