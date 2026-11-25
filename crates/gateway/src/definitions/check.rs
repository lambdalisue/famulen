//! Reading a definition and checking its shape.

use super::{
    AccountField, AppSpec, AppValue, CommandTransport, Connect, Definition, HttpReceive, Login,
    Method, Protocol, Receive, seconds,
};

/// The only format version this gateway reads.
pub const FORMAT: u32 = 1;

/// Read and check one definition.
pub fn parse(text: &str) -> anyhow::Result<Definition> {
    let raw: toml::Table = toml::from_str(text)?;
    let mut def: Definition = raw.clone().try_into()?;
    anyhow::ensure!(
        def.format == FORMAT,
        "format {} is not {FORMAT}",
        def.format
    );
    anyhow::ensure!(
        is_id(&def.id),
        "id {:?} is not lowercase letters, digits and '-'",
        def.id
    );
    anyhow::ensure!(!def.name.trim().is_empty(), "name is empty");
    if let Some(account) = def.account.take() {
        (def.fields, def.display) = account_fields(account)?;
    }
    if let Some(app) = def.app.take() {
        def.app_spec = Some(app_spec(app)?);
        anyhow::ensure!(
            def.setup.is_some(),
            "setup is required when there is an app"
        );
    }
    let has_app = def.app_spec.is_some();
    if let Some(login) = &mut def.login {
        order_tokens(login, &raw);
        check_login(login, has_app)?;
    }
    check_exchange(&def)?;
    check_transport(&def)?;
    check_tools(&def)?;
    for (name, receive) in &def.receive {
        check_receive(name, receive, def.transport.command.as_ref())?;
    }
    Ok(def)
}

/// The fields `[account]` asks for, and how an account is shown.
fn account_fields(account: toml::Table) -> anyhow::Result<(Vec<AccountField>, Option<String>)> {
    let mut fields = Vec::new();
    let mut display = None;
    for (name, value) in account {
        if name == "display" {
            let text = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("account.display is not a string"))?;
            display = Some(text.to_owned());
            continue;
        }
        let at = format!("account.{name}");
        anyhow::ensure!(is_name(&name), "account field {name:?} is not a name");
        let table = table(&value, &["label", "optional", "secret", "generate"], &at)?;
        if flag(table, "generate", &at)? {
            anyhow::ensure!(
                table.len() == 1,
                "{at} is generated, so nothing else is said of it"
            );
            fields.push(AccountField {
                label: String::new(),
                optional: true,
                secret: true,
                generated: true,
                name,
            });
            continue;
        }
        fields.push(AccountField {
            generated: false,
            label: string(table, "label", &at)?,
            optional: flag(table, "optional", &at)?,
            secret: flag(table, "secret", &at)?,
            name,
        });
    }
    Ok((fields, display))
}

fn app_spec(app: toml::Table) -> anyhow::Result<AppSpec> {
    let id = app
        .get("id")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("app.id is missing"))?
        .to_owned();
    anyhow::ensure!(
        is_id(&id),
        "app.id {id:?} is not lowercase letters, digits and '-'"
    );
    let mut values = Vec::new();
    for (name, value) in app {
        if name == "id" {
            continue;
        }
        let at = format!("app.{name}");
        anyhow::ensure!(is_name(&name), "app value {name:?} is not a name");
        let table = table(&value, &["env", "secret", "optional"], &at)?;
        values.push(AppValue {
            env: string(table, "env", &at)?,
            secret: flag(table, "secret", &at)?,
            optional: flag(table, "optional", &at)?,
            name,
        });
    }
    Ok(AppSpec { id, values })
}

/// Keep the order the login's tokens were written in, which their map loses.
fn order_tokens(login: &mut Login, raw: &toml::Table) {
    let written = |kind: &str| -> Vec<String> {
        raw.get("login")
            .and_then(|l| l.get(kind)?.get("tokens")?.as_table())
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default()
    };
    if let Some(session) = &mut login.session {
        session.token_order = written("session");
    }
    if let Some(oauth2) = &mut login.oauth2 {
        oauth2.token_order = written("oauth2");
    }
}

fn check_login(login: &Login, has_app: bool) -> anyhow::Result<()> {
    let kinds = [
        login.key.is_some(),
        login.oauth2.is_some(),
        login.session.is_some(),
        login.mcp == Some(true),
    ];
    anyhow::ensure!(
        exactly_one(&kinds),
        "login is exactly one of key, oauth2, session, mcp"
    );
    if let Some(whoami) = &login.whoami {
        anyhow::ensure!(
            login.oauth2.is_some(),
            "login.whoami goes with login.oauth2"
        );
        anyhow::ensure!(
            whoami.request.target().is_some(),
            "login.whoami needs one method"
        );
    }
    if let Some(session) = &login.session {
        session.lifetime_secs()?;
        anyhow::ensure!(
            !session.tokens.is_empty(),
            "login.session.tokens names no token"
        );
    }
    if let Some(oauth2) = &login.oauth2 {
        anyhow::ensure!(has_app, "login.oauth2 needs an app");
        for name in &oauth2.same {
            anyhow::ensure!(
                oauth2.values.contains_key(name),
                "login.oauth2.same names {name:?}, which is not in values"
            );
        }
    }
    Ok(())
}

fn check_exchange(def: &Definition) -> anyhow::Result<()> {
    let Some(login) = &def.login else {
        return Ok(());
    };
    for (name, exchange) in &login.exchange {
        anyhow::ensure!(is_name(name), "login.exchange {name:?} is not a name");
        anyhow::ensure!(
            login.oauth2.is_some() || login.session.is_some(),
            "login.exchange goes with login.oauth2 or login.session"
        );
        let brought = [
            login.oauth2.as_ref().map(|o| &o.tokens),
            login.session.as_ref().map(|s| &s.tokens),
        ];
        anyhow::ensure!(
            !brought.into_iter().flatten().any(|t| t.contains_key(name)),
            "login.exchange.{name} is a token the login already brings"
        );
        anyhow::ensure!(
            !exchange.renews.is_empty(),
            "login.exchange.{name}.renews names no field"
        );
        for field in exchange.renews.keys() {
            anyhow::ensure!(
                def.fields.iter().any(|f| f.secret && &f.name == field),
                "login.exchange.{name} renews {field:?}, which is no secret account field"
            );
            let embed = format!("{{account.{field}}}");
            for tool in &def.tools {
                anyhow::ensure!(
                    !format!("{}{:?}{:?}", tool.path, tool.query, tool.body).contains(&embed),
                    "{} embeds {embed}, which only the exchange may hand over",
                    tool.name
                );
            }
        }
    }
    Ok(())
}

fn check_transport(def: &Definition) -> anyhow::Result<()> {
    let t = &def.transport;
    let kinds = [
        t.mcp_local.is_some(),
        t.mcp_remote.is_some(),
        t.http.is_some(),
        t.command.is_some(),
    ];
    anyhow::ensure!(
        exactly_one(&kinds),
        "transport is exactly one of mcp_local, mcp_remote, http, command"
    );
    anyhow::ensure!(
        def.tools.is_empty() || t.http.is_some(),
        "tools go with the http transport"
    );
    anyhow::ensure!(
        def.commands.is_empty() != t.command.is_some(),
        "commands go with the command transport"
    );
    duration(
        "timeout",
        t.command.as_ref().and_then(|c| c.timeout.as_ref()),
    )
}

fn check_tools(def: &Definition) -> anyhow::Result<()> {
    for tool in &def.commands {
        anyhow::ensure!(is_name(&tool.name), "command {:?} is not a name", tool.name);
        if let Some(each) = &tool.each_time {
            anyhow::ensure!(
                def.commands
                    .iter()
                    .any(|t| t.name == each.reads && t.read_only),
                "{} shows what {} reads, which is no reading command",
                tool.name,
                each.reads
            );
            anyhow::ensure!(
                each.questions.is_some() == each.fills.is_some(),
                "{}: questions go with fills",
                tool.name
            );
        }
    }
    for tool in &def.tools {
        anyhow::ensure!(is_name(&tool.name), "tool {:?} is not a name", tool.name);
        if let Some(opens) = &tool.opens {
            anyhow::ensure!(
                def.tools
                    .iter()
                    .any(|t| t.name == *opens && t.method == Method::Get && t.same_host),
                "{} opens {opens:?}, which is no GET tool with same_host",
                tool.name
            );
        }
    }
    Ok(())
}

fn check_receive(
    name: &str,
    receive: &Receive,
    command: Option<&CommandTransport>,
) -> anyhow::Result<()> {
    anyhow::ensure!(is_name(name), "receiver {name:?} is not a name");
    anyhow::ensure!(
        receive.http.is_some() != receive.connect.is_some(),
        "receive.{name} is exactly one of http, connect"
    );
    if let Some(http) = &receive.http {
        check_verify(name, http)?;
    }
    if let Some(connect) = &receive.connect {
        check_connect(connect, command)?;
    }
    Ok(())
}

fn check_verify(name: &str, http: &HttpReceive) -> anyhow::Result<()> {
    let v = &http.verify;
    let kinds = [v.hmac_sha256.is_some(), v.token.is_some(), v.jwt.is_some()];
    anyhow::ensure!(
        exactly_one(&kinds),
        "receive.{name}.http.verify is exactly one of hmac_sha256, token, jwt"
    );
    duration(
        "max_age",
        v.hmac_sha256.as_ref().and_then(|h| h.max_age.as_ref()),
    )
}

fn check_connect(connect: &Connect, command: Option<&CommandTransport>) -> anyhow::Result<()> {
    duration("settle", connect.settle.as_ref())?;
    duration("away", connect.away.as_ref())?;
    match connect.protocol {
        Protocol::SlackSocketMode => {
            anyhow::ensure!(connect.token.is_some(), "slack-socket-mode needs a token")
        }
        Protocol::CommandLines => anyhow::ensure!(
            !connect.args.is_empty() && command.is_some(),
            "command-lines needs args and the command transport"
        ),
        Protocol::PubsubPull => anyhow::ensure!(
            connect.subscription.is_some(),
            "pubsub-pull needs a subscription"
        ),
    }
    Ok(())
}

fn exactly_one(kinds: &[bool]) -> bool {
    kinds.iter().filter(|k| **k).count() == 1
}

fn duration(name: &str, value: Option<&String>) -> anyhow::Result<()> {
    if let Some(value) = value {
        anyhow::ensure!(
            seconds(value).is_some(),
            "{name} {value:?} is not a duration"
        );
    }
    Ok(())
}

/// The table a value is, with none but the fields `allowed`.
fn table<'a>(
    value: &'a toml::Value,
    allowed: &[&str],
    at: &str,
) -> anyhow::Result<&'a toml::Table> {
    let table = value
        .as_table()
        .ok_or_else(|| anyhow::anyhow!("{at} is not a table"))?;
    for key in table.keys() {
        anyhow::ensure!(allowed.contains(&key.as_str()), "unknown field {at}.{key}");
    }
    Ok(table)
}

fn is_id(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn string(table: &toml::Table, key: &str, at: &str) -> anyhow::Result<String> {
    Ok(table
        .get(key)
        .and_then(toml::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("{at}.{key} is missing or not a string"))?
        .to_owned())
}

fn flag(table: &toml::Table, key: &str, at: &str) -> anyhow::Result<bool> {
    match table.get(key) {
        None => Ok(false),
        Some(v) => v
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("{at}.{key} is not true or false")),
    }
}
