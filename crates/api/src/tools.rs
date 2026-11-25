//! The secretary's own tools, made from the API's definition: every unary
//! method the console can call, except those only a person at the console
//! does. A tool's description is its method's comment, its arguments are its
//! request message, and a call goes through the API in this process.

use std::collections::HashMap;
use std::sync::Arc;

use buffa::{ExtensionSet, Message};
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, EnumDescriptorProto, FileDescriptorSet, field_descriptor_proto::Label,
    field_descriptor_proto::Type, method_options::IdempotencyLevel,
};
use famulen_core::hub::{OwnTools, ToolDef};
use futures::future::BoxFuture;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::pb::{CONFIRM, CONSOLE_ONLY};

/// A tool and the API method it calls.
#[derive(Debug, Clone)]
pub(crate) struct Tool {
    pub(crate) def: ToolDef,
    /// `/famulen.v1.RecordService/SaveRecord`.
    path: String,
}

/// The tools, answering through an API router.
pub struct ApiTools {
    tools: Vec<Tool>,
    router: axum::Router,
}

impl ApiTools {
    pub fn new(api: crate::Api) -> anyhow::Result<Self> {
        Ok(Self {
            tools: tools_from(crate::DESCRIPTOR_SET)?,
            router: api.axum(),
        })
    }

    pub fn shared(api: crate::Api) -> anyhow::Result<Arc<dyn OwnTools>> {
        Ok(Arc::new(Self::new(api)?))
    }
}

impl OwnTools for ApiTools {
    fn list(&self) -> Vec<ToolDef> {
        self.tools.iter().map(|t| t.def.clone()).collect()
    }

    fn call(&self, name: &str, arguments: Value) -> BoxFuture<'static, anyhow::Result<String>> {
        let tool = self.tools.iter().find(|t| t.def.name == name).cloned();
        let router = self.router.clone();
        let name = name.to_owned();
        Box::pin(async move {
            let tool = tool.ok_or_else(|| anyhow::anyhow!("unknown tool: {name}"))?;
            let body = if arguments.is_null() {
                json!({})
            } else {
                arguments
            };
            let request = http::Request::post(&tool.path)
                .header("content-type", "application/json")
                .header("connect-protocol-version", "1")
                .header(crate::CALLER_HEADER, crate::SECRETARY)
                .body(axum::body::Body::from(serde_json::to_vec(&body)?))?;
            let response = router.oneshot(request).await?;
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024).await?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if !status.is_success() {
                let message = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| v["message"].as_str().map(str::to_owned))
                    .unwrap_or(text);
                anyhow::bail!("{message}");
            }
            Ok(if text == "{}" {
                "done".to_owned()
            } else {
                text
            })
        })
    }
}

/// `SaveRecord` → `save_record`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The comments of a file, by the path of what they describe.
fn comments(
    file: &buffa_descriptor::generated::descriptor::FileDescriptorProto,
) -> HashMap<Vec<i32>, String> {
    file.source_code_info
        .as_option()
        .map(|info| {
            info.location
                .iter()
                .filter_map(|l| {
                    let text = l.leading_comments.as_deref()?.trim();
                    (!text.is_empty())
                        .then(|| (l.path.clone(), text.replace('\n', " ").replace("  ", " ")))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Every message and enum of the set by full name (`.famulen.v1.Record`), with
/// each message's field comments.
struct Types<'a> {
    messages: HashMap<String, (&'a DescriptorProto, Vec<String>)>,
    enums: HashMap<String, &'a EnumDescriptorProto>,
}

impl<'a> Types<'a> {
    fn of(set: &'a FileDescriptorSet) -> Self {
        let mut messages = HashMap::new();
        let mut enums = HashMap::new();
        for file in &set.file {
            let package = file.package.clone().unwrap_or_default();
            let notes = comments(file);
            for (i, m) in file.message_type.iter().enumerate() {
                let name = format!(".{package}.{}", m.name.as_deref().unwrap_or_default());
                let field_notes = (0..m.field.len())
                    .map(|f| {
                        notes
                            .get(&vec![4, i as i32, 2, f as i32])
                            .cloned()
                            .unwrap_or_default()
                    })
                    .collect();
                for nested in &m.nested_type {
                    messages.insert(
                        format!("{name}.{}", nested.name.as_deref().unwrap_or_default()),
                        (nested, vec![String::new(); nested.field.len()]),
                    );
                }
                messages.insert(name, (m, field_notes));
            }
            for e in &file.enum_type {
                enums.insert(
                    format!(".{package}.{}", e.name.as_deref().unwrap_or_default()),
                    e,
                );
            }
        }
        Self { messages, enums }
    }

    /// The JSON Schema of a message as Connect's JSON reads it.
    fn schema(&self, name: &str, depth: usize) -> Value {
        let Some((message, notes)) = self.messages.get(name) else {
            return json!({ "type": "object" });
        };
        if depth > 4 {
            return json!({ "type": "object" });
        }
        let mut properties = serde_json::Map::new();
        for (field, note) in message.field.iter().zip(notes) {
            let mut schema = match field.r#type {
                Some(Type::TYPE_STRING | Type::TYPE_BYTES) => json!({ "type": "string" }),
                Some(Type::TYPE_BOOL) => json!({ "type": "boolean" }),
                Some(Type::TYPE_DOUBLE | Type::TYPE_FLOAT) => json!({ "type": "number" }),
                Some(Type::TYPE_ENUM) => {
                    let values: Vec<String> = self
                        .enums
                        .get(field.type_name.as_deref().unwrap_or_default())
                        .map(|e| {
                            e.value
                                .iter()
                                .filter_map(|v| v.name.clone())
                                .filter(|v| !v.ends_with("_UNSPECIFIED"))
                                .collect()
                        })
                        .unwrap_or_default();
                    json!({ "type": "string", "enum": values })
                }
                Some(Type::TYPE_MESSAGE) => {
                    let type_name = field.type_name.as_deref().unwrap_or_default();
                    match self.messages.get(type_name) {
                        Some((m, _))
                            if m.options.as_option().and_then(|o| o.map_entry) == Some(true) =>
                        {
                            json!({ "type": "object", "additionalProperties": { "type": "string" } })
                        }
                        _ => self.schema(type_name, depth + 1),
                    }
                }
                _ => json!({ "type": "integer" }),
            };
            let repeated = field.label == Some(Label::LABEL_REPEATED)
                && !matches!(&schema["additionalProperties"], Value::Object(_));
            if repeated {
                schema = json!({ "type": "array", "items": schema });
            }
            if !note.is_empty() {
                schema["description"] = json!(note);
            }
            properties.insert(field.name.clone().unwrap_or_default(), schema);
        }
        json!({ "type": "object", "properties": properties, "additionalProperties": false })
    }
}

/// The tools a descriptor set gives.
pub(crate) fn tools_from(bytes: &[u8]) -> anyhow::Result<Vec<Tool>> {
    let set = FileDescriptorSet::decode_from_slice(bytes)?;
    let types = Types::of(&set);
    let mut tools = Vec::new();
    for file in set
        .file
        .iter()
        .filter(|f| f.package.as_deref() == Some("famulen.v1"))
    {
        let notes = comments(file);
        for (s, service) in file.service.iter().enumerate() {
            let service_name = service.name.clone().unwrap_or_default();
            for (m, method) in service.method.iter().enumerate() {
                let options = method.options.as_option();
                let console_only = options
                    .and_then(|o| o.extension(&CONSOLE_ONLY))
                    .unwrap_or(false);
                if console_only
                    || method.server_streaming == Some(true)
                    || method.client_streaming == Some(true)
                {
                    continue;
                }
                let method_name = method.name.clone().unwrap_or_default();
                let description = notes
                    .get(&vec![6, s as i32, 2, m as i32])
                    .cloned()
                    .or_else(|| notes.get(&vec![6, s as i32]).cloned())
                    .unwrap_or_else(|| format!("{service_name}.{method_name}"));
                tools.push(Tool {
                    def: ToolDef {
                        name: snake(&method_name),
                        description,
                        input_schema: types
                            .schema(method.input_type.as_deref().unwrap_or_default(), 0),
                        read_only: options.and_then(|o| o.idempotency_level)
                            == Some(IdempotencyLevel::NO_SIDE_EFFECTS),
                        confirm: options.and_then(|o| o.extension(&CONFIRM)).unwrap_or(false),
                    },
                    path: format!("/famulen.v1.{service_name}/{method_name}"),
                });
            }
        }
    }
    let mut names: Vec<&str> = tools.iter().map(|t| t.def.name.as_str()).collect();
    names.sort();
    if let Some(w) = names.windows(2).find(|w| w[0] == w[1]) {
        anyhow::bail!("two API methods make the same tool name {}", w[0]);
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool<'a>(tools: &'a [Tool], name: &str) -> &'a ToolDef {
        &tools
            .iter()
            .find(|t| t.def.name == name)
            .unwrap_or_else(|| panic!("no tool {name}"))
            .def
    }

    #[test]
    fn every_unary_method_but_the_console_only_ones_is_a_tool() {
        let tools = tools_from(crate::DESCRIPTOR_SET).unwrap();
        let add = tool(&tools, "save_record");
        assert!(add.description.contains("Add a record"));
        assert!(!add.read_only && !add.confirm);
        assert_eq!(add.input_schema["properties"]["title"]["type"], "string");
        assert_eq!(
            add.input_schema["properties"]["workgroups"]["type"],
            "array"
        );
        assert!(tool(&tools, "list_records").read_only);
        assert!(tool(&tools, "delete_record").confirm);
        let field = &add.input_schema["properties"]["fields"]["items"]["properties"];
        assert_eq!(field["name"]["type"], "string", "{field}");
        let mode = &tool(&tools, "set_calendar_mode").input_schema["properties"]["mode"]["enum"];
        assert!(
            mode.as_array()
                .unwrap()
                .contains(&json!("CALENDAR_MODE_FAINT"))
        );
        for console_only in [
            "list_link_previews",
            "send",
            "subscribe_events",
            "answer_approval",
            "connect_known",
            "get_note_origin",
        ] {
            assert!(
                tools.iter().all(|t| t.def.name != console_only),
                "{console_only} is not a tool"
            );
        }
    }

    #[test]
    fn every_tool_told_to_answer_nothing_in_confidence_is_one() {
        let tools = tools_from(crate::DESCRIPTOR_SET).unwrap();
        for plain in famulen_core::disclosure::PLAIN {
            assert!(tools.iter().any(|t| t.def.name == *plain), "{plain}");
        }
    }

    #[test]
    fn the_secretary_has_no_tool_that_changes_a_permission() {
        let tools = tools_from(crate::DESCRIPTOR_SET).unwrap();
        for owners in ["set_permission", "set_permissions", "set_vault_permission"] {
            assert!(
                tools.iter().all(|t| t.def.name != owners),
                "{owners} is for the owner only"
            );
        }
    }

    #[test]
    fn the_secretary_adds_only_keyless_connections_and_asks_the_owner_first() {
        let tools = tools_from(crate::DESCRIPTOR_SET).unwrap();
        assert!(tools.iter().all(|t| t.def.name != "add_connection"));
        let add = tool(&tools, "add_keyless_connection");
        assert!(add.confirm && !add.read_only);
        let values = &add.input_schema["properties"]["values"];
        assert_eq!(values["additionalProperties"]["type"], "string", "{values}");
    }

    #[test]
    fn how_long_a_channel_is_attended_is_the_owners_to_set() {
        let tools = tools_from(crate::DESCRIPTOR_SET).unwrap();
        assert!(tools.iter().all(|t| t.def.name != "set_attention"));
    }

    #[test]
    fn only_the_owner_shares_the_notebook() {
        let tools = tools_from(crate::DESCRIPTOR_SET).unwrap();
        assert!(tools.iter().all(|t| t.def.name != "set_notebook_sharing"));
        assert!(tool(&tools, "get_notebook_sharing").read_only);
    }

    #[test]
    fn only_the_owner_widens_where_the_secretary_may_write() {
        let tools = tools_from(crate::DESCRIPTOR_SET).unwrap();
        assert!(tools.iter().all(|t| t.def.name != "add_writable_place"));
        assert!(tool(&tools, "get_writable_places").read_only);
        assert!(tools.iter().all(|t| t.def.name != "move_notebook"));
        assert!(tool(&tools, "get_notebook_place").read_only);
        assert!(tools.iter().any(|t| t.def.name == "remove_writable_place"));
    }

    #[test]
    fn what_is_remembered_is_the_dreams_to_place() {
        let tools = tools_from(crate::DESCRIPTOR_SET).unwrap();
        let takes = &tool(&tools, "remember").input_schema["properties"];
        assert!(takes.get("text").is_some());
        assert!(takes.get("keep_in").is_none(), "{takes}");
    }

    #[test]
    fn a_field_comment_describes_the_argument() {
        let tools = tools_from(crate::DESCRIPTOR_SET).unwrap();
        let note = &tool(&tools, "remember").input_schema["properties"]["text"]["description"];
        assert!(
            note.as_str()
                .unwrap_or_default()
                .contains("Link every record"),
            "{note}"
        );
    }
}
