//! The HTTP headers that mirror parts of a request body.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

/// A tool parameter the server wants mirrored into `Mcp-Param-{name}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParamHeader {
    pub(crate) name: String,
    pub(crate) path: Vec<String>,
}

const MAP_KEYWORDS: &[&str] = &[
    "$defs",
    "definitions",
    "patternProperties",
    "dependentSchemas",
];
const LIST_KEYWORDS: &[&str] = &["allOf", "anyOf", "oneOf", "prefixItems"];
const SCHEMA_KEYWORDS: &[&str] = &[
    "items",
    "additionalProperties",
    "not",
    "if",
    "then",
    "else",
    "contains",
    "propertyNames",
    "unevaluatedItems",
    "unevaluatedProperties",
];

/// The `x-mcp-header` annotations of an input schema, or why the schema
/// breaks their rules.
pub(crate) fn param_headers(schema: &Value) -> Result<Vec<ParamHeader>, String> {
    let mut headers = Vec::new();
    walk(schema, Some(Vec::new()), &mut headers)?;
    headers.sort_by(|a, b| a.name.cmp(&b.name));
    for (index, header) in headers.iter().enumerate() {
        if headers[..index]
            .iter()
            .any(|other| other.name.eq_ignore_ascii_case(&header.name))
        {
            return Err(format!("x-mcp-header {:?} is not unique", header.name));
        }
    }
    Ok(headers)
}

fn walk(
    schema: &Value,
    path: Option<Vec<String>>,
    headers: &mut Vec<ParamHeader>,
) -> Result<(), String> {
    let Some(object) = schema.as_object() else {
        return Ok(());
    };
    if let Some(name) = object.get("x-mcp-header") {
        let path = path
            .clone()
            .filter(|path| !path.is_empty())
            .ok_or("x-mcp-header on a schema not reachable through properties")?;
        let name = name.as_str().ok_or("x-mcp-header is not a string")?;
        if name.is_empty() || !name.bytes().all(is_tchar) {
            return Err(format!("x-mcp-header {name:?} is not a header name"));
        }
        let primitive = matches!(
            object.get("type").and_then(Value::as_str),
            Some("string" | "integer" | "boolean")
        );
        if !primitive {
            return Err(format!(
                "x-mcp-header {name:?} is not on a string, integer or boolean"
            ));
        }
        headers.push(ParamHeader {
            name: name.into(),
            path,
        });
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (key, property) in properties {
            let child = path.as_ref().map(|path| {
                let mut child = path.clone();
                child.push(key.clone());
                child
            });
            walk(property, child, headers)?;
        }
    }
    for keyword in MAP_KEYWORDS {
        if let Some(map) = object.get(*keyword).and_then(Value::as_object) {
            for value in map.values() {
                walk(value, None, headers)?;
            }
        }
    }
    for keyword in LIST_KEYWORDS {
        if let Some(list) = object.get(*keyword).and_then(Value::as_array) {
            for value in list {
                walk(value, None, headers)?;
            }
        }
    }
    for keyword in SCHEMA_KEYWORDS {
        if let Some(value) = object.get(*keyword) {
            walk(value, None, headers)?;
        }
    }
    Ok(())
}

fn is_tchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// The `Mcp-Param-*` headers for a call with `arguments`.
pub(crate) fn param_header_values(
    headers: &[ParamHeader],
    arguments: &Value,
) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|header| {
            let value = header
                .path
                .iter()
                .try_fold(arguments, |value, key| value.get(key))?;
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Bool(flag) => flag.to_string(),
                Value::Number(number) => number
                    .as_i64()
                    .map(|n| n.to_string())
                    .or_else(|| number.as_u64().map(|n| n.to_string()))?,
                _ => return None,
            };
            Some((
                format!("Mcp-Param-{}", header.name),
                encode_header_value(&text),
            ))
        })
        .collect()
}

/// A value safe to put in a header: as-is when it is plain ASCII, otherwise
/// in the `=?base64?…?=` form.
pub(crate) fn encode_header_value(value: &str) -> String {
    let plain = value
        .bytes()
        .all(|byte| byte == b'\t' || (0x20..=0x7e).contains(&byte));
    let padded = value.starts_with([' ', '\t']) || value.ends_with([' ', '\t']);
    let sentinel = value.starts_with("=?base64?") && value.ends_with("?=");
    if plain && !padded && !sentinel {
        value.to_string()
    } else {
        format!("=?base64?{}?=", STANDARD.encode(value))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn values_are_encoded_as_the_spec_table_shows() {
        assert_eq!(encode_header_value("us-west1"), "us-west1");
        assert_eq!(
            encode_header_value("Hello, 世界"),
            "=?base64?SGVsbG8sIOS4lueVjA==?="
        );
        assert_eq!(encode_header_value(" padded "), "=?base64?IHBhZGRlZCA=?=");
        assert_eq!(
            encode_header_value("line1\nline2"),
            "=?base64?bGluZTEKbGluZTI=?="
        );
        assert_eq!(
            encode_header_value("=?base64?literal?="),
            "=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?="
        );
    }

    #[test]
    fn annotated_properties_reachable_through_properties_become_headers() {
        let schema = json!({
            "type": "object",
            "properties": {
                "region": {"type": "string", "x-mcp-header": "Region"},
                "options": {
                    "type": "object",
                    "properties": {"dry": {"type": "boolean", "x-mcp-header": "Dry-Run"}}
                },
                "query": {"type": "string"}
            }
        });

        let headers = param_headers(&schema).unwrap();

        assert_eq!(
            headers,
            vec![
                ParamHeader {
                    name: "Dry-Run".into(),
                    path: vec!["options".into(), "dry".into()]
                },
                ParamHeader {
                    name: "Region".into(),
                    path: vec!["region".into()]
                },
            ]
        );
    }

    #[test]
    fn annotations_that_break_the_rules_reject_the_schema() {
        let on_number = json!({"properties": {"n": {"type": "number", "x-mcp-header": "N"}}});
        let in_items = json!({"properties": {"list": {"type": "array", "items": {"type": "string", "x-mcp-header": "Item"}}}});
        let in_any_of =
            json!({"anyOf": [{"properties": {"a": {"type": "string", "x-mcp-header": "A"}}}]});
        let bad_name = json!({"properties": {"a": {"type": "string", "x-mcp-header": "Bad Name"}}});
        let duplicated = json!({"properties": {
            "a": {"type": "string", "x-mcp-header": "Same"},
            "b": {"type": "string", "x-mcp-header": "same"}
        }});
        let on_root = json!({"type": "string", "x-mcp-header": "Root"});

        for schema in [
            on_number, in_items, in_any_of, bad_name, duplicated, on_root,
        ] {
            assert!(param_headers(&schema).is_err(), "{schema}");
        }
    }

    #[test]
    fn header_values_come_from_the_annotated_paths_and_absent_ones_are_omitted() {
        let headers = vec![
            ParamHeader {
                name: "Region".into(),
                path: vec!["region".into()],
            },
            ParamHeader {
                name: "Count".into(),
                path: vec!["count".into()],
            },
            ParamHeader {
                name: "Dry".into(),
                path: vec!["options".into(), "dry".into()],
            },
            ParamHeader {
                name: "Missing".into(),
                path: vec!["missing".into()],
            },
        ];

        let values = param_header_values(
            &headers,
            &json!({"region": "東京", "count": -7, "options": {"dry": true}, "missing": null}),
        );

        assert_eq!(
            values,
            vec![
                (
                    "Mcp-Param-Region".to_string(),
                    "=?base64?5p2x5Lqs?=".to_string()
                ),
                ("Mcp-Param-Count".to_string(), "-7".to_string()),
                ("Mcp-Param-Dry".to_string(), "true".to_string()),
            ]
        );
    }
}
