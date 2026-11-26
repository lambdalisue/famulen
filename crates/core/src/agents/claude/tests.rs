use std::path::PathBuf;

use super::*;
use crate::agents::conformance::{self, Recorded};
use crate::agents::{Answer, Fence};

fn read(line: Value) -> Vec<Body> {
    Lines::default().read(&line.to_string())
}

#[test]
fn it_keeps_to_what_every_adapter_does() {
    conformance::check(
        &Claude,
        &Recorded {
            starts: r#"{"type":"system","subtype":"init","session_id":"s1","tools":["Read"]}"#,
            says: r#"{"type":"assistant","message":{"id":"m1","model":"opus","content":[{"type":"text","text":"読む。"}],"usage":{"input_tokens":1,"output_tokens":2}},"parent_tool_use_id":null}"#,
            calls: r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/a"}}]}}"#,
            answers: r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"中身"}]}}"#,
            asks: r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Read","input":{"file_path":"/a"}}}"#,
            asks_unknown: r#"{"type":"control_request","request_id":"r2","request":{"subtype":"may_fly"}}"#,
            ends_turn: r#"{"type":"result","subtype":"success","is_error":false,"result":"済み","session_id":"s1","usage":{"input_tokens":3,"output_tokens":4}}"#,
            unknown: r#"{"type":"of_tomorrow"}"#,
        },
    );
}

fn work() -> Work {
    Work {
        model: Some("opus".into()),
        tools: vec!["Read".into()],
        withheld: vec!["Task".into()],
        servers: vec!["mail".into()],
        fence: Fence {
            sealed: vec![PathBuf::from("/data/notebook")],
            unread: vec![PathBuf::from("/runs")],
            readable: vec![PathBuf::from("/w")],
            hosts: vec!["127.0.0.1".into()],
            ..Fence::default()
        },
        ..Work::default()
    }
}

#[test]
fn it_is_started_with_nothing_but_what_it_was_handed() {
    let launch = Claude.launch(&work(), None).unwrap();
    let args = &launch.args;
    let after = |flag: &str| {
        let at = args.iter().position(|a| a == flag).unwrap();
        args[at + 1].clone()
    };
    assert_eq!(after("--tools"), "Read");
    assert_eq!(after("--setting-sources"), "");
    assert_eq!(after("--permission-mode"), "manual");
    assert_eq!(after("--model"), "opus");
    assert!(args.contains(&"--strict-mcp-config".to_owned()));
    let servers: Value = serde_json::from_str(&after("--mcp-config")).unwrap();
    assert_eq!(
        servers["mcpServers"],
        json!({ "mail": { "type": "sdk", "name": "mail" } })
    );
    let settings: Value = serde_json::from_str(&after("--settings")).unwrap();
    let files = &settings["sandbox"]["filesystem"];
    assert_eq!(files["denyRead"], json!(["/data/notebook", "/runs"]));
    assert_eq!(files["allowRead"], json!(["/w"]));
    let network = &settings["sandbox"]["network"];
    assert_eq!(network["deniedDomains"], json!(["127.0.0.1"]));
    assert_eq!(network["allowUnixSockets"], json!([]));
    for open in ["allowLocalBinding", "allowAllUnixSockets"] {
        assert_eq!(network[open], false, "{open}");
    }
    assert!(after("--disallowedTools").contains("Task"));
    assert_eq!(files["allowWrite"], json!([]));
    assert_eq!(launch.env["ENABLE_CLAUDEAI_MCP_SERVERS"], "false");
    assert_eq!(launch.program, "claude");
    for flag in ["--resume", "--include-partial-messages"] {
        assert!(!args.contains(&flag.to_owned()), "{flag}");
    }
}

#[test]
fn a_conversation_is_gone_on_with_and_told_in_parts_only_when_asked() {
    let work = Work {
        parts: true,
        ..work()
    };
    let args = Claude.launch(&work, Some("s1")).unwrap().args;
    assert!(args.windows(2).any(|w| w == ["--resume", "s1"]));
    assert!(args.contains(&"--include-partial-messages".to_owned()));
}

#[test]
fn a_new_conversation_takes_the_id_chosen_for_it_and_one_gone_on_with_keeps_its_own() {
    let work = Work {
        conversation: Some("c1".into()),
        ..work()
    };
    let new = Claude.launch(&work, None).unwrap().args;
    assert!(new.windows(2).any(|w| w == ["--session-id", "c1"]));
    let resumed = Claude.launch(&work, Some("c1")).unwrap().args;
    assert!(resumed.windows(2).any(|w| w == ["--resume", "c1"]));
    assert!(!resumed.contains(&"--session-id".to_owned()));
}

#[test]
fn what_it_writes_beside_its_directory_is_named_and_what_inside_that_it_may_not() {
    let work = Work {
        fence: Fence {
            writable: vec![PathBuf::from("/src/.jj"), PathBuf::from("/data/notebook/x")],
            unwritable: vec![PathBuf::from("/src/.jj/repo/config.toml")],
            ..work().fence
        },
        ..work()
    };
    let args = Claude.launch(&work, None).unwrap().args;
    let at = args.iter().position(|a| a == "--settings").unwrap();
    let settings: Value = serde_json::from_str(&args[at + 1]).unwrap();
    let files = &settings["sandbox"]["filesystem"];
    assert_eq!(
        files["allowWrite"],
        json!(["/src/.jj"]),
        "never what is sealed"
    );
    assert_eq!(
        files["denyWrite"],
        json!(["/data/notebook", "/src/.jj/repo/config.toml"])
    );
}

#[test]
fn a_conversation_is_found_where_claude_code_keeps_it_and_nothing_else_is() {
    let dir = crate::notebook::testing::TempDir::new("claude-find");
    let project = dir.path().join("projects/-src-a");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("c1.jsonl"), "{}\n").unwrap();
    let kept = |id: &str| conversation_in(dir.path(), id);
    assert!(kept("c1"));
    for not in ["c2", "", "../projects/-src-a/c1", "*"] {
        assert!(!kept(not), "{not}");
    }
}

#[test]
fn what_is_being_written_is_told_in_parts() {
    let mut lines = Lines::default();
    let mut part = |event: Value| {
        lines.read(
            &json!({ "type": "stream_event", "event": event, "parent_tool_use_id": null })
                .to_string(),
        )
    };
    part(json!({ "type": "message_start", "message": { "id": "m1" } }));
    assert_eq!(
        part(
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "読" } })
        ),
        [Body::SaidMore {
            message: "m1".into(),
            text: "読".into()
        }]
    );
    part(
        json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "tool_use", "id": "t1" } }),
    );
    assert_eq!(
        part(
            json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{\"a" } })
        ),
        [Body::ToolCalledMore {
            call: "t1".into(),
            input: "{\"a".into()
        }]
    );
}

#[test]
fn a_changed_file_is_told_with_its_diff() {
    let answered = |result: Value| {
        read(json!({
            "type": "user",
            "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "ok" }] },
            "tool_use_result": result,
        }))
    };
    let diff = |bodies: &[Body]| {
        bodies.iter().find_map(|b| match b {
            Body::FileChanged { path, diff, .. } => Some((path.clone(), diff.clone())),
            _ => None,
        })
    };
    assert_eq!(
        diff(&answered(json!({ "filePath": "/a", "structuredPatch": [
            { "oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 1, "lines": ["-x", "+y"] }
        ] }))),
        Some(("/a".into(), "@@ -1,1 +1,1 @@\n-x\n+y\n".into()))
    );
    assert_eq!(
        diff(&answered(
            json!({ "filePath": "/b", "content": "x\ny\n", "structuredPatch": [] })
        )),
        Some(("/b".into(), "@@ -0,0 +1,2 @@\n+x\n+y\n".into()))
    );
    assert_eq!(diff(&answered(json!({ "stdout": "x" }))), None);
}

#[test]
fn a_question_to_its_user_is_answered_with_what_was_chosen() {
    let asked = read(json!({
        "type": "control_request", "request_id": "r1",
        "request": { "subtype": "can_use_tool", "tool_name": "AskUserQuestion", "input": { "questions": [
            { "question": "どれ？", "options": [{ "label": "A" }, { "label": "B" }], "multiSelect": false }
        ] } },
    }));
    let question = asked
        .into_iter()
        .find_map(|b| match b {
            Body::Asked { question } => Some(question),
            _ => None,
        })
        .unwrap();
    let Ask::Tool {
        means: Means::Choose { choices },
        ..
    } = &question.ask
    else {
        panic!("{question:?}");
    };
    assert_eq!(choices[0].options, ["A", "B"]);
    let line = Claude
        .answer(
            &question,
            &Answer::Chosen {
                answers: BTreeMap::from([("どれ？".into(), "B".into())]),
            },
        )
        .unwrap();
    let line: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        line.pointer("/response/response/updatedInput/answers/どれ？"),
        Some(&json!("B"))
    );
}

#[test]
fn it_waits_while_a_tool_of_its_own_is_asked_about_and_goes_on_after() {
    let mut lines = Lines::default();
    let mut standings = |line: Value| {
        lines
            .read(&line.to_string())
            .into_iter()
            .filter_map(|b| match b {
                Body::Stands { standing } => Some(standing),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let asks = json!({ "type": "control_request", "request_id": "r", "request": { "subtype": "can_use_tool", "tool_name": "Read", "input": {} } });
    let goes_on = json!({ "type": "user", "message": { "content": [] } });
    let serves = json!({ "type": "control_request", "request_id": "r", "request": { "subtype": "mcp_message", "server_name": "a", "message": {} } });
    assert_eq!(standings(serves), []);
    assert_eq!(standings(asks), [Standing::Waiting]);
    assert_eq!(standings(goes_on.clone()), [Standing::Working]);
    assert_eq!(standings(goes_on), []);
}

#[test]
fn versions_it_was_not_made_for_are_told_apart() {
    for (version, is) in [
        ("2.1.331", true),
        ("2.9.0", true),
        ("2.0.9", false),
        ("3.0.0", false),
        ("", false),
    ] {
        assert_eq!(supported(version), is, "{version}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn it_is_asked_of_its_version_and_login_alone() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = crate::notebook::testing::TempDir::new("claude-probe");
    let program = dir.path().join("claude");
    std::fs::write(
        &program,
        "#!/bin/sh\ncase \"$1\" in --version) echo '2.1.331 (Claude Code)';; auth) echo '{\"loggedIn\": false}'; exit 1;; *) exit 2;; esac\n",
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let probe = Claude.probe(program.to_str()).await.unwrap();
    assert_eq!(probe.version, "2.1.331");
    assert!(probe.supported);
    assert_eq!(probe.logged_in, Some(false));
    assert!(Claude.probe(Some("/nowhere/claude")).await.is_err());
}

/// What a real Claude Code lets a shell command reach, told by the command
/// itself. The model is made up here and asks for that one command, so no
/// login is needed.
#[cfg(unix)]
#[tokio::test]
#[ignore = "needs claude and its sandbox on this machine"]
async fn a_shell_command_reaches_nothing_the_fence_keeps() {
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

    let dir = crate::notebook::testing::TempDir::new("claude-fence");
    // Where none of a system's own temporary places is.
    let home = crate::sealed::home()
        .unwrap()
        .join(format!(".famulen-fence-{}", std::process::id()));
    let (workdir, notebook, runs) = (home.join("w"), home.join("notebook"), home.join("runs"));
    for made in [&workdir, &notebook, &runs] {
        std::fs::create_dir_all(made).unwrap();
        std::fs::write(made.join("a.md"), "written").unwrap();
    }
    let api = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = api.local_addr().unwrap().port();
    let socket = home.join("s");
    let _unix = UnixListener::bind(&socket).unwrap();
    let probe = format!(
        r#"import socket, urllib.request
def told(name, do):
    try: print(name, (do(), "reached")[1])
    except Exception as e: print(name, "kept", type(e).__name__)
def tcp(): socket.create_connection(("127.0.0.1", {port}), 3)
def unix():
    s = socket.socket(socket.AF_UNIX); s.connect("{socket}")
def http(): urllib.request.urlopen("http://localhost:{port}/", timeout=3)
told("own", lambda: open("{w}/a.md").read())
told("notebook", lambda: open("{n}/a.md").read())
told("write", lambda: open("{n}/b.md", "w"))
told("tcp", tcp)
told("http", http)
told("unix", unix)
told("bind", lambda: socket.create_server(("127.0.0.1", 0)))
"#,
        socket = socket.display(),
        w = workdir.display(),
        n = notebook.display(),
    );
    std::fs::write(workdir.join("probe.py"), probe).unwrap();

    let heard: Arc<Mutex<Vec<Value>>> = Arc::default();
    let model = {
        let heard = heard.clone();
        move |body: String| {
            let asked: Value = serde_json::from_str(&body).unwrap_or_default();
            let answered = body.contains("tool_result");
            heard.lock().unwrap().push(asked);
            let command = json!({ "command": "python3 probe.py", "description": "probe" });
            let (block, delta, stop) = if answered {
                (
                    json!({ "type": "text", "text": "" }),
                    json!({ "type": "text_delta", "text": "done" }),
                    "end_turn",
                )
            } else {
                (
                    json!({ "type": "tool_use", "id": "t1", "name": "Bash", "input": {} }),
                    json!({ "type": "input_json_delta", "partial_json": command.to_string() }),
                    "tool_use",
                )
            };
            let events = [
                json!({ "type": "message_start", "message": { "id": "m", "type": "message", "role": "assistant", "model": "opus", "content": [], "stop_reason": null, "usage": { "input_tokens": 1, "output_tokens": 1 } } }),
                json!({ "type": "content_block_start", "index": 0, "content_block": block }),
                json!({ "type": "content_block_delta", "index": 0, "delta": delta }),
                json!({ "type": "content_block_stop", "index": 0 }),
                json!({ "type": "message_delta", "delta": { "stop_reason": stop, "stop_sequence": null }, "usage": { "output_tokens": 1 } }),
                json!({ "type": "message_stop" }),
            ];
            let text: String = events
                .iter()
                .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap()))
                .collect();
            async move { ([("content-type", "text/event-stream")], text) }
        }
    };
    let app = axum::Router::new().fallback(model);
    let at = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", at.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(at, app).await });

    let work = Work {
        message: "probe".into(),
        tools: vec!["Bash".into()],
        workdir: workdir.clone(),
        fence: Fence {
            sealed: vec![notebook.clone()],
            unread: vec![runs],
            readable: vec![workdir.clone()],
            hosts: vec!["127.0.0.1".into(), "localhost".into()],
            ..Fence::default()
        },
        ..Work::default()
    };
    let launch = Claude.launch(&work, None).unwrap();
    let mut child = tokio::process::Command::new(&launch.program)
        .args(&launch.args)
        .envs(&launch.env)
        .env("ANTHROPIC_BASE_URL", base)
        .env("ANTHROPIC_AUTH_TOKEN", "made-up")
        .env("CLAUDE_CONFIG_DIR", dir.path())
        .current_dir(&workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in &launch.input {
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut codec = Claude.codec();
    let told = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let mut told = String::new();
        while let Some(line) = lines.next_line().await.unwrap() {
            for body in codec.read(&line) {
                match body {
                    Body::Asked { question } => {
                        let answer = match &question.ask {
                            // As the worker's bounds would: nothing but it.
                            Ask::Tool { tool, .. } if tool == "Bash" => Answer::Allow,
                            Ask::Tool { .. } => Answer::Refuse { why: "no".into() },
                            Ask::Server { .. } | Ask::Unknown { .. } => continue,
                        };
                        let line = Claude.answer(&question, &answer).unwrap();
                        stdin
                            .write_all(format!("{line}\n").as_bytes())
                            .await
                            .unwrap();
                    }
                    Body::ToolAnswered { output, .. } => told.push_str(&output),
                    Body::TurnEnded { .. } => return told,
                    _ => {}
                }
            }
        }
        told
    })
    .await;
    let written = notebook.join("b.md").exists();
    std::fs::remove_dir_all(&home).ok();
    assert!(!written, "what it wrote is nowhere");
    let told = told.expect("an end in time");
    assert!(told.contains("own reached"), "{told}");
    for kept in ["notebook", "tcp", "http", "unix"] {
        assert!(told.contains(&format!("{kept} kept")), "{kept}: {told}");
    }
}
