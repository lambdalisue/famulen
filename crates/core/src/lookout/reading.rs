//! What is read from a source, as items.

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Item, feed};
use crate::mcp::ToolResult;

/// How much of a text an item's title holds.
const TITLE_CHARS: usize = 120;
/// How much of a tool's answer an item's body holds.
const BODY_CHARS: usize = 4000;

/// Something read: the item, and what there is to open beyond it.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub item: Item,
    pub more: Option<More>,
}

/// What an item has to open.
#[derive(Debug, Clone, PartialEq)]
pub enum More {
    /// The whole of it, read along with it.
    Text(String),
    /// The address of the page it is on.
    Page(String),
}

impl From<Item> for Reading {
    fn from(item: Item) -> Self {
        Self { item, more: None }
    }
}

/// A title for `text`: the start of its first line.
pub(crate) fn title_of(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(TITLE_CHARS)
        .collect()
}

/// What tells an item of `source` from every other.
pub(crate) fn dedup_key(source: &str, item: &Item) -> String {
    let mut hash = Sha256::new();
    hash.update(source.as_bytes());
    hash.update([0]);
    hash.update(item.title.as_bytes());
    hash.update([0]);
    hash.update(item.body.as_bytes());
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Turn a tool's answer into items: each element of a JSON array (also one
/// nested under a common key), each entry of a feed, else the whole text as
/// one item.
pub fn items_from(result: &ToolResult) -> Vec<Reading> {
    let value = result
        .structured_content
        .clone()
        .or_else(|| serde_json::from_str::<Value>(&result.text).ok());
    let array = value.as_ref().and_then(|v| match v {
        Value::Array(a) => Some(a.clone()),
        Value::Object(o) => o.values().find_map(|v| v.as_array().cloned()),
        _ => None,
    });
    match array {
        Some(elements) => elements.iter().map(|v| item_from_value(v).into()).collect(),
        None if value.is_none()
            && let Some(entries) = feed::entries(&result.text) =>
        {
            entries
        }
        None if !result.text.trim().is_empty() => vec![
            Item {
                title: title_of(&result.text),
                body: result.text.chars().take(BODY_CHARS).collect(),
            }
            .into(),
        ],
        None => Vec::new(),
    }
}

fn item_from_value(v: &Value) -> Item {
    let title = ["title", "subject", "name", "summary", "text", "snippet"]
        .iter()
        .find_map(|k| v.get(k).and_then(Value::as_str))
        .unwrap_or_default()
        .chars()
        .take(TITLE_CHARS)
        .collect();
    let body = serde_json::to_string(v)
        .unwrap_or_default()
        .chars()
        .take(BODY_CHARS)
        .collect();
    Item { title, body }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_come_from_an_array_or_the_whole_text() {
        let array = ToolResult {
            content: vec![],
            structured_content: None,
            is_error: false,
            text: r#"{"messages":[{"subject":"請求書"},{"subject":"会議"}]}"#.into(),
        };
        let items = items_from(&array);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].item.title, "請求書");
        let text = ToolResult {
            content: vec![],
            structured_content: None,
            is_error: false,
            text: "一行目\n二行目".into(),
        };
        assert_eq!(items_from(&text)[0].item.title, "一行目");
    }
}
