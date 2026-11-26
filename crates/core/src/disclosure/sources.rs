//! Which answers of tools hold something in confidence, and of what kind.
//! Facts about work, what anyone can read, and whether the owner is free do
//! not count: the owner is not to be asked of everything.

use std::path::{Path, PathBuf};

use futures_util::future::BoxFuture;
use serde_json::Value;

use super::taint::{Kind, Taint};
use crate::attachments::DIR as ATTACHMENTS;
use crate::claude::skills::{PLUGINS_DIR, SKILLS_DIR};
use crate::connections::{Connection, Transport};
use crate::input::{Input, Source};
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

/// Built-in tools of the model CLI whose answers tell nothing it did not
/// know: what it wrote, what it planned, and what anyone may find.
const BUILTIN_PLAIN: &[&str] = &[
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillBash",
    "KillShell",
    "Skill",
    "SlashCommand",
    "TodoWrite",
    "ToolSearch",
    "WebSearch",
    "Write",
];

/// Built-in tools that read files, and those that answer with what a file
/// held around what they changed.
const FILE_READERS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "LS",
    "NotebookRead",
    "Edit",
    "MultiEdit",
    "NotebookEdit",
];

const LOCAL: &str = "手元のファイル";
const ATTACHED: &str = "添えられたファイル";

/// What a built-in tool of the model CLI reads. Most read without asking
/// Famulen, so it is told by the call itself. One not known to tell nothing
/// counts as private, so that a new one does not slip through.
pub(crate) fn of_builtin(
    memory_dir: &Path,
    workdir: &Path,
    tool: &str,
    input: &Value,
) -> Vec<Taint> {
    if tool.starts_with("mcp__") || BUILTIN_PLAIN.contains(&tool) {
        // What a server answers is read where the answer passes.
        return Vec::new();
    }
    let told = |taint: Taint| vec![taint.told_by(&input.to_string())];
    match tool {
        "WebFetch" => match input["url"].as_str() {
            Some(url) if super::web::is_public_page(url) => Vec::new(),
            _ => told(private("ウェブの公開でない頁")),
        },
        tool if FILE_READERS.contains(&tool) => of_files_read(memory_dir, workdir, tool, input),
        "Bash" | "BashOutput" | "PowerShell" | "TaskOutput" | "Monitor" => {
            told(private("シェルの出力"))
        }
        tool => told(private(format!("組み込みの道具（{tool}）の答え"))),
    }
}

fn of_files_read(memory_dir: &Path, workdir: &Path, tool: &str, input: &Value) -> Vec<Taint> {
    let path = ["file_path", "path", "notebook_path"]
        .iter()
        .find_map(|key| input[*key].as_str());
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
    // A pattern that steps up finds what is outside where it starts.
    let climbs = tool == "Glob" && input["pattern"].as_str().is_some_and(|p| p.contains(".."));
    let local = || private(LOCAL).told_by(&format!("{} {}", path.display(), input["pattern"]));
    if inside(&path) {
        let named = path.to_string_lossy();
        let mut taints: Vec<Taint> = of_path(memory_dir, &named)
            .into_iter()
            .map(|t| t.told_by(&format!("{named} {}", input["pattern"])))
            .collect();
        taints.extend(climbs.then(local));
        return taints;
    }
    if spelt.as_deref().is_some_and(inside) {
        return vec![people(), belongings(FILES)];
    }
    // A step up may lead out of where it seems to be once it is followed.
    let under = |dirs: &[PathBuf], p: &Path| {
        let steps_up = p.components().any(|c| c == std::path::Component::ParentDir);
        let p = real(p);
        !climbs && !steps_up && dirs.iter().any(|dir| p.starts_with(dir))
    };
    let skills = [SKILLS_DIR, PLUGINS_DIR].map(|dir| real(&workdir.join(dir)));
    if under(&skills, &path) && spelt.as_deref().is_none_or(|p| under(&skills, p)) {
        return Vec::new();
    }
    let attached = [real(&workdir.join(ATTACHMENTS))];
    if under(&attached, &path) && spelt.as_deref().is_none_or(|p| under(&attached, p)) {
        return vec![private(ATTACHED).told_by(&path.to_string_lossy())];
    }
    vec![local()]
}

/// Have what a file attached to `input` holds be of the place it was attached
/// in, so that it may go back there. A file attached to another input is not.
pub(crate) fn attached_in(
    taints: Vec<Taint>,
    input: &Input,
    workdir: &Path,
    called: &Value,
) -> Vec<Taint> {
    let Source::Slack {
        workspace,
        channel,
        attached,
        ..
    } = &input.source
    else {
        return taints;
    };
    let Some(read) = called["file_path"].as_str().map(|p| real(&workdir.join(p))) else {
        return taints;
    };
    let its_own = attached.iter().any(|a| match a {
        crate::attachments::Attached::Kept(kept) => real(&workdir.join(&kept.path)) == read,
        _ => false,
    });
    if !its_own {
        return taints;
    }
    taints
        .into_iter()
        .map(|t| match t.label.as_str() {
            ATTACHED => {
                let mut t = t.in_place(channel);
                t.connection = workspace.clone();
                t
            }
            _ => t,
        })
        .collect()
}

/// What a file of the notebook says, by its path, when it is one.
pub(super) fn notebook_text(memory_dir: &Path, path: &str) -> Option<String> {
    let given = Path::new(path);
    let found = if given.is_absolute() {
        given.to_owned()
    } else {
        root(memory_dir).join(given)
    };
    let root = real(root(memory_dir));
    let found = real(&found);
    (found.starts_with(&root) && found.is_file())
        .then(|| std::fs::read_to_string(&found).ok())
        .flatten()
}

/// What a built-in tool reads of the notebook: the file it names with what
/// that says, or `None` for more than one file.
pub(crate) fn read_of_notebook(
    memory_dir: &Path,
    workdir: &Path,
    tool: &str,
    input: &Value,
) -> Option<Option<String>> {
    if !FILE_READERS.contains(&tool) {
        return None;
    }
    let path = ["file_path", "path", "notebook_path"]
        .iter()
        .find_map(|key| input[*key].as_str());
    let path = path.map_or_else(|| workdir.to_owned(), |p| workdir.join(p));
    let root = real(root(memory_dir));
    let found = real(&path);
    if !(found.starts_with(&root) || root.starts_with(&found)) {
        return None;
    }
    let named = path.to_string_lossy().into_owned();
    match notebook_text(memory_dir, &named) {
        Some(text) if tool != "Glob" => Some(Some(format!("{named}\n{text}"))),
        _ => Some(None),
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

/// Tools of Slack that tell only what they did.
const SLACK_DONE: &[&str] = &[
    "get_permalink",
    "open_dm",
    "post_message",
    "post_ephemeral",
    "update_message",
    "set_status",
    "add_reaction",
    "set_app_icon",
];

/// Tools of Slack that tell who and what there is in a workspace.
const SLACK_DIRECTORY: &[&str] = &["list_channels", "user_info"];

/// Tools of Bluesky that read what anyone sees without logging in, unless
/// its author asked otherwise. The timeline and notifications are the
/// account's own.
const ANYONE_SEES_ON_BLUESKY: &[&str] = &[
    "get_author_feed",
    "get_post_thread",
    "get_profile",
    "search_posts",
];

/// Tools of GitHub that read what a repository holds, of the repository the
/// `owner` and `repo` arguments name. Anyone may read it when the repository
/// is public. What is the account's own, such as notifications, is not here.
const REPOSITORY_CONTENT: &[&str] = &[
    "get_commit",
    "get_discussion",
    "get_discussion_comments",
    "get_file_contents",
    "get_label",
    "get_latest_release",
    "get_release_by_tag",
    "get_repository_tree",
    "get_tag",
    "issue_read",
    "list_branches",
    "list_commits",
    "list_discussion_categories",
    "list_discussions",
    "list_issue_types",
    "list_issues",
    "list_label",
    "list_pull_requests",
    "list_releases",
    "list_tags",
    "pull_request_read",
];

/// Tells whether what is read is what anyone on the internet may read.
pub(crate) trait Looks: Sync {
    /// Whether the GitHub repository `owner/name` is public; `None` when it
    /// cannot be told.
    fn repository<'a>(&'a self, full_name: &'a str) -> BoxFuture<'a, Option<bool>>;
}

/// What a connection's answer to a message holds in confidence.
pub(crate) async fn of_connection<S: Slack>(
    clients: Option<&SlackClients<S>>,
    looks: &dyn Looks,
    connection: &Connection,
    message: &Value,
    response: &Value,
) -> Vec<Taint> {
    let told = response["result"].to_string();
    read_through(clients, looks, connection, message, response)
        .await
        .into_iter()
        .map(|mut t| match defined_as(connection) {
            // A repository is not a channel: what was read in it is told apart.
            Some("github") => {
                let place = std::mem::take(&mut t.place);
                t.told_by(&told).in_place(&place)
            }
            _ => t.told_by(&told),
        })
        .collect()
}

/// What was read through a connection. Only what can be told to be public is
/// not confidential, so that a new connection or tool does not slip through.
async fn read_through<S: Slack>(
    clients: Option<&SlackClients<S>>,
    looks: &dyn Looks,
    connection: &Connection,
    message: &Value,
    response: &Value,
) -> Vec<Taint> {
    let id = connection.id.as_str();
    let content = || vec![Taint::named(id, &connection.name, "の内容")];
    match message["method"].as_str() {
        Some("tools/call") => {}
        Some("resources/read") => return content(),
        _ => return Vec::new(),
    }
    if response.get("error").is_some() || response["result"]["isError"] == true {
        return Vec::new();
    }
    let tool = message
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let arguments = &message["params"]["arguments"];
    if is_slack(connection) {
        return of_slack(clients, connection, tool, arguments, response).await;
    }
    let public = match defined_as(connection) {
        Some("bluesky") => {
            ANYONE_SEES_ON_BLUESKY.contains(&tool)
                && !response.to_string().contains("no-unauthenticated")
        }
        Some("rss") => anyone_reads_the_feed(connection),
        Some("github") => {
            let repository = repository_of(arguments);
            let public = REPOSITORY_CONTENT.contains(&tool)
                && !kept_back(response)
                && match &repository {
                    Some(name) => looks.repository(name).await == Some(true),
                    None => false,
                };
            if public {
                return Vec::new();
            }
            // What was found across repositories goes back to none of them.
            let from = repository.unwrap_or_else(|| "*".to_owned());
            return vec![Taint::named(id, &connection.name, "の内容").in_place(&from)];
        }
        _ => false,
    };
    if public { Vec::new() } else { content() }
}

/// The bundled service definition a connection is reached by.
fn defined_as(connection: &Connection) -> Option<&str> {
    match &connection.transport {
        Transport::Defined { definition, .. } => Some(definition),
        _ => None,
    }
}

/// What a public repository shows only to those who may write to it: a
/// review not sent yet, a release not published yet.
fn kept_back(response: &Value) -> bool {
    let text = response.to_string().replace(['\\', ' '], "");
    text.contains("PENDING") || text.contains("\"draft\":true")
}

/// `owner/name` of the repository a call is of, in lower case.
pub(super) fn repository_of(arguments: &Value) -> Option<String> {
    let field = |key: &str| arguments[key].as_str().filter(|v| !v.is_empty());
    let (owner, repo) = (field("owner")?, field("repo")?);
    let plain = |part: &str| {
        part.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (plain(owner) && plain(repo) && !repo.starts_with('.'))
        .then(|| format!("{owner}/{repo}").to_ascii_lowercase())
}

/// Whether a feed is read on the public internet with nothing that tells who
/// reads it.
fn anyone_reads_the_feed(connection: &Connection) -> bool {
    let Transport::Defined { values, public, .. } = &connection.transport else {
        return false;
    };
    *public
        && connection.secret == Default::default()
        && values
            .get("url")
            .is_some_and(|url| super::web::opens_to_anyone(url))
}

/// What a connection of Slack answered: what was said in a channel, by who
/// can read the channel.
async fn of_slack<S: Slack>(
    clients: Option<&SlackClients<S>>,
    connection: &Connection,
    tool: &str,
    arguments: &Value,
    response: &Value,
) -> Vec<Taint> {
    let id = connection.id.as_str();
    match tool {
        tool if SLACK_DONE.contains(&tool) => Vec::new(),
        tool if SLACK_DIRECTORY.contains(&tool) => {
            vec![Taint::internal(id, &connection.name, "の顔ぶれ")]
        }
        "read_channel" | "channel_info" => {
            let channel = arguments["channel"].as_str().unwrap_or_default();
            let room = room_of(clients, id, channel).await;
            vec![of_room(id, &connection.name, channel, &room)]
        }
        "search_messages" => of_search(connection, response),
        _ => vec![Taint::named(id, &connection.name, "のやりとり")],
    }
}

/// What was said in a channel of Slack: kept to its channel when not everyone
/// in the workspace can read it, to the workspace when they can.
pub(crate) fn of_room(workspace: &str, name: &str, channel: &str, room: &Room) -> Taint {
    match room.private {
        Some(false) => {
            Taint::internal(workspace, name, "の公開のチャンネルの内容").in_place(channel)
        }
        _ => Taint::room(workspace, channel, room),
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
fn of_search(connection: &Connection, response: &Value) -> Vec<Taint> {
    let id = connection.id.as_str();
    let unknown = || vec![Taint::named(id, &connection.name, "のやりとり")];
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
        .map(|found| {
            let channel = &found["channel"];
            let room = Room::of(channel);
            of_room(
                id,
                &connection.name,
                channel["id"].as_str().unwrap_or_default(),
                &room,
            )
        })
        .collect()
}
