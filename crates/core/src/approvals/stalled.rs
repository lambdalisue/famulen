//! Famulen's own question to the owner on delegated work that stopped with
//! its plan unfinished and that nothing but the owner moves on.

use serde_json::{Value, json};

use famulen_gateway::permissions::Marks;
use famulen_gateway::{Call, Choice, Question, Ticket, Verdict};

use crate::hub::SERVER_NAME;
use crate::permissions::{Permissions, SECRETARY};

/// Asked as a call of Famulen's own tool, though nothing can call it.
pub(crate) const NAME: &str = "ask_stalled";

/// The name the approvals show it under.
pub(crate) const TOOL: &str = "mcp__famulen__ask_stalled";

const GO_ON: &str = "続ける";
const REDO: &str = "やり直す";
const DROP: &str = "やめる";

/// How much of what a blocked step says is shown.
const NOTE: usize = 300;

/// Why the owner is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Why {
    /// What is left waits on a decision.
    Blocked,
    /// Asking it to go on moved nothing.
    Stuck,
    /// A message sent to it was not taken in, so nothing more is sent.
    Undelivered,
}

pub(crate) struct Stall<'a> {
    pub run: i64,
    pub code: &'a str,
    pub title: &'a str,
    pub session: &'a str,
    pub why: Why,
    pub blocked: &'a [String],
    pub turn: i64,
}

/// Hold the question for the owner, giving its ticket.
pub(crate) fn put(permissions: &Permissions, stall: &Stall) -> anyhow::Result<i64> {
    let why = match stall.why {
        Why::Blocked => "blocked",
        Why::Stuck => "stuck",
        Why::Undelivered => "undelivered",
    };
    let shown = json!({
        "run": stall.run,
        "code": stall.code,
        "title": stall.title,
        "session": stall.session,
        "why": why,
        "blocked": stall.blocked.iter().map(|b| cut(b, NOTE)).collect::<Vec<_>>(),
        "turn": stall.turn,
    });
    let call = Call {
        marks: Some(Marks {
            read_only: Some(false),
            always_ask: true,
        }),
        ask: Some("止まった委託をどうするかは主人が決める".into()),
        ..Call::new(SECRETARY, SERVER_NAME, NAME, shown)
    };
    match permissions.gateway().judge(&call)? {
        Verdict::Hold(ticket) => Ok(ticket.id),
        Verdict::Allow { .. } => anyhow::bail!("{NAME} was not held"),
        Verdict::Deny { reason } => anyhow::bail!("{NAME} was denied: {reason}"),
    }
}

pub(crate) fn is_stall(ticket: &Ticket) -> bool {
    ticket.connection == SERVER_NAME && ticket.tool == NAME
}

pub(crate) fn question(shown: &Value) -> Question {
    let text = |key: &str| shown[key].as_str().unwrap_or_default();
    let (why, choices) = match text("why") {
        "undelivered" => (
            "送った指示が届いていないので、これ以上は送れません。新しいセッションでやり直しますか（いまのセッションは残ります）。",
            [
                (REDO, "新しいセッションで始め直す"),
                (DROP, "この委託を取りやめる"),
            ],
        ),
        why => (
            if why == "blocked" {
                "残りは判断を待つ段だけです。どう進めますか（言葉で書けば、そのまま伝えます）。"
            } else {
                "続けるよう伝えても段が進みません。どうしますか（言葉で書けば、そのまま伝えます）。"
            },
            [
                (GO_ON, "同じセッションに続きを進めさせる"),
                (DROP, "この委託を取りやめる"),
            ],
        ),
    };
    let mut question = format!(
        "委託 #{} {}「{}」は予定を残して止まっています（Adroit のセッション {}）。\n{why}",
        shown["run"],
        text("code"),
        text("title"),
        text("session"),
    );
    for note in shown["blocked"].as_array().into_iter().flatten() {
        question.push_str("\n・");
        question.push_str(note.as_str().unwrap_or_default());
    }
    Question {
        question,
        options: choices
            .into_iter()
            .map(|(label, description)| Choice {
                label: label.into(),
                description: description.into(),
            })
            .collect(),
        several: false,
    }
}

/// What the owner answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Word {
    GoOn,
    /// Start the work again in a new session.
    Redo,
    /// Call the work off.
    Drop,
    /// The owner's own words, for the session.
    Say(String),
    /// Leave it as it stands.
    Leave,
}

impl Word {
    pub(crate) fn of(shown: &Value, approved: bool, said: Option<&str>) -> Self {
        match said.map(str::trim) {
            Some(GO_ON) => Word::GoOn,
            Some(REDO) => Word::Redo,
            Some(DROP) => Word::Drop,
            Some(words) if !words.is_empty() => Word::Say(words.into()),
            _ if !approved => Word::Leave,
            _ if shown["why"] == "undelivered" => Word::Redo,
            _ => Word::GoOn,
        }
    }

    pub(crate) fn kept(&self) -> String {
        match self {
            Word::GoOn => GO_ON.into(),
            Word::Redo => REDO.into(),
            Word::Drop => DROP.into(),
            Word::Say(words) => format!("say:{words}"),
            Word::Leave => "leave".into(),
        }
    }

    pub(crate) fn from_kept(kept: &str) -> Option<Self> {
        match kept {
            GO_ON => Some(Word::GoOn),
            REDO => Some(Word::Redo),
            DROP => Some(Word::Drop),
            "leave" => Some(Word::Leave),
            _ => Some(Word::Say(kept.strip_prefix("say:")?.into())),
        }
    }
}

fn cut(text: &str, most: usize) -> String {
    match text.char_indices().nth(most) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.into(),
    }
}
