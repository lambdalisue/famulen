use super::*;

const HTTP: &str = r#"
format = 1
id = "billing"
name = "Billing"

[account]
display = "{account.email}"
email = { label = "Address" }

[login]
key = { label = "API key", header = "Authorization", value = "Bearer {token}" }

[transport]
http = { base = "https://api.example.com" }

[[tools]]
name = "list_invoices"
description = "List invoices."
method = "GET"
path = "/invoices"

[permissions]
list_invoices = "allow"
"#;

#[test]
fn a_definition_reads_with_its_account_login_tools_and_permissions() {
    let def = parse(HTTP).unwrap();
    assert_eq!(def.id, "billing");
    assert_eq!(def.display.as_deref(), Some("{account.email}"));
    assert_eq!(def.fields[0].name, "email");
    assert_eq!(def.tools[0].method, Method::Get);
    assert_eq!(def.permissions["list_invoices"], PermissionName::Allow);
    assert!(def.login.unwrap().key.is_some());
}

#[test]
fn an_account_field_may_be_a_secret() {
    let def = parse(&HTTP.replace(
        "email = { label = \"Address\" }",
        "email = { label = \"Address\" }\npass = { label = \"Pass\", secret = true }",
    ))
    .unwrap();
    let secret: Vec<_> = def
        .fields
        .iter()
        .map(|f| (f.name.as_str(), f.secret))
        .collect();
    assert_eq!(secret, [("email", false), ("pass", true)]);
}

#[test]
fn an_unknown_field_a_missing_one_or_a_wrong_shape_is_refused() {
    let unknown = parse(&HTTP.replace("name = \"Billing\"", "name = \"Billing\"\ncolour = 1"));
    assert!(unknown.unwrap_err().to_string().contains("colour"));
    assert!(parse(&HTTP.replace("id = \"billing\"\n", "")).is_err());
    assert!(parse(&HTTP.replace("format = 1", "format = 2")).is_err());
    assert!(parse(&HTTP.replace("method = \"GET\"", "method = \"FETCH\"")).is_err());
    let two = HTTP.replace(
        "http = { base = \"https://api.example.com\" }",
        "http = { base = \"https://a\" }\nmcp_remote = { url = \"https://b\" }",
    );
    assert!(parse(&two).unwrap_err().to_string().contains("exactly one"));
}

#[test]
fn an_app_needs_setup_and_oauth2_needs_an_app() {
    let oauth = r#"
format = 1
id = "chat"
name = "Chat"
setup = "README.md"

[app]
id = "chat"
client_id = { env = "CHAT_ID" }
client_secret = { env = "CHAT_SECRET", secret = true }

[login.oauth2]
authorize = "https://chat.example/authorize"
token = "https://chat.example/token"
tokens = { bot = "{/access_token}", user = "{/authed_user/access_token}" }
values = { team = "{/team/id}" }
same = ["team"]

[transport]
http = { base = "https://chat.example/api", auth = "bot" }
"#;
    let def = parse(oauth).unwrap();
    let app = def.app_spec.unwrap();
    assert_eq!(app.values.len(), 2);
    assert!(
        app.values
            .iter()
            .any(|v| v.name == "client_secret" && v.secret)
    );
    assert_eq!(
        def.login.unwrap().oauth2.unwrap().token_order,
        ["bot", "user"]
    );
    assert!(parse(&oauth.replace("setup = \"README.md\"\n", "")).is_err());
    assert!(parse(&oauth.replace("same = [\"team\"]", "same = [\"org\"]")).is_err());
}

#[test]
fn receivers_are_named_and_each_is_one_kind_that_checks_its_needs() {
    let base = "format = 1\nid = \"hook\"\nname = \"Hook\"\n[transport]\nhttp = { base = \"https://h\" }\n";
    let hmac = format!(
        "{base}[receive.events.http]\nverify = {{ hmac_sha256 = {{ secret = \"s\", base = \"{{body}}\", signature = \"{{header.X-Sig}}\", max_age = \"5m\" }} }}\nchallenge = \"{{/challenge}}\"\n"
    );
    assert!(parse(&hmac).is_ok());
    assert!(parse(&hmac.replace("5m", "soon")).is_err());
    let socket = format!("{base}[receive.events.connect]\nprotocol = \"slack-socket-mode\"\n");
    assert!(parse(&socket).unwrap_err().to_string().contains("token"));
    let both = format!(
        "{hmac}[receive.mail.connect]\nprotocol = \"pubsub-pull\"\nsubscription = \"projects/p/subscriptions/s\"\n"
    );
    let def = parse(&both).unwrap();
    assert_eq!(def.receive.keys().collect::<Vec<_>>(), ["events", "mail"]);
    assert_eq!(def.receive["events"].destination, None);
    assert!(def.receive["events"].http.is_some());
    assert!(def.receive["mail"].connect.is_some());
    let neither = format!("{base}[receive.events]\n");
    assert!(
        parse(&neither)
            .unwrap_err()
            .to_string()
            .contains("exactly one")
    );
    let suggesting = parse(&format!(
        "{base}[receive.events]\ndestination = \"channel\"\n{}",
        &hmac[base.len()..]
    ))
    .unwrap();
    assert_eq!(
        suggesting.receive["events"].destination.as_deref(),
        Some("channel")
    );
    let badly_named = hmac.replace("receive.events", "receive.Events");
    assert!(
        parse(&badly_named)
            .unwrap_err()
            .to_string()
            .contains("name")
    );
}

#[test]
fn later_definitions_win_and_a_broken_one_says_why() {
    let dir = std::env::temp_dir().join(format!("gateway-defs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("billing.toml"), HTTP.replace("Billing", "Mine")).unwrap();
    std::fs::write(dir.join("broken.toml"), "format = 1").unwrap();
    std::fs::write(dir.join("other.toml"), HTTP).unwrap();
    let user = vec![("billing.toml".to_owned(), HTTP.to_owned())];

    let defs = Definitions::load(Some(&dir), &user);

    assert_eq!(defs.get("billing").unwrap().name, "Mine");
    assert!(defs.why_not("broken").unwrap().contains("missing"));
    assert!(defs.why_not("other").unwrap().contains("named other.toml"));
    assert!(
        defs.why_not("nothing")
            .unwrap()
            .contains("no service definition")
    );
    std::fs::remove_dir_all(dir).unwrap();
}

fn with_tool(base: &str, tool: &str) -> String {
    base.replace("[permissions]", &format!("{tool}\n[permissions]"))
}

const DELETE: &str = "[[tools]]\nname = \"delete_invoice\"\ndescription = \"Delete one.\"\nmethod = \"DELETE\"\npath = \"/invoices/{/id}\"\n";

#[test]
fn a_definition_keeps_the_text_it_was_read_from() {
    let dir = std::env::temp_dir().join(format!("gateway-defs-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("billing.toml"), HTTP).unwrap();

    let defs = Definitions::load(Some(&dir), &[]);

    assert_eq!(defs.get("billing").unwrap().text, HTTP);
    std::fs::remove_dir_all(dir).unwrap();
}

const CREATE: &str = "[[tools]]\nname = \"create_invoice\"\ndescription = \"Create one.\"\nmethod = \"POST\"\npath = \"/invoices\"\n";

#[test]
fn a_new_writing_tool_that_asks_loosens_nothing() {
    let current = parse(HTTP).unwrap();
    let delete = with_tool(HTTP, DELETE);
    let marked = with_tool(HTTP, &CREATE.replace("method", "open_world = true\nmethod"));
    let confirmed = with_tool(HTTP, CREATE) + "create_invoice = \"confirm\"\n";
    for text in [delete, marked, confirmed] {
        assert!(
            loosened(Some(&current), &parse(&text).unwrap()).is_empty(),
            "{text}"
        );
    }
}

#[test]
fn a_new_writing_tool_left_to_the_assessor_must_ask_instead() {
    let current = parse(HTTP).unwrap();
    let proposed = parse(&with_tool(HTTP, CREATE)).unwrap();

    let reasons = loosened(Some(&current), &proposed);

    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].contains("create_invoice"), "{reasons:?}");
    assert!(reasons[0].contains("confirm"), "{reasons:?}");
}

#[test]
fn a_writing_tool_may_not_start_at_allow_however_it_says_so() {
    let current = parse(HTTP).unwrap();
    let by_permission = with_tool(HTTP, DELETE) + "delete_invoice = \"allow\"\n";
    let by_read_only = with_tool(HTTP, &CREATE.replace("method", "read_only = true\nmethod"));
    for text in [by_permission, by_read_only] {
        let reasons = loosened(Some(&current), &parse(&text).unwrap());
        assert_eq!(reasons.len(), 1, "{reasons:?}");
        assert!(reasons[0].contains("_invoice"), "{reasons:?}");
    }
}

#[test]
fn a_tool_never_becomes_looser_than_the_current_definition_has_it() {
    let current =
        parse(&HTTP.replace("list_invoices = \"allow\"", "list_invoices = \"forbid\"")).unwrap();
    let confirm = HTTP.replace("list_invoices = \"allow\"", "list_invoices = \"confirm\"");
    let dropped = HTTP.replace("list_invoices = \"allow\"\n", "");
    assert!(
        loosened(Some(&current), &parse(&confirm).unwrap())[0].contains("from forbidden to asked")
    );
    assert!(!loosened(Some(&current), &parse(&dropped).unwrap()).is_empty());
    assert!(loosened(Some(&parse(HTTP).unwrap()), &parse(HTTP).unwrap()).is_empty());
}

#[test]
fn dropping_the_mark_that_makes_a_tool_ask_loosens_it() {
    let sends = CREATE.replace("method", "open_world = true\nmethod");
    let current = parse(&with_tool(HTTP, &sends)).unwrap();
    let unmarked = parse(&with_tool(HTTP, CREATE)).unwrap();

    let reasons = loosened(Some(&current), &unmarked);

    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(
        reasons[0].contains("create_invoice would go from asked to assessed"),
        "{reasons:?}"
    );
}

#[test]
fn a_permission_for_a_tool_the_definition_does_not_write_may_not_be_allow() {
    let mcp = "format = 1\nid = \"notes\"\nname = \"Notes\"\n[transport]\nmcp_remote = { url = \"https://n\" }\n[permissions]\n";
    let proposed = parse(&format!("{mcp}remove_note = \"allow\"\n")).unwrap();
    assert!(!loosened(None, &proposed).is_empty());
    let asks = parse(&format!("{mcp}remove_note = \"confirm\"\n")).unwrap();
    assert!(loosened(None, &asks).is_empty());
}

#[test]
fn a_proposal_may_not_change_what_a_command_run_unasked_runs() {
    let text = |args: &str, read_only: bool| {
        format!(
            "format = 1\nid = \"c\"\nname = \"C\"\n[transport.command]\nprogram = \"p\"\n[[commands]]\nname = \"list\"\ndescription = \"List.\"\nargs = {args}\nread_only = {read_only}\n"
        )
    };
    let def = |args: &str, read_only| parse(&text(args, read_only)).unwrap();
    let current = def("[\"list\"]", true);

    assert!(loosened(Some(&current), &def("[\"list\"]", true)).is_empty());
    assert_eq!(loosened(Some(&current), &def("[\"rm\"]", true)).len(), 1);
    let asked = format!(
        "{}[permissions]\nlist = \"confirm\"\n",
        text("[\"rm\"]", true)
    );
    assert!(loosened(Some(&current), &parse(&asked).unwrap()).is_empty());
    assert_eq!(
        loosened(None, &def("[\"list\"]", true)).len(),
        1,
        "a new one is asked first"
    );
}

#[test]
fn durations_read_as_seconds() {
    assert_eq!(seconds("30s"), Some(30));
    assert_eq!(seconds("5m"), Some(300));
    assert_eq!(seconds("1h"), Some(3600));
    assert_eq!(seconds("soon"), None);
    assert_eq!(seconds("18446744073709551615h"), None);
}
