//! The CLI as the owner runs it on a gateway directory, without Famulen.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const SECRET: &str = "s3cret-value-that-must-not-show";

fn temp(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("famulen-gateway-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn key() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_famulen-gateway"))
        .arg("new-key")
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn command(dir: &Path, key: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_famulen-gateway"));
    c.arg("--dir")
        .arg(dir)
        .args(args)
        .env_remove("FAMULEN_GATEWAY_DIR")
        .env("FAMULEN_VAULT_KEY", key);
    c
}

fn run(dir: &Path, key: &str, args: &[&str], stdin: &str) -> Output {
    let mut child = command(dir, key, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn ok(dir: &Path, key: &str, args: &[&str], stdin: &str) -> String {
    let out = run(dir, key, args, stdin);
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    );
    assert!(out.status.success(), "{args:?}: {stderr}");
    assert!(!stdout.contains(SECRET) && !stderr.contains(SECRET));
    stdout
}

fn fails(dir: &Path, key: &str, args: &[&str]) -> String {
    let out = run(dir, key, args, "");
    assert!(!out.status.success(), "{args:?} should fail");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Answer HTTP requests with what each asked: its request line and the
/// `Authorization` it carried.
fn stub() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut auth = String::new();
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header.trim().is_empty() {
                    break;
                }
                let (name, value) = header.split_once(':').unwrap();
                match name.to_ascii_lowercase().as_str() {
                    "authorization" => auth = value.trim().to_owned(),
                    "content-length" => length = value.trim().parse().unwrap(),
                    _ => {}
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let answer = serde_json::json!({
                "target": line.split_whitespace().nth(1).unwrap_or_default(),
                "auth_ok": auth == format!("Bearer {SECRET}"),
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                answer.len()
            )
            .unwrap();
        }
    });
    format!("http://{addr}")
}

fn write_definition(dir: &Path) {
    std::fs::create_dir_all(dir.join("services")).unwrap();
    std::fs::write(
        dir.join("services/stub.toml"),
        r#"format = 1
id = "stub"
name = "Stub"

[account]
base = { label = "Base" }
token = { label = "合言葉", secret = true, optional = true }

[login]
key = { label = "鍵", header = "Authorization", value = "Bearer {token}" }

[transport]
http = { base = "{account.base}" }

[[tools]]
name = "ping"
description = "Ask the stub what it was asked."
method = "GET"
path = "/ping"
query = { word = "{args.word}" }
input = { type = "object", properties = { word = { type = "string" } } }

[[tools]]
name = "wipe"
description = "Wipe everything."
method = "POST"
path = "/wipe"

[receive.events.http]
verify = { token = { value = "{header.X-Token}", equals = "{account.token}" } }
"#,
    )
    .unwrap();
}

#[test]
fn definitions_are_listed_bundled_and_the_owners_and_a_file_is_checked() {
    let (dir, key) = (temp("definitions"), key());
    write_definition(&dir);
    let listed = ok(&dir, &key, &["definitions", "list"], "");
    for id in ["google", "slack", "stub"] {
        assert!(listed.contains(id), "{listed}");
    }
    ok(
        &dir,
        &key,
        &[
            "definitions",
            "check",
            dir.join("services/stub.toml").to_str().unwrap(),
        ],
        "",
    );
    let broken = dir.join("broken.toml");
    std::fs::write(&broken, "format = 1\nid = \"broken\"\nnope = 1\n").unwrap();
    let why = fails(
        &dir,
        &key,
        &["definitions", "check", broken.to_str().unwrap()],
    );
    assert!(why.contains("nope") || why.contains("unknown"), "{why}");
}

#[test]
fn the_vault_takes_a_secret_from_stdin_lists_it_without_its_value_and_deletes_it() {
    let (dir, key) = (temp("vault"), key());
    ok(
        &dir,
        &key,
        &["vault", "set", "bank-password", "--purpose", "the bank"],
        SECRET,
    );
    let listed = ok(&dir, &key, &["vault", "list"], "");
    assert!(listed.contains("bank-password") && listed.contains("the bank"));
    let file = std::fs::read(dir.join("vault/bank-password.age")).unwrap();
    assert!(file.starts_with(b"age-encryption.org/v1"));
    ok(
        &dir,
        &key,
        &["vault", "describe", "bank-password", "the bank's site"],
        "",
    );
    let listed = ok(&dir, &key, &["vault", "list"], "");
    assert!(listed.contains("the bank's site"), "{listed}");
    assert!(!listed.contains(&SECRET[..4]), "{listed}");
    assert_eq!(
        std::fs::read(dir.join("vault/bank-password.age")).unwrap(),
        file,
        "describing leaves the value as it was"
    );
    ok(&dir, &key, &["vault", "delete", "bank-password"], "");
    assert!(!ok(&dir, &key, &["vault", "list"], "").contains("bank-password"));
    let log = ok(&dir, &key, &["log"], "");
    assert!(
        log.contains("vault.put") && log.contains("vault.describe") && log.contains("vault.delete"),
        "{log}"
    );
    assert!(log.contains("owner"));
}

#[test]
fn a_connection_is_added_its_key_set_from_stdin_and_its_tool_called_as_the_owner() {
    let (dir, key, base) = (temp("call"), key(), stub());
    write_definition(&dir);
    let added = ok(
        &dir,
        &key,
        &[
            "connections",
            "add",
            "stub",
            "--value",
            &format!("base={base}"),
        ],
        "",
    );
    assert!(added.contains("stub"), "{added}");
    ok(&dir, &key, &["connections", "key", "stub"], SECRET);
    let listed = ok(&dir, &key, &["connections", "list"], "");
    assert!(listed.contains("stub") && listed.contains("Stub"));
    let tools = ok(&dir, &key, &["tools", "stub"], "");
    assert!(tools.contains("ping") && tools.contains("wipe"), "{tools}");
    let answer = ok(
        &dir,
        &key,
        &["call", "stub", "ping", r#"{"word":"hi"}"#],
        "",
    );
    assert!(answer.contains("/ping?word=hi"), "{answer}");
    assert!(answer.contains(r#""auth_ok":true"#), "{answer}");
    let log = ok(&dir, &key, &["log", "--connection", "stub"], "");
    assert!(log.contains("ping") && log.contains("allow"), "{log}");
    ok(&dir, &key, &["connections", "remove", "stub"], "");
    assert!(!ok(&dir, &key, &["connections", "list"], "").contains("Stub"));
}

#[test]
fn permissions_are_shown_and_set_and_a_forbidden_tool_is_refused_even_for_the_owner() {
    let (dir, key, base) = (temp("permissions"), key(), stub());
    write_definition(&dir);
    ok(
        &dir,
        &key,
        &[
            "connections",
            "add",
            "stub",
            "--value",
            &format!("base={base}"),
        ],
        "",
    );
    let shown = ok(&dir, &key, &["permissions", "stub"], "");
    assert!(
        shown.contains("ping\tauto") && shown.contains("wipe\tauto"),
        "{shown}"
    );
    ok(&dir, &key, &["permissions", "stub", "ping", "forbid"], "");
    assert!(ok(&dir, &key, &["permissions", "stub"], "").contains("ping\tforbid"));
    let why = fails(&dir, &key, &["call", "stub", "ping"]);
    assert!(why.contains("forbid") || why.contains("禁止"), "{why}");
    let log = ok(&dir, &key, &["log", "--decision", "deny"], "");
    assert!(log.contains("ping"), "{log}");
    ok(&dir, &key, &["permissions", "stub", "ping", "default"], "");
    assert!(ok(&dir, &key, &["permissions", "stub"], "").contains("ping\tauto"));
}

#[test]
fn a_receiver_sent_to_the_cli_prints_what_arrives_as_it_arrived() {
    let (dir, key) = (temp("receive"), key());
    write_definition(&dir);
    ok(
        &dir,
        &key,
        &[
            "connections",
            "add",
            "stub",
            "--value",
            "base=http://127.0.0.1:9",
            "--value",
            "token=hush",
        ],
        "",
    );
    ok(&dir, &key, &["destinations", "stub", "events", "cli"], "");
    assert!(ok(&dir, &key, &["destinations", "stub"], "").contains("events\tcli"));
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut serving = command(
        &dir,
        &key,
        &["receive", "--listen", &format!("127.0.0.1:{port}")],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let post = |token: &str| {
        for _ in 0..50 {
            if let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
                let body = r#"{"hello":"there"}"#;
                write!(
                    s,
                    "POST /receive/stub/events HTTP/1.1\r\nhost: x\r\nx-token: {token}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                let mut answer = String::new();
                s.read_to_string(&mut answer).unwrap();
                return answer;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("the receiver did not open");
    };
    assert!(post("wrong").starts_with("HTTP/1.1 401"));
    assert!(post("hush").starts_with("HTTP/1.1 200"));
    let mut line = String::new();
    BufReader::new(serving.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    serving.kill().unwrap();
    serving.wait().unwrap();
    let arrival: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(arrival["connection"], "stub");
    assert_eq!(arrival["receiver"], "events");
    assert_eq!(arrival["payload"]["hello"], "there");
    assert!(!line.contains("hush"));
}

#[test]
fn without_a_vault_key_the_cli_says_how_to_make_one() {
    let dir = temp("nokey");
    let out = Command::new(env!("CARGO_BIN_EXE_famulen-gateway"))
        .arg("--dir")
        .arg(&dir)
        .args(["vault", "list"])
        .env_remove("FAMULEN_VAULT_KEY")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let why = String::from_utf8_lossy(&out.stderr);
    assert!(
        why.contains("FAMULEN_VAULT_KEY") && why.contains("AGE-SECRET-KEY-1"),
        "{why}"
    );
}
