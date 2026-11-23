//! Remote MCP servers the gateway knows how to connect, picked from a list
//! instead of typed in. Each registers the gateway itself when it logs in.

use std::collections::BTreeMap;

/// A service the gateway knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Known {
    pub id: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    pub what: &'static str,
    /// Where it is reached; `{name}` is filled from `asks`.
    pub url: &'static str,
    /// What the owner is asked for before logging in.
    pub asks: &'static [Ask],
    /// What comes before the names of the organization and the user in the
    /// server's instructions, for one that tells of them only there.
    pub introduces: &'static [&'static str],
}

/// One label of a host name the owner gives, such as a team's subdomain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ask {
    pub name: &'static str,
    pub label: &'static str,
    /// What follows it in the address, taken off when pasted along.
    pub suffix: &'static str,
}

impl Known {
    /// The address with what the owner gave filled in. Only a plain label
    /// fits, so nothing given can lead to another host.
    pub fn address(&self, given: &BTreeMap<String, String>) -> anyhow::Result<String> {
        let mut url = self.url.to_owned();
        for ask in self.asks {
            let value = given.get(ask.name).map(String::as_str).unwrap_or_default();
            let value = value.trim().to_ascii_lowercase();
            let value = value.strip_prefix("https://").unwrap_or(&value);
            let value = value.split('/').next().unwrap_or_default();
            let value = value.strip_suffix(ask.suffix).unwrap_or(value);
            anyhow::ensure!(
                !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "{}を入れる（英数字と - だけ）",
                ask.label
            );
            url = url.replace(&format!("{{{}}}", ask.name), value);
        }
        Ok(url)
    }
}

pub const KNOWN: &[Known] = &[
    Known {
        id: "slite",
        name: "Slite",
        icon: "notes",
        what: "ドキュメント",
        url: "https://api.slite.com/mcp",
        asks: &[],
        introduces: &["Organization - Name: ", "User - Name: "],
    },
    Known {
        id: "docbase",
        name: "DocBase",
        icon: "notes",
        what: "メモとドキュメント",
        url: "https://mcp.docbase.io/mcp",
        asks: &[],
        introduces: &[],
    },
    Known {
        id: "kibela",
        name: "Kibela",
        icon: "notes",
        what: "記事とドキュメント",
        url: "https://{team}.kibe.la/mcp",
        asks: &[Ask {
            name: "team",
            label: "チームのサブドメイン（◯◯.kibe.la の ◯◯）",
            suffix: ".kibe.la",
        }],
        introduces: &[],
    },
    Known {
        id: "freee",
        name: "freee",
        icon: "calculator",
        what: "会計・人事労務・請求書",
        url: "https://mcp.freee.co.jp/mcp",
        asks: &[],
        introduces: &[],
    },
    Known {
        id: "notion",
        name: "Notion",
        icon: "brand-notion",
        what: "ページとデータベース",
        url: "https://mcp.notion.com/mcp",
        asks: &[],
        introduces: &[],
    },
    Known {
        id: "linear",
        name: "Linear",
        icon: "list-check",
        what: "課題とプロジェクト",
        url: "https://mcp.linear.app/mcp",
        asks: &[],
        introduces: &[],
    },
    Known {
        id: "todoist",
        name: "Todoist",
        icon: "check",
        what: "タスク",
        url: "https://ai.todoist.net/mcp",
        asks: &[],
        introduces: &[],
    },
    Known {
        id: "clickup",
        name: "ClickUp",
        icon: "list-details",
        what: "タスクとドキュメント",
        url: "https://mcp.clickup.com/mcp",
        asks: &[],
        introduces: &[],
    },
    Known {
        id: "atlassian",
        name: "Jira・Confluence",
        icon: "brand-jira",
        what: "課題とページ",
        url: "https://mcp.atlassian.com/v1/mcp",
        asks: &[],
        introduces: &[],
    },
];

pub fn find(id: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_teams_address_is_filled_with_its_subdomain_however_it_was_pasted() {
        let kibela = find("kibela").unwrap();
        let address = |team: &str| kibela.address(&BTreeMap::from([("team".into(), team.into())]));

        for team in [
            "acme",
            " Acme ",
            "acme.kibe.la",
            "https://acme.kibe.la/notes",
        ] {
            assert_eq!(address(team).unwrap(), "https://acme.kibe.la/mcp");
        }
        for team in [
            "",
            "evil.example",
            "evil.example#",
            "a b",
            "a@b",
            "acme.kibe.la:99",
        ] {
            assert!(address(team).is_err(), "{team}");
        }
    }

    #[test]
    fn a_service_that_asks_nothing_has_one_address() {
        for known in KNOWN.iter().filter(|k| k.asks.is_empty()) {
            assert_eq!(known.address(&BTreeMap::new()).unwrap(), known.url);
            assert!(!known.url.contains('{'));
        }
    }
}
