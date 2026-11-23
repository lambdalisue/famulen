//! Whose account a remote MCP connection is. The protocol's login does not
//! say, so it is read from what the server itself tells of its user.

use serde_json::{Value, json};

use crate::catalog;
use crate::connections::{Connection, Transport};
use crate::mcp;

use super::Gateway;

/// The names servers give the tool that tells of the logged-in user, after
/// any prefix of their own.
const WHO: &[&str] = &[
    "whoami",
    "who_am_i",
    "get_me",
    "get_self",
    "get_current_user",
    "current_user",
    "get_authenticated_user",
    "get_user_info",
    "user_info",
    "get_user",
];

/// What the user is called, the most telling first.
const CALLED: &[&str] = &["display_name", "username", "name", "login"];

/// The tool that tells of the user: one that only reads and asks nothing.
fn who_tool(tools: &[Value]) -> Option<&str> {
    let usable = |t: &&Value| {
        t["annotations"]["readOnlyHint"] == true
            && t["inputSchema"]["required"]
                .as_array()
                .is_none_or(Vec::is_empty)
    };
    let named = |t: &Value, who: &str| {
        let name = snake(t["name"].as_str().unwrap_or_default());
        name == who || name.ends_with(&format!("_{who}"))
    };
    WHO.iter()
        .find_map(|who| tools.iter().filter(usable).find(|t| named(t, who)))
        .and_then(|t| t["name"].as_str())
}

/// `getUserInfo` and `get-user-info` as `get_user_info`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_uppercase() && out.ends_with(|p: char| p.is_ascii_lowercase()) {
            out.push('_');
        }
        out.push(if c == '-' {
            '_'
        } else {
            c.to_ascii_lowercase()
        });
    }
    out
}

/// The first text under `key`, the shallowest first.
fn text_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    let mut level = vec![value];
    while !level.is_empty() {
        let mut next = Vec::new();
        for v in level {
            match v {
                Value::Object(map) => {
                    if let Some(text) = map.iter().find_map(|(k, v)| {
                        (snake(k).replace('_', "") == key.replace('_', ""))
                            .then(|| v.as_str())
                            .flatten()
                            .filter(|t| !t.trim().is_empty())
                    }) {
                        return Some(text);
                    }
                    next.extend(map.values());
                }
                Value::Array(list) => next.extend(list),
                _ => {}
            }
        }
        level = next;
    }
    None
}

/// The user as the owner tells them apart: the name and the email.
fn shown(answer: &Value) -> Option<String> {
    let name = CALLED.iter().find_map(|key| text_at(answer, key));
    let mut parts: Vec<&str> = [name, text_at(answer, "email")]
        .into_iter()
        .flatten()
        .map(str::trim)
        .collect();
    parts.dedup();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// What follows each of `introduces` in `text`, up to a parenthesis or the
/// end of the line.
fn names_in(text: &str, introduces: &[&str]) -> Option<String> {
    let names: Vec<&str> = introduces
        .iter()
        .filter_map(|before| {
            let rest = &text[text.find(before)? + before.len()..];
            let line = rest.lines().next().unwrap_or_default();
            let name = line.split(" (").next().unwrap_or_default().trim();
            (!name.is_empty()).then_some(name)
        })
        .collect();
    (!names.is_empty()).then(|| names.join(" · "))
}

impl Gateway {
    /// Find whose account a remote MCP connection is, when nothing says yet.
    pub(super) async fn find_account(&self, connection: &Connection) -> anyhow::Result<()> {
        if !matches!(connection.transport, Transport::McpRemote { .. })
            || !connection.account.is_empty()
        {
            return Ok(());
        }
        let tools = self.list_tools(connection).await?;
        let answer = match who_tool(&tools) {
            Some(tool) => {
                let result = self.call_as_owner(connection, tool, json!({})).await?;
                anyhow::ensure!(!result.is_error, "{tool} failed");
                match result.structured_content {
                    Some(value) => value,
                    None => serde_json::from_str(&result.text)?,
                }
            }
            None => self.userinfo(connection).await.unwrap_or_else(|error| {
                tracing::debug!(%error, connection = %connection.id, "its login does not tell of its user");
                Value::Null
            }),
        };
        let account = match shown(&answer) {
            Some(account) => Some(account),
            None => self.introduced(connection).await?,
        };
        if let Some(account) = account {
            self.connections
                .update(&connection.id, None, None, Some(&account))?;
        }
        Ok(())
    }

    /// The organization and the user as a known server's instructions name
    /// them.
    async fn introduced(&self, connection: &Connection) -> anyhow::Result<Option<String>> {
        let Some(known) = catalog::find(&connection.kind) else {
            return Ok(None);
        };
        let client = self.client(connection).await?;
        Ok(names_in(
            client.instructions().unwrap_or_default(),
            known.introduces,
        ))
    }

    /// What the login's own server says of the user, when it speaks OpenID
    /// Connect. The token goes nowhere but the server that gave it.
    async fn userinfo(&self, connection: &Connection) -> anyhow::Result<Value> {
        let secret = &connection.secret;
        let issuer = secret
            .auth_server
            .as_ref()
            .and_then(|s| s["issuer"].as_str())
            .ok_or_else(|| anyhow::anyhow!("it was not logged in to"))?;
        let token = secret
            .bearer
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("it has no token"))?;
        let endpoint = mcp::discover_userinfo(&self.http, issuer).await?;
        let response = self
            .http
            .get(endpoint)
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::{Connections, Secret};
    use crate::mcp::fake::{Era, FakeServer};
    use crate::store::Store;

    #[tokio::test]
    async fn a_server_with_no_such_tool_is_asked_through_its_login() {
        let fake = FakeServer::start(Era::Modern, Some("at-1")).await;
        let store = Store::open_in_memory().unwrap();
        let g = Gateway::new(
            store.clone(),
            Connections::new(store),
            reqwest::Client::new(),
            None,
        );
        let add = |id: &str, issuer: String, account: &str| {
            g.connections
                .add(
                    id,
                    "Notes",
                    "notes",
                    Transport::McpRemote { url: fake.url() },
                    Secret {
                        bearer: Some("at-1".into()),
                        auth_server: Some(json!({ "issuer": issuer })),
                        ..Default::default()
                    },
                    account,
                )
                .unwrap();
        };
        add("told", format!("{}/tenant", fake.base()), "");
        add("named", format!("{}/tenant", fake.base()), "会社の");
        add("silent", fake.base().to_owned(), "");

        for id in ["told", "named", "silent"] {
            g.read_tools(id).await.unwrap();
        }

        let account = |id: &str| g.connections.get(id).unwrap().unwrap().account;
        assert_eq!(account("told"), "me@example.com");
        assert_eq!(account("named"), "会社の", "the owner's own words stay");
        assert_eq!(account("silent"), "");
    }

    fn tool(name: &str, read: bool, required: &[&str]) -> Value {
        json!({ "name": name, "inputSchema": { "required": required }, "annotations": { "readOnlyHint": read } })
    }

    #[test]
    fn the_tool_that_tells_of_the_user_only_reads_and_asks_nothing() {
        for (name, read, required, found) in [
            ("clickup_get_user", true, &[][..], true),
            ("whoami", true, &[], true),
            ("notion-get-self", true, &[], true),
            ("atlassianUserInfo", true, &[], true),
            ("get-user", true, &["id"], false),
            ("get_me", false, &[], false),
            ("get_user_group", true, &[], false),
        ] {
            let tools = [tool("search", true, &[]), tool(name, read, required)];
            assert_eq!(who_tool(&tools).is_some(), found, "{name}");
        }
    }

    #[test]
    fn a_known_servers_instructions_name_the_organization_and_the_user() {
        let text = "## Context\nUser - Name: 山田 (id: e-1)\nOrganization - Name: acme (domain: acme) - id: k-1\n";
        let slite = catalog::find("slite").unwrap().introduces;
        assert_eq!(names_in(text, slite).as_deref(), Some("acme · 山田"));
        assert_eq!(names_in("Hello.", slite), None);
        assert_eq!(names_in(text, &[]), None);
    }

    #[test]
    fn the_user_is_shown_by_name_and_email_as_far_as_the_server_tells() {
        for (answer, expected) in [
            (
                json!({ "username": "Me", "email": "me@example.com" }),
                Some("Me · me@example.com"),
            ),
            (
                json!({ "user": { "userName": "Me", "email": "" } }),
                Some("Me"),
            ),
            (json!({ "email": "me@example.com" }), Some("me@example.com")),
            (
                json!({ "name": "me@example.com", "email": "me@example.com" }),
                Some("me@example.com"),
            ),
            (json!([1, null]), None),
        ] {
            assert_eq!(shown(&answer).as_deref(), expected);
        }
    }
}
