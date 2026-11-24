//! Which Slack channels the channel is attending: after the owner mentions
//! the secretary in a channel, what people say there is weighed, one message
//! at a time, for whether it is said to the secretary, for a while.
//!
//! Nothing ticks: whether a channel is attended is decided when something
//! arrives from it, by comparing the time with the channel's deadline, and a
//! deadline that has passed is dropped then.

use chrono::{DateTime, Duration, Utc};

use crate::store::Store;

/// How long a channel stays attended when the owner has not said otherwise.
pub const DEFAULT_MINUTES: u32 = 60;

const LENGTH_KEY: &str = "channel.attention_minutes";
/// What was said in an attended channel but not to the secretary, kept for
/// the next words that are.
const CONTEXT_LINES: usize = 20;
const CONTEXT_CHARS: usize = 500;

fn until_key(workspace: &str, channel: &str) -> String {
    format!("channel.attend.{workspace}.{channel}")
}

fn context_key(workspace: &str, channel: &str) -> String {
    format!("channel.attend_context.{workspace}.{channel}")
}

/// How many minutes a channel stays attended after the last exchange; zero
/// means the channel answers only mentions and direct messages.
pub fn minutes(store: &Store) -> anyhow::Result<u32> {
    Ok(store
        .state(LENGTH_KEY)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MINUTES))
}

pub fn set_minutes(store: &Store, minutes: u32) -> anyhow::Result<()> {
    store.set_state(LENGTH_KEY, minutes.to_string())
}

/// Start attending `channel`, or keep attending it from `now`.
pub fn attend(
    store: &Store,
    workspace: &str,
    channel: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let minutes = minutes(store)?;
    if minutes == 0 {
        return forget(store, workspace, channel);
    }
    let until = now + Duration::minutes(i64::from(minutes));
    store.set_state(until_key(workspace, channel), until.to_rfc3339())
}

/// Whether `channel` is attended at `now`. A lapsed channel is forgotten,
/// with what others said there.
pub fn attending(
    store: &Store,
    workspace: &str,
    channel: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let until = store
        .state(until_key(workspace, channel))?
        .and_then(|v| DateTime::parse_from_rfc3339(&v).ok())
        .map(|t| t.with_timezone(&Utc));
    match until {
        Some(until) if now < until && minutes(store)? > 0 => Ok(true),
        Some(_) => forget(store, workspace, channel).map(|()| false),
        None => Ok(false),
    }
}

/// Keep attending `channel` from `now`, if it is attended.
pub fn extend(
    store: &Store,
    workspace: &str,
    channel: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    if attending(store, workspace, channel, now)? {
        attend(store, workspace, channel, now)?;
    }
    Ok(())
}

fn forget(store: &Store, workspace: &str, channel: &str) -> anyhow::Result<()> {
    store.clear_state(until_key(workspace, channel))?;
    store.clear_state(context_key(workspace, channel))
}

/// Keep a line said in an attended channel but not to the secretary, for
/// the next words there that are.
pub fn remember(store: &Store, workspace: &str, channel: &str, line: &str) -> anyhow::Result<()> {
    let key = context_key(workspace, channel);
    let mut lines: Vec<String> = store
        .state(&key)?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default();
    lines.push(line.chars().take(CONTEXT_CHARS).collect());
    let over = lines.len().saturating_sub(CONTEXT_LINES);
    lines.drain(..over);
    store.set_state(&key, serde_json::to_string(&lines)?)
}

/// What was said in `channel` but not to the secretary since it was last
/// spoken to there, oldest first.
pub fn context(store: &Store, workspace: &str, channel: &str) -> anyhow::Result<Vec<String>> {
    Ok(store
        .state(context_key(workspace, channel))?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default())
}

/// [`context`], emptied once taken.
pub fn take_context(store: &Store, workspace: &str, channel: &str) -> anyhow::Result<Vec<String>> {
    let key = context_key(workspace, channel);
    let lines = store
        .state(&key)?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default();
    store.clear_state(&key)?;
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-22T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::minutes(minutes)
    }

    #[test]
    fn a_channel_is_attended_for_the_set_minutes_after_the_mention() {
        let store = Store::open_in_memory().unwrap();
        set_minutes(&store, 10).unwrap();
        attend(&store, "w1", "C1", at(0)).unwrap();

        assert!(attending(&store, "w1", "C1", at(9)).unwrap());
        assert!(!attending(&store, "w1", "C2", at(9)).unwrap());
        assert!(!attending(&store, "w2", "C1", at(9)).unwrap());
        assert!(!attending(&store, "w1", "C1", at(10)).unwrap());
    }

    #[test]
    fn an_hour_unless_the_owner_chose_otherwise() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(minutes(&store).unwrap(), 60);
        attend(&store, "w1", "C1", at(0)).unwrap();
        assert!(attending(&store, "w1", "C1", at(59)).unwrap());
        assert!(!attending(&store, "w1", "C1", at(60)).unwrap());
    }

    #[test]
    fn the_length_the_owner_chose_stands_over_the_default() {
        let store = Store::open_in_memory().unwrap();
        set_minutes(&store, 10).unwrap();
        assert_eq!(minutes(&store).unwrap(), 10);
    }

    #[test]
    fn each_exchange_extends_it_and_a_lapsed_channel_stays_lapsed() {
        let store = Store::open_in_memory().unwrap();
        set_minutes(&store, 10).unwrap();
        attend(&store, "w1", "C1", at(0)).unwrap();
        extend(&store, "w1", "C1", at(8)).unwrap();
        assert!(attending(&store, "w1", "C1", at(17)).unwrap());

        assert!(!attending(&store, "w1", "C1", at(30)).unwrap());
        extend(&store, "w1", "C1", at(31)).unwrap();
        assert!(!attending(&store, "w1", "C1", at(32)).unwrap());
    }

    #[test]
    fn zero_minutes_turns_it_off() {
        let store = Store::open_in_memory().unwrap();
        attend(&store, "w1", "C1", at(0)).unwrap();
        set_minutes(&store, 0).unwrap();
        assert!(!attending(&store, "w1", "C1", at(1)).unwrap());
        attend(&store, "w1", "C1", at(2)).unwrap();
        assert!(!attending(&store, "w1", "C1", at(3)).unwrap());
    }

    #[test]
    fn what_others_said_is_taken_once_and_dropped_when_it_lapses() {
        let store = Store::open_in_memory().unwrap();
        set_minutes(&store, 10).unwrap();
        attend(&store, "w1", "C1", at(0)).unwrap();
        remember(&store, "w1", "C1", "<@U2>: 今どこ？").unwrap();
        remember(&store, "w1", "C1", "<@U3>: 会議室です").unwrap();
        assert_eq!(
            take_context(&store, "w1", "C1").unwrap(),
            ["<@U2>: 今どこ？", "<@U3>: 会議室です"]
        );
        assert!(take_context(&store, "w1", "C1").unwrap().is_empty());

        remember(&store, "w1", "C1", "<@U2>: あとで").unwrap();
        assert!(!attending(&store, "w1", "C1", at(20)).unwrap());
        assert!(take_context(&store, "w1", "C1").unwrap().is_empty());
    }

    #[test]
    fn only_the_latest_lines_are_kept() {
        let store = Store::open_in_memory().unwrap();
        for i in 0..(CONTEXT_LINES + 5) {
            remember(&store, "w1", "C1", &format!("line {i}")).unwrap();
        }
        let lines = take_context(&store, "w1", "C1").unwrap();
        assert_eq!(lines.len(), CONTEXT_LINES);
        assert_eq!(lines[0], "line 5");
    }
}
