//! What a held call is, in words the owner reads at a glance: never a tool's
//! name, which says nothing to a person.

use famulen_gateway::{CHANGE_DEFINITION, DEFINITIONS, DELETE_SECRET, DESCRIBE_SECRET, VAULT};
use serde_json::Value;

/// How long the words of a call may be, in characters.
const SHOWN_CHARS: usize = 240;
/// And one of its values, so that a long one hides no other.
const VALUE_CHARS: usize = 100;

/// What a tool's first word does, with the particle its object takes.
const VERBS: &[(&[&str], &str, &str)] = &[
    (&["send", "forward"], "を", "送る"),
    (&["reply", "respond", "answer"], "に", "返事をする"),
    (&["post"], "を", "投稿する"),
    (&["publish"], "を", "公開する"),
    (&["create", "new"], "を", "作る"),
    (&["add", "insert"], "を", "足す"),
    (
        &[
            "update", "edit", "set", "patch", "modify", "change", "rename",
        ],
        "を",
        "変える",
    ),
    (&["write", "append", "save"], "を", "書く"),
    (
        &["delete", "remove", "destroy", "purge", "erase"],
        "を",
        "消す",
    ),
    (&["trash"], "を", "ごみ箱に入れる"),
    (&["archive"], "を", "アーカイブする"),
    (&["move"], "を", "移す"),
    (&["copy", "duplicate"], "を", "写す"),
    (&["merge"], "を", "マージする"),
    (&["share"], "を", "共有する"),
    (&["invite"], "を", "招待する"),
    (&["label"], "に", "ラベルを付ける"),
    (&["unlabel"], "の", "ラベルを外す"),
    (&["react"], "に", "リアクションを付ける"),
    (&["schedule"], "を", "予約する"),
    (&["cancel"], "を", "取り消す"),
    (&["decline"], "を", "断る"),
    (&["complete", "close"], "を", "終える"),
    (&["start", "open"], "を", "始める"),
    (&["stop"], "を", "止める"),
    (&["run", "execute"], "を", "実行する"),
    (&["upload"], "を", "アップロードする"),
    (&["download"], "を", "ダウンロードする"),
    (&["pay", "purchase"], "を", "支払う"),
    (&["transfer"], "を", "移す"),
    (
        &[
            "get", "read", "fetch", "list", "search", "find", "query", "view",
        ],
        "を",
        "読む",
    ),
];

const NOUNS: &[(&str, &str)] = &[
    ("account", "アカウント"),
    ("branch", "ブランチ"),
    ("calendar", "カレンダー"),
    ("channel", "チャンネル"),
    ("comment", "コメント"),
    ("connection", "つなぎ先"),
    ("contact", "連絡先"),
    ("conversation", "会話"),
    ("dm", "メッセージ"),
    ("doc", "文書"),
    ("document", "文書"),
    ("draft", "下書き"),
    ("email", "メール"),
    ("event", "予定"),
    ("file", "ファイル"),
    ("folder", "フォルダ"),
    ("issue", "Issue "),
    ("label", "ラベル"),
    ("lookout", "見張り"),
    ("mail", "メール"),
    ("member", "メンバー"),
    ("message", "メッセージ"),
    ("model", "モデル"),
    ("note", "メモ"),
    ("page", "ページ"),
    ("post", "投稿"),
    ("pull_request", "プルリクエスト"),
    ("reaction", "リアクション"),
    ("record", "記録"),
    ("release", "リリース"),
    ("reminder", "リマインダー"),
    ("repository", "リポジトリ"),
    ("secret", "秘密"),
    ("session", "セッション"),
    ("sheet", "シート"),
    ("slack", "Slack のメッセージ"),
    ("task", "タスク"),
    ("thread", "スレッド"),
    ("user", "ユーザー"),
    ("workgroup", "ワークグループ"),
];

const BUILTINS: &[(&str, &str)] = &[
    ("Bash", "コマンドを実行する"),
    ("PowerShell", "コマンドを実行する"),
    ("Write", "ファイルを書く"),
    ("Edit", "ファイルを書き換える"),
    ("MultiEdit", "ファイルを書き換える"),
    ("NotebookEdit", "ファイルを書き換える"),
    ("Read", "ファイルを読む"),
    ("WebFetch", "Web のページを読む"),
    ("WebSearch", "Web で探す"),
];

/// What the call does, as 「Gmail」でメールを送る. `place` is the name the
/// owner gave the connection, if the call is on one.
pub(super) fn deed(connection: &str, tool: &str, place: Option<&str>) -> String {
    match (connection, tool) {
        (VAULT, DELETE_SECRET) => return "金庫の秘密を消す".into(),
        (VAULT, DESCRIBE_SECRET) => return "金庫の秘密の説明を変える".into(),
        (VAULT, _) => return "金庫に秘密を入れる".into(),
        (DEFINITIONS, CHANGE_DEFINITION) => return "サービス定義を変える".into(),
        ("", _) => {
            if let Some((_, done)) = BUILTINS.iter().find(|(name, _)| *name == tool) {
                return (*done).into();
            }
        }
        _ => {}
    }
    let (server, name) = famulen_gateway::permissions::split_tool(tool)
        .filter(|_| connection.is_empty())
        .unwrap_or(("", tool));
    let place = match (place, server) {
        (Some(place), _) => format!("「{place}」"),
        (None, _) if !connection.is_empty() => "つなぎ先".into(),
        (None, crate::hub::SERVER_NAME) => "Famulen ".into(),
        (None, _) => "手元の道具".into(),
    };
    match doing(name) {
        Some(done) if done.starts_with(|c: char| c.is_ascii()) => format!("{place}で {done}"),
        Some(done) => format!("{place}で{done}"),
        None => format!("{place}で操作をする"),
    }
}

fn doing(tool: &str) -> Option<String> {
    let words = words(tool);
    if words.first().is_some_and(|w| w == "http") {
        return Some("API を呼ぶ".into());
    }
    let verb = |w: &String| VERBS.iter().find(|(names, ..)| names.contains(&w.as_str()));
    // A service's own name may come before the verb.
    let at = words.iter().take(2).position(|w| verb(w).is_some())?;
    // `label_delete` labels nothing: what reads as its verb is a noun.
    if words[at + 1..]
        .iter()
        .any(|w| ENDINGS.contains(&w.as_str()))
    {
        return None;
    }
    let (_, particle, done) = verb(&words[at])?;
    let rest = &words[at + 1..];
    let rest = match rest.iter().position(|w| PREPOSITIONS.contains(&w.as_str())) {
        Some(end) => &rest[..end],
        None => rest,
    };
    match noun(rest) {
        Some(noun) => Some(format!("{noun}{particle}{done}")),
        None => Some((*done).into()),
    }
}

/// Verbs never taken for a noun, which stand last in names that put the noun
/// first.
const ENDINGS: &[&str] = &[
    "add", "archive", "cancel", "create", "delete", "destroy", "edit", "erase", "purge", "remove",
    "send", "set", "trash", "update", "write",
];

const PREPOSITIONS: &[&str] = &["to", "from", "in", "on", "with", "for", "by"];

/// What the words name: the last of them that is known, as the head of an
/// English phrase comes last.
fn noun(words: &[String]) -> Option<&'static str> {
    let singular = |w: &str| w.strip_suffix('s').unwrap_or(w).to_owned();
    let known = |name: &str| NOUNS.iter().find(|(n, _)| *n == name).map(|(_, ja)| *ja);
    let last = singular(words.last()?);
    if let [.., before, _] = words
        && let Some(found) = known(&format!("{before}_{last}"))
    {
        return Some(found);
    }
    known(&last)
}

/// `deleteFile`, `delete_file` and `delete-file` alike.
fn words(tool: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    for c in tool.chars() {
        if matches!(c, '_' | '-' | '.') {
            out.push(String::new());
            continue;
        }
        if c.is_ascii_uppercase() && out.last().is_some_and(|w| !w.is_empty()) {
            out.push(String::new());
        }
        if let Some(word) = out.last_mut() {
            word.push(c.to_ascii_lowercase());
        }
    }
    out.retain(|w| !w.is_empty());
    out
}

/// The call's own values, without the names of its fields.
pub(super) fn values(input: &Value) -> String {
    let mut found = Vec::new();
    gather(input, &mut found);
    let text = found.join(" / ");
    cut(&text, SHOWN_CHARS)
}

/// One line of at most `most` characters.
fn cut(text: &str, most: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= most {
        return text;
    }
    text.chars().take(most - 1).chain(['…']).collect()
}

fn gather(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::String(text) if !text.trim().is_empty() => found.push(cut(text, VALUE_CHARS)),
        Value::Number(n) => found.push(n.to_string()),
        Value::Array(items) => items.iter().for_each(|v| gather(v, found)),
        Value::Object(fields) => fields.values().for_each(|v| gather(v, found)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_call_on_a_connection_is_told_by_its_name_and_what_it_does() {
        for (tool, told) in [
            ("send_message", "「会社の Gmail」でメッセージを送る"),
            ("create_draft", "「会社の Gmail」で下書きを作る"),
            ("respond_to_event", "「会社の Gmail」で返事をする"),
            ("delete_events", "「会社の Gmail」で予定を消す"),
            ("gmail_send_email", "「会社の Gmail」でメールを送る"),
            (
                "createPullRequest",
                "「会社の Gmail」でプルリクエストを作る",
            ),
            (
                "merge_pull_request",
                "「会社の Gmail」でプルリクエストをマージする",
            ),
            ("create_pull_request_review", "「会社の Gmail」で作る"),
            ("label_delete", "「会社の Gmail」で操作をする"),
            ("task_list_delete", "「会社の Gmail」で操作をする"),
            ("http_delete", "「会社の Gmail」で API を呼ぶ"),
            ("frobnicate", "「会社の Gmail」で操作をする"),
        ] {
            assert_eq!(deed("gmail-work", tool, Some("会社の Gmail")), told);
        }
    }

    #[test]
    fn what_is_not_on_a_connection_is_told_without_its_name_too() {
        for (connection, tool, told) in [
            ("", "Bash", "コマンドを実行する"),
            ("", "mcp__famulen__delete_record", "Famulen で記録を消す"),
            ("", "mcp__famulen__register_model", "Famulen で操作をする"),
            ("", "Frobnicate", "手元の道具で操作をする"),
            (VAULT, DELETE_SECRET, "金庫の秘密を消す"),
            (DEFINITIONS, CHANGE_DEFINITION, "サービス定義を変える"),
        ] {
            assert_eq!(deed(connection, tool, None), told);
        }
    }

    #[test]
    fn values_are_cut_by_the_character() {
        let told = values(
            &json!({ "to": "a@example.com", "a": "あ".repeat(300), "b": "い".repeat(300), "text": "う".repeat(300) }),
        );
        assert_eq!(told.chars().count(), SHOWN_CHARS);
        assert!(told.ends_with('…'));
        let told = values(&json!({ "text": "あ".repeat(300), "to": "a@example.com" }));
        assert!(told.ends_with("… / a@example.com"), "{told}");
    }
}
