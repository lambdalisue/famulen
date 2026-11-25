//! Which requests are asked only in the owner's DM, whoever sees the
//! conversation: those that may show a secret. It errs to the DM.

use famulen_gateway::{DEFINITIONS, Ticket, VAULT};
use serde_json::Value;

use crate::wording;

/// Words that name a secret, as a field or in a text before its value.
const NAMES: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "authorization",
    "bearer ",
    "credential",
    "private_key",
    "private key",
    "cookie",
    "パスワード",
    "暗証番号",
    "合言葉",
];

/// How the keys of well-known services begin.
const KEYS: &[&str] = &[
    "AGE-SECRET-KEY-",
    "AKIA",
    "AIza",
    "eyJ",
    "ghp_",
    "gho_",
    "ghs_",
    "github_pat_",
    "glpat-",
    "sk-",
    "sk_live_",
    "xapp-",
    "xoxb-",
    "xoxp-",
];

/// A run of letters and digits this long is taken for a key.
const KEY_CHARS: usize = 32;

/// Whether a request is asked only in the owner's DM: it changes the vault or
/// a definition, carries questions or words from elsewhere that the owner
/// decides on, or may show a secret.
pub(super) fn kept_to_dm(ticket: &Ticket) -> bool {
    matches!(ticket.connection.as_str(), VAULT | DEFINITIONS)
        || super::asked::is_ask(ticket)
        || ticket.shown.get("decides").is_some()
        || may_hold_secret(&ticket.shown)
}

fn may_hold_secret(value: &Value) -> bool {
    match value {
        Value::String(text) => text_may_hold_secret(text),
        Value::Array(items) => items.iter().any(may_hold_secret),
        Value::Object(fields) => fields
            .iter()
            .any(|(name, value)| names_secret(name) || may_hold_secret(value)),
        _ => false,
    }
}

fn names_secret(text: &str) -> bool {
    let text = text.to_lowercase();
    NAMES.iter().any(|name| text.contains(name))
}

fn text_may_hold_secret(text: &str) -> bool {
    names_secret(text)
        // It is shown as what it says, which its own form hides from the rest.
        || wording::letter(text).is_some()
        || KEYS.iter().any(|key| text.contains(key))
        || url_with_password(text)
        || text
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|run| run.len() >= KEY_CHARS)
}

/// `scheme://user:password@host`
fn url_with_password(text: &str) -> bool {
    text.split("://").skip(1).any(|rest| {
        let authority = rest
            .split(|c: char| c == '/' || c.is_whitespace())
            .next()
            .unwrap_or_default();
        authority
            .split_once('@')
            .is_some_and(|(user, _)| user.contains(':'))
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn what_may_hold_a_secret_is_told_from_what_does_not() {
        for secret in [
            json!({ "apiKey": "x" }),
            json!({ "headers": { "Authorization": "x" } }),
            json!({ "command": "curl -H 'Authorization: x' https://example.com" }),
            json!({ "command": "DB_PASSWORD=x run" }),
            json!({ "content": ["パスワードは x です"] }),
            json!({ "text": "ghp_x" }),
            json!({ "url": "postgres://me:x@host/db" }),
            json!({ "text": "a1".repeat(16) }),
            // "To: a\n\n>>>???>>>???", whose form is short runs.
            json!({ "raw": "VG86IGEKCj4-Pj8_Pz4-Pj8_Pw" }),
        ] {
            assert!(may_hold_secret(&secret), "{secret}");
        }
        for plain in [
            json!({ "to": "a@example.com", "body": "明日の会議について" }),
            json!({ "command": "cargo test --workspace" }),
            json!({ "id": "01234567-89ab-cdef-0123-456789abcdef" }),
            json!({ "url": "https://example.com/a?at=1" }),
        ] {
            assert!(!may_hold_secret(&plain), "{plain}");
        }
    }
}
