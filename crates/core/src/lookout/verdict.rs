//! What the lookout's model is asked, and how its answer is read.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Item, Outcome};

/// How soon an item needs the owner, as the model judges it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Urgency {
    Now,
    Today,
    Later,
}

impl Urgency {
    pub fn label(self) -> &'static str {
        match self {
            Self::Now => "急ぐ",
            Self::Today => "今日のうち",
            Self::Later => "急がない",
        }
    }
}

/// What the model answers for one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screened {
    pub related: bool,
    pub urgency: Urgency,
}

impl Screened {
    /// An item the model left out counts as related: a missed item costs more
    /// than a line in the digest.
    pub const UNJUDGED: Self = Self {
        related: true,
        urgency: Urgency::Later,
    };
}

/// Related items the model put together as one subject, and where they go.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    /// Indices into the batch the model was given.
    pub items: Vec<usize>,
    pub outcome: Outcome,
    #[serde(default)]
    pub title: String,
    pub reason: String,
}

/// The model's one answer for a batch: every item judged, and the related
/// ones grouped.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub items: Vec<Screened>,
    pub groups: Vec<Group>,
}

pub fn verdict_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "index": { "type": "integer" },
                        "related": { "type": "boolean" },
                        "urgency": { "type": "string", "enum": ["now", "today", "later"] }
                    },
                    "required": ["index", "related", "urgency"],
                    "additionalProperties": false
                }
            },
            "groups": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "type": "integer" } },
                        "outcome": { "type": "string", "enum": ["act", "notify", "digest"] },
                        "title": { "type": "string" },
                        "reason": { "type": "string" }
                    },
                    "required": ["items", "outcome", "title", "reason"],
                    "additionalProperties": false
                }
            },
            "purchases": { "type": "array", "items": { "type": "integer" } },
            "contracts": { "type": "array", "items": { "type": "integer" } }
        },
        "required": ["items", "groups", "purchases", "contracts"],
        "additionalProperties": false
    })
}

/// Read the model's answer for a batch of `count` items.
pub fn verdict_from(answer: &Value, count: usize) -> Verdict {
    let mut items = vec![Screened::UNJUDGED; count];
    for v in answer
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(i) = v.get("index").and_then(Value::as_u64).map(|i| i as usize) else {
            continue;
        };
        if i >= count {
            continue;
        }
        let related = v.get("related").and_then(Value::as_bool).unwrap_or(true);
        items[i].related = related;
        items[i].urgency = v
            .get("urgency")
            .cloned()
            .and_then(|u| serde_json::from_value(u).ok())
            .filter(|_| related)
            .unwrap_or(Urgency::Later);
    }
    let groups = answer
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| serde_json::from_value(g.clone()).ok())
        .collect();
    Verdict { items, groups }
}

pub fn skim_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "read": { "type": "array", "items": { "type": "integer" } } },
        "required": ["read"],
        "additionalProperties": false
    })
}

/// Which items the skim picked to read in full.
pub fn picked_from(answer: &Value) -> anyhow::Result<Vec<usize>> {
    Ok(serde_json::from_value(answer["read"].clone())?)
}

/// The items the model reads, numbered.
pub fn numbered(items: &[Item]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| format!("[{i}] {}\n{}", item.title, item.body))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookout::tests::{group, related, unrelated};

    #[test]
    fn the_answer_is_read_and_a_missing_item_counts_as_related() {
        let v = verdict_from(
            &json!({
                "items": [{ "index": 1, "related": false, "urgency": "now" }],
                "groups": [{ "items": [0], "outcome": "notify", "title": "t", "reason": "r" }]
            }),
            2,
        );
        assert_eq!(v.items, vec![related(Urgency::Later), unrelated()]);
        assert_eq!(v.groups, vec![group(&[0], Outcome::Notify, "t", "r")]);
    }

    #[test]
    fn a_group_goes_to_the_secretary_or_the_digest_and_nowhere_else() {
        let schema = verdict_schema();
        assert_eq!(
            schema["required"],
            json!(["items", "groups", "purchases", "contracts"])
        );
        assert_eq!(
            schema["properties"]["groups"]["items"]["properties"]["outcome"]["enum"],
            json!(["act", "notify", "digest"])
        );
    }

    #[test]
    fn the_skims_answer_is_the_indices_to_read_and_anything_else_fails() {
        assert_eq!(picked_from(&json!({ "read": [2, 0] })).unwrap(), [2, 0]);
        assert!(picked_from(&json!({})).is_err());
        assert!(picked_from(&json!({ "read": ["a"] })).is_err());
    }
}
