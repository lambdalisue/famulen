//! How to reach an MCP server.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerConfig {
    /// A server the gateway runs as a child process and talks to over stdio.
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    /// A remote server reached over streamable HTTP.
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

impl ServerConfig {
    #[cfg(test)]
    pub(crate) fn stdio(
        command: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::Stdio {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
            env: BTreeMap::new(),
        }
    }

    pub fn http(url: impl Into<String>) -> Self {
        Self::Http {
            url: url.into(),
            headers: BTreeMap::new(),
        }
    }
}
