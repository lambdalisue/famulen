//! Tools from an OpenAPI 3 description: one per operation with an
//! `operationId`, its parameters and JSON body as the tool's arguments.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::template::text_of;

/// How deep local `$ref`s are followed into schemas, so a schema that
/// refers to itself still ends.
const REF_DEPTH: usize = 8;

const METHODS: &[&str] = &["get", "put", "post", "delete", "patch", "head"];

/// Where one argument of an operation goes in the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum In {
    Path,
    Query,
}

/// One operation of the description, as a tool.
#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    /// The tool's name: the `operationId` in lowercase snake case.
    pub name: String,
    pub description: String,
    /// In upper case, such as `GET`.
    pub method: String,
    /// The path as the description writes it, with `{name}` for its
    /// path parameters.
    pub path: String,
    pub params: Vec<(String, In)>,
    /// Whether it takes a JSON body, given as the `body` argument.
    pub body: bool,
    /// The arguments' JSON Schema.
    pub input: Value,
    pub read_only: bool,
}

impl Operation {
    /// The path with its parameters filled from `args`, each encoded, and
    /// the query; `None` when a path parameter is missing.
    pub fn request(&self, args: &Value) -> Option<(String, BTreeMap<String, String>)> {
        let mut path = self.path.clone();
        let mut query = BTreeMap::new();
        for (name, place) in &self.params {
            let value = args.get(name).filter(|v| !v.is_null());
            match place {
                In::Path => {
                    let text = text_of(value?);
                    path = path.replace(&format!("{{{name}}}"), &encode_segment(&text));
                }
                In::Query => {
                    if let Some(value) = value {
                        query.insert(name.clone(), text_of(value));
                    }
                }
            }
        }
        Some((path, query))
    }
}

fn encode_segment(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `listPets`, `list-pets` and `List Pets` all as `list_pets`.
pub fn tool_name(operation_id: &str) -> String {
    let mut out = String::new();
    let mut last_lower = false;
    for c in operation_id.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && last_lower && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            last_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        } else {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            last_lower = false;
        }
    }
    out.trim_end_matches('_').to_owned()
}

/// The operations of an OpenAPI 3 description that have an `operationId`.
pub fn operations(doc: &Value) -> anyhow::Result<Vec<Operation>> {
    let version = doc
        .get("openapi")
        .and_then(Value::as_str)
        .unwrap_or_default();
    anyhow::ensure!(
        version.starts_with("3."),
        "not an OpenAPI 3 description (openapi is {version:?})"
    );
    let paths = doc
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("the description has no paths"))?;
    let mut out = Vec::new();
    for (path, item) in paths {
        let item = resolve(doc, item, 0);
        let shared = item.get("parameters").cloned().unwrap_or(json!([]));
        for method in METHODS {
            let Some(op) = item.get(*method) else {
                continue;
            };
            let Some(id) = op.get("operationId").and_then(Value::as_str) else {
                continue;
            };
            out.push(operation(doc, path, method, id, op, &shared));
        }
    }
    Ok(out)
}

fn operation(
    doc: &Value,
    path: &str,
    method: &str,
    id: &str,
    op: &Value,
    shared: &Value,
) -> Operation {
    let mut by_name: BTreeMap<(String, String), Value> = BTreeMap::new();
    let listed = shared.as_array().into_iter().flatten().chain(
        op.get("parameters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten(),
    );
    for p in listed {
        let p = resolve(doc, p, 0);
        let (Some(name), Some(place)) = (
            p.get("name").and_then(Value::as_str),
            p.get("in").and_then(Value::as_str),
        ) else {
            continue;
        };
        by_name.insert((place.to_owned(), name.to_owned()), p.clone());
    }
    let mut params = Vec::new();
    let mut properties = Map::new();
    let mut required = Vec::new();
    for ((place, name), p) in &by_name {
        let place = match place.as_str() {
            "path" => In::Path,
            "query" => In::Query,
            _ => continue,
        };
        let mut schema = p
            .get("schema")
            .map(|s| inline(doc, s, 0))
            .unwrap_or_else(|| json!({ "type": "string" }));
        if let (Some(d), Some(s)) = (p.get("description"), schema.as_object_mut()) {
            s.entry("description").or_insert(d.clone());
        }
        properties.insert(name.clone(), schema);
        if place == In::Path || p.get("required") == Some(&Value::Bool(true)) {
            required.push(Value::String(name.clone()));
        }
        params.push((name.clone(), place));
    }
    let body_schema = op
        .get("requestBody")
        .map(|b| resolve(doc, b, 0))
        .and_then(|b| {
            let schema = b.pointer("/content/application~1json/schema")?;
            Some((
                inline(doc, schema, 0),
                b.get("required") == Some(&Value::Bool(true)),
            ))
        });
    let body = body_schema.is_some();
    if let Some((schema, needed)) = body_schema {
        properties.insert("body".into(), schema);
        if needed {
            required.push(Value::String("body".into()));
        }
    }
    let description = [op.get("summary"), op.get("description")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|s| !s.trim().is_empty())
        .unwrap_or(id)
        .to_owned();
    let mut input = json!({ "type": "object", "properties": properties });
    if !required.is_empty() {
        input["required"] = Value::Array(required);
    }
    Operation {
        name: tool_name(id),
        description,
        method: method.to_ascii_uppercase(),
        path: path.to_owned(),
        params,
        body,
        input,
        read_only: matches!(method, "get" | "head"),
    }
}

/// What a local `$ref` points at, or the value itself.
fn resolve<'a>(doc: &'a Value, value: &'a Value, depth: usize) -> &'a Value {
    match value.get("$ref").and_then(Value::as_str) {
        Some(at) if depth < REF_DEPTH => match at.strip_prefix('#').and_then(|p| doc.pointer(p)) {
            Some(found) => resolve(doc, found, depth + 1),
            None => value,
        },
        _ => value,
    }
}

/// A schema with its local `$ref`s put in place, to a bounded depth.
fn inline(doc: &Value, schema: &Value, depth: usize) -> Value {
    if depth >= REF_DEPTH {
        return json!({});
    }
    let schema = resolve(doc, schema, 0);
    match schema {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), inline(doc, v, depth + 1)))
                .collect(),
        ),
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| inline(doc, v, depth + 1)).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pets() -> Value {
        json!({
            "openapi": "3.0.3",
            "paths": {
                "/pets": {
                    "get": {
                        "operationId": "listPets",
                        "summary": "List pets.",
                        "parameters": [{ "name": "limit", "in": "query", "schema": { "type": "integer" } }]
                    },
                    "post": {
                        "operationId": "createPet",
                        "requestBody": {
                            "required": true,
                            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Pet" } } }
                        }
                    }
                },
                "/pets/{petId}": {
                    "parameters": [{ "$ref": "#/components/parameters/PetId" }],
                    "get": { "operationId": "show-pet-by-id", "description": "One pet." },
                    "delete": { "summary": "No id, so no tool." }
                }
            },
            "components": {
                "parameters": {
                    "PetId": { "name": "petId", "in": "path", "required": true, "schema": { "type": "string" } }
                },
                "schemas": {
                    "Pet": { "type": "object", "properties": { "name": { "type": "string" }, "parent": { "$ref": "#/components/schemas/Pet" } } }
                }
            }
        })
    }

    #[test]
    fn each_operation_with_an_id_is_a_tool_with_its_parameters_and_body() {
        let ops = operations(&pets()).unwrap();
        let names: Vec<&str> = ops.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["list_pets", "create_pet", "show_pet_by_id"]);

        let list = &ops[0];
        assert_eq!((list.method.as_str(), list.read_only), ("GET", true));
        assert_eq!(list.description, "List pets.");
        assert_eq!(list.input["properties"]["limit"]["type"], "integer");
        assert!(list.input.get("required").is_none());

        let create = &ops[1];
        assert!(create.body && !create.read_only);
        assert_eq!(create.input["required"], json!(["body"]));
        assert_eq!(
            create.input["properties"]["body"]["properties"]["name"]["type"], "string",
            "a referred schema is put in place"
        );

        let show = &ops[2];
        assert_eq!(show.params, [("petId".to_owned(), In::Path)]);
        assert_eq!(show.input["required"], json!(["petId"]));
        assert_eq!(show.description, "One pet.");
    }

    #[test]
    fn a_request_fills_and_encodes_the_path_and_takes_the_query() {
        let ops = operations(&pets()).unwrap();

        let (path, query) = ops[2].request(&json!({ "petId": "a b/c" })).unwrap();
        let (_, listed) = ops[0].request(&json!({ "limit": 5 })).unwrap();

        assert_eq!(path, "/pets/a%20b%2Fc");
        assert!(query.is_empty());
        assert_eq!(listed["limit"], "5");
        assert_eq!(
            ops[2].request(&json!({})),
            None,
            "a path parameter is needed"
        );
    }

    #[test]
    fn only_an_openapi_3_description_is_read() {
        assert!(operations(&json!({ "swagger": "2.0", "paths": {} })).is_err());
    }

    #[test]
    fn operation_ids_become_tool_names() {
        assert_eq!(tool_name("listPets"), "list_pets");
        assert_eq!(tool_name("list-pets"), "list_pets");
        assert_eq!(tool_name("List Pets"), "list_pets");
        assert_eq!(tool_name("getV2Users"), "get_v2_users");
    }
}
