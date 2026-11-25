//! What the secretary is doing now for the owner, as the conversation on the
//! console shows it until the answer comes.

use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

use crate::console_log::{ConsoleLog, Event};
use crate::input::{Sender, Source};
use crate::slack::presence::WORKING;
use crate::store::{Status, Store, StoredInput, now};
use crate::workgroups::ALL;

/// What waits while the secretary works on something that came before it.
pub const QUEUED: &str = "先の仕事が終わるのを待っています…";
/// Nothing is shown once the secretary gave no sign for this long.
const GIVE_UP_SECS: i64 = 30 * 60;
/// How often a screen is told again that the work goes on.
const TELL_AGAIN_SECS: i64 = 60;

/// What a screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub text: String,
    /// How long it holds unless told again.
    pub seconds: i64,
}

#[derive(Debug, Default)]
struct Turn {
    input: i64,
    answered: bool,
    /// The calls that have not returned, by their ids, with what each does.
    calls: Vec<(String, Option<String>)>,
    /// When the secretary last gave a sign.
    at: i64,
    told: i64,
}

impl Turn {
    fn doing(&self) -> Option<&str> {
        self.calls
            .iter()
            .rev()
            .find_map(|(_, text)| text.as_deref())
    }
}

/// Clones share one state.
#[derive(Clone)]
pub struct Working {
    store: Store,
    console: ConsoleLog,
    turn: Arc<Mutex<Turn>>,
}

impl Working {
    pub fn new(store: Store, console: ConsoleLog) -> Self {
        Self {
            store,
            console,
            turn: Arc::default(),
        }
    }

    fn turn(&self) -> MutexGuard<'_, Turn> {
        self.turn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Inputs came, began or ended: every open console looks again.
    pub fn tell(&self) {
        self.console.notify(Event::Working);
    }

    /// The turn of `input` began, for the first time or again.
    pub fn began(&self, input: i64) {
        *self.turn() = Turn {
            input,
            at: now(),
            told: now(),
            ..Turn::default()
        };
        self.tell();
    }

    /// The secretary said something on the console.
    pub fn answered(&self) {
        self.turn().answered = true;
        self.tell();
    }

    /// The secretary said something on Slack: an answer, unless the owner
    /// asked on the console, where posting is part of the work.
    pub fn said_on_slack(&self) -> anyhow::Result<()> {
        let on_console = self
            .store
            .running_input()?
            .is_some_and(|row| matches!(row.input.source, Source::Console { .. }));
        if !on_console {
            self.answered();
        }
        Ok(())
    }

    /// Follow a message of the session. `doing` puts a tool in words.
    pub fn follow(&self, message: &Value, doing: impl Fn(&str) -> Option<String>) {
        let blocks = message
            .pointer("/message/content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        let text = |block: &Value, key: &str| {
            block
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let tell = {
            let mut turn = self.turn();
            let before = turn.doing().map(str::to_owned);
            turn.at = now();
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("tool_use") => {
                        let doing = doing(&text(block, "name"));
                        turn.calls.push((text(block, "id"), doing));
                    }
                    Some("tool_result") => {
                        let id = text(block, "tool_use_id");
                        turn.calls.retain(|(call, _)| *call != id);
                    }
                    _ => {}
                }
            }
            let tell = before.as_deref() != turn.doing() || turn.at - turn.told >= TELL_AGAIN_SECS;
            if tell {
                turn.told = turn.at;
            }
            tell
        };
        if tell {
            self.tell();
        }
    }

    /// What a console that shows `workgroup` shows now.
    pub fn shown(&self, workgroup: impl AsRef<str>) -> anyhow::Result<Option<Shown>> {
        let in_hand = self.store.in_hand()?;
        let running = in_hand.iter().find(|row| row.status == Status::Running);
        let turn = self.turn();
        let turn = running.filter(|row| row.id == turn.input).map(|_| &*turn);
        let mut asked = in_hand
            .iter()
            .filter(|row| self.asked_here(row, workgroup.as_ref()))
            .filter(|row| row.status != Status::Running || !turn.is_some_and(|t| t.answered));
        // A running one comes before those that wait, whichever came first.
        let Some(row) = asked
            .clone()
            .find(|row| row.status == Status::Running)
            .or_else(|| asked.next())
        else {
            return Ok(None);
        };
        let text = match running {
            Some(running) if running.id != row.id => QUEUED,
            _ => turn.and_then(Turn::doing).unwrap_or(WORKING),
        };
        let mut sign = self.store.touched_at(row.id)?;
        if let Some(running) = running {
            sign = sign.max(self.store.touched_at(running.id)?);
        }
        sign = sign.max(turn.map_or(0, |t| t.at));
        let seconds = sign + GIVE_UP_SECS - now();
        Ok((seconds > 0).then(|| Shown {
            text: text.to_owned(),
            seconds,
        }))
    }

    /// The owner asked it on the console, in what `workgroup` shows.
    fn asked_here(&self, row: &StoredInput, workgroup: &str) -> bool {
        row.input.sender == Sender::Owner
            && match &row.input.source {
                Source::Console {
                    workgroup: said, ..
                } => workgroup.is_empty() || workgroup == ALL || said == ALL || said == workgroup,
                Source::Approval { .. } => true,
                _ => false,
            }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::input::Input;

    struct Famulen {
        store: Store,
        working: Working,
        said: std::cell::Cell<i64>,
    }

    impl Famulen {
        fn new() -> Self {
            let store = Store::open_in_memory().unwrap();
            let working = Working::new(store.clone(), ConsoleLog::new(store.clone()));
            Self {
                store,
                working,
                said: Default::default(),
            }
        }

        fn say(&self, workgroup: &str) -> i64 {
            self.said.set(self.said.get() + 1);
            self.take(
                Sender::Owner,
                Source::Console {
                    message: self.said.get(),
                    workgroup: workgroup.into(),
                    view: String::new(),
                    selection: None,
                    attached: Vec::new(),
                    reply_to: None,
                },
            )
        }

        fn take(&self, sender: Sender, source: Source) -> i64 {
            let input = Input {
                sender,
                source,
                text: "x".into(),
            };
            self.store.enqueue(&input).unwrap().unwrap()
        }

        fn begin(&self, id: i64) {
            assert!(self.store.start_turn(id, "all").unwrap());
            self.working.began(id);
        }

        fn calls(&self, id: &str, tool: &str) {
            self.working.follow(
                &json!({ "message": { "content": [{ "type": "tool_use", "id": id, "name": tool }] } }),
                |name| (name != "quiet").then(|| name.to_owned()),
            );
        }

        fn returns(&self, id: &str) {
            self.working.follow(
                &json!({ "message": { "content": [{ "type": "tool_result", "tool_use_id": id }] } }),
                |_| None,
            );
        }

        fn shown(&self) -> Option<String> {
            self.shown_in("all")
        }

        fn shown_in(&self, workgroup: &str) -> Option<String> {
            self.working.shown(workgroup).unwrap().map(|s| s.text)
        }

        /// As if everything so far had happened `secs` earlier.
        fn pass(&self, secs: i64) {
            self.store
                .conn()
                .execute(
                    "UPDATE inputs SET updated_at = updated_at - ?1",
                    rusqlite::params![secs],
                )
                .unwrap();
            let mut turn = self.working.turn();
            turn.at -= secs;
            turn.told -= secs;
        }
    }

    #[test]
    fn it_is_shown_from_the_moment_the_owner_says_something_until_the_turn_ends() {
        let f = Famulen::new();
        assert_eq!(f.shown(), None);
        let id = f.say("work");
        assert_eq!(f.shown().as_deref(), Some(WORKING));
        f.begin(id);
        assert_eq!(f.shown().as_deref(), Some(WORKING));
        f.store.end_turn().unwrap();
        assert_eq!(f.shown(), None);
    }

    #[test]
    fn an_answer_takes_it_away_before_the_turn_ends_and_the_next_message_shows_it_again() {
        let f = Famulen::new();
        let id = f.say("work");
        f.begin(id);
        f.working.answered();
        assert_eq!(f.shown(), None);
        let next = f.say("work");
        assert_eq!(f.shown().as_deref(), Some(QUEUED));
        f.store.end_turn().unwrap();
        f.begin(next);
        assert_eq!(f.shown().as_deref(), Some(WORKING));
    }

    #[test]
    fn what_comes_while_the_secretary_works_on_something_else_waits_for_it() {
        let f = Famulen::new();
        let other = f.take(Sender::System, wake(1));
        f.begin(other);
        assert_eq!(f.shown(), None, "nobody in the conversation asked it");
        f.calls("a", "探しています…");
        f.say("work");
        assert_eq!(f.shown().as_deref(), Some(QUEUED));
    }

    fn wake(id: i64) -> Source {
        Source::Wake {
            id,
            at: String::new(),
        }
    }

    #[test]
    fn the_call_made_last_says_what_is_done_until_it_returns() {
        let f = Famulen::new();
        let id = f.say("work");
        f.begin(id);
        f.calls("a", "一");
        f.calls("b", "二");
        f.calls("c", "quiet");
        assert_eq!(f.shown().as_deref(), Some("二"));
        f.returns("b");
        assert_eq!(f.shown().as_deref(), Some("一"));
        f.returns("a");
        assert_eq!(f.shown().as_deref(), Some(WORKING));
    }

    #[test]
    fn a_turn_begun_again_forgets_what_the_one_cut_off_did() {
        let f = Famulen::new();
        let id = f.say("work");
        f.begin(id);
        f.calls("a", "一");
        f.working.answered();
        f.store.interrupt_turn().unwrap();
        assert_eq!(f.shown().as_deref(), Some(WORKING));
        f.begin(id);
        assert_eq!(f.shown().as_deref(), Some(WORKING));
    }

    #[test]
    fn a_turn_begun_before_a_restart_is_still_shown() {
        let f = Famulen::new();
        let id = f.say("work");
        f.begin(id);
        let again = Working::new(f.store.clone(), ConsoleLog::new(f.store.clone()));
        assert_eq!(again.shown("all").unwrap().unwrap().text, WORKING);
    }

    #[test]
    fn it_is_shown_only_where_the_message_is() {
        let f = Famulen::new();
        f.say("work");
        for (workgroup, shown) in [("work", true), ("all", true), ("", true), ("home", false)] {
            assert_eq!(f.shown_in(workgroup).is_some(), shown, "{workgroup}");
        }
    }

    #[test]
    fn an_answered_request_shows_it_and_what_did_not_come_from_the_owner_does_not() {
        let f = Famulen::new();
        f.take(Sender::System, wake(1));
        assert_eq!(f.shown(), None);
        f.take(
            Sender::Owner,
            Source::Approval {
                id: 1,
                user: "U1".into(),
            },
        );
        assert!(f.shown().is_some());
    }

    #[test]
    fn a_post_on_slack_answers_a_request_but_not_what_was_asked_on_the_console() {
        let f = Famulen::new();
        let asked = f.say("work");
        f.begin(asked);
        f.working.said_on_slack().unwrap();
        assert!(f.shown().is_some());
        f.store.end_turn().unwrap();
        let request = f.take(
            Sender::Owner,
            Source::Approval {
                id: 1,
                user: "U1".into(),
            },
        );
        f.begin(request);
        f.working.said_on_slack().unwrap();
        assert_eq!(f.shown(), None);
    }

    #[test]
    fn it_is_given_up_once_no_sign_came_for_long_and_a_sign_puts_it_off() {
        let f = Famulen::new();
        let id = f.say("work");
        f.begin(id);
        f.pass(GIVE_UP_SECS - 5);
        let left = f.working.shown("all").unwrap().unwrap().seconds;
        assert!((1..=5).contains(&left), "{left}");
        f.calls("a", "一");
        f.pass(5);
        assert_eq!(f.shown().as_deref(), Some("一"));
        f.pass(GIVE_UP_SECS);
        assert_eq!(f.shown(), None);
    }

    #[test]
    fn consoles_are_told_when_what_is_shown_changes_and_now_and_then_while_it_goes_on() {
        let f = Famulen::new();
        let id = f.say("work");
        f.begin(id);
        let mut events = f.working.console.subscribe();
        let mut told = || events.try_recv().is_ok();
        f.calls("a", "quiet");
        assert!(!told());
        f.calls("b", "一");
        assert!(told());
        f.calls("c", "一");
        assert!(!told());
        f.pass(TELL_AGAIN_SECS);
        f.returns("c");
        assert!(told());
        f.working.answered();
        assert!(told());
    }
}
