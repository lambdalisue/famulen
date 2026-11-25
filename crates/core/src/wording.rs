//! A held call in the owner's words: what it does, where, and with what.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;

use famulen_gateway::connections::Connection;
use famulen_gateway::permissions::split_tool;

/// Calls that read wrong when put together from their words.
const WHOLE: &[(&str, &str)] = &[
    ("Agent", "エージェントを始める"),
    ("Bash", "コマンドを実行する"),
    ("BashOutput", "コマンドの出力を読む"),
    ("Edit", "ファイルを直す"),
    ("Glob", "ファイルを探す"),
    ("Grep", "ファイルの中を探す"),
    ("KillShell", "動いているコマンドを止める"),
    ("MultiEdit", "ファイルを直す"),
    ("NotebookEdit", "ノートブックを直す"),
    ("PowerShell", "コマンドを実行する"),
    ("Read", "ファイルを読む"),
    ("Task", "エージェントを始める"),
    ("WebFetch", "ウェブの頁を読む"),
    ("WebSearch", "ウェブを探す"),
    ("Write", "ファイルを書く"),
    ("adroit.answer_permission", "セッションの許可の求めに答える"),
    ("adroit.put_question", "セッションからの質問に答える"),
    ("adroit.send_to_session", "セッションに伝える"),
    ("bluesky.create_post", "投稿する"),
    ("definitions.change_definition", "サービス定義を変える"),
    ("google.get_message", "メールを読む"),
    ("google.respond_to_event", "予定の出欠を答える"),
    ("google.search_messages", "メールを探す"),
    ("google.send_message", "メールを送る"),
    ("google.stop_watching", "変更の知らせを止める"),
    ("google.watch_events", "予定の変更を知らせてもらう"),
    ("google.watch_mailbox", "新しいメールを知らせてもらう"),
    ("kingoftime.record_time", "打刻する"),
    ("slack.open_dm", "DM を開く"),
    ("slack.set_status", "考え中の表示を変える"),
    ("vault.delete_secret", "秘密を消す"),
    ("vault.describe_secret", "秘密の説明を変える"),
    ("vault.put_secret", "秘密を入れる"),
];

const VERBS: &[(&str, &str)] = &[
    ("accept", "受ける"),
    ("add", "足す"),
    ("answer", "答える"),
    ("append", "書き足す"),
    ("approve", "承認する"),
    ("archive", "しまう"),
    ("assign", "割り当てる"),
    ("cancel", "取り消す"),
    ("change", "変える"),
    ("clear", "空にする"),
    ("close", "閉じる"),
    ("complete", "済ませる"),
    ("copy", "写す"),
    ("create", "作る"),
    ("decline", "断る"),
    ("delete", "消す"),
    ("destroy", "消す"),
    ("disable", "切る"),
    ("download", "ダウンロードする"),
    ("duplicate", "写す"),
    ("edit", "直す"),
    ("enable", "入れる"),
    ("end", "終える"),
    ("erase", "消す"),
    ("execute", "実行する"),
    ("export", "書き出す"),
    ("fetch", "読む"),
    ("find", "探す"),
    ("forward", "転送する"),
    ("generate", "作る"),
    ("get", "読む"),
    ("import", "取り込む"),
    ("insert", "足す"),
    ("interrupt", "止める"),
    ("invite", "招待する"),
    ("label", "ラベルを付ける"),
    ("list", "一覧で読む"),
    ("mark", "印を付ける"),
    ("merge", "マージする"),
    ("modify", "変える"),
    ("move", "移す"),
    ("new", "作る"),
    ("open", "開く"),
    ("patch", "変える"),
    ("pause", "一時停止する"),
    ("pay", "支払う"),
    ("post", "投稿する"),
    ("publish", "公開する"),
    ("purchase", "支払う"),
    ("purge", "消す"),
    ("put", "入れる"),
    ("query", "調べる"),
    ("react", "リアクションを付ける"),
    ("read", "読む"),
    ("remove", "消す"),
    ("rename", "名前を変える"),
    ("reply", "返事をする"),
    ("request", "頼む"),
    ("resolve", "解決にする"),
    ("respond", "返事をする"),
    ("restore", "戻す"),
    ("resume", "続ける"),
    ("revoke", "取り消す"),
    ("run", "実行する"),
    ("save", "保存する"),
    ("schedule", "予約する"),
    ("search", "探す"),
    ("send", "送る"),
    ("set", "決める"),
    ("share", "共有する"),
    ("start", "始める"),
    ("stop", "止める"),
    ("submit", "出す"),
    ("transfer", "移す"),
    ("trash", "ごみ箱に入れる"),
    ("unlabel", "ラベルを外す"),
    ("unregister", "登録から外す"),
    ("update", "変える"),
    ("upload", "アップロードする"),
    ("view", "読む"),
    ("write", "書く"),
];

/// Verbs of a call that only looks.
const LOOKS: &[&str] = &[
    "fetch", "find", "get", "list", "query", "read", "search", "view",
];

/// What the secretary does while a call is under way, where its place and
/// verb do not say it well.
const UNDER_WAY: &[(&str, &str)] = &[
    ("Bash", "コマンドを実行しています…"),
    ("Edit", "ファイルを書いています…"),
    ("Glob", "ファイルを調べています…"),
    ("Grep", "ファイルを調べています…"),
    ("MultiEdit", "ファイルを書いています…"),
    ("NotebookEdit", "ファイルを書いています…"),
    ("PowerShell", "コマンドを実行しています…"),
    ("Read", "ファイルを読んでいます…"),
    ("WebFetch", "ウェブを調べています…"),
    ("WebSearch", "ウェブを調べています…"),
    ("Write", "ファイルを書いています…"),
    ("mcp__famulen__delegate_task", "仕事を委ねています…"),
    ("mcp__famulen__list_events", "予定を調べています…"),
    ("mcp__famulen__remember", "覚え書きを書いています…"),
    ("mcp__famulen__save_record", "記録を書いています…"),
    ("mcp__famulen__search_notebook", "手帳を調べています…"),
];

/// The particle the object of a verb takes, where it is not を.
const PARTICLES: &[(&str, &str)] = &[
    ("answer", "に"),
    ("label", "に"),
    ("mark", "に"),
    ("react", "に"),
    ("reply", "に"),
    ("respond", "に"),
    ("unlabel", "の"),
];

/// Verbs never taken for a noun, which stand last in names that put the noun
/// first.
const ENDINGS: &[&str] = &[
    "add", "archive", "cancel", "create", "delete", "destroy", "edit", "erase", "purge", "remove",
    "send", "set", "trash", "update", "write",
];

const PREPOSITIONS: &[&str] = &["to", "from", "in", "on", "with", "for", "by"];

const NOUNS: &[(&str, &str)] = &[
    ("account", "アカウント"),
    ("block", "ブロック"),
    ("board", "ボード"),
    ("branch", "ブランチ"),
    ("bucket", "バケット"),
    ("calendar", "カレンダー"),
    ("channel", "チャンネル"),
    ("collection", "コレクション"),
    ("comment", "コメント"),
    ("connection", "つなぎ先"),
    ("contact", "連絡先"),
    ("conversation", "会話"),
    ("database", "データベース"),
    ("definition", "サービス定義"),
    ("dm", "メッセージ"),
    ("doc", "文書"),
    ("document", "文書"),
    ("draft", "下書き"),
    ("email", "メール"),
    ("event", "予定"),
    ("feed", "フィード"),
    ("file", "ファイル"),
    ("folder", "フォルダー"),
    ("form", "フォーム"),
    ("issue", "Issue"),
    ("item", "項目"),
    ("label", "ラベル"),
    ("list", "リスト"),
    ("lookout", "見張り"),
    ("mail", "メール"),
    ("member", "メンバー"),
    ("memory", "記憶"),
    ("message", "メッセージ"),
    ("model", "モデル"),
    ("note", "メモ"),
    ("page", "頁"),
    ("permission", "権限"),
    ("post", "投稿"),
    ("project", "プロジェクト"),
    ("pull_request", "プルリクエスト"),
    ("question", "質問"),
    ("reaction", "リアクション"),
    ("record", "記録"),
    ("release", "リリース"),
    ("reminder", "リマインダー"),
    ("repository", "リポジトリ"),
    ("review", "レビュー"),
    ("routine", "定期タスク"),
    ("row", "行"),
    ("secret", "秘密"),
    ("session", "セッション"),
    ("sheet", "シート"),
    ("slack", "Slack のメッセージ"),
    ("space", "スペース"),
    ("table", "表"),
    ("task", "タスク"),
    ("team", "チーム"),
    ("terminal", "端末"),
    ("thread", "スレッド"),
    ("user", "ユーザー"),
    ("value", "値"),
    ("view", "ビュー"),
    ("workgroup", "ワークグループ"),
    ("workspace", "ワークスペース"),
];

/// What a call is given, by its argument.
const NAMES: &[(&str, &str)] = &[
    ("agent", "エージェント"),
    ("attendees", "参加者"),
    ("base", "もとの版"),
    ("bcc", "BCC"),
    ("blocks", "ブロック"),
    ("body", "本文"),
    ("calendar", "カレンダー"),
    ("cc", "CC"),
    ("channel", "チャンネル"),
    ("command", "コマンド"),
    ("content", "中身"),
    ("cwd", "ディレクトリ"),
    ("date", "日付"),
    ("definition", "サービス定義"),
    ("description", "説明"),
    ("dir", "ディレクトリ"),
    ("email", "メール"),
    ("emoji", "絵文字"),
    ("end", "終わり"),
    ("event", "予定"),
    ("file_path", "ファイル"),
    ("id", "ID"),
    ("input", "中身"),
    ("kind", "種類"),
    ("labels", "ラベル"),
    ("link", "リンク"),
    ("location", "場所"),
    ("message", "メッセージ"),
    ("model", "モデル"),
    ("name", "名前"),
    ("new_string", "新しい文"),
    ("old_string", "もとの文"),
    ("owner", "持ち主"),
    ("path", "パス"),
    ("permissions", "権限"),
    ("priority", "優先度"),
    ("project", "プロジェクト"),
    ("prompt", "伝えること"),
    ("query", "探す言葉"),
    ("raw", "送るメール"),
    ("repo", "リポジトリ"),
    ("response", "答え"),
    ("send_updates", "知らせる相手"),
    ("session", "セッション"),
    ("start", "始まり"),
    ("status", "状態"),
    ("subject", "件名"),
    ("summary", "題名"),
    ("text", "本文"),
    ("thread", "スレッド"),
    ("timeout", "待つ長さ（ミリ秒）"),
    ("title", "題名"),
    ("to", "宛先"),
    ("tool", "道具"),
    ("truncated", "途中で切れている"),
    ("url", "URL"),
    ("user", "ユーザー"),
    ("values", "値"),
    ("why", "わけ"),
    ("workgroup", "ワークグループ"),
    ("worktree", "新しいワークツリー"),
];

/// Arguments read as they are typed, such as a command or a path.
const CODE: &[&str] = &[
    "command",
    "path",
    "file_path",
    "dir",
    "cwd",
    "old_string",
    "new_string",
];

/// How deep what a call is given is laid out; what lies deeper is JSON.
const DEPTH: usize = 6;

/// One thing a call is given.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Given {
    pub name: String,
    pub text: String,
    /// It is read as it is typed.
    pub code: bool,
    /// What it is made of, when it is not one text.
    pub rows: Vec<Given>,
}

fn look_up<'a>(table: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// The connection and the tool a call is named by, as far as it says one.
fn parts(tool: &str) -> (Option<&str>, &str) {
    if let Some((connection, tool)) = split_tool(tool) {
        return (Some(connection), tool);
    }
    match tool.split_once('.') {
        Some((own @ (famulen_gateway::VAULT | famulen_gateway::DEFINITIONS), tool)) => {
            (Some(own), tool)
        }
        _ => (None, tool),
    }
}

/// How a call is named to the owner and the session, from the connection it
/// is made through and its tool: as the model called it, or by the change to
/// the vault or to a definition it is.
pub fn tool_of(connection: &str, tool: &str) -> String {
    match connection {
        "" => tool.to_owned(),
        famulen_gateway::VAULT | famulen_gateway::DEFINITIONS => format!("{connection}.{tool}"),
        connection => format!("mcp__{connection}__{tool}"),
    }
}

/// The connection a call is made through, by its id.
pub fn connection_of(tool: &str) -> Option<&str> {
    parts(tool).0
}

/// The words of a tool's name: `send_message`, `sendMessage` and
/// `send-message` alike.
fn words(tool: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut lower = false;
    for c in tool.chars() {
        if !c.is_alphanumeric() {
            lower = false;
            words.push(String::new());
            continue;
        }
        if words.is_empty() || (lower && c.is_uppercase()) {
            words.push(String::new());
        }
        lower = c.is_lowercase() || c.is_numeric();
        if let Some(word) = words.last_mut() {
            word.push(c);
        }
    }
    words.retain(|w| !w.is_empty());
    words
}

/// What the words name: all of them, or the last that is known, as the head
/// of an English phrase comes last.
fn noun(words: &[String]) -> Option<&'static str> {
    let known = |words: &[String]| {
        let said = words.join("_").to_lowercase();
        let one = match said.strip_suffix("ies") {
            Some(stem) => format!("{stem}y"),
            None => said.strip_suffix('s').unwrap_or(&said).to_owned(),
        };
        look_up(NOUNS, &said).or_else(|| look_up(NOUNS, &one))
    };
    (0..words.len()).find_map(|from| known(&words[from..]))
}

/// What a tool does as its name says it.
struct Doing {
    done: &'static str,
    particle: &'static str,
    object: Vec<String>,
}

impl Doing {
    /// Nothing when the name says no verb that is known, or may say it last.
    fn of(tool: &str) -> Option<Self> {
        let words = words(tool);
        let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
        if lower.first().is_some_and(|w| w == "http") {
            return Some(Self {
                done: "API を呼ぶ",
                particle: "",
                object: Vec::new(),
            });
        }
        // A service's own name may come before the verb.
        let at = lower
            .iter()
            .take(2)
            .position(|w| look_up(VERBS, w).is_some())?;
        // `label_delete` labels nothing: what reads as its verb is a noun.
        if lower[at + 1..]
            .iter()
            .any(|w| ENDINGS.contains(&w.as_str()))
        {
            return None;
        }
        let end = lower
            .iter()
            .skip(at + 1)
            .position(|w| PREPOSITIONS.contains(&w.as_str()))
            .map_or(words.len(), |n| at + 1 + n);
        Some(Self {
            done: look_up(VERBS, &lower[at])?,
            particle: look_up(PARTICLES, &lower[at]).unwrap_or("を"),
            object: words[at + 1..end].to_vec(),
        })
    }
}

/// How a call is looked up among those that read wrong when put together.
fn whole(tool: &str, connection: Option<&Connection>) -> Option<&'static str> {
    let (id, name) = parts(tool);
    let Some(id) = id else {
        return look_up(WHOLE, name);
    };
    let kind = match connection {
        Some(c) if !c.kind.is_empty() => c.kind.as_str(),
        _ => match id.rsplit_once('-') {
            Some((kind, n)) if n.chars().all(|c| c.is_ascii_digit()) => kind,
            _ => id,
        },
    };
    look_up(WHOLE, &format!("{kind}.{name}"))
}

/// What a call does, from its tool alone: nothing it is given is in it.
/// `connection` is the one it is made through, where it is still there.
pub fn deed(tool: &str, connection: Option<&Connection>) -> String {
    if let Some(whole) = whole(tool, connection) {
        return whole.to_owned();
    }
    let name = parts(tool).1;
    let Some(doing) = Doing::of(name) else {
        return format!("「{}」を使う", words(name).join(" "));
    };
    let object = match noun(&doing.object) {
        Some(noun) => noun.to_owned(),
        None if doing.object.is_empty() => return doing.done.to_owned(),
        None => format!("「{}」", doing.object.join(" ")),
    };
    format!("{object}{}{}", doing.particle, doing.done)
}

/// As [`deed`], in words alone: nothing of a tool's name is left in it, and
/// there is none when the name does not say what it does.
pub fn deed_in_words(tool: &str, connection: Option<&Connection>) -> Option<String> {
    if let Some(whole) = whole(tool, connection) {
        return Some(whole.to_owned());
    }
    let doing = Doing::of(parts(tool).1)?;
    Some(match noun(&doing.object) {
        Some(noun) => format!("{noun}{}{}", doing.particle, doing.done),
        None => doing.done.to_owned(),
    })
}

/// Where a call is made: the service it goes to, as the owner named it.
pub fn place(tool: &str, connection: Option<&Connection>) -> String {
    match parts(tool).0 {
        None => "この機械".into(),
        Some("famulen") => "Famulen".into(),
        Some(famulen_gateway::VAULT) => "金庫".into(),
        Some(famulen_gateway::DEFINITIONS) => "サービス定義".into(),
        Some(id) => connection
            .map(|c| c.name.as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or(id)
            .to_owned(),
    }
}

/// What the secretary is doing while a call is under way, where that is more
/// than thinking. Nothing the call is given is in it.
pub fn under_way(tool: &str, connection: Option<&Connection>) -> Option<String> {
    if let Some(whole) = look_up(UNDER_WAY, tool) {
        return Some(whole.to_owned());
    }
    let (id, name) = parts(tool);
    let id = id?;
    let verb = words(name)
        .into_iter()
        .take(2)
        .map(|w| w.to_lowercase())
        .find(|w| look_up(VERBS, w).is_some());
    let looks = verb.is_some_and(|verb| LOOKS.contains(&verb.as_str()));
    if id == crate::hub::SERVER_NAME {
        return looks.then(|| "手元の記録を調べています…".to_owned());
    }
    let place = place(tool, connection);
    Some(if looks {
        format!("{place} を調べています…")
    } else {
        format!("{place} で作業しています…")
    })
}

/// An email as Gmail is given one, in the text it is.
pub(crate) fn letter(raw: &str) -> Option<String> {
    let bytes = URL_SAFE_NO_PAD.decode(raw.trim_end_matches('=')).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let head = text.lines().next()?.split_once(": ")?.0;
    (!head.is_empty() && head.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .then_some(text)
}

fn text_of(value: &Value) -> String {
    match value {
        Value::Null => "（なし）".into(),
        Value::Bool(true) => "はい".into(),
        Value::Bool(false) => "いいえ".into(),
        Value::String(s) if s.is_empty() => "（空）".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn row(key: &str, value: &Value, depth: usize) -> Given {
    let given = Given {
        name: look_up(NAMES, key).map_or_else(|| key.replace('_', " "), str::to_owned),
        code: CODE.contains(&key),
        ..Given::default()
    };
    match value {
        Value::Object(map) if map.is_empty() => Given {
            text: "（空）".into(),
            ..given
        },
        Value::Array(items) if items.is_empty() => Given {
            text: "（空）".into(),
            ..given
        },
        Value::Object(map) if depth > 0 => Given {
            rows: map.iter().map(|(k, v)| row(k, v, depth - 1)).collect(),
            ..given
        },
        Value::Array(items) if items.iter().all(|v| !v.is_object() && !v.is_array()) => Given {
            text: items.iter().map(text_of).collect::<Vec<_>>().join("、"),
            ..given
        },
        Value::Array(items) if depth > 0 => Given {
            rows: items
                .iter()
                .enumerate()
                .map(|(i, v)| Given {
                    name: (i + 1).to_string(),
                    ..row("", v, depth - 1)
                })
                .collect(),
            ..given
        },
        Value::String(s) if key == "raw" => Given {
            text: letter(s).unwrap_or_else(|| text_of(value)),
            ..given
        },
        other => Given {
            text: text_of(other),
            ..given
        },
    }
}

/// Everything a call is given, a row for each; nothing when it is not given
/// named things.
pub fn given(input: &Value) -> Option<Vec<Given>> {
    Some(
        input
            .as_object()?
            .iter()
            .map(|(k, v)| row(k, v, DEPTH))
            .collect(),
    )
}

/// `rows` as plain text. What is given stands behind a bar wherever it has a
/// line of its own, so that none of it can pass for another row.
pub fn plain(rows: &[Given]) -> String {
    let mut out = Vec::new();
    lay(rows, "", &mut out);
    out.join("\n")
}

fn lay(rows: &[Given], indent: &str, out: &mut Vec<String>) {
    for r in rows {
        let name: String = r
            .name
            .chars()
            .map(|c| if is_break(c) { ' ' } else { c })
            .collect();
        if !r.rows.is_empty() {
            out.push(format!("{indent}{name}"));
            lay(&r.rows, &format!("{indent}　"), out);
        } else if r.text.contains(is_break) {
            out.push(format!("{indent}{name}"));
            out.extend(
                r.text
                    .split(is_break)
                    .map(|line| format!("{indent}│ {line}")),
            );
        } else {
            out.push(format!("{indent}{name}：{}", r.text));
        }
    }
}

/// Whether a line is broken here, wherever it is shown.
fn is_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use famulen_gateway::connections::{Secret, Transport};
    use serde_json::json;

    fn connection(id: &str, kind: &str, name: &str) -> Connection {
        Connection {
            id: id.into(),
            name: name.into(),
            kind: kind.into(),
            transport: Transport::McpRemote { url: String::new() },
            secret: Secret::default(),
            account: "me@example.com".into(),
            status: String::new(),
        }
    }

    #[test]
    fn a_call_reads_as_what_it_does() {
        for (tool, does) in [
            ("Bash", "コマンドを実行する"),
            ("mcp__adroit__start_session", "セッションを始める"),
            ("mcp__google-2__send_message", "メールを送る"),
            ("mcp__slack-2__post_message", "メッセージを投稿する"),
            ("mcp__famulen__delete_record", "記録を消す"),
            ("mcp__notion__notion-create-pages", "頁を作る"),
            ("mcp__asana__createTask", "タスクを作る"),
            ("mcp__github__reply_comments", "コメントに返事をする"),
            ("mcp__x__create_widget_kit", "「widget kit」を作る"),
            ("mcp__x__frobnicate", "「frobnicate」を使う"),
            ("mcp__x__list_delete", "「list delete」を使う"),
            ("mcp__x__post_remove", "「post remove」を使う"),
            ("mcp__x__http_delete", "API を呼ぶ"),
            ("vault.put_secret", "秘密を入れる"),
            ("definitions.change_definition", "サービス定義を変える"),
        ] {
            assert_eq!(deed(tool, None), does, "{tool}");
        }
    }

    #[test]
    fn a_call_under_way_reads_as_what_the_secretary_is_doing() {
        for (tool, does) in [
            ("Bash", Some("コマンドを実行しています…")),
            ("WebSearch", Some("ウェブを調べています…")),
            ("ToolSearch", None),
            ("TodoWrite", None),
            ("mcp__famulen__delegate_task", Some("仕事を委ねています…")),
            (
                "mcp__famulen__list_records",
                Some("手元の記録を調べています…"),
            ),
            ("mcp__famulen__post_message", None),
            ("mcp__famulen__set_working", None),
            ("mcp__notion__notion-search", Some("notion を調べています…")),
            ("mcp__asana__createTask", Some("asana で作業しています…")),
        ] {
            assert_eq!(under_way(tool, None).as_deref(), does, "{tool}");
        }
        let mine = connection("google-2", "google", "会社の Gmail");
        assert_eq!(
            under_way("mcp__google-2__search_messages", Some(&mine)).as_deref(),
            Some("会社の Gmail を調べています…")
        );
    }

    #[test]
    fn a_connections_kind_says_what_its_tools_do_whatever_its_id() {
        let mine = connection("work-mail", "google", "会社の Gmail");
        assert_eq!(
            deed("mcp__work-mail__send_message", Some(&mine)),
            "メールを送る"
        );
        assert_eq!(
            place("mcp__work-mail__send_message", Some(&mine)),
            "会社の Gmail"
        );
    }

    #[test]
    fn where_a_call_is_made_never_names_the_account() {
        let mine = connection("google", "google", "");
        assert_eq!(place("mcp__google__send_message", Some(&mine)), "google");
        assert_eq!(place("mcp__gone__x", None), "gone");
        assert_eq!(place("Bash", None), "この機械");
        assert_eq!(place("mcp__famulen__delete_record", None), "Famulen");
        assert_eq!(place("vault.put", None), "金庫");
    }

    #[test]
    fn everything_given_has_a_row() {
        let rows = given(&json!({
            "to": ["a@example.com", "b@example.com"],
            "body": "",
            "worktree": true,
            "odd_key": 3,
            "event": { "start": "10:00", "attendees": [{ "email": "c@example.com" }] },
        }))
        .unwrap();
        assert_eq!(
            plain(&rows),
            "本文：（空）\n予定\n　参加者\n　　1\n　　　メール：c@example.com\n　始まり：10:00\nodd key：3\n宛先：a@example.com、b@example.com\n新しいワークツリー：はい"
        );
        assert_eq!(given(&json!("text")), None);
    }

    #[test]
    fn what_is_given_cannot_pass_for_a_row_of_its_own() {
        for br in ["\n", "\r", "\r\n", "\u{2028}", "\u{85}"] {
            let rows = given(&json!({ "body": format!("やあ{br}宛先：x") })).unwrap();
            let said = plain(&rows);
            assert!(said.ends_with("│ 宛先：x"), "{said:?}");
            assert!(
                said.lines().skip(1).all(|l| l.starts_with("│ ")),
                "{said:?}"
            );
        }
        let rows = given(&json!({ "a\n宛先：x": "1" })).unwrap();
        assert_eq!(plain(&rows), "a 宛先：x：1");
    }

    #[test]
    fn a_letter_is_read_as_the_text_it_is() {
        let raw = URL_SAFE_NO_PAD.encode("To: a@example.com\r\n\r\nこんにちは");
        assert!(plain(&given(&json!({ "raw": raw })).unwrap()).contains("│ こんにちは"));
        assert_eq!(
            plain(&given(&json!({ "raw": "???" })).unwrap()),
            "送るメール：???"
        );
    }

    #[test]
    fn what_lies_too_deep_is_json() {
        let mut value = json!("x");
        for _ in 0..20 {
            value = json!({ "a": value });
        }
        assert!(plain(&given(&value).unwrap()).contains("{\"a\":"));
    }
}
