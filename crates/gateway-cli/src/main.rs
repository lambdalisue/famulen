//! `famulen-gateway`: the owner's own door to a gateway directory, without
//! the Famulen service. Every call is made as the owner and goes through the
//! same permissions, vault and log the library keeps.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use famulen_gateway::connections::Connections;
use famulen_gateway::log::LogQuery;
use famulen_gateway::permissions::{Permission, Permissions};
use famulen_gateway::store::Store;
use famulen_gateway::{
    Arrival, Call, Gateway, KEY_VAR, OWNER, Outcome, Vault, VaultKey, definitions,
};
use serde_json::{Value, json};

/// The destination whose arrivals `receive` prints.
const CLI: &str = "cli";

#[derive(Parser)]
#[command(
    name = "famulen-gateway",
    version,
    about = "Use a gateway directory as its owner: connections, tools, permissions, the vault and the log"
)]
struct Cli {
    /// The gateway's directory.
    #[arg(
        long,
        short = 'C',
        env = "FAMULEN_GATEWAY_DIR",
        default_value = ".",
        global = true
    )]
    dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print a new vault key, for FAMULEN_VAULT_KEY.
    NewKey,
    /// The service definitions.
    #[command(subcommand)]
    Definitions(Definitions),
    /// The connections.
    #[command(subcommand)]
    Connections(ConnectionCommand),
    /// Start a connection's login: print the URL to open.
    Login {
        id: String,
        /// Where the service sends the answer.
        #[arg(long)]
        redirect: String,
    },
    /// Finish a login with the URL (or its query) the service sent back.
    Finish { answer: String },
    /// A connection's tools.
    Tools { id: String },
    /// Call a connection's tool; the input is JSON, `{}` when left out.
    Call {
        id: String,
        tool: String,
        input: Option<String>,
    },
    /// A connection's permissions, or set one: auto, allow, confirm, forbid or default.
    Permissions {
        id: String,
        tool: Option<String>,
        permission: Option<String>,
    },
    /// The vault. Values are never printed.
    #[command(subcommand)]
    Vault(VaultCommand),
    /// A connection's receivers and where each goes, or send one to a
    /// destination (`cli`, a name the program using the gateway registers,
    /// or `none` to close it).
    Destinations {
        id: String,
        receiver: Option<String>,
        destination: Option<String>,
    },
    /// The log, newest first.
    Log {
        #[arg(long, default_value = "")]
        actor: String,
        #[arg(long, default_value = "")]
        connection: String,
        /// allow, confirm or deny.
        #[arg(long, default_value = "")]
        decision: String,
        /// Only entries before this one, to read on.
        #[arg(long, default_value_t = 0)]
        before: i64,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Open the receivers and print what arrives for those sent to `cli`,
    /// one JSON line each, as it arrived.
    Receive {
        #[arg(long, env = "FAMULEN_RECEIVE_LISTEN")]
        listen: SocketAddr,
    },
}

#[derive(Subcommand)]
enum Definitions {
    /// The definitions read, and those that did not read with why.
    List,
    /// Check one definition file.
    Check { file: PathBuf },
}

#[derive(Subcommand)]
enum ConnectionCommand {
    List,
    /// Add a connection from a definition, with its account values.
    Add {
        definition: String,
        /// An account value, `name=value`.
        #[arg(long = "value")]
        values: Vec<String>,
        #[arg(long)]
        name: Option<String>,
    },
    /// Set a connection's key, read from stdin.
    Key {
        id: String,
        /// The header it goes in, when the definition does not say.
        #[arg(long, default_value = "")]
        header: String,
    },
    /// Set one account value after adding; a secret one is read from stdin.
    Set {
        id: String,
        field: String,
    },
    Remove {
        id: String,
    },
}

#[derive(Subcommand)]
enum VaultCommand {
    List,
    /// Put a secret in, or replace it; the value is read from stdin.
    Set {
        name: String,
        #[arg(long, default_value = "")]
        purpose: String,
    },
    /// Say what a secret is for, leaving its value as it is.
    Describe {
        name: String,
        description: String,
    },
    Delete {
        name: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn open(dir: &std::path::Path) -> anyhow::Result<Gateway> {
    let key = std::env::var(KEY_VAR)
        .ok()
        .filter(|k| !k.trim().is_empty())
        .with_context(|| {
            format!(
                "{KEY_VAR} is not set; the vault needs it. A new one: {}",
                VaultKey::generate()
            )
        })?;
    std::fs::create_dir_all(dir)?;
    let store = Store::open(dir.join("gateway.sqlite3"))?;
    let vault = Vault::open(dir.join("vault"), &VaultKey::parse(key)?)?;
    Ok(Gateway::new(
        store.clone(),
        Connections::new(store),
        reqwest::Client::new(),
        Some(vault),
    )
    .with_definitions(Some(&dir.join("services")), &[]))
}

fn stdin_value() -> anyhow::Result<String> {
    let mut value = String::new();
    std::io::stdin().read_to_string(&mut value)?;
    let value = value.trim_end_matches(['\r', '\n']).to_owned();
    anyhow::ensure!(!value.is_empty(), "nothing was given on stdin");
    Ok(value)
}

fn time(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::NewKey => {
            println!("{}", VaultKey::generate());
            return Ok(());
        }
        Command::Definitions(Definitions::Check { file }) => {
            let text = std::fs::read_to_string(&file)?;
            definitions::parse(&text).with_context(|| file.display().to_string())?;
            println!("ok");
            return Ok(());
        }
        Command::Definitions(Definitions::List) => {
            let defs = definitions::Definitions::load(Some(&cli.dir.join("services")), &[]);
            for d in defs.list() {
                let receivers: Vec<&str> = d.receive.keys().map(String::as_str).collect();
                println!(
                    "{}\t{}\t{}\t{}",
                    d.id,
                    d.name,
                    receivers.join(","),
                    d.source
                );
            }
            for (id, why) in defs.broken() {
                println!("{id}\t(did not read)\t\t{why}");
            }
            return Ok(());
        }
        _ => {}
    }
    let gateway = open(&cli.dir)?;
    match cli.command {
        Command::NewKey | Command::Definitions(_) => unreachable!("answered above"),
        Command::Connections(c) => connections(&gateway, c).await,
        Command::Login { id, redirect } => {
            println!("{}", gateway.begin_defined_login(&id, &redirect)?);
            Ok(())
        }
        Command::Finish { answer } => {
            let query = match url::Url::parse(&answer) {
                Ok(url) => url.query().unwrap_or_default().to_owned(),
                Err(_) => answer.trim_start_matches('?').to_owned(),
            };
            println!("{}", gateway.finish_defined_login(&query).await?);
            Ok(())
        }
        Command::Tools { id } => {
            for t in gateway.tools_of(&id).await? {
                let read = if t["annotations"]["readOnlyHint"] == json!(true) {
                    "read"
                } else {
                    "write"
                };
                println!(
                    "{}\t{read}\t{}",
                    t["name"].as_str().unwrap_or_default(),
                    t["description"].as_str().unwrap_or_default()
                );
            }
            Ok(())
        }
        Command::Call { id, tool, input } => {
            let input: Value = match input {
                Some(text) => serde_json::from_str(&text).context("the input is not JSON")?,
                None => json!({}),
            };
            match gateway.call(&Call::new(OWNER, &id, &tool, input)).await? {
                Outcome::Done(result) => {
                    println!("{}", result.text);
                    anyhow::ensure!(!result.is_error, "the tool failed");
                    Ok(())
                }
                Outcome::Held(ticket) => anyhow::bail!("held as #{}", ticket.id),
            }
        }
        Command::Permissions {
            id,
            tool,
            permission,
        } => permissions(&gateway, &id, tool, permission).await,
        Command::Vault(v) => vault(&gateway, v),
        Command::Destinations {
            id,
            receiver,
            destination,
        } => {
            if let Some(receiver) = receiver {
                let destination = destination.context("which destination: cli, none or a name")?;
                let name = (destination != "none").then_some(destination.as_str());
                gateway.set_destination(OWNER, &id, &receiver, name)?;
                return Ok(());
            }
            for (receiver, to) in gateway.receivers_of(&id)? {
                println!("{receiver}\t{}", to.as_deref().unwrap_or("none"));
            }
            Ok(())
        }
        Command::Log {
            actor,
            connection,
            decision,
            before,
            limit,
        } => {
            let query = LogQuery {
                actor,
                connection,
                decision,
                before,
                limit,
            };
            for (n, e) in gateway.log_page(&query)? {
                println!(
                    "{n}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}ms",
                    time(e.at),
                    e.actor,
                    e.connection,
                    e.tool,
                    e.decision,
                    e.reason,
                    e.secret,
                    e.outcome,
                    e.digest,
                    e.ms
                );
            }
            Ok(())
        }
        Command::Receive { listen } => receive(gateway, listen).await,
    }
}

async fn connections(gateway: &Gateway, command: ConnectionCommand) -> anyhow::Result<()> {
    let list = Connections::new(gateway.store().clone());
    match command {
        ConnectionCommand::List => {
            for c in list.list()? {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    c.id, c.name, c.kind, c.account, c.status
                );
            }
        }
        ConnectionCommand::Add {
            definition,
            values,
            name,
        } => {
            let values = values
                .iter()
                .map(|v| {
                    v.split_once('=')
                        .map(|(k, v)| (k.trim().to_owned(), v.to_owned()))
                        .with_context(|| format!("{v} is not name=value"))
                })
                .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
            let c = gateway.add_defined(&definition, values, name.as_deref())?;
            println!("{}", c.id);
        }
        ConnectionCommand::Key { id, header } => {
            gateway.set_api_key(&id, stdin_value()?, header)?;
        }
        ConnectionCommand::Set { id, field } => {
            gateway
                .set_account_value(OWNER, &id, &field, &stdin_value()?)
                .await?;
        }
        ConnectionCommand::Remove { id } => gateway.remove_connection(OWNER, &id)?,
    }
    Ok(())
}

async fn permissions(
    gateway: &Gateway,
    id: &str,
    tool: Option<String>,
    permission: Option<String>,
) -> anyhow::Result<()> {
    if let Some(tool) = tool {
        let permission =
            permission.context("which permission: auto, allow, confirm, forbid or default")?;
        let permission = match permission.as_str() {
            "default" => None,
            other => Some(Permission::parse(other)?),
        };
        return gateway.set_permission(OWNER, format!("conn:{id}"), tool, permission);
    }
    let permissions = Permissions::new(gateway.store().clone());
    for t in gateway.tools_of(id).await? {
        let name = t["name"].as_str().unwrap_or_default();
        let (permission, from) = match permissions.chosen(id, name)? {
            Some((permission, from)) => (permission, format!("{from:?}").to_lowercase()),
            None => (permissions.default_of(id, name)?, "default".to_owned()),
        };
        println!("{name}\t{}\t{from}", permission.as_str());
    }
    Ok(())
}

fn vault(gateway: &Gateway, command: VaultCommand) -> anyhow::Result<()> {
    match command {
        VaultCommand::List => {
            for e in gateway.secrets()? {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    e.name,
                    e.used_by,
                    e.description,
                    e.by,
                    time(e.updated_at)
                );
            }
        }
        VaultCommand::Set { name, purpose } => {
            if let Outcome::Held(t) = gateway.put_secret(OWNER, &name, stdin_value()?, purpose)? {
                anyhow::bail!("held as #{}", t.id);
            }
        }
        VaultCommand::Describe { name, description } => {
            if let Outcome::Held(t) = gateway.describe_secret(OWNER, &name, description)? {
                anyhow::bail!("held as #{}", t.id);
            }
        }
        VaultCommand::Delete { name } => {
            if let Outcome::Held(t) = gateway.delete_secret(OWNER, &name)? {
                anyhow::bail!("held as #{}", t.id);
            }
        }
    }
    Ok(())
}

async fn receive(gateway: Gateway, listen: SocketAddr) -> anyhow::Result<()> {
    let out = Arc::new(std::sync::Mutex::new(std::io::stdout()));
    gateway.register_destination(
        CLI,
        Arc::new(move |a: Arrival| {
            let line = json!({
                "connection": a.connection,
                "receiver": a.receiver,
                "received_at": a.received_at,
                "headers": a.headers,
                "payload": a.payload,
            });
            let mut out = out.lock().unwrap_or_else(|p| p.into_inner());
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        }),
    );
    gateway.sync_connections()?;
    eprintln!("receiving at http://{listen}/receive/<connection>/<receiver>");
    gateway.serve_receivers(listen).await
}
