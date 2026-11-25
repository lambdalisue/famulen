//! Embedding values in a service definition's strings: `{account.email}`,
//! `{args.channel}`, `{/team/id}`. Embedding only replaces; it neither
//! computes nor compares, so a definition stays a list of facts.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

/// What a definition's strings may be filled with at one place.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub account: BTreeMap<String, String>,
    pub app: BTreeMap<String, String>,
    /// Names of `app` values that are secret.
    pub app_secrets: Vec<String>,
    /// The tokens by name; the first one is `{token}`.
    pub tokens: BTreeMap<String, String>,
    pub first_token: Option<String>,
    pub args: Value,
    pub headers: BTreeMap<String, String>,
    pub body: Option<String>,
    /// What `{/…}` points into.
    pub doc: Value,
    /// Whether secrets may be filled in: only inside a request going out
    /// and inside the check of what arrived.
    pub secrets: bool,
    /// Whether what is filled goes only to the public internet.
    pub public: bool,
}

impl Context {
    /// Only what `{/…}` points into.
    pub fn of(doc: Value) -> Self {
        Self {
            doc,
            ..Self::default()
        }
    }

    /// The value `{name}` stands for, if there is one.
    fn lookup(&self, name: &str) -> Option<Value> {
        if let Some(pointer) = name.strip_prefix('/') {
            return self.doc.pointer(&format!("/{pointer}")).cloned();
        }
        if name == "body" {
            return self.body.clone().map(Value::String);
        }
        if name == "token" {
            if !self.secrets {
                return None;
            }
            let first = self.first_token.as_ref()?;
            return self.tokens.get(first).cloned().map(Value::String);
        }
        let (scope, key) = name.split_once('.')?;
        match scope {
            "account" => self.account.get(key).cloned().map(Value::String),
            "app" => {
                if self.app_secrets.iter().any(|s| s == key) && !self.secrets {
                    return None;
                }
                self.app.get(key).cloned().map(Value::String)
            }
            "token" => self
                .secrets
                .then(|| self.tokens.get(key).cloned().map(Value::String))
                .flatten(),
            "args" => self.args.get(key).cloned().filter(|v| !v.is_null()),
            "header" => self
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| Value::String(v.clone())),
            _ => None,
        }
    }

    /// Fill one string. `None` when something it embeds has no value.
    pub fn render(&self, text: &str) -> Option<Value> {
        let parts = split(text);
        if let [Part::Embed(name)] = parts.as_slice() {
            return self.lookup(name);
        }
        let mut out = String::new();
        for part in parts {
            match part {
                Part::Text(t) => out.push_str(&t),
                Part::Embed(name) => match self.lookup(&name)? {
                    Value::String(s) => out.push_str(&s),
                    other => out.push_str(&other.to_string()),
                },
            }
        }
        Some(Value::String(out))
    }

    /// Fill one string, as text.
    pub fn render_text(&self, text: &str) -> Option<String> {
        self.render(text).as_ref().map(text_of)
    }

    /// Fill every string in a value; a field whose string has nothing to
    /// embed is left out.
    pub fn render_value(&self, value: &Value) -> Option<Value> {
        match value {
            Value::String(s) => self.render(s),
            Value::Array(items) => Some(Value::Array(
                items.iter().filter_map(|v| self.render_value(v)).collect(),
            )),
            Value::Object(map) => {
                let mut out = Map::new();
                for (k, v) in map {
                    if let Some(v) = self.render_value(v) {
                        out.insert(k.clone(), v);
                    }
                }
                Some(Value::Object(out))
            }
            other => Some(other.clone()),
        }
    }

    /// Fill a table of strings, leaving out what has nothing to embed.
    pub fn render_map(&self, map: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        map.iter()
            .filter_map(|(k, v)| Some((k.clone(), self.render_text(v)?)))
            .collect()
    }
}

/// A value as text: a string as it is, anything else as JSON.
pub(crate) fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[derive(Debug, PartialEq)]
enum Part {
    Text(String),
    Embed(String),
}

fn split(text: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut text_buf = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text_buf.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text_buf.push('}');
            }
            '{' => {
                let mut name = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    name.push(c);
                }
                if !text_buf.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text_buf)));
                }
                parts.push(Part::Embed(name));
            }
            c => text_buf.push(c),
        }
    }
    if !text_buf.is_empty() {
        parts.push(Part::Text(text_buf));
    }
    parts
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn context() -> Context {
        Context {
            account: BTreeMap::from([("email".into(), "me@example.com".into())]),
            app: BTreeMap::from([
                ("client_id".into(), "cid".into()),
                ("signing_secret".into(), "shh".into()),
            ]),
            app_secrets: vec!["signing_secret".into()],
            tokens: BTreeMap::from([("bot".into(), "xoxb-1".into())]),
            first_token: Some("bot".into()),
            args: json!({ "channel": "C1", "limit": 5 }),
            headers: BTreeMap::from([("X-Sig".into(), "abc".into())]),
            body: Some("{\"a\":1}".into()),
            doc: json!({ "team": { "id": "T1" } }),
            secrets: false,
            public: false,
        }
    }

    #[test]
    fn embeds_are_replaced_and_a_lone_embed_keeps_its_type() {
        let c = context();
        assert_eq!(
            c.render("to {account.email}"),
            Some(json!("to me@example.com"))
        );
        assert_eq!(c.render("{args.limit}"), Some(json!(5)));
        assert_eq!(c.render("{/team/id}"), Some(json!("T1")));
        assert_eq!(c.render("{header.x-sig}"), Some(json!("abc")));
        assert_eq!(c.render("{{literal}}"), Some(json!("{literal}")));
    }

    #[test]
    fn a_missing_value_drops_the_field() {
        let c = context();
        assert_eq!(c.render("{args.thread}"), None);
        let body = c
            .render_value(&json!({ "channel": "{args.channel}", "thread_ts": "{args.thread}" }))
            .unwrap();
        assert_eq!(body, json!({ "channel": "C1" }));
    }

    #[test]
    fn secrets_are_filled_only_where_allowed() {
        let mut c = context();
        assert_eq!(c.render("Bearer {token}"), None);
        assert_eq!(c.render("{app.signing_secret}"), None);
        assert_eq!(c.render("{app.client_id}"), Some(json!("cid")));
        c.secrets = true;
        assert_eq!(c.render("Bearer {token}"), Some(json!("Bearer xoxb-1")));
        assert_eq!(c.render("{token.bot}"), Some(json!("xoxb-1")));
        assert_eq!(c.render("{app.signing_secret}"), Some(json!("shh")));
    }
}
