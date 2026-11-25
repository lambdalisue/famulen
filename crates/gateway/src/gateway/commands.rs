//! Tools that run one program with arguments a definition writes. No shell
//! is involved, so a value can only ever be an argument.

use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};

use crate::connections::Connection;
use crate::definitions::{CommandArg, CommandTool, Definition, EachTime};
use crate::mcp::ToolResult;
use crate::template::{Context, text_of};

use super::Gateway;

/// A question put to the owner, with what they may choose.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Question {
    pub question: String,
    #[serde(default)]
    pub options: Vec<Choice>,
    #[serde(default, rename = "multiSelect")]
    pub several: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Choice {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

/// How much of what a program prints is kept.
const KEPT: usize = 64 * 1024;

/// The arguments `tool` runs its program with, filled from `ctx`.
fn arguments(
    tool: &CommandTool,
    args: &[CommandArg],
    ctx: &Context,
) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    for arg in args {
        match arg {
            CommandArg::Text(text) => out.push(
                ctx.render_text(text)
                    .ok_or_else(|| anyhow::anyhow!("{} needs {text}", tool.name))?,
            ),
            CommandArg::Group(group) => {
                let filled: Option<Vec<String>> =
                    group.iter().map(|text| ctx.render_text(text)).collect();
                out.extend(filled.unwrap_or_default());
            }
            CommandArg::Flag { flag, when } => {
                if ctx.render(when) == Some(Value::Bool(true)) {
                    out.push(flag.clone());
                }
            }
            CommandArg::Each { flag, each } => {
                if let Some(Value::Array(items)) = ctx.render(each) {
                    for item in items {
                        out.extend([flag.clone(), text_of(&item)]);
                    }
                }
            }
        }
    }
    Ok(out)
}

fn kept(bytes: &[u8]) -> String {
    let cut = &bytes[..bytes.len().min(KEPT)];
    String::from_utf8_lossy(cut).trim().to_owned()
}

impl Gateway {
    /// The tools a definition's commands are.
    pub(super) fn command_tools(def: &Definition) -> Vec<Value> {
        def.commands
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.input.clone().unwrap_or_else(|| json!({ "type": "object" })),
                    "annotations": {
                        "readOnlyHint": t.read_only,
                        "destructiveHint": t.destructive,
                        "openWorldHint": t.open_world,
                    },
                })
            })
            .collect()
    }

    /// The command tool a call is of, when the owner is asked of each call.
    pub(super) fn each_time(
        &self,
        connection: &str,
        tool: &str,
    ) -> anyhow::Result<Option<(Connection, Definition, EachTime)>> {
        let Some(c) = self.sealed().get(connection)? else {
            return Ok(None);
        };
        let Some(Ok(def)) = self.definition_of(&c) else {
            return Ok(None);
        };
        let each = def
            .commands
            .iter()
            .find(|t| t.name == tool)
            .and_then(|t| t.each_time.clone());
        Ok(each.map(|each| (c, def, each)))
    }

    /// Whether the owner is asked of every call of a tool, whatever
    /// permission is chosen.
    pub fn asks_each_time(&self, connection: &str, tool: &str) -> anyhow::Result<bool> {
        Ok(self.each_time(connection, tool)?.is_some())
    }

    /// What is being decided by a call the owner is asked of each time, as
    /// its reading tool tells it now; `None` for any other call.
    pub(super) async fn decided_by(
        &self,
        connection: &str,
        tool: &str,
        args: &Value,
    ) -> anyhow::Result<Option<Value>> {
        let Some((c, def, each)) = self.each_time(connection, tool)? else {
            return Ok(None);
        };
        let read = self
            .call_command(&c, &def, &each.reads, args.clone())
            .await?;
        anyhow::ensure!(!read.is_error, "{}", read.text);
        let answer: Value = serde_json::from_str(&read.text)?;
        let found = answer
            .pointer(&each.at)
            .filter(|v| !v.is_null())
            .ok_or_else(|| anyhow::anyhow!("there is nothing to decide there now"))?;
        let mine = self.context(&c, &def, args.clone(), false)?;
        for (at, expected) in &each.same {
            let there = answer.pointer(at).map(text_of);
            anyhow::ensure!(
                there.is_some() && there == mine.render_text(expected),
                "what is to be decided there now is another"
            );
        }
        Ok(Some(found.clone()))
    }

    /// The questions a held call puts to the owner, when they answer those
    /// instead of approving it.
    pub fn questions_of(
        &self,
        connection: &str,
        tool: &str,
        shown: &Value,
    ) -> anyhow::Result<Vec<Question>> {
        let Some((_, _, each)) = self.each_time(connection, tool)? else {
            return Ok(Vec::new());
        };
        let Some(at) = each.questions else {
            return Ok(Vec::new());
        };
        let questions = shown["decides"].pointer(&at).cloned().unwrap_or_default();
        Ok(serde_json::from_value(questions)?)
    }

    /// Make the call the owner was asked of and answered.
    pub(super) async fn run_decided(
        &self,
        connection: &str,
        tool: &str,
        args: Value,
        rejected: bool,
    ) -> anyhow::Result<()> {
        let (c, def, each) = self
            .each_time(connection, tool)?
            .ok_or_else(|| anyhow::anyhow!("{connection} has no tool {tool} any more"))?;
        if rejected && each.on_reject.is_none() {
            return Ok(());
        }
        let result = self.run_command(&c, &def, tool, args, rejected).await?;
        anyhow::ensure!(!result.is_error, "{}", result.text);
        Ok(())
    }

    /// A definition's program with `args`, seeing none of the gateway's
    /// secrets.
    pub(super) fn command<'a>(
        &self,
        connection: &Connection,
        def: &Definition,
        args: impl IntoIterator<Item = &'a String>,
    ) -> anyhow::Result<tokio::process::Command> {
        let transport = def
            .transport
            .command
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("{} runs no command", def.id))?;
        let ctx = self.context(connection, def, Value::Null, true)?;
        let program = ctx
            .render_text(&transport.program)
            .ok_or_else(|| anyhow::anyhow!("the program names a value it does not have"))?;
        let mut command = tokio::process::Command::new(&program);
        crate::process::hide_secrets(&mut command);
        for name in &transport.unset {
            command.env_remove(name);
        }
        command
            .args(args)
            .envs(ctx.render_map(&transport.env))
            .stdin(Stdio::null())
            .kill_on_drop(true);
        Ok(command)
    }

    /// Run a command tool with `args`.
    pub(super) async fn call_command(
        &self,
        connection: &Connection,
        def: &Definition,
        tool: &str,
        args: Value,
    ) -> anyhow::Result<ToolResult> {
        self.run_command(connection, def, tool, args, false).await
    }

    /// Run a command tool, or with `rejected` what it runs when the owner
    /// says no.
    async fn run_command(
        &self,
        connection: &Connection,
        def: &Definition,
        tool: &str,
        args: Value,
        rejected: bool,
    ) -> anyhow::Result<ToolResult> {
        let transport = def
            .transport
            .command
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("{} runs no command", def.id))?;
        let tool = def
            .commands
            .iter()
            .find(|t| t.name == tool)
            .ok_or_else(|| anyhow::anyhow!("{} has no tool {tool}", connection.id))?;
        let ctx = self.context(connection, def, args, true)?;
        let written = match (rejected, &tool.each_time) {
            (false, _) => &tool.args,
            (true, Some(each)) => each
                .on_reject
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("{} says nothing when rejected", tool.name))?,
            (true, None) => anyhow::bail!("{} is not asked each time", tool.name),
        };
        let args = arguments(tool, written, &ctx)?;
        let mut command = self.command(connection, def, &args)?;
        let input = match &tool.stdin {
            Some(text) => {
                let text = ctx
                    .render_text(text)
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("{} needs {text}", tool.name))?;
                command.stdin(Stdio::piped());
                Some(text)
            }
            None => None,
        };
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let program = &transport.program;
        let limit = Duration::from_secs(transport.timeout_secs());
        let run = async {
            let mut child = command.spawn()?;
            let stdin = child.stdin.take();
            let write = async {
                if let (Some(text), Some(mut stdin)) = (input, stdin) {
                    use tokio::io::AsyncWriteExt;
                    // A program that ends without reading it is told by its exit.
                    let _ = stdin.write_all(text.as_bytes()).await;
                }
            };
            // Not one after the other: a program that writes as it reads
            // stops reading once nobody takes what it wrote.
            tokio::join!(write, child.wait_with_output()).1
        };
        let output = tokio::time::timeout(limit, run)
            .await
            .map_err(|_| anyhow::anyhow!("{program} did not end within {} s", limit.as_secs()))?
            .map_err(|error| anyhow::anyhow!("cannot run {program}: {error}"))?;
        let failed = !output.status.success();
        let mut text = kept(&output.stdout);
        if failed || text.is_empty() {
            let said = kept(&output.stderr);
            if !said.is_empty() {
                text = [text, said].join("\n").trim().to_owned();
            }
        }
        Ok(super::text_result(text, failed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::Connections;
    use crate::permissions::{AutoCourse, auto_course};
    use crate::store::Store;

    const SAY: &str = r#"
format = 1
id = "say"
name = "Say"

[transport.command]
program = "perl"
unset = ["FAMULEN_TEST_UNSET"]
timeout = "TIMEOUT"

[[commands]]
name = "echo"
description = "Say the arguments back."
args = ["-e", "print join('|', @ARGV)", "--", "{args.text}", ["--to", "{args.to}"], { flag = "--loud", when = "{args.loud}" }]

[[commands]]
name = "fail"
description = "Fail."
args = ["-e", "print STDERR 'it broke'; exit 3"]
read_only = true

[[commands]]
name = "copy"
description = "Say what is read back."
args = ["-pe", "1"]
stdin = "{args.text}"

[[commands]]
name = "hang"
description = "Never end."
args = ["-e", "sleep 30"]
"#;

    /// Long enough for a busy machine to start the program.
    fn gateway() -> (Gateway, Connection) {
        gateway_waiting("2m")
    }

    fn gateway_waiting(timeout: &str) -> (Gateway, Connection) {
        let say = SAY.replace("TIMEOUT", timeout);
        let store = Store::open_in_memory().unwrap();
        let g = Gateway::new(
            store.clone(),
            Connections::new(store),
            reqwest::Client::new(),
            None,
        )
        .with_definitions(None, &[("say.toml".into(), say)]);
        let c = g.add_defined("say", Default::default(), None).unwrap();
        (g, c)
    }

    #[tokio::test]
    async fn a_value_is_only_ever_one_argument_and_what_is_not_given_is_left_out() {
        let (g, c) = gateway();
        for (args, said) in [
            (json!({ "text": "a; b $(c) 'd'" }), "a; b $(c) 'd'"),
            (
                json!({ "text": "--loud", "to": "x y", "loud": true }),
                "--loud|--to|x y|--loud",
            ),
            (json!({ "text": "t", "loud": false }), "t"),
        ] {
            let result = g.call_as_owner(&c, "echo", args).await.unwrap();
            assert!(!result.is_error);
            assert_eq!(result.text, said);
        }
        assert!(g.call_as_owner(&c, "echo", json!({})).await.is_err());
        assert!(g.call_as_owner(&c, "other", json!({})).await.is_err());
    }

    #[tokio::test]
    async fn a_command_that_fails_or_does_not_end_is_a_failure_that_says_why() {
        let (g, c) = gateway();
        let failed = g.call_as_owner(&c, "fail", json!({})).await.unwrap();
        assert!(failed.is_error);
        assert_eq!(failed.text, "it broke");
        let (g, c) = gateway_waiting("2s");
        let hung = g.call_as_owner(&c, "hang", json!({})).await.unwrap_err();
        assert!(hung.to_string().contains("did not end"), "{hung}");
    }

    #[tokio::test]
    async fn a_command_that_writes_while_it_reads_does_not_stall() {
        let (g, c) = gateway();
        let text = "a line of what it reads\n".repeat(40_000);

        let result = g.call_as_owner(&c, "copy", json!({ "text": text })).await;

        assert!(result.unwrap().text.starts_with("a line of what it reads"));
    }

    #[tokio::test]
    async fn the_tools_carry_the_marks_the_definition_gives() {
        let (g, c) = gateway();
        g.read_tools(&c.id).await.unwrap();
        let tools = g.connections.tools(&c.id).unwrap();
        let reads: Vec<_> = tools
            .iter()
            .map(|t| (t.name.as_str(), t.read_only))
            .collect();
        assert!(reads.contains(&("fail", Some(true))), "{reads:?}");
        assert!(reads.contains(&("echo", Some(false))), "{reads:?}");
        let def = g.definitions().get("adroit").cloned().unwrap();
        let start = def
            .commands
            .iter()
            .find(|t| t.name == "start_session")
            .unwrap();
        assert_eq!(auto_course(start.marks(), &start.name), AutoCourse::Ask);
    }

    #[test]
    fn a_session_is_started_with_its_prompt_after_every_option() {
        let g = gateway().0;
        let def = g.definitions().get("adroit").cloned().unwrap();
        let start = def
            .commands
            .iter()
            .find(|t| t.name == "start_session")
            .unwrap();
        let ctx = Context {
            args: json!({ "dir": "/src/api", "prompt": "--dir / fix it", "worktree": true, "agent": "codex" }),
            ..Default::default()
        };
        assert_eq!(
            arguments(start, &start.args, &ctx).unwrap(),
            [
                "session",
                "start",
                "--dir",
                "/src/api",
                "--worktree",
                "--agent",
                "codex",
                "--json",
                "--",
                "--dir / fix it"
            ]
        );
    }

    const ASK: &str = r#"
format = 1
id = "ask"
name = "Ask"

[account]
key = { label = "Key", secret = true, optional = true }
file = { label = "File" }

[transport.command]
program = "perl"

[[commands]]
name = "read"
description = "Read the question."
args = ["-e", "print qq({{\"question\":{{\"id\":\"q1\",\"text\":\"rm -rf?\"}}}})"]
read_only = true

[[commands]]
name = "answer"
description = "Answer it."
args = ["-e", "my $k = <STDIN>; open my $f, '>>', shift; print $f qq($k @ARGV\n)", "--", "{account.file}", "allow", "{args.question}", { flag = "--reply", each = "{args.replies}" }]
stdin = "{account.key}"

[commands.each_time]
reads = "read"
at = "/question"
same = { "/question/id" = "{args.question}" }
on_reject = ["-e", "open my $f, '>>', shift; print $f qq(deny @ARGV\n)", "--", "{account.file}", "{args.question}"]
"#;

    #[tokio::test]
    async fn a_call_asked_each_time_is_held_with_what_it_decides_and_made_once_as_answered() {
        use crate::permissions::Permission;
        use crate::{Answer, Call, Verdict};
        let file = std::env::temp_dir().join(format!("famulen-ask-{}", std::process::id()));
        std::fs::remove_file(&file).ok();
        let store = Store::open_in_memory().unwrap();
        let g = Gateway::new(
            store.clone(),
            Connections::new(store),
            reqwest::Client::new(),
            None,
        )
        .with_definitions(None, &[("ask.toml".into(), ASK.into())]);
        let values = [("key", "k-1"), ("file", file.to_str().unwrap())]
            .map(|(k, v)| (k.to_owned(), v.to_owned()));
        g.add_defined("ask", values.into(), None).unwrap();
        g.grant("secretary", "*", "*").unwrap();
        g.set_permission("owner", "conn:ask", "answer", Some(Permission::Allow))
            .unwrap();
        let call = |question: &str| {
            Call::new(
                "secretary",
                "ask",
                "answer",
                json!({ "question": question, "replies": ["a b", "c"] }),
            )
        };
        let hold = async |c: Call| match g.judge_assessed(&c).await.unwrap() {
            Verdict::Hold(ticket) => ticket,
            other => panic!("{other:?}"),
        };

        let first = hold(call("q1")).await;
        assert_eq!(hold(call("q1")).await.id, first.id, "asked once");
        assert_eq!(first.shown["decides"]["text"], "rm -rf?");
        assert_eq!(first.shown["call"]["question"], "q1");
        assert!(matches!(
            g.judge_assessed(&call("q2")).await.unwrap(),
            Verdict::Deny { .. }
        ));
        assert!(matches!(
            g.judge(&call("q1")).unwrap(),
            Verdict::Deny { .. }
        ));

        let answered = g
            .answer_ticket(first.id, Answer::AlwaysAllow)
            .await
            .unwrap()
            .unwrap();
        answered.ran.unwrap().unwrap();
        let second = hold(call("q1")).await;
        let answered = g
            .answer_ticket(second.id, Answer::Reject)
            .await
            .unwrap()
            .unwrap();
        answered.ran.unwrap().unwrap();

        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "k-1 allow q1 --reply a b --reply c\ndeny q1\n"
        );
        std::fs::remove_file(&file).ok();
    }

    #[tokio::test]
    async fn questions_are_answered_instead_of_approved() {
        use crate::{Answer, Call, Verdict};
        let file = std::env::temp_dir().join(format!("famulen-fill-{}", std::process::id()));
        std::fs::remove_file(&file).ok();
        let asks = ASK
            .replace(
                r#"\"text\":\"rm -rf?\""#,
                r#"\"qs\":[{{\"question\":\"which?\",\"options\":[{{\"label\":\"a\"}}]}}]"#,
            )
            .replace(
                "at = \"/question\"",
                "at = \"/question\"\nquestions = \"/qs\"\nfills = \"replies\"",
            );
        let store = Store::open_in_memory().unwrap();
        let g = Gateway::new(
            store.clone(),
            Connections::new(store),
            reqwest::Client::new(),
            None,
        )
        .with_definitions(None, &[("ask.toml".into(), asks)]);
        let values = [("key", "k-1"), ("file", file.to_str().unwrap())]
            .map(|(k, v)| (k.to_owned(), v.to_owned()));
        g.add_defined("ask", values.into(), None).unwrap();
        g.grant("secretary", "*", "*").unwrap();
        let hold = async || {
            let call = Call::new("secretary", "ask", "answer", json!({ "question": "q1" }));
            match g.judge_assessed(&call).await.unwrap() {
                Verdict::Hold(ticket) => ticket,
                other => panic!("{other:?}"),
            }
        };

        let ticket = hold().await;
        let asked = g.questions_of("ask", "answer", &ticket.shown).unwrap();
        assert_eq!(asked[0].options[0].label, "a");
        assert!(g.fill_ticket(ticket.id, vec![]).await.is_err());
        assert!(g.fill_ticket(ticket.id, vec![" ".into()]).await.is_err());
        let done = g.fill_ticket(ticket.id, vec!["b c".into()]).await.unwrap();
        done.unwrap().ran.unwrap().unwrap();
        assert!(
            g.fill_ticket(ticket.id, vec!["d".into()])
                .await
                .unwrap()
                .is_none()
        );

        let unasked = Call::new("owner", "ask", "answer", json!({ "question": "q1" }));
        let mut told = g.subscribe_held();
        let lacking = g.answer_unasked(&unasked, Answer::Approve, vec![]);
        assert!(lacking.await.is_err(), "its questions were not answered");
        assert_eq!(g.ticket(ticket.id + 1).unwrap().unwrap().status, "lapsed");
        g.answer_unasked(&unasked, Answer::Approve, vec!["e f".into()])
            .await
            .unwrap();
        let stale = Call::new("owner", "ask", "answer", json!({ "question": "q0" }));
        for (call, replies) in [(&stale, vec!["g".into()]), (&unasked, vec![" ".into()])] {
            assert!(
                g.answer_unasked(call, Answer::Approve, replies)
                    .await
                    .is_err()
            );
        }
        assert!(told.try_recv().is_err(), "nothing is asked of the owner");
        assert!(g.held_tickets().unwrap().is_empty());
        g.set_permission(
            "owner",
            "conn:ask",
            "answer",
            Some(crate::permissions::Permission::Forbid),
        )
        .unwrap();
        let forbidden = g.answer_unasked(&unasked, Answer::Approve, vec!["h".into()]);
        assert!(forbidden.await.is_err());
        g.set_permission("owner", "conn:ask", "answer", None)
            .unwrap();

        let approved = g
            .answer_ticket(hold().await.id, Answer::Approve)
            .await
            .unwrap();
        assert!(approved.unwrap().ran.unwrap().is_err(), "not answered");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "k-1 allow q1 --reply b c\nk-1 allow q1 --reply e f\n"
        );
        std::fs::remove_file(&file).ok();
    }
}
