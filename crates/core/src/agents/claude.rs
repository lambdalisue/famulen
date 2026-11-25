//! Claude Code, spoken to in stream-json on its stdin and stdout.

use std::collections::{BTreeMap, HashMap};
use std::process::Stdio;

use anyhow::Context as _;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use super::{
    Adapter, Answer, Ask, Body, By, Capabilities, Capability, Choice, Codec, Counted, Launch,
    Means, PlanStep, Probe, Question, Sandbox, Standing, When, Work,
};
use crate::claude::protocol::{self, ControlRequest, Inbound};
use crate::claude::session::sandbox_settings;
use crate::permissions::Caller;

const PROGRAM: &str = "claude";
/// The versions its lines are known of: from the first to before the second.
const SUPPORTED: ((u64, u64), (u64, u64)) = ((2, 1), (3, 0));
/// Kinds of line that tell nothing an event is made of.
const QUIET: &[&str] = &[
    "system",
    "control_response",
    "keep_alive",
    "prompt_suggestion",
    "tool_use_summary",
    "auth_status",
];

#[derive(Debug, Clone, Copy, Default)]
pub struct Claude;

impl Adapter for Claude {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            can: [
                Capability::AsksPermission,
                Capability::AsksQuestions,
                Capability::Steering,
                Capability::Interrupting,
                Capability::Resuming,
                Capability::Servers,
                Capability::Usage,
                Capability::Plan,
            ]
            .into(),
            sandbox: Sandbox::Shell,
        }
    }

    fn probe<'a>(&'a self, program: Option<&'a str>) -> BoxFuture<'a, anyhow::Result<Probe>> {
        Box::pin(async move {
            let program = program.unwrap_or(PROGRAM);
            let ask = |args: &'static [&'static str]| {
                let mut command = tokio::process::Command::new(program);
                crate::config::hide_secrets(&mut command);
                command.args(args).stdin(Stdio::null()).kill_on_drop(true);
                async move {
                    command
                        .output()
                        .await
                        .with_context(|| format!("could not run {program}"))
                }
            };
            let told = ask(&["--version"]).await?;
            anyhow::ensure!(told.status.success(), "{program} does not tell its version");
            let version = String::from_utf8_lossy(&told.stdout);
            let version = version
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            let status = ask(&["auth", "status"]).await?;
            let status: Option<Value> = serde_json::from_slice(&status.stdout).ok();
            Ok(Probe {
                supported: supported(&version),
                version,
                logged_in: status.and_then(|s| s["loggedIn"].as_bool()),
                capabilities: self.capabilities(),
            })
        })
    }

    fn launch(&self, work: &Work, resume: Option<&str>) -> anyhow::Result<Launch> {
        self.capabilities().check(&work.needs)?;
        Ok(Launch {
            program: work.program.clone().unwrap_or_else(|| PROGRAM.to_owned()),
            args: args(work, resume),
            // The connectors of the account it is logged in with would
            // otherwise join the tools, outside the gateway.
            env: BTreeMap::from([("ENABLE_CLAUDEAI_MCP_SERVERS".into(), "false".into())]),
            input: vec![
                protocol::initialize_request("famulen-init").to_string(),
                protocol::user_message(&work.message).to_string(),
            ],
        })
    }

    fn answer(&self, question: &Question, answer: &Answer) -> anyhow::Result<String> {
        let id = &question.id;
        let line = match (&question.ask, answer) {
            (Ask::Tool { input, .. }, Answer::Allow) => protocol::allow_tool(id, input),
            (Ask::Tool { .. }, Answer::Refuse { why }) => protocol::deny_tool(id, why),
            (
                Ask::Tool {
                    input,
                    means: Means::Choose { .. },
                    ..
                },
                Answer::Chosen { answers },
            ) => {
                let mut input = input.clone();
                input["answers"] = json!(answers);
                protocol::allow_tool(id, &input)
            }
            (Ask::Server { .. }, Answer::Served { response }) => {
                protocol::mcp_response(id, response.clone())
            }
            (Ask::Unknown { what }, _) => {
                protocol::control_error(id, &format!("unsupported: {what}"))
            }
            (_, Answer::Unsupported) => protocol::control_error(id, "unsupported"),
            (ask, answer) => anyhow::bail!("{answer:?} is no answer to {ask:?}"),
        };
        Ok(line.to_string())
    }

    fn codec(&self) -> Box<dyn Codec> {
        Box::<Lines>::default()
    }
}

fn supported(version: &str) -> bool {
    let mut numbers = version.split('.').map(|n| n.parse::<u64>().ok());
    let (Some(Some(major)), Some(Some(minor))) = (numbers.next(), numbers.next()) else {
        return false;
    };
    let (from, before) = SUPPORTED;
    (from..before).contains(&(major, minor))
}

/// The directory of the work is not given as one of its own, where it would
/// read without asking Famulen.
fn args(work: &Work, resume: Option<&str>) -> Vec<String> {
    let mut servers = serde_json::Map::new();
    for id in &work.servers {
        servers.insert(id.clone(), json!({ "type": "sdk", "name": id }));
    }
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
    ]
    .map(String::from)
    .into();
    if work.parts {
        args.push("--include-partial-messages".into());
    }
    let mut put = |flag: &str, value: String| args.extend([flag.to_owned(), value]);
    put("--mcp-config", json!({ "mcpServers": servers }).to_string());
    put("--tools", work.tools.join(","));
    let mut settings = sandbox_settings(&work.fence.sealed, &[]);
    let files = &mut settings["sandbox"]["filesystem"];
    if let Some(denied) = files["denyRead"].as_array_mut() {
        denied.extend(
            work.fence
                .unread
                .iter()
                .map(|p| p.display().to_string().into()),
        );
    }
    files["allowRead"] = json!(work.fence.readable);
    put("--settings", settings.to_string());
    put("--append-system-prompt", work.instructions.clone());
    put("--disallowedTools", work.withheld.join(","));
    if let Some(model) = &work.model {
        put("--model", model.clone());
    }
    if let Some(effort) = &work.effort {
        put("--effort", effort.clone());
    }
    if let Some(conversation) = resume {
        put("--resume", conversation.to_owned());
    }
    args
}

#[derive(Default)]
struct Lines {
    standing: Option<Standing>,
    interrupts: u64,
    /// The message being written, by the call of the subagent that writes it.
    writing: HashMap<String, String>,
    /// The tool call being written, by the subagent and the block's place.
    calling: HashMap<(String, u64), String>,
}

impl Codec for Lines {
    fn read(&mut self, line: &str) -> Vec<Body> {
        if line.trim().is_empty() {
            return Vec::new();
        }
        let mut bodies = Vec::new();
        match protocol::parse_line(line) {
            Ok(Inbound::Init { session_id, .. }) => {
                bodies.push(Body::Started {
                    conversation: session_id,
                });
                self.stand(Standing::Working, &mut bodies);
            }
            Ok(Inbound::Control {
                request_id,
                request,
            }) => {
                let question = question(request_id, request);
                match &question.ask {
                    Ask::Tool { .. } => self.stand(Standing::Waiting, &mut bodies),
                    Ask::Unknown { what } => bodies.push(Body::Warned {
                        what: format!("it asks what is not known here: {what}"),
                    }),
                    Ask::Server { .. } => {}
                }
                bodies.push(Body::Asked { question });
            }
            Ok(Inbound::Result {
                is_error,
                text,
                usage,
                ..
            }) => {
                bodies.push(Body::Used {
                    of: Counted::Turn,
                    model: None,
                    input: usage.input_tokens.unwrap_or(0),
                    cached: 0,
                    output: usage.output_tokens.unwrap_or(0),
                });
                bodies.push(Body::TurnEnded {
                    ok: !is_error,
                    text,
                });
                self.stand(Standing::Idle, &mut bodies);
            }
            Ok(Inbound::Other(line)) => self.other(&line, &mut bodies),
            Err(error) => bodies.push(Body::Warned {
                what: format!("a line that cannot be read: {error}"),
            }),
        }
        bodies
    }

    fn send(&mut self, text: &str, when: When) -> anyhow::Result<String> {
        let mut message = protocol::user_message(text);
        // Without a priority it joins the turn at the next chance.
        if when == When::Between {
            message["priority"] = json!("later");
        }
        Ok(message.to_string())
    }

    fn interrupt(&mut self) -> anyhow::Result<String> {
        self.interrupts += 1;
        Ok(json!({
            "type": "control_request",
            "request_id": format!("famulen-interrupt-{}", self.interrupts),
            "request": { "subtype": "interrupt" },
        })
        .to_string())
    }
}

impl Lines {
    fn stand(&mut self, standing: Standing, bodies: &mut Vec<Body>) {
        if self.standing != Some(standing) {
            self.standing = Some(standing);
            bodies.push(Body::Stands { standing });
        }
    }

    fn other(&mut self, line: &Value, bodies: &mut Vec<Body>) {
        match line["type"].as_str().unwrap_or_default() {
            kind @ ("assistant" | "user") => {
                self.stand(Standing::Working, bodies);
                message(kind, line, bodies);
            }
            "stream_event" => self.part(line, bodies),
            "tool_progress" => bodies.push(Body::ToolWorks {
                call: text(line, "tool_use_id"),
                output: String::new(),
                seconds: line["elapsed_time_seconds"].as_f64(),
            }),
            "control_cancel_request" => bodies.push(Body::Withdrawn {
                question: text(line, "request_id"),
            }),
            "rate_limit_event" => {
                let status = text(&line["rate_limit_info"], "status");
                if status != "allowed" {
                    bodies.push(Body::Warned {
                        what: format!("what its account may use: {status}"),
                    });
                }
            }
            kind if QUIET.contains(&kind) => {}
            kind => bodies.push(Body::Warned {
                what: format!("a kind of line that is not known here: {kind:?}"),
            }),
        }
    }

    fn part(&mut self, line: &Value, bodies: &mut Vec<Body>) {
        let by = text(line, "parent_tool_use_id");
        let event = &line["event"];
        let place = event["index"].as_u64().unwrap_or_default();
        match event["type"].as_str().unwrap_or_default() {
            "message_start" => {
                self.writing.insert(by, text(&event["message"], "id"));
            }
            "content_block_start" if event["content_block"]["type"] == "tool_use" => {
                self.calling
                    .insert((by, place), text(&event["content_block"], "id"));
            }
            "content_block_delta" => {
                let delta = &event["delta"];
                let message = || self.writing.get(&by).cloned().unwrap_or_default();
                match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => bodies.push(Body::SaidMore {
                        message: message(),
                        text: text(delta, "text"),
                    }),
                    "thinking_delta" => bodies.push(Body::ThoughtMore {
                        message: message(),
                        text: text(delta, "thinking"),
                    }),
                    "input_json_delta" => bodies.push(Body::ToolCalledMore {
                        call: self
                            .calling
                            .get(&(by.clone(), place))
                            .cloned()
                            .unwrap_or_default(),
                        input: text(delta, "partial_json"),
                    }),
                    _ => {}
                }
            }
            "message_stop" => {
                self.writing.remove(&by);
                self.calling.retain(|(of, _), _| *of != by);
            }
            _ => {}
        }
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

fn each<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value[key].as_array().into_iter().flatten()
}

fn question(id: String, request: ControlRequest) -> Question {
    let ask = match request {
        ControlRequest::CanUseTool {
            caller,
            tool_name,
            input,
        } => Ask::Tool {
            by: match caller {
                Caller::Secretary => By::Itself,
                Caller::Subagent => By::Subagent,
            },
            means: means(&tool_name, &input),
            tool: tool_name,
            input,
        },
        ControlRequest::McpMessage {
            server_name,
            message,
        } => Ask::Server {
            server: server_name,
            message,
        },
        ControlRequest::Unknown { subtype } => Ask::Unknown { what: subtype },
    };
    Question { id, ask }
}

fn means(tool: &str, input: &Value) -> Means {
    match tool {
        "AskUserQuestion" => Means::Choose {
            choices: each(input, "questions")
                .map(|q| Choice {
                    text: text(q, "question"),
                    options: each(q, "options").map(|o| text(o, "label")).collect(),
                    several: q["multiSelect"] == true,
                })
                .collect(),
        },
        "ExitPlanMode" => Means::Plan {
            text: text(input, "plan"),
        },
        _ => Means::Use,
    }
}

fn message(kind: &str, line: &Value, bodies: &mut Vec<Body>) {
    let message = &line["message"];
    let id = text(message, "id");
    for block in each(message, "content") {
        match block["type"].as_str().unwrap_or_default() {
            "text" if kind == "assistant" => bodies.push(Body::Said {
                message: id.clone(),
                text: text(block, "text"),
            }),
            "thinking" => bodies.push(Body::Thought {
                message: id.clone(),
                text: text(block, "thinking"),
            }),
            "tool_use" if block["name"] == "TodoWrite" => bodies.push(Body::Planned {
                steps: steps(&block["input"]),
            }),
            "tool_use" => bodies.push(Body::ToolCalled {
                call: text(block, "id"),
                tool: text(block, "name"),
                input: block["input"].clone(),
            }),
            "tool_result" => {
                let call = text(block, "tool_use_id");
                bodies.extend(changed(&call, &line["tool_use_result"]));
                bodies.push(Body::ToolAnswered {
                    call,
                    ok: block["is_error"] != true,
                    output: output(&block["content"]),
                });
            }
            _ => {}
        }
    }
    if kind == "assistant" && !id.is_empty() {
        let usage = protocol::turn_usage(message);
        bodies.push(Body::Used {
            of: Counted::Message(id),
            model: message["model"].as_str().map(str::to_owned),
            input: usage.input_tokens.unwrap_or(0),
            cached: message
                .pointer("/usage/cache_read_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            output: usage.output_tokens.unwrap_or(0),
        });
    }
}

fn output(content: &Value) -> String {
    match content {
        Value::String(answer) => answer.clone(),
        parts => parts
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// What a call of a file tool changed.
fn changed(call: &str, result: &Value) -> Option<Body> {
    let path = result["filePath"].as_str()?;
    let mut diff = String::new();
    for hunk in result["structuredPatch"].as_array()? {
        let at = |key: &str| hunk[key].as_u64().unwrap_or_default();
        diff.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            at("oldStart"),
            at("oldLines"),
            at("newStart"),
            at("newLines")
        ));
        for line in each(hunk, "lines").filter_map(Value::as_str) {
            diff.push_str(line);
            diff.push('\n');
        }
    }
    // A file made anew has no patch.
    if diff.is_empty() {
        let made = result["content"].as_str()?;
        diff = format!("@@ -0,0 +1,{} @@\n", made.lines().count());
        for line in made.lines() {
            diff.push_str(&format!("+{line}\n"));
        }
    }
    Some(Body::FileChanged {
        call: call.to_owned(),
        path: path.to_owned(),
        diff,
    })
}

fn steps(input: &Value) -> Vec<PlanStep> {
    each(input, "todos")
        .enumerate()
        .map(|(at, t)| PlanStep {
            id: at.to_string(),
            title: text(t, "content"),
            status: text(t, "status"),
            weight: 1.0,
            ..PlanStep::default()
        })
        .collect()
}

#[cfg(test)]
mod tests;
