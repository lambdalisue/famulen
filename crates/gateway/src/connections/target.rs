//! Reading what the owner typed as a connection's target.

/// The service kind a connection's target suggests: `notion` for
/// `https://mcp.notion.com/mcp`, `github` for `npx -y …/server-github`.
pub fn guess_kind(target: &str) -> String {
    let raw = if let Ok(url) = url::Url::parse(target) {
        url.host_str()
            .unwrap_or_default()
            .split('.')
            .find(|label| !matches!(*label, "mcp" | "api" | "www" | "app"))
            .unwrap_or_default()
            .to_owned()
    } else {
        words(target)
            .into_iter()
            .rfind(|w| !w.starts_with('-'))
            .unwrap_or_default()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned()
    };
    let mut kind: String = raw
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while let Some(rest) = ["server-", "mcp-"]
        .iter()
        .find_map(|a| kind.strip_prefix(a))
    {
        kind = rest.to_owned();
    }
    while let Some(rest) = ["-server", "-mcp"]
        .iter()
        .find_map(|a| kind.strip_suffix(a))
    {
        kind = rest.to_owned();
    }
    let kind = kind.trim_matches('-').to_owned();
    if kind.is_empty() {
        "service".to_owned()
    } else {
        kind
    }
}

/// A command line split into words, honouring quotes.
pub fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
                started = false;
            }
            (None, c) => word.push(c),
        }
    }
    if started || !word.is_empty() {
        out.push(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_is_guessed_from_the_target() {
        assert_eq!(guess_kind("https://mcp.notion.com/mcp"), "notion");
        assert_eq!(
            guess_kind("https://api.githubcopilot.com/mcp/"),
            "githubcopilot"
        );
        assert_eq!(
            guess_kind("npx -y @modelcontextprotocol/server-github"),
            "github"
        );
        assert_eq!(guess_kind("uvx mcp-server-fetch"), "fetch");
        assert_eq!(words(r#"run "a b" c"#), vec!["run", "a b", "c"]);
    }
}
