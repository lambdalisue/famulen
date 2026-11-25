//! What a held call is, in words the owner reads at a glance: never a tool's
//! name, which says nothing to a person.

use famulen_gateway::connections::Connection;
use famulen_gateway::{DEFINITIONS, VAULT};
use serde_json::Value;

use crate::wording;

/// How long the words of a call may be, in characters.
const SHOWN_CHARS: usize = 240;
/// And one of its values, so that a long one hides no other.
const VALUE_CHARS: usize = 100;

/// What the call does, as 「Gmail」でメールを送る. `connection` is the one it
/// is on, if it is on one.
pub(super) fn deed(on: &str, tool: &str, connection: Option<&Connection>) -> String {
    let done = wording::deed_in_words(&wording::tool_of(on, tool), connection);
    let place = match (on, connection) {
        (VAULT, _) => "金庫".into(),
        (DEFINITIONS, _) => return done.unwrap_or_else(|| "サービス定義を変える".into()),
        (_, Some(c)) => format!("「{}」", c.name),
        ("", None) => match famulen_gateway::permissions::split_tool(tool) {
            Some((crate::hub::SERVER_NAME, _)) => "Famulen ".into(),
            None if done.is_some() => return done.unwrap_or_default(),
            _ => "手元の道具".into(),
        },
        (_, None) => "つなぎ先".into(),
    };
    match done {
        Some(done) if done.starts_with(|c: char| c.is_ascii()) => format!("{place}で {done}"),
        Some(done) => format!("{place}で{done}"),
        None => format!("{place}で操作をする"),
    }
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
    use famulen_gateway::connections::{Secret, Transport};
    use famulen_gateway::{CHANGE_DEFINITION, DELETE_SECRET};
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
            (
                "create_pull_request_review",
                "「会社の Gmail」でレビューを作る",
            ),
            ("label_delete", "「会社の Gmail」で操作をする"),
            ("task_list_delete", "「会社の Gmail」で操作をする"),
            ("http_delete", "「会社の Gmail」で API を呼ぶ"),
            ("frobnicate", "「会社の Gmail」で操作をする"),
        ] {
            let mine = Connection {
                id: "gmail-work".into(),
                name: "会社の Gmail".into(),
                kind: "gmail".into(),
                transport: Transport::McpRemote { url: String::new() },
                secret: Secret::default(),
                account: String::new(),
                status: String::new(),
            };
            assert_eq!(deed("gmail-work", tool, Some(&mine)), told);
        }
    }

    #[test]
    fn what_is_not_on_a_connection_is_told_without_its_name_too() {
        for (connection, tool, told) in [
            ("", "Bash", "コマンドを実行する"),
            ("", "mcp__famulen__delete_record", "Famulen で記録を消す"),
            ("", "mcp__famulen__register_model", "Famulen で操作をする"),
            ("", "Frobnicate", "手元の道具で操作をする"),
            (VAULT, DELETE_SECRET, "金庫で秘密を消す"),
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
