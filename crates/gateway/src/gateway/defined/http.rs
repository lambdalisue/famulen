//! The tools of an HTTP connection made from a definition, and the requests
//! they and the logins send.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde_json::{Value, json};

use super::super::{Gateway, api_method, api_tools, text_result};
use super::AGENT;
use crate::connections::Connection;
use crate::definitions::{Definition, HttpTool, Method};
use crate::mcp::ToolResult;
use crate::template::{Context, text_of};

/// One request going out: its method, path or URL, query, body, and which
/// of the login's tokens it carries.
#[derive(Clone, Copy)]
pub(super) struct Outgoing<'a> {
    pub(super) method: &'a str,
    pub(super) target: &'a str,
    pub(super) query: &'a BTreeMap<String, String>,
    pub(super) body: Option<&'a Value>,
    pub(super) auth: Option<&'a str>,
    /// Keep to the base's host.
    pub(super) same_host: bool,
}

const PAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

const MAX_REDIRECTS: usize = 5;

/// How long a file may take to arrive.
const FILE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// The largest file a tool answers.
pub const MAX_FILE_BYTES: usize = 20 * 1024 * 1024;

/// What a file tool answers when it is sent on to another host, such as to
/// log in.
pub const SENT_ELSEWHERE: &str = "sent on to another host";

/// A file a tool answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub mime: String,
    pub bytes: Vec<u8>,
}

impl Fetched {
    fn into_result(self, url: &str) -> ToolResult {
        let text = format!("{} bytes of {}", self.bytes.len(), self.mime);
        ToolResult {
            content: vec![
                json!({ "type": "text", "text": text }),
                json!({ "type": "resource", "resource": {
                    "uri": url,
                    "mimeType": self.mime,
                    "blob": base64::engine::general_purpose::STANDARD.encode(&self.bytes),
                } }),
            ],
            structured_content: None,
            is_error: false,
            text,
        }
    }

    /// The file in what a tool answered.
    pub fn of(result: &ToolResult) -> Option<Self> {
        let resource = result
            .content
            .iter()
            .find_map(|part| part.get("resource"))?;
        Some(Self {
            mime: resource.get("mimeType")?.as_str()?.to_owned(),
            bytes: base64::engine::general_purpose::STANDARD
                .decode(resource.get("blob")?.as_str()?)
                .ok()?,
        })
    }
}

impl Gateway {
    /// The operations of the OpenAPI description a definition names, read
    /// once per place and kept.
    async fn openapi_operations(
        &self,
        connection: &Connection,
        def: &Definition,
    ) -> anyhow::Result<std::sync::Arc<Vec<crate::openapi::Operation>>> {
        let Some(location) = def.transport.http.as_ref().and_then(|h| h.openapi.as_ref()) else {
            return Ok(Default::default());
        };
        let ctx = self.context(connection, def, Value::Null, true)?;
        let place = ctx
            .render_text(location)
            .ok_or_else(|| anyhow::anyhow!("the openapi place names a value it does not have"))?;
        if let Some(ops) = crate::lock::read(&self.openapi).get(&place) {
            return Ok(ops.clone());
        }
        let text = if place.starts_with("http://") || place.starts_with("https://") {
            let mut request = self.http_client(&ctx, &place)?.get(&place);
            if let Some((name, value)) = self.auth_header(def, &ctx, None) {
                request = request.header(name, value);
            }
            let response = request.send().await?;
            anyhow::ensure!(
                response.status().is_success(),
                "the OpenAPI description at {place} answered {}",
                response.status()
            );
            response.text().await?
        } else {
            let dir = std::path::Path::new(&def.source)
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_default();
            std::fs::read_to_string(dir.join(&place))?
        };
        let doc: Value = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("the OpenAPI description is not JSON: {e}"))?;
        let ops = std::sync::Arc::new(crate::openapi::operations(&doc)?);
        crate::lock::write(&self.openapi).insert(place, ops.clone());
        Ok(ops)
    }

    /// The tools of an HTTP connection made from a definition: its
    /// OpenAPI operations and its `[[tools]]`, which win on the same name,
    /// or the `http_*` tools when it has neither.
    pub(in crate::gateway) async fn defined_tools(
        &self,
        connection: &Connection,
        def: &Definition,
    ) -> anyhow::Result<Vec<Value>> {
        let ops = self.openapi_operations(connection, def).await?;
        let mut tools: Vec<Value> = ops
            .iter()
            .filter(|o| !def.tools.iter().any(|t| t.name == o.name))
            .map(|o| {
                json!({
                    "name": o.name,
                    "description": o.description,
                    "inputSchema": o.input,
                    "annotations": { "readOnlyHint": o.read_only, "destructiveHint": o.method == "DELETE" },
                })
            })
            .collect();
        tools.extend(self.written_tools(def));
        Ok(tools)
    }

    /// The `[[tools]]` a definition writes, or the `http_*` tools when it
    /// writes none and names no OpenAPI description.
    fn written_tools(&self, def: &Definition) -> Vec<Value> {
        let described = def
            .transport
            .http
            .as_ref()
            .is_some_and(|h| h.openapi.is_some());
        if def.tools.is_empty() {
            if described {
                return Vec::new();
            }
            return api_tools(&def.name);
        }
        def.tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.input.clone().unwrap_or_else(|| json!({ "type": "object" })),
                    "annotations": {
                        "readOnlyHint": t.read_only.unwrap_or(t.method == Method::Get),
                        "destructiveHint": t.destructive.unwrap_or(t.method == Method::Delete),
                        "openWorldHint": t.open_world,
                    },
                })
            })
            .collect()
    }

    /// Call a tool of an HTTP connection made from a definition.
    pub(in crate::gateway) async fn call_defined(
        &self,
        connection: &Connection,
        def: &Definition,
        tool: &str,
        args: Value,
    ) -> anyhow::Result<ToolResult> {
        let mut ctx = self.context(connection, def, args.clone(), true)?;
        let http = def.transport.http.as_ref();
        let described = http.is_some_and(|h| h.openapi.is_some());
        let operation = if def.tools.iter().any(|t| t.name == tool) || !described {
            None
        } else {
            self.openapi_operations(connection, def)
                .await?
                .iter()
                .find(|o| o.name == tool)
                .cloned()
        };
        let auth = http.and_then(|h| h.auth.as_deref());
        let mut ok = None;
        let (status, text) = if let Some(op) = operation {
            let (path, query) = op
                .request(&args)
                .ok_or_else(|| anyhow::anyhow!("{tool} needs every path parameter"))?;
            let out = Outgoing {
                method: &op.method,
                target: &path,
                query: &query,
                body: args.get("body").filter(|_| op.body),
                auth,
                same_host: false,
            };
            self.send_filled(def, &ctx, out).await?
        } else if let Some(t) = def.tools.iter().find(|t| t.name == tool) {
            ok = t.ok.as_deref();
            let auth = t.auth.as_deref().or(auth);
            if let Some(name) = auth
                && let Some(token) = self.exchange(&connection.id, def, name).await?
            {
                ctx.tokens.insert(name.to_owned(), token);
            }
            if t.file {
                return self.fetch_file(def, &ctx, t, auth).await;
            }
            let out = Outgoing {
                method: t.method.as_str(),
                target: &t.path,
                query: &t.query,
                body: t.body.as_ref(),
                auth,
                same_host: t.same_host,
            };
            self.send_defined(def, &ctx, out).await?
        } else if def.tools.is_empty()
            && !described
            && let Some(method) = api_method(tool)
        {
            let path = args.get("path").and_then(Value::as_str).unwrap_or_default();
            // The caller names only the path: a whole URL would carry the
            // connection's key to whatever host it names.
            anyhow::ensure!(
                !path.contains("://"),
                "{tool} takes a path below the service, not a URL"
            );
            let query: BTreeMap<String, String> = args
                .get("query")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .map(|(k, v)| (k.clone(), text_of(v)))
                .collect();
            let out = Outgoing {
                method: &method.to_ascii_uppercase(),
                target: path,
                query: &query,
                body: args.get("body"),
                auth,
                same_host: false,
            };
            self.send_filled(def, &ctx, out).await?
        } else {
            anyhow::bail!("{} has no tool {tool}", connection.id);
        };
        let failed = status >= 400
            || ok.is_some_and(|ok| {
                let doc = serde_json::from_str(&text).unwrap_or(Value::Null);
                Context::of(doc).render(ok) != Some(Value::Bool(true))
            });
        Ok(text_result(text, failed))
    }

    /// The client to ask `url` with, for the connection `ctx` is of.
    fn http_client(&self, ctx: &Context, url: &str) -> anyhow::Result<reqwest::Client> {
        if ctx.public {
            return self.public.client_for(url);
        }
        Ok(self.http.clone())
    }

    /// [`Self::http_client`] that follows where it is sent on only on the host of
    /// `base`.
    fn http_client_on(
        &self,
        ctx: &Context,
        base: url::Url,
        url: &str,
        timeout: std::time::Duration,
    ) -> anyhow::Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder().timeout(timeout);
        if ctx.public {
            builder = self.public.guard(builder, url)?;
        }
        Ok(builder.redirect(staying_on(base)).build()?)
    }

    /// Send one request a definition describes, its strings filled from
    /// `ctx`; the status and the body.
    pub(super) async fn send_defined(
        &self,
        def: &Definition,
        ctx: &Context,
        out: Outgoing<'_>,
    ) -> anyhow::Result<(u16, String)> {
        let target = ctx
            .render_text(out.target)
            .ok_or_else(|| anyhow::anyhow!("the request names a value it does not have"))?;
        let query = ctx.render_map(out.query);
        let body = out.body.and_then(|b| ctx.render_value(b));
        let filled = Outgoing {
            target: &target,
            query: &query,
            body: body.as_ref(),
            ..out
        };
        self.send_filled(def, ctx, filled).await
    }

    /// Send one request whose strings are already filled.
    async fn send_filled(
        &self,
        def: &Definition,
        ctx: &Context,
        out: Outgoing<'_>,
    ) -> anyhow::Result<(u16, String)> {
        let Outgoing {
            method,
            target,
            query,
            body,
            auth,
            same_host,
        } = out;
        let base = def
            .transport
            .http
            .as_ref()
            .and_then(|h| ctx.render_text(&h.base))
            .unwrap_or_default();
        anyhow::ensure!(!same_host || is_url(target), "{target:?} is not a URL");
        let url = url_of(&base, target);
        let http = if same_host {
            let base = url::Url::parse(&base)?;
            anyhow::ensure!(
                on_host(&base, &url::Url::parse(&url)?),
                "{url} is not on the service's host"
            );
            self.http_client_on(ctx, base, &url, PAGE_TIMEOUT)?
        } else {
            self.http_client(ctx, &url)?
        };
        let mut request = http
            .request(reqwest::Method::from_bytes(method.as_bytes())?, &url)
            .header(reqwest::header::USER_AGENT, AGENT)
            .query(&query.iter().collect::<Vec<_>>());
        if let Some((name, value)) = self.auth_header(def, ctx, auth) {
            request = request.header(name, value);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        let status = response.status().as_u16();
        anyhow::ensure!(
            !same_host || is_text(response.headers()),
            "{url} is not text"
        );
        if ctx.public {
            return Ok((status, text_within(response).await?));
        }
        Ok((status, response.text().await?))
    }

    /// Fetch the file a tool names. A redirect is followed only on the host
    /// asked, so a login page elsewhere is never taken for the file.
    async fn fetch_file(
        &self,
        def: &Definition,
        ctx: &Context,
        tool: &HttpTool,
        auth: Option<&str>,
    ) -> anyhow::Result<ToolResult> {
        let base = def
            .transport
            .http
            .as_ref()
            .and_then(|h| ctx.render_text(&h.base))
            .unwrap_or_default();
        let target = ctx
            .render_text(&tool.path)
            .ok_or_else(|| anyhow::anyhow!("the request names a value it does not have"))?;
        let url = url::Url::parse(&url_of(&base, &target))?;
        let mut request = self
            .http_client_on(ctx, url.clone(), url.as_str(), FILE_TIMEOUT)?
            .get(url.clone())
            .header(reqwest::header::USER_AGENT, AGENT)
            .query(&ctx.render_map(&tool.query).iter().collect::<Vec<_>>());
        if let Some((name, value)) = self.auth_header(def, ctx, auth) {
            request = request.header(name, value);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) if error.is_redirect() => {
                return Ok(text_result(SENT_ELSEWHERE.to_owned(), true));
            }
            Err(error) => return Err(error.into()),
        };
        let status = response.status();
        if !status.is_success() {
            return Ok(text_result(format!("answered {status}"), true));
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .unwrap_or("application/octet-stream")
            .trim()
            .to_owned();
        let bytes = all_of(response).await?;
        Ok(Fetched { mime, bytes }.into_result(url.as_str()))
    }

    /// The tool that reads `url`, a link in what `tool` of a connection
    /// answered, when the definition lets it be opened.
    pub fn opener(
        &self,
        connection: &Connection,
        tool: impl AsRef<str>,
        url: impl AsRef<str>,
    ) -> Option<String> {
        let def = self.definition_of(connection)?.ok()?;
        let opens = def
            .tools
            .iter()
            .find(|t| t.name == tool.as_ref())?
            .opens
            .clone()?;
        let ctx = self.context(connection, &def, Value::Null, false).ok()?;
        let base = ctx.render_text(&def.transport.http.as_ref()?.base)?;
        let base = url::Url::parse(&base).ok()?;
        let url = url::Url::parse(url.as_ref()).ok()?;
        on_host(&base, &url).then_some(opens)
    }
}

/// The whole body, when it is not too large.
async fn all_of(mut response: reqwest::Response) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= MAX_FILE_BYTES,
            "the file is larger than {MAX_FILE_BYTES} bytes"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// The body as text in the charset the server names, when it is not too large.
async fn text_within(response: reqwest::Response) -> anyhow::Result<String> {
    let kind = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .cloned();
    let mut whole = axum::http::Response::new(all_of(response).await?);
    if let Some(kind) = kind {
        whole
            .headers_mut()
            .insert(reqwest::header::CONTENT_TYPE, kind);
    }
    Ok(reqwest::Response::from(whole).text().await?)
}

fn is_url(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

/// Where a request goes: a whole URL as it is, anything else below `base`.
fn url_of(base: &str, target: &str) -> String {
    if is_url(target) {
        target.to_owned()
    } else if target.is_empty() {
        base.to_owned()
    } else {
        format!(
            "{}/{}",
            base.trim_end_matches('/'),
            target.trim_start_matches('/')
        )
    }
}

/// Following where a request is sent on only on the host of `base`.
fn staying_on(base: url::Url) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |to| {
        if to.previous().len() > MAX_REDIRECTS {
            to.error("too many redirects")
        } else if on_host(&base, to.url()) {
            to.follow()
        } else {
            to.error(SENT_ELSEWHERE)
        }
    })
}

/// Whether `url` is on the host of `base`, where http may become https but
/// not the other way.
fn on_host(base: &url::Url, url: &url::Url) -> bool {
    let at = |u: &url::Url| (u.scheme().to_owned(), u.port_or_known_default());
    let (from, to) = (at(base), at(url));
    let upgraded = from == ("http".to_owned(), Some(80)) && to == ("https".to_owned(), Some(443));
    ["http", "https"].contains(&url.scheme())
        && base.host_str() == url.host_str()
        && (from == to || upgraded)
}

fn is_text(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|kind| kind.starts_with("text/") || kind.contains("xml"))
}
