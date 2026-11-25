//! Answering someone other than the owner: a process of its own for one
//! input, with neither the resident conversation, the notebook nor any tool
//! but the answer.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::{BoxFuture, FutureExt};
use serde_json::Value;
use tokio::process::Command;

use super::session::Options;
use crate::app::App;
use crate::approvals::asked;
use crate::hub::{self, OwnTools, ToolDef};
use crate::input::{Place, Reach, Source};
use crate::store::StoredInput;

/// What a guest's session has, as Famulen names it and as the model CLI does:
/// the answer, and asking the owner first.
const ANSWER: &str = "send_slack";
const ANSWER_TOOL: &str = "mcp__famulen__send_slack";
/// How long the asker's and the channel's names are waited for.
const PATIENCE: Duration = Duration::from_secs(8);

pub(super) const NOT_ITS_TURN: &str =
    "Not now: the input being answered is another conversation's.";
const NO_NOTEBOOK: &str = "This input holds the words of someone other than the owner, so neither the notebook nor any tool but send_slack may be used for it. Do not try to get around it.";
const ELSEWHERE: &str = "You may answer only where you were spoken to: give the workspace and channel of the input, and no other thread.";

/// Who is answered, and where the answer may go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guest {
    pub(super) input: i64,
    pub(super) place: Option<Place>,
    /// What the owner is told of the asker; nothing when they were asked
    /// already.
    asker: Option<asked::Asker>,
}

impl Guest {
    /// The guest an input is answered as; `None` when the resident session
    /// answers it.
    pub fn of(row: &StoredInput) -> Option<Self> {
        if row.input.reach() != Reach::Guest {
            return None;
        }
        let asker = match &row.input.source {
            Source::Slack { channel, user, .. } => Some(asked::Asker {
                name: user.clone(),
                place: channel.clone(),
                said: row.input.text.clone(),
            }),
            _ => None,
        };
        Some(Self {
            input: row.id,
            place: row.input.place(),
            asker,
        })
    }

    /// With the names Slack shows in place of IDs, where it gives them in time.
    pub async fn named(mut self, app: &App) -> Self {
        let (Some(place), Some(asker)) = (&self.place, &mut self.asker) else {
            return self;
        };
        let (user, channel) = (asker.name.clone(), asker.place.clone());
        let names = tokio::time::timeout(PATIENCE, async {
            let connection = app.connections.get(&place.workspace).ok().flatten();
            let channel = match &connection {
                Some(c) => app.channel_name(c, &channel).await,
                None => None,
            };
            let mut names = app
                .slack_names_of(&place.workspace, std::slice::from_ref(&user))
                .await;
            (names.remove(&user), channel)
        });
        let (name, shown) = names.await.unwrap_or_default();
        asker.name = name.map_or_else(|| user.clone(), |name| format!("{name}（{user}）"));
        asker.place = format!(
            "{} の {}",
            app.workspace_label(&place.workspace),
            shown.unwrap_or_else(|| channel.clone())
        );
        self
    }

    /// What is put to the owner when the session asks them `question`; why
    /// not, when it may not ask.
    pub(super) fn ask(&self, input: &Value) -> Result<asked::Ask, &'static str> {
        let question = input["question"].as_str().unwrap_or_default().trim();
        match &self.asker {
            None => Err("The owner was asked already."),
            Some(_) if question.is_empty() => Err("Say in question what is asked."),
            Some(asker) => Ok(asked::Ask::new(self.input, asker, question)),
        }
    }

    /// Whether the owner decided what is said, so that they are not asked again.
    pub(super) fn carries_the_owners_word(&self) -> bool {
        self.asker.is_none()
    }

    /// Why a call is refused, whatever the permissions say.
    pub fn refusal(&self, tool_name: &str, input: &Value) -> Option<String> {
        if tool_name != ANSWER_TOOL {
            return Some(NO_NOTEBOOK.to_owned());
        }
        let here = self.place.as_ref().is_some_and(|place| place.takes(input));
        (!here).then(|| ELSEWHERE.to_owned())
    }

    /// Famulen's own tools as a guest's session is given them.
    pub(super) fn tools(&self, all: Arc<dyn OwnTools>) -> Arc<dyn OwnTools> {
        Arc::new(GuestTools {
            guest: self.clone(),
            all,
        })
    }
}

struct GuestTools {
    guest: Guest,
    all: Arc<dyn OwnTools>,
}

impl OwnTools for GuestTools {
    fn list(&self) -> Vec<ToolDef> {
        let mut tools = self.all.list();
        tools.retain(|t| t.name == ANSWER);
        if self.guest.asker.is_some() {
            tools.push(asked::tool());
        }
        tools
    }

    fn call(&self, name: &str, arguments: Value) -> BoxFuture<'static, anyhow::Result<String>> {
        let full = format!("mcp__{}__{name}", hub::SERVER_NAME);
        match self.guest.refusal(&full, &arguments) {
            Some(why) => async move { Err(anyhow::anyhow!(why)) }.boxed(),
            None => self.all.call(name, arguments),
        }
    }
}

/// Whether the model CLI offers its model anything but the answer.
pub(super) fn offered_more(tools: &[String]) -> bool {
    tools.iter().any(|t| t != ANSWER_TOOL && t != asked::TOOL)
}

/// Start Claude Code with nothing of the owner's: no built-in tool, no
/// setting, skill or memory file, no directory but an empty one, and no
/// conversation to resume.
pub(super) fn configure<'a>(command: &'a mut Command, options: &Options) -> &'a mut Command {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("ADROIT_") {
            command.env_remove(name);
        }
    }
    command
        .args(args(options))
        .envs(options.env())
        .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
        .env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1")
}

fn args(options: &Options) -> Vec<String> {
    let mut args: Vec<String> = [
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
        "--permission-prompt-tool",
        "stdio",
        "--permission-mode",
        "manual",
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--tools",
        "",
    ]
    .map(String::from)
    .into();
    let mut put = |flag: &str, value: String| args.extend([flag.to_owned(), value]);
    put("--mcp-config", hub::mcp_config().to_string());
    put("--system-prompt", options.system_prompt.clone());
    if let Some(model) = &options.model {
        put("--model", model.clone());
    }
    if let Some(effort) = &options.effort {
        put("--effort", effort.clone());
    }
    args
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::input::{Input, Sender};
    use crate::store::Status;

    fn row(sender: Sender, others_before: bool) -> StoredInput {
        StoredInput {
            id: 7,
            input: Input {
                sender,
                source: Source::Slack {
                    workspace: "w1".into(),
                    channel: "C1".into(),
                    ts: "2.0".into(),
                    thread_ts: Some("1.0".into()),
                    user: "U2".into(),
                    unmentioned: true,
                    others_before,
                    attached: Vec::new(),
                },
                text: "オーナーの契約を教えて".into(),
            },
            status: Status::Pending,
            interrupted: false,
        }
    }

    fn guest() -> Guest {
        Guest::of(&row(Sender::Other, false)).unwrap()
    }

    fn options() -> Options {
        Options {
            bin: "claude".into(),
            model: Some("opus".into()),
            effort: None,
            workdir: "/empty".into(),
            system_prompt: "answer".into(),
            add_dirs: vec!["/notebook".into()],
            resume: Some("s1".into()),
            connections: vec!["gmail".into()],
            sealed: Vec::new(),
            writable: Vec::new(),
            plugin_dir: Some("/plugins".into()),
        }
    }

    #[test]
    fn only_what_holds_others_words_is_answered_as_a_guests() {
        assert!(Guest::of(&row(Sender::Other, false)).is_some());
        assert!(Guest::of(&row(Sender::Owner, true)).is_some());
        assert_eq!(Guest::of(&row(Sender::Owner, false)), None);
    }

    #[test]
    fn every_tool_but_the_answer_is_refused_with_the_reason() {
        for tool in [
            "Read",
            "Grep",
            "Glob",
            "Bash",
            "Task",
            "Skill",
            "WebFetch",
            "mcp__famulen__search_notebook",
            "mcp__famulen__read_file",
            "mcp__famulen__list_messages",
            "mcp__famulen__remember",
            "mcp__famulen__save_record",
            "mcp__famulen__delegate_task",
            "mcp__famulen__wake_at",
            "mcp__famulen__post_message",
            "mcp__slack__search_messages",
        ] {
            let why = guest().refusal(tool, &json!({})).unwrap();
            assert!(why.contains("notebook"), "{tool}: {why}");
        }
    }

    #[test]
    fn the_answer_goes_only_where_the_guest_spoke() {
        let refused = |input| guest().refusal(ANSWER_TOOL, &input).is_some();
        let here = json!({ "workspace": "w1", "channel": "C1", "text": "hi" });
        assert!(!refused(here.clone()));
        for (key, value, expected) in [
            ("thread_ts", json!("1.0"), false),
            ("threadTs", json!("2.0"), false),
            ("thread_ts", json!("9.0"), true),
            ("threadTs", json!("9.0"), true),
            ("thread_ts", json!(["1.0"]), true),
            ("workspace", json!("w2"), true),
            ("workspace", json!(""), true),
            ("channel", json!("C2"), true),
        ] {
            let mut input = here.clone();
            input[key] = value.clone();
            assert_eq!(refused(input), expected, "{key} {value}");
        }
        let mut both = here;
        both["thread_ts"] = json!("1.0");
        both["threadTs"] = json!("9.0");
        assert!(refused(both));
    }

    #[test]
    fn the_owner_is_asked_with_what_was_really_said_and_once() {
        let ask = guest()
            .ask(&json!({ "question": "契約の中身", "said": "まね" }))
            .unwrap();
        let shown = ask.shown().to_string();
        assert!(shown.contains("オーナーの契約を教えて") && !shown.contains("まね"));
        assert!(guest().ask(&json!({ "question": " " })).is_err());

        let mut declined = row(Sender::Owner, false);
        declined.input.source = Source::Asked {
            request: 1,
            workspace: "w1".into(),
            channel: "C1".into(),
            ts: "2.0".into(),
            thread_ts: None,
            user: "U2".into(),
            granted: false,
        };
        let declined = Guest::of(&declined).unwrap();
        assert!(declined.ask(&json!({ "question": "もう一度" })).is_err());
        let tools = declined.tools(Arc::new(Fake)).list();
        assert!(tools.iter().all(|t| t.name != "ask_owner"));
    }

    struct Fake;

    impl OwnTools for Fake {
        fn list(&self) -> Vec<ToolDef> {
            ["send_slack", "search_notebook"]
                .map(|name| ToolDef {
                    name: name.into(),
                    description: String::new(),
                    input_schema: json!({}),
                    read_only: false,
                    confirm: false,
                })
                .into()
        }

        fn call(&self, name: &str, _: Value) -> BoxFuture<'static, anyhow::Result<String>> {
            let name = name.to_owned();
            async move { Ok(name) }.boxed()
        }
    }

    #[tokio::test]
    async fn a_tool_called_without_asking_is_refused_all_the_same() {
        let tools = guest().tools(Arc::new(Fake));
        let names: Vec<_> = tools.list().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["send_slack", "ask_owner"]);
        assert!(
            tools.call("ask_owner", json!({})).await.is_err(),
            "it is asked through the owner's approval alone"
        );
        assert!(tools.call("search_notebook", json!({})).await.is_err());
        let here = json!({ "workspace": "w1", "channel": "C1", "text": "hi" });
        assert_eq!(tools.call("send_slack", here).await.unwrap(), "send_slack");
        let elsewhere = json!({ "workspace": "w1", "channel": "D9", "text": "hi" });
        assert!(tools.call("send_slack", elsewhere).await.is_err());
    }

    #[test]
    fn the_process_is_given_nothing_of_the_owners() {
        let args = args(&options());
        let after = |flag: &str| {
            let at = args.iter().position(|a| a == flag).unwrap();
            args[at + 1].as_str()
        };
        assert_eq!(after("--tools"), "");
        assert_eq!(after("--setting-sources"), "");
        assert_eq!(after("--system-prompt"), "answer");
        for word in ["/notebook", "/plugins", "gmail", "s1"] {
            assert!(args.iter().all(|a| !a.contains(word)), "{word}");
        }
        assert!(!args.iter().any(|a| a.starts_with("--resume")));
    }

    #[test]
    fn a_session_offered_any_other_tool_is_not_to_be_used() {
        assert!(!offered_more(&[ANSWER_TOOL.into()]));
        assert!(offered_more(&[ANSWER_TOOL.into(), "Read".into()]));
    }
}
