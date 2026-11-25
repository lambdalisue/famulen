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
