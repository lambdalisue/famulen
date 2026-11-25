//! One memory item: a Markdown list item with its attributes as a YAML flow
//! mapping at the end of the line.

use chrono::{NaiveDate, NaiveTime};
use serde_norway::Value;

/// Where a memory came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Src {
    /// The owner said it.
    Said,
    /// The owner confirmed it.
    Confirmed,
    /// It was inferred.
    Inferred,
}

impl Src {
    pub fn as_str(self) -> &'static str {
        match self {
            Src::Said => "said",
            Src::Confirmed => "confirmed",
            Src::Inferred => "inferred",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "said" => Some(Src::Said),
            "confirmed" => Some(Src::Confirmed),
            "inferred" => Some(Src::Inferred),
            _ => None,
        }
    }
}

/// What became of a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpisodeState {
    /// It waits to be consolidated.
    Waiting,
    /// It went into core or long-term memory at once.
    Applied,
    /// It is a fact that belongs to the records.
    Record,
}

impl EpisodeState {
    pub fn as_str(self) -> &'static str {
        match self {
            EpisodeState::Waiting => "waiting",
            EpisodeState::Applied => "applied",
            EpisodeState::Record => "record",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "waiting" => Some(EpisodeState::Waiting),
            "applied" => Some(EpisodeState::Applied),
            "record" => Some(EpisodeState::Record),
            _ => None,
        }
    }
}

/// The value of one attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attr {
    Text(String),
    List(Vec<String>),
}

impl Attr {
    /// The value as one text; a list is joined with commas.
    pub fn to_text(&self) -> String {
        match self {
            Attr::Text(t) => t.clone(),
            Attr::List(items) => items.join(","),
        }
    }
}

/// The attributes written as YAML lists.
pub const LIST_ATTRS: [&str; 2] = ["workgroup", "against"];

/// One memory item.
#[derive(Debug, Clone)]
pub struct Entry {
    indent: String,
    text: String,
    attrs: Vec<(String, Attr)>,
    /// The line as read, kept so an untouched entry is written back byte for
    /// byte even when it is not in the canonical shape.
    raw: Option<String>,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.indent == other.indent && self.text == other.text && self.attrs == other.attrs
    }
}

impl Eq for Entry {}

impl Entry {
    /// A new top-level item. Line breaks in `text` become spaces, since an
    /// item is one line.
    pub fn new(text: &str) -> Self {
        Entry::with_attrs(text, Vec::new())
    }

    pub(crate) fn with_attrs(text: &str, attrs: Vec<(String, Attr)>) -> Self {
        Entry {
            indent: String::new(),
            text: one_line(text),
            attrs,
            raw: None,
        }
    }

    /// Read one line as an item, or `None` when it is not a list item.
    pub fn parse(line: &str) -> Option<Self> {
        let trimmed = line.trim_start_matches([' ', '\t']);
        let indent = &line[..line.len() - trimmed.len()];
        let rest = trimmed.strip_prefix("- ").or_else(|| {
            let bare = trimmed.trim_end();
            (bare == "-").then_some("")
        })?;
        let (text, attrs) = split_attrs(rest);
        Some(Entry {
            indent: indent.to_string(),
            text: text.to_string(),
            attrs,
            raw: Some(line.to_string()),
        })
    }

    /// The line as it is written to the file.
    pub fn render(&self) -> String {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        let mut line = format!("{}-", self.indent);
        if !self.text.is_empty() {
            line.push(' ');
            line.push_str(&self.text);
        }
        if !self.attrs.is_empty() {
            let attrs: Vec<String> = self
                .attrs
                .iter()
                .map(|(k, v)| format!("{k}: {}", flow_value(v)))
                .collect();
            line.push_str(" {");
            line.push_str(&attrs.join(", "));
            line.push('}');
        }
        line
    }

    /// The text before the attributes, including a note's time.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = one_line(text);
        self.raw = None;
    }

    /// Every attribute in the order written.
    pub fn attrs(&self) -> &[(String, Attr)] {
        &self.attrs
    }

    /// An attribute that is one text.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| match v {
                Attr::Text(t) => Some(t.as_str()),
                Attr::List(_) => None,
            })
    }

    /// Set an attribute in place, or add it at the end when it is new. A
    /// list attribute given as text is read as a flow sequence, or split at
    /// commas.
    pub fn set(&mut self, key: &str, value: impl Into<String>) -> anyhow::Result<()> {
        let value = value.into();
        let value = if LIST_ATTRS.contains(&key) {
            Attr::List(split_list(&value))
        } else {
            Attr::Text(one_line(&value))
        };
        self.put(key, value)
    }

    pub fn set_list(&mut self, key: &str, items: Vec<String>) -> anyhow::Result<()> {
        self.put(key, Attr::List(items))
    }

    fn put(&mut self, key: &str, value: Attr) -> anyhow::Result<()> {
        anyhow::ensure!(is_key(key), "invalid attribute key: {key:?}");
        match self.attrs.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => self.attrs.push((key.to_string(), value)),
        }
        self.raw = None;
        Ok(())
    }

    /// Builder form of [`Entry::set`].
    pub fn with(mut self, key: &str, value: impl Into<String>) -> anyhow::Result<Self> {
        self.set(key, value)?;
        Ok(self)
    }

    pub fn remove_attr(&mut self, key: &str) -> Option<Attr> {
        let at = self.attrs.iter().position(|(k, _)| k == key)?;
        self.raw = None;
        Some(self.attrs.remove(at).1)
    }

    pub fn id(&self) -> Option<&str> {
        self.get("id")
    }

    pub fn src(&self) -> Option<Src> {
        self.get("src").and_then(Src::parse)
    }

    pub fn from(&self) -> Option<NaiveDate> {
        self.date("from")
    }

    pub fn to(&self) -> Option<NaiveDate> {
        self.date("to")
    }

    /// The number of separate pieces of evidence.
    pub fn ev(&self) -> Option<u32> {
        self.number("ev")
    }

    /// The workgroups it applies to; empty means every workgroup.
    pub fn workgroup(&self) -> Vec<String> {
        self.list("workgroup")
    }

    /// A note's leading `HH:MM`.
    pub fn time(&self) -> Option<NaiveTime> {
        let head = self.text.get(..5)?;
        let after = &self.text[5..];
        if !(after.is_empty() || after.starts_with(' ')) {
            return None;
        }
        NaiveTime::parse_from_str(head, "%H:%M").ok()
    }

    /// The text without a leading `HH:MM`.
    pub fn body(&self) -> &str {
        match self.time() {
            Some(_) => self.text[5..].trim_start(),
            None => &self.text,
        }
    }

    pub fn state(&self) -> Option<EpisodeState> {
        self.get("state").and_then(EpisodeState::parse)
    }

    /// How many pieces of evidence a pending judgement needs.
    pub fn need(&self) -> Option<u32> {
        self.number("need")
    }

    /// The ids of the memories a pending judgement contradicts.
    pub fn against(&self) -> Vec<String> {
        self.list("against")
    }

    fn date(&self, key: &str) -> Option<NaiveDate> {
        self.get(key)
            .and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok())
    }

    fn number(&self, key: &str) -> Option<u32> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    /// An attribute as a list; a text is split at commas.
    pub fn list(&self, key: &str) -> Vec<String> {
        match self.attrs.iter().find(|(k, _)| k == key) {
            Some((_, Attr::List(items))) => items.clone(),
            Some((_, Attr::Text(t))) => split_list(t),
            None => Vec::new(),
        }
    }
}

fn split_list(text: &str) -> Vec<String> {
    if text.trim_start().starts_with('[')
        && let Ok(items) = serde_norway::from_str::<Vec<String>>(text)
    {
        return items;
    }
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn one_line(text: &str) -> String {
    text.split(['\r', '\n'])
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A value written inside a flow mapping: plain when YAML reads it back as
/// the same text, double-quoted otherwise.
fn flow_value(value: &Attr) -> String {
    match value {
        Attr::Text(t) => flow_scalar(t),
        Attr::List(items) => {
            let items: Vec<String> = items.iter().map(|i| flow_scalar(i)).collect();
            format!("[{}]", items.join(", "))
        }
    }
}

fn flow_scalar(text: &str) -> String {
    let plain_chars = !text.is_empty()
        && text.trim() == text
        && !text.contains([',', '[', ']', '{', '}', ':', '#', '"', '\''])
        && !text.starts_with(['-', '?', '!', '&', '*', '|', '>', '%', '@', '`']);
    let reads_back = || {
        serde_norway::from_str::<Value>(&format!("[{text}]"))
            .ok()
            .and_then(|v| match v {
                Value::Sequence(mut items) if items.len() == 1 => scalar(&items.remove(0)),
                _ => None,
            })
            .is_some_and(|back| back == text)
    };
    if plain_chars && reads_back() {
        text.to_owned()
    } else {
        serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""))
    }
}

/// A YAML scalar as text; anything else is not a scalar.
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A YAML value as an attribute: a scalar or a list of scalars.
pub(crate) fn attr_of(value: &Value) -> Option<Attr> {
    match value {
        Value::Sequence(items) => items
            .iter()
            .map(scalar)
            .collect::<Option<Vec<_>>>()
            .map(Attr::List),
        other => scalar(other).map(Attr::Text),
    }
}

/// The attributes of a `{…}` group, when it is a YAML mapping of attribute
/// keys to scalars or lists of scalars.
fn mapping(group: &str) -> Option<Vec<(String, Attr)>> {
    let Value::Mapping(map) = serde_norway::from_str::<Value>(group).ok()? else {
        return None;
    };
    map.iter()
        .map(|(k, v)| {
            let k = k.as_str().filter(|k| is_key(k))?;
            Some((k.to_owned(), attr_of(v)?))
        })
        .collect::<Option<Vec<_>>>()
        .filter(|attrs| !attrs.is_empty())
}

/// Split `text {k: v, …}` into the text and the attributes. A trailing
/// brace group that is not a YAML mapping of attributes stays part of the
/// text.
fn split_attrs(rest: &str) -> (&str, Vec<(String, Attr)>) {
    let trimmed = rest.trim_end();
    let none = (trimmed, Vec::new());
    if !trimmed.ends_with('}') {
        return none;
    }
    for (open, _) in trimmed
        .match_indices('{')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let before = &trimmed[..open];
        if !(before.is_empty() || before.ends_with(' ')) {
            continue;
        }
        if let Some(attrs) = mapping(&trimmed[open..]) {
            return (before.trim_end(), attrs);
        }
    }
    none
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_with_attributes_reads_its_typed_values() {
        let line = "- 返事は短く、要点を先に {id: m1, src: said, from: 2026-09-03, ev: 2, workgroup: [work, personal]}";

        let entry = Entry::parse(line).unwrap();

        assert_eq!(entry.text(), "返事は短く、要点を先に");
        assert_eq!(entry.id(), Some("m1"));
        assert_eq!(entry.src(), Some(Src::Said));
        assert_eq!(entry.from(), NaiveDate::from_ymd_opt(2026, 9, 3));
        assert_eq!(entry.to(), None);
        assert_eq!(entry.ev(), Some(2));
        assert_eq!(entry.workgroup(), vec!["work", "personal"]);
    }

    #[test]
    fn a_list_attribute_is_set_from_commas_or_from_a_flow_sequence() {
        for text in ["work, personal", "[work, personal]", "[\"work\", personal]"] {
            let mut entry = Entry::new("text");

            entry.set("workgroup", text).unwrap();

            assert_eq!(entry.workgroup(), ["work", "personal"], "{text}");
        }
        let mut entry = Entry::new("text");
        entry.set("workgroup", "[]").unwrap();
        assert!(entry.workgroup().is_empty());
    }

    #[test]
    fn an_untouched_item_is_written_back_byte_for_byte() {
        let line = "  - spaced   text   {b:   2,a: 1}  ";

        assert_eq!(Entry::parse(line).unwrap().render(), line);
    }

    #[test]
    fn unknown_attributes_survive_an_edit_in_their_order() {
        let mut entry = Entry::parse("- text {zeta: 1, id: m2, alpha: x}").unwrap();

        entry.set("ev", "3").unwrap();
        entry.set("zeta", "2").unwrap();

        assert_eq!(entry.render(), "- text {zeta: 2, id: m2, alpha: x, ev: 3}");
    }

    #[test]
    fn a_trailing_brace_group_that_is_not_attributes_stays_in_the_text() {
        for line in [
            "- set {x} in code",
            "- use {not attrs}",
            "- word{a: 1}",
            "- not YAML {id=m1 src=said}",
            "- nested {a: {b: 1}}",
            "- see {x}",
        ] {
            let entry = Entry::parse(line).unwrap();
            assert!(entry.attrs().is_empty(), "{line}");
            assert_eq!(entry.render(), line);
        }
    }

    #[test]
    fn braces_inside_the_text_do_not_hide_the_attributes() {
        let entry = Entry::parse(r#"- write {x} here {id: m1, reason: "a {b} c"}"#).unwrap();

        assert_eq!(entry.text(), "write {x} here");
        assert_eq!(entry.get("reason"), Some("a {b} c"));
    }

    #[test]
    fn a_value_with_spaces_or_symbols_is_quoted_and_reads_back_the_same() {
        let mut entry = Entry::new("text");
        for value in [
            "two words",
            "a}b",
            "key: value",
            "null",
            "1e3",
            "- dash",
            "\"quoted\"",
            "",
        ] {
            entry.set("reason", value).unwrap();
            let again = Entry::parse(&entry.render()).unwrap();
            assert_eq!(again.get("reason"), Some(value), "{}", entry.render());
        }
        entry.set("reason", "plain").unwrap();
        assert_eq!(entry.render(), "- text {reason: plain}");
        assert!(entry.set("bad key", "x").is_err());
    }

    #[test]
    fn a_new_item_is_one_line_with_its_attributes_at_the_end() {
        let entry = Entry::new("first\nsecond")
            .with("id", "m3")
            .unwrap()
            .with("workgroup", "work,personal")
            .unwrap();

        assert_eq!(
            entry.render(),
            "- first second {id: m3, workgroup: [work, personal]}"
        );
        assert_eq!(Entry::parse(&entry.render()).unwrap(), entry);
    }

    #[test]
    fn an_episodic_item_reads_its_time_body_and_state() {
        let entry =
            Entry::parse("- 09:30 会議は午後が良いらしい {id: h1, state: waiting}").unwrap();

        assert_eq!(entry.time(), NaiveTime::from_hms_opt(9, 30, 0));
        assert_eq!(entry.body(), "会議は午後が良いらしい");
        assert_eq!(entry.state(), Some(EpisodeState::Waiting));
    }

    #[test]
    fn text_that_only_starts_with_digits_has_no_time() {
        let entry = Entry::parse("- 12:345 is not a time").unwrap();

        assert_eq!(entry.time(), None);
        assert_eq!(entry.body(), "12:345 is not a time");
    }

    #[test]
    fn a_pending_item_reads_what_it_needs_and_what_it_contradicts() {
        let entry =
            Entry::parse("- 朝型かもしれない {id: p1, src: inferred, need: 2, against: [m4, m9]}")
                .unwrap();

        assert_eq!(entry.src(), Some(Src::Inferred));
        assert_eq!(entry.need(), Some(2));
        assert_eq!(entry.against(), vec!["m4", "m9"]);
    }
}
