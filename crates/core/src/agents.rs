//! What Famulen and an agent CLI say to each other, the same for every agent.
//! An adapter turns one agent's own protocol into it. Outside the adapters
//! nothing knows how an agent is started, what its lines look like or how it
//! ends.
//!
//! An adapter judges nothing: what an agent asks is passed on as a
//! [`Question`], and the permissions decide it.

pub mod claude;
#[cfg(test)]
mod conformance;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Something an agent may be able to do. What it cannot is not made up for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// It asks before it uses a tool.
    AsksPermission,
    /// It asks its user questions.
    AsksQuestions,
    /// What is sent while it works joins its turn.
    Steering,
    Interrupting,
    Resuming,
    /// It calls the tools of servers it is handed.
    Servers,
    Usage,
    Plan,
}

/// How far an agent's own sandbox reaches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sandbox {
    #[default]
    None,
    /// The shell commands it runs are bounded.
    Shell,
    /// All of it is.
    Whole,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub can: BTreeSet<Capability>,
    pub sandbox: Sandbox,
}

impl Capabilities {
    /// Refuse work that needs what is not there, before it is started.
    pub fn check(&self, needs: &Capabilities) -> anyhow::Result<()> {
        let missing: Vec<_> = needs.can.difference(&self.can).collect();
        anyhow::ensure!(
            missing.is_empty(),
            "the agent cannot do what the work needs: {missing:?}"
        );
        anyhow::ensure!(
            self.sandbox >= needs.sandbox,
            "the agent's sandbox is {:?}, and the work needs {:?}",
            self.sandbox,
            needs.sandbox
        );
        Ok(())
    }
}

/// What is known of an agent installed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub version: String,
    /// Whether the version is one the adapter was made for.
    pub supported: bool,
    /// `None` when it does not say.
    pub logged_in: Option<bool>,
    pub capabilities: Capabilities,
}

/// Work as an agent is started on it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Work {
    /// The agent's program; the adapter's usual one when not said.
    pub program: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// How it is to work, beside the message.
    pub instructions: String,
    pub message: String,
    /// The built-in tools it is offered.
    pub tools: Vec<String>,
    /// Built-in tools it must not be offered, whatever its version adds.
    pub withheld: Vec<String>,
    /// The servers whose tools it is offered, answered by Famulen.
    pub servers: Vec<String>,
    pub workdir: PathBuf,
    pub fence: Fence,
    pub needs: Capabilities,
    /// The id a new conversation is to have, chosen before it is started.
    pub conversation: Option<String>,
    /// Whether to tell of what is written while it is being written.
    pub parts: bool,
}

/// What an agent's sandbox keeps from it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fence {
    /// Neither read nor written.
    pub sealed: Vec<PathBuf>,
    /// Not read, but for what is `readable`.
    pub unread: Vec<PathBuf>,
    pub readable: Vec<PathBuf>,
    /// Written, beside the directory it works in.
    pub writable: Vec<PathBuf>,
    /// Not written, though they are inside what is `writable`.
    pub unwritable: Vec<PathBuf>,
    /// Not connected to, whatever else is allowed; nor is a socket that is a
    /// file.
    pub hosts: Vec<String>,
}

/// How an agent's process is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// The lines its stdin begins with.
    pub input: Vec<String>,
}

/// When what is sent is to reach the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Once the turn it is in is over.
    Between,
    /// At once, in the turn it is in.
    Now,
}

/// The agents there is an adapter for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Agent {
    Claude,
}

impl Agent {
    pub fn adapter(self) -> &'static dyn Adapter {
        match self {
            Agent::Claude => &claude::Claude,
        }
    }
}

/// One kind of agent.
pub trait Adapter: Send + Sync {
    /// What it can do, whichever version is installed.
    fn capabilities(&self) -> Capabilities;
    /// Ask the installed program of itself. No credential is read.
    fn probe<'a>(&'a self, program: Option<&'a str>) -> BoxFuture<'a, anyhow::Result<Probe>>;
    /// How `work` is started, or a conversation gone on with. It fails when
    /// the agent cannot do what the work needs.
    fn launch(&self, work: &Work, resume: Option<&str>) -> anyhow::Result<Launch>;
    /// Whether a conversation it was started on is kept where it can be gone
    /// on with.
    fn find(&self, conversation: &str) -> bool;
    /// The line that answers a question, and nothing else: it may be written
    /// by another process than read the question. `Err` when the answer is
    /// none to what was asked.
    fn answer(&self, question: &Question, answer: &Answer) -> anyhow::Result<String>;
    /// What reads and writes the lines of one process.
    fn codec(&self) -> Box<dyn Codec>;
}

/// Reads what one process of an agent writes, and writes what it reads.
pub trait Codec: Send {
    /// What one line of its stdout tells. A line that cannot be read tells
    /// so; none is dropped unseen.
    fn read(&mut self, line: &str) -> Vec<Body>;
    fn send(&mut self, text: &str, when: When) -> anyhow::Result<String>;
    /// Stop the turn it is in.
    fn interrupt(&mut self) -> anyhow::Result<String>;
}

/// What an agent did, in the order it did it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Counts the lines of its run, from 1; the same whenever it is read.
    pub seq: u64,
    /// Counts the events of one line.
    pub part: u32,
    /// Unix milliseconds.
    pub at: i64,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Started {
        conversation: String,
    },
    Stands {
        standing: Standing,
    },
    /// What it wrote, whole.
    Said {
        message: String,
        text: String,
    },
    /// More of what it is writing.
    SaidMore {
        message: String,
        text: String,
    },
    Thought {
        message: String,
        text: String,
    },
    ThoughtMore {
        message: String,
        text: String,
    },
    ToolCalled {
        call: String,
        tool: String,
        input: Value,
    },
    /// More of the input it is writing, as a piece of its JSON.
    ToolCalledMore {
        call: String,
        input: String,
    },
    /// A tool is still at work.
    ToolWorks {
        call: String,
        /// What it put out since the last time.
        output: String,
        seconds: Option<f64>,
    },
    ToolAnswered {
        call: String,
        ok: bool,
        output: String,
    },
    FileChanged {
        call: String,
        path: String,
        /// A unified diff, without the lines that name the file.
        diff: String,
    },
    Asked {
        question: Question,
    },
    /// A question that is to be answered no more.
    Withdrawn {
        question: String,
    },
    Planned {
        steps: Vec<PlanStep>,
    },
    Used {
        of: Counted,
        model: Option<String>,
        /// All that was read, with what came from a cache.
        input: u64,
        cached: u64,
        output: u64,
    },
    Warned {
        what: String,
    },
    TurnEnded {
        ok: bool,
        /// What it wrote last, or what went wrong.
        text: String,
    },
    /// Its process is gone.
    Ended {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    Working,
    /// For an answer.
    Waiting,
    Idle,
}

/// What a number of tokens is of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Counted {
    /// So far of one message, told of again as it grows.
    Message(String),
    /// Of the whole turn, its messages included.
    Turn,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: String,
    pub parent: String,
    pub title: String,
    /// `pending`, `in_progress`, `completed` or `blocked`.
    pub status: String,
    pub weight: f64,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub ask: Ask,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ask {
    /// May it use this tool?
    Tool {
        by: By,
        tool: String,
        input: Value,
        means: Means,
    },
    /// A JSON-RPC message to a server it was handed.
    Server { server: String, message: Value },
    /// Something the adapter does not know, to be refused.
    Unknown { what: String },
}

/// Who in an agent asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum By {
    Itself,
    Subagent,
}

/// What a tool the agent asks to use is to its user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Means {
    Use,
    /// It asks its user to choose.
    Choose {
        choices: Vec<Choice>,
    },
    /// It asks whether to go on as planned.
    Plan {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub text: String,
    pub options: Vec<String>,
    pub several: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Answer {
    Allow,
    Refuse {
        why: String,
    },
    /// What was chosen, by the text of each choice.
    Chosen {
        answers: BTreeMap<String, String>,
    },
    /// What the server answers; nothing to what wants no answer.
    Served {
        response: Option<Value>,
    },
    Unsupported,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_that_needs_what_an_agent_cannot_do_is_refused() {
        let able = Capabilities {
            can: [Capability::Usage, Capability::Servers].into(),
            sandbox: Sandbox::Shell,
        };
        able.check(&able).unwrap();
        able.check(&Capabilities::default()).unwrap();
        let needs = |can: &[Capability], sandbox| Capabilities {
            can: can.iter().copied().collect(),
            sandbox,
        };
        let why = able
            .check(&needs(&[Capability::AsksPermission], Sandbox::None))
            .unwrap_err();
        assert!(why.to_string().contains("AsksPermission"), "{why}");
        assert!(able.check(&needs(&[], Sandbox::Whole)).is_err());
    }
}
