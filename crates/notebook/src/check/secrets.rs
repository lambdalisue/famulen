/// A known token's prefix, and how much must follow it.
const TOKENS: [(&str, usize); 9] = [
    ("ghp_", 30),
    ("gho_", 30),
    ("github_pat_", 30),
    ("xoxb-", 20),
    ("xoxp-", 20),
    ("xapp-", 20),
    ("sk-", 20),
    ("AKIA", 16),
    ("AIza", 30),
];

const PASSWORD_LABELS: [&str; 4] = ["password:", "password=", "パスワード:", "パスワード："];

/// What in a text looks like a secret: a private key, a well-known token,
/// or a password written beside its label. Each is shown cut, never whole.
pub(super) fn secrets(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    if text.contains("PRIVATE KEY-----") {
        found.push("a private key".to_owned());
    }
    for word in text.split(|c: char| c.is_whitespace() || "\"'`()[]{}<>,;".contains(c)) {
        for (prefix, at_least) in TOKENS {
            let Some(rest) = word.strip_prefix(prefix) else {
                continue;
            };
            let tokenish = rest.len() >= at_least
                && rest
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if tokenish {
                found.push(format!("{prefix}… (a token)"));
            }
        }
    }
    for line in text.lines() {
        let lower = line.to_lowercase();
        for label in PASSWORD_LABELS {
            if let Some((_, value)) = lower.split_once(label)
                && !value.trim().is_empty()
            {
                found.push(format!("{label} … (a password)"));
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use crate::check::check;
    use crate::testing::{notebook, record};

    #[test]
    fn text_that_looks_like_a_secret_is_told_without_showing_it() {
        let (_tmp, nb) = notebook("check-secrets");
        record(
            &nb,
            "memos",
            "01kat3v8q4zj6m2x9r5c7w1hbn",
            "# 鍵\n\nslack: xoxb-1234567890-abcdefghijklmnop\npassword: hunter2\n",
        );

        let found = check(&nb).unwrap();
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|f| f.rule == "secret"
            && !f.why.contains("hunter2")
            && !f.why.contains("abcdef")));
    }
}
