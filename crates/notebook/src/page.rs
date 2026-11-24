//! One Markdown file of the notebook: YAML frontmatter for the fixed fields,
//! then free text. Fields the tools do not know are kept as they are, in
//! their order.

use std::path::Path;

use anyhow::Context as _;
use serde_norway::{Mapping, Value};

const FENCE: &str = "---";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Page {
    fields: Mapping,
    /// The frontmatter as written. A change rewrites only its own lines, so
    /// the rest stays as someone wrote it by hand.
    yaml: String,
    body: String,
}

impl Page {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a file's text. Text without a frontmatter block is all body.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let Some((yaml, body)) = split(text) else {
            return Ok(Self {
                fields: Mapping::new(),
                yaml: String::new(),
                body: text.to_owned(),
            });
        };
        let fields =
            match serde_norway::from_str::<Value>(yaml).context("reading the frontmatter")? {
                Value::Mapping(fields) => fields,
                Value::Null => Mapping::new(),
                _ => anyhow::bail!("the frontmatter is not a set of fields"),
            };
        Ok(Self {
            fields,
            yaml: yaml.replace("\r\n", "\n"),
            body: body.to_owned(),
        })
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("reading {}", path.display()))
    }

    pub fn render(&self) -> String {
        if self.fields.is_empty() {
            return self.body.clone();
        }
        let mut text = format!("{FENCE}\n{}{FENCE}\n", self.yaml);
        if !self.body.is_empty() {
            if !self.body.starts_with(['\n', '\r']) {
                text.push('\n');
            }
            text.push_str(&self.body);
        }
        text
    }

    /// Write one field's lines anew, or drop them.
    fn write_yaml(&mut self, name: &str, value: Option<&Value>) {
        let lines = value.map(|v| yaml_lines(name, v)).unwrap_or_default();
        let mut out = String::new();
        let mut written = false;
        let mut inside = false;
        for line in self.yaml.split_inclusive('\n') {
            let top = !line.starts_with([' ', '\t', '-', '#', '\n']);
            if top {
                inside = line
                    .split_once(':')
                    .is_some_and(|(k, _)| k.trim().trim_matches(['"', '\'']) == name);
                if inside {
                    out.push_str(&lines);
                    written = true;
                }
            }
            if !inside || line.starts_with(['#', '\n']) {
                out.push_str(line);
            }
        }
        if !written {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&lines);
        }
        self.yaml = out;
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.render()).with_context(|| format!("writing {}", path.display()))
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn set_body(&mut self, body: impl Into<String>) {
        self.body = body.into();
    }

    /// A field as text: strings as they are, numbers and booleans written
    /// out, anything else absent.
    pub fn text(&self, key: &str) -> Option<String> {
        match self.fields.get(key)? {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
        .filter(|s| !s.trim().is_empty())
    }

    /// A field as a list of text. A single value is a list of one, and a
    /// comma-separated string is split.
    pub fn list(&self, key: &str) -> Vec<String> {
        let scalar = |v: &Value| match v {
            Value::String(s) => Some(s.trim().to_owned()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        };
        match self.fields.get(key) {
            Some(Value::Sequence(items)) => items.iter().filter_map(scalar).collect(),
            Some(Value::String(s)) => s.split(',').map(|s| s.trim().to_owned()).collect(),
            Some(other) => scalar(other).into_iter().collect(),
            None => Vec::new(),
        }
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect()
    }

    pub fn value(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }

    /// Every field whose name is text, in the order written.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.fields
            .iter()
            .filter_map(|(k, v)| Some((k.as_str()?, v)))
    }

    /// Set a field in place, or add it at the end when it is new.
    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        let value = value.into();
        if self.fields.get(key) == Some(&value) {
            return;
        }
        self.write_yaml(key, Some(&value));
        self.fields.insert(Value::String(key.to_owned()), value);
    }

    /// Set a field, or remove it when there is no value.
    pub fn set_opt(&mut self, key: &str, value: Option<impl Into<Value>>) {
        match value {
            Some(value) => self.set(key, value),
            None => self.remove(key),
        }
    }

    /// Set a list field, or remove it when the list is empty.
    pub fn set_list(&mut self, key: &str, items: &[String]) {
        if items.is_empty() {
            self.remove(key);
        } else {
            let items: Vec<Value> = items.iter().cloned().map(Value::String).collect();
            self.set(key, Value::Sequence(items));
        }
    }

    pub fn remove(&mut self, key: &str) {
        if self.fields.remove(key).is_some() {
            self.write_yaml(key, None);
        }
    }
}

/// One field as YAML lines. A list of plain values is written `[a, b]`.
fn yaml_lines(name: &str, value: &Value) -> String {
    let one = |value: &Value| {
        serde_norway::to_string(value)
            .unwrap_or_default()
            .trim_end()
            .to_owned()
    };
    let key = one(&Value::String(name.to_owned()));
    if let Value::Sequence(items) = value {
        let items: Vec<String> = items.iter().map(one).collect();
        let plain = items
            .iter()
            .all(|i| !i.contains(['\n', ',', '[', ']', '{', '}']));
        if plain {
            return format!("{key}: [{}]\n", items.join(", "));
        }
    }
    let mut map = Mapping::new();
    map.insert(Value::String(name.to_owned()), value.clone());
    serde_norway::to_string(&map).unwrap_or_default()
}

/// Split `---\n<yaml>\n---\n<body>` into the YAML and the body.
pub(crate) fn split(text: &str) -> Option<(&str, &str)> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == FENCE {
            return Some((&rest[..at], &rest[at + line.len()..]));
        }
        at += line.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_and_body_are_read_and_unknown_fields_survive_an_edit_in_their_order() {
        let text = "---\nid: t42\nzeta: keep me\nassignee: secretary\nworkgroup:\n- work\nextra:\n  nested: 1\n---\n\n# 見積もりの返事\n\n田中さんから。\n";
        let mut page = Page::parse(text).unwrap();

        assert_eq!(page.text("id").as_deref(), Some("t42"));
        assert_eq!(page.list("workgroup"), ["work"]);
        page.set("assignee", "owner");
        page.set("due", "2026-11-28");

        let again = Page::parse(&page.render()).unwrap();
        assert_eq!(again.text("zeta").as_deref(), Some("keep me"));
        assert_eq!(again.text("assignee").as_deref(), Some("owner"));
        assert!(again.value("extra").is_some());
        let keys: Vec<String> = again
            .fields
            .keys()
            .filter_map(|k| k.as_str().map(str::to_owned))
            .collect();
        assert_eq!(
            keys,
            ["id", "zeta", "assignee", "workgroup", "extra", "due"]
        );
        assert_eq!(again.body(), "\n# 見積もりの返事\n\n田中さんから。\n");
    }

    #[test]
    fn changing_one_field_leaves_every_other_line_as_it_was() {
        let text = "---\n# mine\nid: 'a'\nworkgroup: [work,  home]\nsync:\n  to: x:y\n  id: 1\nitems:\n- a\n- b\nlast: \"z\"\n---\n\n\n# 題\n";
        let mut page = Page::parse(text).unwrap();

        page.set("id", "b");
        page.remove("items");
        page.set_list("labels", &["x".to_owned(), "y: z".to_owned()]);

        assert_eq!(
            page.render(),
            "---\n# mine\nid: b\nworkgroup: [work,  home]\nsync:\n  to: x:y\n  id: 1\nlast: \"z\"\nlabels: [x, 'y: z']\n---\n\n\n# 題\n"
        );
        page.set("sync", "gone");
        assert!(page.render().contains("\nsync: gone\nlast:"));
        assert_eq!(Page::parse(&page.render()).unwrap(), page);
    }

    #[test]
    fn text_without_frontmatter_is_all_body() {
        let page = Page::parse("# メモ\n\n本文\n").unwrap();

        assert_eq!(page.text("id"), None);
        assert_eq!(page.render(), "# メモ\n\n本文\n");
    }

    #[test]
    fn a_list_field_may_be_written_as_one_value_or_comma_separated() {
        let page = Page::parse("---\na: work\nb: work, personal\nc: [x, y]\n---\n").unwrap();

        assert_eq!(page.list("a"), ["work"]);
        assert_eq!(page.list("b"), ["work", "personal"]);
        assert_eq!(page.list("c"), ["x", "y"]);
        assert!(page.list("missing").is_empty());
    }

    #[test]
    fn frontmatter_that_is_not_fields_is_an_error() {
        assert!(Page::parse("---\n- a\n- b\n---\n").is_err());
        assert!(Page::parse("---\na: [\n---\n").is_err());
    }
}
