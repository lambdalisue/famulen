//! What a kind's README declares: its fields, its sections and what they
//! may hold.

use anyhow::Context as _;
use serde_norway::{Mapping, Value};

use super::record_of;
use crate::body::{COMMON_SECTIONS, Item, Layout, RELATED, same, split_commas};
use crate::{Page, STANDARD_KINDS, link, link_target};

/// What a declared field may hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldType {
    Text,
    Number,
    /// `yes` or `no`.
    Flag,
    /// `YYYY-MM-DD`.
    Date,
    /// `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`.
    DateOrMinute,
    /// An amount and its currency: `398000 JPY`.
    Money,
    Url,
    List,
    /// A wiki link, to a record of a kind when one is named.
    Link(Option<String>),
    Links(Option<String>),
    Any,
    /// One of these values.
    OneOf(Vec<String>),
}

impl FieldType {
    fn parse(value: &Value) -> anyhow::Result<Self> {
        let name = match value {
            Value::Sequence(items) => {
                return Ok(FieldType::OneOf(
                    items
                        .iter()
                        .map(|v| v.as_str().map(str::to_owned).context("a choice is text"))
                        .collect::<anyhow::Result<_>>()?,
                ));
            }
            Value::String(s) => s.as_str(),
            _ => anyhow::bail!("a field type is a name or a list of choices"),
        };
        let (name, of) = match name.split_once(':') {
            Some((name, of)) => (name.trim(), Some(of.trim().to_owned())),
            None => (name, None),
        };
        anyhow::ensure!(
            of.is_none() || matches!(name, "link" | "links"),
            "only a link names a kind"
        );
        Ok(match name {
            "text" => FieldType::Text,
            "number" => FieldType::Number,
            "flag" => FieldType::Flag,
            "date" => FieldType::Date,
            "date-or-minute" => FieldType::DateOrMinute,
            "money" => FieldType::Money,
            "url" => FieldType::Url,
            "list" => FieldType::List,
            "link" => FieldType::Link(of),
            "links" => FieldType::Links(of),
            "any" => FieldType::Any,
            other => anyhow::bail!("unknown field type: {other}"),
        })
    }

    /// Why a field's item does not fit, or `None` when it does.
    pub fn misfit(&self, item: &Item) -> Option<String> {
        match self {
            FieldType::Links(kind) => item
                .values()
                .iter()
                .find_map(|v| FieldType::Link(kind.clone()).misfit_text(v)),
            _ => self.misfit_text(&item.value()),
        }
    }

    /// Why a value does not fit, or `None` when it does. An empty value is
    /// one not yet given.
    pub fn misfit_text(&self, text: &str) -> Option<String> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let fits = match self {
            FieldType::Any | FieldType::Text | FieldType::List => true,
            FieldType::Number => text.parse::<f64>().is_ok(),
            FieldType::Flag => flag(text).is_some(),
            FieldType::Date => is_date(text),
            FieldType::DateOrMinute => is_date(text) || is_minute(text),
            FieldType::Money => is_money(text),
            FieldType::Url => is_url(text),
            FieldType::Link(kind) => is_link(text, kind.as_deref()),
            FieldType::Links(kind) => split_commas(text)
                .iter()
                .all(|t| is_link(t, kind.as_deref())),
            FieldType::OneOf(choices) => choices.iter().any(|c| c == text),
        };
        (!fits).then(|| format!("expected {}, not {text:?}", self.describe()))
    }

    fn describe(&self) -> String {
        match self {
            FieldType::Text => "text".into(),
            FieldType::Number => "a number".into(),
            FieldType::Flag => "yes or no".into(),
            FieldType::Date => "YYYY-MM-DD".into(),
            FieldType::DateOrMinute => "YYYY-MM-DD or YYYY-MM-DD HH:MM".into(),
            FieldType::Money => "an amount and a currency, such as 1200 JPY".into(),
            FieldType::Url => "a URL".into(),
            FieldType::List => "a list".into(),
            FieldType::Link(None) => "a wiki link".into(),
            FieldType::Link(Some(kind)) => format!("a wiki link to a record of {kind}"),
            FieldType::Links(None) => "wiki links".into(),
            FieldType::Links(Some(kind)) => format!("wiki links to records of {kind}"),
            FieldType::Any => "anything".into(),
            FieldType::OneOf(choices) => format!("one of {}", choices.join(", ")),
        }
    }
}

/// `yes` or `no`, as written.
pub fn flag(text: &str) -> Option<bool> {
    match text.trim().to_lowercase().as_str() {
        "yes" | "true" => Some(true),
        "no" | "false" => Some(false),
        _ => None,
    }
}

pub(crate) fn is_date(text: &str) -> bool {
    text.len() == 10 && chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
}

fn is_minute(text: &str) -> bool {
    minute(text).is_some()
}

/// A time of the day, `YYYY-MM-DD HH:MM`. A `T` between is read too.
pub fn minute(text: &str) -> Option<chrono::NaiveDateTime> {
    let text = text.trim();
    (text.len() == 16)
        .then(|| {
            ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M"]
                .iter()
                .find_map(|f| chrono::NaiveDateTime::parse_from_str(text, f).ok())
        })
        .flatten()
}

fn is_money(text: &str) -> bool {
    text.split_once(' ').is_some_and(|(amount, currency)| {
        amount.replace(',', "").parse::<f64>().is_ok()
            && currency.len() == 3
            && currency.chars().all(|c| c.is_ascii_uppercase())
    })
}

fn is_url(text: &str) -> bool {
    let text = text.trim_start_matches('<').trim_end_matches('>');
    ["http://", "https://"]
        .iter()
        .any(|s| text.strip_prefix(s).is_some_and(|rest| !rest.is_empty()))
        && !text.contains(char::is_whitespace)
}

fn is_link(text: &str, kind: Option<&str>) -> bool {
    let text = text.trim();
    link::is_link(text)
        && kind.is_none_or(|kind| record_of(link_target(text)).is_some_and(|(k, _)| k == kind))
}

/// What a declared section holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionType {
    /// One set of fields.
    Fields(Vec<(String, FieldType)>),
    /// Items, whose text is the first field's value.
    Items(Vec<(String, FieldType)>),
}

impl SectionType {
    fn parse(value: &Value) -> anyhow::Result<Self> {
        match value {
            Value::Mapping(fields) => Ok(SectionType::Fields(parse_fields(fields)?)),
            Value::Sequence(items) => match &items[..] {
                [Value::Mapping(fields)] if !fields.is_empty() => {
                    Ok(SectionType::Items(parse_fields(fields)?))
                }
                _ => anyhow::bail!("a section of items is a list of one set of fields"),
            },
            _ => anyhow::bail!("a section is a set of fields, or a list of one"),
        }
    }
}

fn parse_fields(fields: &Mapping) -> anyhow::Result<Vec<(String, FieldType)>> {
    fields
        .iter()
        .map(|(key, value)| {
            let key = key.as_str().context("a field name is text")?;
            let ty = FieldType::parse(value).with_context(|| format!("field {key}"))?;
            Ok((key.to_owned(), ty))
        })
        .collect()
}

/// A kind of record, as its directory and README declare it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Kind {
    pub name: String,
    /// The word its codes start with.
    pub code: String,
    /// The fields under the title, in their order.
    pub fields: Vec<(String, FieldType)>,
    pub sections: Vec<(String, SectionType)>,
    pub layout: Layout,
}

impl Kind {
    /// A kind read from its README's frontmatter; a kind without a README or
    /// a declaration has no fields of its own. A standard kind also has its
    /// standard fields and sections its README does not declare.
    pub fn declared(name: &str, readme: Option<&Page>) -> anyhow::Result<Self> {
        let mut kind = Self::written(name, readme)?;
        if let Some((_, text)) = STANDARD_KINDS.iter().find(|(n, _)| *n == name)
            && !kind.layout.free
        {
            let standard = Self::written(name, Some(&Page::parse(text)?))?;
            for (field, ty) in standard.fields {
                if kind.field(&field).is_none() {
                    kind.fields.push((field, ty));
                }
            }
            for (section, ty) in standard.sections {
                if kind.section(&section).is_none() {
                    kind.layout.sections.push(section.clone());
                    kind.sections.push((section, ty));
                }
            }
        }
        Ok(kind)
    }

    fn written(name: &str, readme: Option<&Page>) -> anyhow::Result<Self> {
        let code = readme
            .and_then(|r| r.text("code"))
            .unwrap_or_else(|| name.to_owned());
        let fields = match readme.and_then(|r| r.value("fields")) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Mapping(fields)) => parse_fields(fields)?,
            Some(_) => anyhow::bail!("fields is a set of fields"),
        };
        let sections = match readme.and_then(|r| r.value("sections")) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Mapping(declared)) => declared
                .iter()
                .map(|(key, value)| {
                    let key = key.as_str().context("a section name is text")?;
                    anyhow::ensure!(
                        !COMMON_SECTIONS.iter().any(|s| same(s, key)),
                        "every kind has the section {key} already"
                    );
                    let ty = SectionType::parse(value).with_context(|| format!("section {key}"))?;
                    Ok((key.to_owned(), ty))
                })
                .collect::<anyhow::Result<_>>()?,
            Some(_) => anyhow::bail!("sections is a set of sections"),
        };
        let free = match readme.and_then(|r| r.text("body")).as_deref() {
            None => false,
            Some("free") => true,
            Some(other) => anyhow::bail!("a body is free or not given, not {other}"),
        };
        // Not the README's to declare, so that a kind whose README was
        // written before, or is not there, has it too.
        let related = (RELATED.to_owned(), FieldType::Links(None));
        let mut fields = if free { Vec::new() } else { fields };
        if !fields.iter().any(|(n, _)| same(n, RELATED)) {
            fields.push(related);
        }
        Ok(Self {
            name: name.to_owned(),
            code,
            layout: Layout {
                free,
                sections: sections.iter().map(|(n, _)| n.clone()).collect(),
            },
            fields,
            sections,
        })
    }

    pub fn field(&self, name: &str) -> Option<&(String, FieldType)> {
        self.fields.iter().find(|(n, _)| same(n, name))
    }

    pub fn section(&self, name: &str) -> Option<&(String, SectionType)> {
        self.sections.iter().find(|(n, _)| same(n, name))
    }
}
