//! Which answers of tools hold something in confidence, and of what kind.
//! Facts about work, what anyone can read, and whether the owner is free do
//! not count: the owner is not to be asked of everything.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::taint::{Kind, Taint};
use crate::connections::{Connection, Transport};
use crate::slack::clients::SlackClients;
use crate::slack::web::{Room, Slack};

/// Famulen's own tools that tell nothing in confidence, however they answer.
/// A tool not named here, nor read more closely below, counts as private, so
/// that a new one does not slip through.
pub const PLAIN: &[&str] = &[
    "add_calendar",
    "add_keyless_connection",
    "add_lookout",
    "add_workgroup",
    "cancel_delegated",
    "cancel_wake",
    "check_notebook",
    "close_working",
    "cron_from",
    "delegate_task",
    "fix_notebook",
    "get_connection",
    "get_definition_text",
    "get_kind",
    "get_lookout_choices",
    "get_models",
    "get_status",
    "get_usage",
    "get_worker_settings",
    "get_workgroup",
    "give_feedback",
    "list_builtin_tools",
    "list_calendars",
    "list_channels",
    "list_connections",
    "list_definitions",
    "list_delegated",
    "list_events",
    "list_kinds",
    "list_known",
    "list_layers",
    "list_routines",
    "list_slack_names",
    "list_workgroups",
    "list_workspaces",
    "post_message",
    "refresh_calendars",
    "refresh_tools",
    "remember",
    "remove_calendar",
    "retry_delegated",
    "send_slack",
    "set_calendar_mode",
    "set_working",
    "skip_run",
    "update_workgroup",
    "wake_at",
];

/// Tools that have to do with the vault.
const VAULT: &[&str] = &[
    "list_secrets",
    "describe_secret",
    "set_secret_value",
    "delete_secret",
];

/// Kinds of record that tell of work, not of people or of the owner's own.
const WORK: &[&str] = &["tasks", "routines", "events", "calendars"];

const PEOPLE: &str = "人に関わる記憶";
const FILES: &str = "手帳のファイル";

/// Marks in what the notebook says that it is about a party.
fn tells_of_people(text: &str) -> bool {
    [
        "records/parties/",
        "[[party-",
        "\"party\":\"",
        "\"party\": \"",
        "party: ",
    ]
    .iter()
    .any(|mark| text.contains(mark))
}

fn people() -> Taint {
    Taint::new(Kind::People, PEOPLE)
}

fn belongings(label: impl Into<String>) -> Taint {
    Taint::new(Kind::Belongings, label)
}

fn private(label: impl Into<String>) -> Taint {
    Taint::new(Kind::Private, label)
}

/// What a record of `kind` holds. `name` tells which; empty is several.
fn of_kind(kind: &str, name: &str, text: &str) -> Option<Taint> {
    let which = |what: &str| match name {
        "" => format!("{what}の一覧"),
        name => format!("{what}「{name}」"),
    };
    match kind {
        "parties" => Some(Taint::new(Kind::People, which("相手の記録"))),
        "holdings" => Some(belongings(which("物の記録"))),
        "contracts" => Some(belongings(which("契約の記録"))),
        "memos" => tells_of_people(text).then(people),
        kind if WORK.contains(&kind) => None,
        kind => Some(belongings(which(&format!("「{kind}」の記録")))),
    }
}

fn of_record(memory_dir: &Path, record: &Value) -> Vec<Taint> {
    let field = |key: &str| record[key].as_str().unwrap_or_default();
    let name = match field("code") {
        "" => field("id").to_owned(),
        code => format!("{}（{code}）", field("title")),
    };
    let memories = record["memories"].as_array().into_iter().flatten();
    of_kind(field("kind"), &name, &record.to_string())
        .into_iter()
        .chain(memories.flat_map(|m| of_path(memory_dir, m["file"].as_str().unwrap_or_default())))
        .collect()
}

/// What an own tool's answer holds in confidence.
pub(crate) fn of_own(memory_dir: &Path, tool: &str, arguments: &Value, answer: &str) -> Vec<Taint> {
    let read: Option<Value> = serde_json::from_str(answer).ok();
    let all = || vec![people(), belongings(FILES)];
    let taints = match tool {
        "search_notebook" => match read.as_ref().map(|r| r["hits"].as_array()) {
            Some(hits) => hits
                .into_iter()
                .flatten()
                .flat_map(|hit| of_hit(memory_dir, hit))
                .collect(),
            None if answer == "done" => Vec::new(),
            None => all(),
        },
        "read_file" => of_path(memory_dir, arguments["path"].as_str().unwrap_or_default()),
        "list_records" => {
            let records = read.as_ref().and_then(|r| r["records"].as_array());
            let empty = records.is_none_or(Vec::is_empty);
            let kind = arguments["kind"].as_str().unwrap_or_default();
            match (empty, kind) {
                (true, _) if answer == "done" => Vec::new(),
                (_, "") => all(),
                (_, kind) => of_kind(kind, "", answer).into_iter().collect(),
            }
        }
        "get_record" | "save_record" | "clear_obsolete" | "link_account" => match &read {
            Some(record) => of_record(memory_dir, record),
            None if answer == "done" => Vec::new(),
            None => all(),
        },
        "delete_record" => Vec::new(),
        "get_layer" => tells_of_people(answer).then(people).into_iter().collect(),
        "list_party_memories" => vec![people()],
        "call_api" => vec![Taint::of_connection(
            arguments["connection"].as_str().unwrap_or_default(),
            "のやりとり",
        )],
        "list_messages" => vec![private("秘書との会話の履歴")],
        tool if VAULT.contains(&tool) => vec![Taint::new(Kind::Vault, "金庫の一覧")],
        tool if PLAIN.contains(&tool) => Vec::new(),
        _ if answer == "done" => Vec::new(),
        tool => vec![private(format!("Famulen の控え（{tool}）"))],
    };
    taints.into_iter().map(|t| t.told_by(answer)).collect()
}

fn of_hit(memory_dir: &Path, hit: &Value) -> Vec<Taint> {
    let field = |key: &str| hit[key].as_str().unwrap_or_default();
    if let Some(kind) = field("scope").strip_prefix("records/") {
        let text = std::fs::read_to_string(root(memory_dir).join(field("path")))
            .unwrap_or_else(|_| hit.to_string());
        return of_kind(kind, field("title"), &text).into_iter().collect();
    }
    let path = field("path");
    // Such a file holds many things, and only the one found was read.
    let many = path.starts_with("memory/episodic/") || path.starts_with("memory/forgotten/");
    let found = match std::fs::read_to_string(root(memory_dir).join(path)) {
        Ok(text) if many && !field("id").is_empty() => text
            .lines()
            .filter(|line| line.contains(field("id")))
            .collect::<Vec<_>>()
            .join("\n"),
        Ok(text) => text,
        Err(_) => String::new(),
    };
    tells_of_people(&format!("{found}\n{hit}"))
        .then(people)
        .into_iter()
        .collect()
}

/// The notebook's directory.
fn root(memory_dir: &Path) -> &Path {
    memory_dir.parent().unwrap_or(memory_dir)
}

/// What a file or directory of the notebook holds, by its path: whole, or
/// from the notebook or its memory directory.
fn of_path(memory_dir: &Path, path: &str) -> Vec<Taint> {
    let root = root(memory_dir);
    let given = crate::paths::plain(PathBuf::from(path));
    let real = |path: &Path| path.canonicalize().ok();
    let found = [root.join(&given), memory_dir.join(&given)]
        .iter()
        .find_map(|p| real(p));
    let Some(found) = found else {
        // Not there under this name, so it is not told what was read.
        return vec![people(), belongings(FILES)];
    };
    if real(root).is_some_and(|root| root.starts_with(&found)) {
        return vec![people(), belongings(FILES)];
    }
    let Some(inside) =
        real(root).and_then(|root| found.strip_prefix(root).ok().map(Path::to_owned))
    else {
        return Vec::new();
    };
    let parts: Vec<_> = inside
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    match parts.as_slice() {
        ["records", kind, ..] if found.is_dir() => {
            of_kind(kind, "", "records/parties/").into_iter().collect()
        }
        ["records", kind, .., file] => {
            let text = std::fs::read_to_string(&found).unwrap_or_default();
            of_kind(kind, file, &text).into_iter().collect()
        }
        ["memory", ..] if found.is_file() => std::fs::read_to_string(&found)
            .map_or(true, |text| tells_of_people(&text))
            .then(people)
            .into_iter()
            .collect(),
        ["memory", ..] => vec![people()],
        [file] if found.is_file() && *file == "README.md" => Vec::new(),
        _ => vec![people(), belongings(FILES)],
    }
}

/// Where `path` leads once its links and `..` are followed, as far as it is
/// there.
fn real(path: &Path) -> PathBuf {
    path.ancestors()
        .find_map(|there| {
            let rest = path.strip_prefix(there).ok()?;
            Some(there.canonicalize().ok()?.join(rest))
        })
        .unwrap_or_else(|| path.to_owned())
}

/// What files of the notebook handed to a worker hold.
pub(crate) fn of_files<'a>(memory_dir: &Path, files: impl Iterator<Item = &'a str>) -> Vec<Taint> {
    files
        .flat_map(|file| {
            let taints = of_path(memory_dir, file);
            taints.into_iter().map(move |t| t.told_by(file))
        })
        .collect()
}

/// What a built-in tool of the model CLI reads. Most read without asking
/// Famulen, so it is told by the call itself.
pub(crate) fn of_builtin(
    memory_dir: &Path,
    workdir: &Path,
    tool: &str,
    input: &Value,
) -> Vec<Taint> {
    let path = ["file_path", "path", "notebook_path"]
        .iter()
        .find_map(|key| input[*key].as_str());
    match tool {
        "Bash" | "BashOutput" | "PowerShell" | "TaskOutput" | "Monitor" => {
            vec![private("シェルの出力").told_by(&input.to_string())]
        }
        "Read" | "Grep" | "Glob" | "LS" | "NotebookRead" => {
            let path = path.map_or_else(|| workdir.to_owned(), |p| workdir.join(p));
            let root = real(root(memory_dir));
            let inside = |p: &Path| {
                let p = real(p);
                p.starts_with(&root) || root.starts_with(&p)
            };
            // What a pattern spells out leads further, or elsewhere.
            let spelt = input["pattern"]
                .as_str()
                .filter(|_| tool == "Glob")
                .map(|p| {
                    let end = p.find(['*', '?', '[', '{']).unwrap_or(p.len());
                    path.join(&p[..end])
                });
            if inside(&path) {
                let named = path.to_string_lossy();
                return of_path(memory_dir, &named)
                    .into_iter()
                    .map(|t| t.told_by(&format!("{named} {}", input["pattern"])))
                    .collect();
            }
            if spelt.is_some_and(|p| inside(&p)) {
                return vec![people(), belongings(FILES)];
            }
            // Claude Code puts long answers of tools and whole conversations here.
            let kept = path.to_string_lossy().replace('\\', "/");
            let kept = kept.contains("/.claude/projects") || kept.contains("/tool-results/");
            kept.then(|| private("会話の控え")).into_iter().collect()
        }
        _ => Vec::new(),
    }
}

/// The words of a tool's name, however they are put together.
pub(super) fn words(tool: &str) -> impl Iterator<Item = String> {
    let mut spaced = String::new();
    let mut lower = false;
    for c in tool.chars() {
        if c.is_ascii_uppercase() && lower {
            spaced.push(' ');
        }
        lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        spaced.push(if c.is_ascii_alphanumeric() {
            c.to_ascii_lowercase()
        } else {
            ' '
        });
    }
    spaced
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .into_iter()
}

pub(super) fn is_slack(connection: &Connection) -> bool {
    connection.kind == "slack"
        || matches!(&connection.transport, Transport::Defined { definition, .. } if definition == "slack")
}

/// Words in the name of a tool that reads what people wrote to each other.
const TALK: &[&str] = &[
    "message",
    "messages",
    "mail",
    "email",
    "thread",
    "threads",
    "dm",
    "dms",
    "chat",
    "chats",
    "conversation",
    "conversations",
    "emails",
    "mails",
    "history",
    "slack",
    "inbox",
    "draft",
    "drafts",
    "attachment",
    "attachments",
];

/// Tools of Slack that tell only who and what there is.
const SLACK_PLAIN: &[&str] = &[
    "list_channels",
    "channel_info",
    "user_info",
    "get_permalink",
    "open_dm",
    "post_message",
    "post_ephemeral",
    "update_message",
    "set_status",
    "add_reaction",
    "set_app_icon",
];

/// What a connection's answer to a message holds in confidence.
pub(crate) async fn of_connection<S: Slack>(
    clients: Option<&SlackClients<S>>,
    connection: &Connection,
    message: &Value,
    response: &Value,
) -> Vec<Taint> {
    let told = response["result"].to_string();
    read_through(clients, connection, message, response)
        .await
        .into_iter()
        .map(|t| t.told_by(&told))
        .collect()
}

async fn read_through<S: Slack>(
    clients: Option<&SlackClients<S>>,
    connection: &Connection,
    message: &Value,
    response: &Value,
) -> Vec<Taint> {
    let id = connection.id.as_str();
    match message["method"].as_str() {
        Some("tools/call") => {}
        Some("resources/read") => return vec![Taint::of_connection(id, "の内容")],
        _ => return Vec::new(),
    }
    if response.get("error").is_some() || response["result"]["isError"] == true {
        return Vec::new();
    }
    let tool = message
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !is_slack(connection) {
        let talks = words(tool).any(|word| TALK.contains(&word.as_str()))
            || matches!(tool, "http_get" | "http_post");
        let talks = talks || (connection.kind == "discord" && tool == "read_channel");
        return talks
            .then(|| Taint::of_connection(id, "のやりとり"))
            .into_iter()
            .collect();
    }
    match tool {
        tool if SLACK_PLAIN.contains(&tool) => Vec::new(),
        "read_channel" => {
            let channel = message
                .pointer("/params/arguments/channel")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let room = room_of(clients, id, channel).await;
            (room.private != Some(false))
                .then(|| Taint::room(id, channel, &room))
                .into_iter()
                .collect()
        }
        "search_messages" => of_search(id, response),
        _ => vec![Taint::of_connection(id, "のやりとり")],
    }
}

async fn room_of<S: Slack>(
    clients: Option<&SlackClients<S>>,
    workspace: &str,
    channel: &str,
) -> Room {
    match clients.and_then(|c| c.get(Some(workspace)).ok()) {
        Some(slack) => slack.room_or_unknown(channel).await,
        None => Room::default(),
    }
}

/// What was found by a search, by the channel each was said in.
fn of_search(id: &str, response: &Value) -> Vec<Taint> {
    let unknown = || vec![Taint::of_connection(id, "のやりとり")];
    let text = response
        .pointer("/result/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Ok(answer) = serde_json::from_str::<Value>(text) else {
        return unknown();
    };
    let Some(matches) = answer
        .pointer("/messages/matches")
        .and_then(Value::as_array)
    else {
        return unknown();
    };
    matches
        .iter()
        .filter_map(|found| {
            let channel = &found["channel"];
            let room = Room::of(channel);
            (room.private != Some(false))
                .then(|| Taint::room(id, channel["id"].as_str().unwrap_or_default(), &room))
        })
        .collect()
}
