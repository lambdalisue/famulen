//! Service definitions: how to reach a service, written as data (SERVICES.md).
//! The gateway knows no service by name; it reads these and does what they
//! say. A definition is checked as it is read, and one that does not read
//! is kept with the reason instead of being half used.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::permissions::{Marks, Permission};

mod check;
mod loosened;
mod set;
pub use check::{FORMAT, parse};
pub use loosened::loosened;
pub use set::{BUNDLED, Definitions};

/// One service definition, as SERVICES.md describes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub format: u32,
    pub id: String,
    pub name: String,
    /// What it gives, in a few words, for the owner choosing among services.
    #[serde(default)]
    pub about: Option<String>,
    #[serde(default)]
    pub setup: Option<String>,
    #[serde(default)]
    account: Option<toml::Table>,
    #[serde(default)]
    app: Option<toml::Table>,
    #[serde(default)]
    pub login: Option<Login>,
    pub transport: Transport,
    #[serde(default)]
    pub tools: Vec<HttpTool>,
    #[serde(default)]
    pub commands: Vec<CommandTool>,
    /// The receivers by name.
    #[serde(default)]
    pub receive: BTreeMap<String, Receive>,
    #[serde(default)]
    pub permissions: BTreeMap<String, PermissionName>,
    /// Filled from `account` once checked.
    #[serde(skip)]
    pub fields: Vec<AccountField>,
    #[serde(skip)]
    pub display: Option<String>,
    /// Filled from `app` once checked.
    #[serde(skip)]
    pub app_spec: Option<AppSpec>,
    /// Where the definition was read from.
    #[serde(skip)]
    pub source: String,
    /// The text it was read from, as written.
    #[serde(skip)]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountField {
    pub name: String,
    pub label: String,
    pub optional: bool,
    /// Kept in the vault and filled only into what goes out.
    pub secret: bool,
    /// Made by the gateway when missing; nobody is asked for it.
    pub generated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppSpec {
    pub id: String,
    pub values: Vec<AppValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppValue {
    pub name: String,
    pub env: String,
    pub secret: bool,
    pub optional: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionName {
    Allow,
    Confirm,
    Forbid,
}

impl From<PermissionName> for Permission {
    fn from(name: PermissionName) -> Self {
        match name {
            PermissionName::Allow => Permission::Allow,
            PermissionName::Confirm => Permission::Confirm,
            PermissionName::Forbid => Permission::Forbid,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    #[serde(default)]
    pub key: Option<KeyLogin>,
    #[serde(default)]
    pub oauth2: Option<OAuth2>,
    #[serde(default)]
    pub session: Option<Session>,
    #[serde(default)]
    pub whoami: Option<WhoAmI>,
    #[serde(default)]
    pub mcp: Option<bool>,
    /// Tokens traded for just before a tool that carries one, by name.
    #[serde(default)]
    pub exchange: BTreeMap<String, Exchange>,
}

/// A token got by handing over an account value that is spent in doing so:
/// the answer brings the value to hand over next time.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exchange {
    pub post: String,
    pub form: BTreeMap<String, String>,
    /// Where in the answer the token is.
    pub token: String,
    /// The secret account fields the answer replaces, and where they are.
    pub renews: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyLogin {
    pub label: String,
    pub header: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuth2 {
    pub authorize: String,
    pub token: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub query: BTreeMap<String, String>,
    #[serde(default)]
    pub tokens: BTreeMap<String, String>,
    /// The order `tokens` was written in; the first is `{token}`.
    #[serde(skip)]
    pub token_order: Vec<String>,
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    #[serde(default)]
    pub same: Vec<String>,
    #[serde(default)]
    pub refresh: bool,
}

/// A login that trades the account's values for tokens that last a while.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub post: String,
    pub body: Value,
    pub tokens: BTreeMap<String, String>,
    /// The order `tokens` was written in; the first is `{token}`.
    #[serde(skip)]
    pub token_order: Vec<String>,
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    /// How long the tokens last before they are asked for again.
    pub lifetime: String,
}

impl Session {
    pub fn lifetime_secs(&self) -> anyhow::Result<i64> {
        seconds(&self.lifetime)
            .and_then(|s| i64::try_from(s).ok())
            .ok_or_else(|| anyhow::anyhow!("login.session.lifetime is like \"1h\""))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WhoAmI {
    #[serde(flatten)]
    pub request: Request,
    #[serde(default)]
    pub values: BTreeMap<String, String>,
}

/// A request written with its method as the key: `get = "/path"`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Request {
    #[serde(default)]
    get: Option<String>,
    #[serde(default)]
    post: Option<String>,
    #[serde(default)]
    put: Option<String>,
    #[serde(default)]
    patch: Option<String>,
    #[serde(default)]
    delete: Option<String>,
    #[serde(default)]
    pub query: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<Value>,
}

impl Request {
    /// The method and the path or URL.
    pub fn target(&self) -> Option<(&'static str, &str)> {
        let all = [
            ("GET", &self.get),
            ("POST", &self.post),
            ("PUT", &self.put),
            ("PATCH", &self.patch),
            ("DELETE", &self.delete),
        ];
        let mut set = all
            .into_iter()
            .filter_map(|(m, p)| Some((m, p.as_deref()?)));
        let first = set.next()?;
        set.next().is_none().then_some(first)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transport {
    #[serde(default)]
    pub mcp_local: Option<McpLocal>,
    #[serde(default)]
    pub mcp_remote: Option<McpRemote>,
    #[serde(default)]
    pub http: Option<HttpTransport>,
    #[serde(default)]
    pub command: Option<CommandTransport>,
}

/// A program on this machine, run anew for every call.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandTransport {
    pub program: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Variables of the gateway's own environment it must not see.
    #[serde(default)]
    pub unset: Vec<String>,
    #[serde(default)]
    pub timeout: Option<String>,
}

impl CommandTransport {
    /// How long a call may run.
    pub fn timeout_secs(&self) -> u64 {
        self.timeout.as_deref().and_then(seconds).unwrap_or(60)
    }
}

/// A tool that runs the program with these arguments.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandTool {
    pub name: String,
    pub description: String,
    pub args: Vec<CommandArg>,
    #[serde(default)]
    pub input: Option<Value>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default)]
    pub open_world: bool,
    /// What the program reads on its standard input.
    #[serde(default)]
    pub stdin: Option<String>,
    /// The owner is asked of every call, whatever permission is chosen, and
    /// the gateway itself makes the call they approve.
    #[serde(default)]
    pub each_time: Option<EachTime>,
}

/// What the owner is shown beside a call they are asked of each time.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EachTime {
    /// The reading tool, called with the same arguments, whose answer holds
    /// what is being decided.
    pub reads: String,
    /// Where in its answer that is.
    pub at: String,
    /// What must be the same there as in the call, by where it is.
    #[serde(default)]
    pub same: BTreeMap<String, String>,
    /// Where, in what is decided, the questions are that the owner answers
    /// instead of approving, each with its `question`, its `options` (a
    /// `label` and a `description`) and whether several may be chosen
    /// (`multiSelect`).
    #[serde(default)]
    pub questions: Option<String>,
    /// The argument their answers are put in, one for each question.
    #[serde(default)]
    pub fills: Option<String>,
    /// The arguments run when the owner says no; nothing without them.
    #[serde(default)]
    pub on_reject: Option<Vec<CommandArg>>,
}

/// One argument; arguments left out together when one of them has no value;
/// or one given only when a value is true.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum CommandArg {
    Text(String),
    Group(Vec<String>),
    Flag {
        flag: String,
        when: String,
    },
    /// The flag before each item of a list.
    Each {
        flag: String,
        each: String,
    },
}

impl CommandTool {
    pub fn marks(&self) -> Marks {
        Marks {
            read_only: Some(self.read_only),
            always_ask: self.destructive || self.open_world,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpLocal {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpRemote {
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpTransport {
    pub base: String,
    #[serde(default)]
    pub auth: Option<String>,
    #[serde(default)]
    pub openapi: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpTool {
    pub name: String,
    pub description: String,
    pub method: Method,
    pub path: String,
    #[serde(default)]
    pub query: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<Value>,
    #[serde(default)]
    pub input: Option<Value>,
    #[serde(default)]
    pub read_only: Option<bool>,
    /// It cannot be undone; when unsaid, only `DELETE` cannot.
    #[serde(default)]
    pub destructive: Option<bool>,
    /// It reaches other people, such as sending a message or an invitation.
    #[serde(default)]
    pub open_world: bool,
    #[serde(default)]
    pub auth: Option<String>,
    #[serde(default)]
    pub ok: Option<String>,
    /// Send it, and follow where it is sent on, only on the base's host.
    #[serde(default)]
    pub same_host: bool,
    /// The tool that reads the pages this one's answer links to.
    #[serde(default)]
    pub opens: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receive {
    /// What whoever takes what arrives here should know of it.
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub http: Option<HttpReceive>,
    #[serde(default)]
    pub connect: Option<Connect>,
    /// The destination suggested to the program using the gateway.
    #[serde(default)]
    pub destination: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpReceive {
    pub verify: Verify,
    #[serde(default)]
    pub challenge: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verify {
    #[serde(default)]
    pub hmac_sha256: Option<Hmac>,
    #[serde(default)]
    pub token: Option<TokenVerify>,
    #[serde(default)]
    pub jwt: Option<JwtVerify>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hmac {
    pub secret: String,
    pub base: String,
    pub signature: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub max_age: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenVerify {
    pub value: String,
    pub equals: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JwtVerify {
    pub value: String,
    pub issuer: String,
    pub audience: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connect {
    pub protocol: Protocol,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub account: BTreeMap<String, Places>,
    #[serde(default)]
    pub subscription: Option<String>,
    #[serde(default)]
    pub auth: Option<String>,
    /// The arguments the transport's program is kept running with.
    #[serde(default)]
    pub args: Vec<String>,
    /// More arguments, filled from the last line taken, that make it go on
    /// from there.
    #[serde(default)]
    pub resume: Vec<String>,
    /// Which lines are handed over: those whose value at each place is one
    /// of these.
    #[serde(default)]
    pub when: BTreeMap<String, Vec<String>>,
    /// Hand a line over only once no other line of the same `per` came for
    /// this long.
    #[serde(default)]
    pub settle: Option<String>,
    /// Do not wait to settle once the owner has not touched the machine for
    /// this long.
    #[serde(default)]
    pub away: Option<String>,
    /// The app, by its bundle id, the owner would see a line in: wait to
    /// settle only while it is in front.
    #[serde(default)]
    pub front: Option<String>,
    /// What lines are about, for `settle`.
    #[serde(default)]
    pub per: Option<String>,
}

/// Where a value is: one place, or the first of several that has it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Places {
    One(String),
    First(Vec<String>),
}

impl Places {
    pub fn each(&self) -> &[String] {
        match self {
            Self::One(at) => std::slice::from_ref(at),
            Self::First(all) => all,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    SlackSocketMode,
    PubsubPull,
    CommandLines,
}

/// `30s`, `5m`, `1h` as seconds.
pub fn seconds(text: &str) -> Option<u64> {
    let text = text.trim();
    let (number, unit) = text.split_at(text.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = number.parse().ok()?;
    n.checked_mul(match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        _ => return None,
    })
}

impl Definition {
    /// Whether the token `name` is traded for at each use.
    pub fn exchanges(&self, name: &str) -> bool {
        self.login
            .as_ref()
            .is_some_and(|l| l.exchange.contains_key(name))
    }

    /// Whether a written tool carries a token traded for at each use.
    fn spends(&self, tool: &HttpTool) -> bool {
        let usual = self.transport.http.as_ref().and_then(|h| h.auth.as_ref());
        tool.auth
            .as_ref()
            .or(usual)
            .is_some_and(|a| self.exchanges(a))
    }

    /// Where its `setup` is: a URL as written, or a path beside the
    /// definition's file.
    pub fn setup_place(&self) -> Option<String> {
        let setup = self.setup.as_ref()?;
        if setup.starts_with("http://") || setup.starts_with("https://") {
            return Some(setup.clone());
        }
        let dir = Path::new(&self.source).parent()?;
        let place = dir.join(setup).display().to_string();
        Some(if cfg!(windows) {
            place.replace('\\', "/")
        } else {
            place
        })
    }
}

impl HttpTool {
    /// What the tool says of itself, as the auto permission weighs it.
    pub fn marks(&self) -> Marks {
        Marks {
            read_only: Some(self.read_only.unwrap_or(self.method == Method::Get)),
            always_ask: self.destructive.unwrap_or(self.method == Method::Delete)
                || self.open_world,
        }
    }
}

#[cfg(test)]
mod tests;
