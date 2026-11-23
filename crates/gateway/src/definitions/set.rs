//! The definitions the gateway can use: those it ships, those the program
//! using it passes, and those in its directory.

use std::collections::BTreeMap;
use std::path::Path;

use super::{Definition, parse};

/// The definitions the gateway ships, by the path they are kept at.
pub const BUNDLED: &[(&str, &str)] = &[
    (
        "crates/gateway/services/adroit/adroit.toml",
        include_str!("../../services/adroit/adroit.toml"),
    ),
    (
        "crates/gateway/services/bluesky/bluesky.toml",
        include_str!("../../services/bluesky/bluesky.toml"),
    ),
    (
        "crates/gateway/services/discord/discord.toml",
        include_str!("../../services/discord/discord.toml"),
    ),
    (
        "crates/gateway/services/github/github.toml",
        include_str!("../../services/github/github.toml"),
    ),
    (
        "crates/gateway/services/google/google.toml",
        include_str!("../../services/google/google.toml"),
    ),
    (
        "crates/gateway/services/kingoftime/kingoftime.toml",
        include_str!("../../services/kingoftime/kingoftime.toml"),
    ),
    (
        "crates/gateway/services/kintone/kintone.toml",
        include_str!("../../services/kintone/kintone.toml"),
    ),
    (
        "crates/gateway/services/rss/rss.toml",
        include_str!("../../services/rss/rss.toml"),
    ),
    (
        "crates/gateway/services/slack/slack.toml",
        include_str!("../../services/slack/slack.toml"),
    ),
];

/// The definitions the gateway can use, and those that did not read.
#[derive(Debug, Clone, Default)]
pub struct Definitions {
    defs: BTreeMap<String, Definition>,
    broken: BTreeMap<String, String>,
}

impl Definitions {
    /// Read the bundled definitions, then those `user` passes, then the
    /// files in `dir`: a later one with the same id wins.
    pub fn load(dir: Option<&Path>, user: &[(String, String)]) -> Self {
        let mut out = Self::default();
        for (path, text) in BUNDLED {
            out.add(path, text);
        }
        for (path, text) in user {
            out.add(path, text);
        }
        if let Some(dir) = dir
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            let mut paths: Vec<_> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .collect();
            paths.sort();
            for path in paths {
                match std::fs::read_to_string(&path) {
                    Ok(text) => out.add(&path.display().to_string(), &text),
                    Err(error) => {
                        let id = stem(&path.display().to_string());
                        out.broken.insert(id, error.to_string());
                    }
                }
            }
        }
        out
    }

    fn add(&mut self, path: &str, text: &str) {
        let id = stem(path);
        match parse(text) {
            Ok(mut def) if def.id == id => {
                def.source = path.to_owned();
                def.text = text.to_owned();
                self.broken.remove(&id);
                self.defs.insert(id, def);
            }
            Ok(def) => {
                self.broken.insert(
                    id.clone(),
                    format!("the file is named {id}.toml but its id is {}", def.id),
                );
                self.defs.remove(&id);
            }
            Err(error) => {
                self.broken.insert(id.clone(), error.to_string());
                self.defs.remove(&id);
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&Definition> {
        self.defs.get(id)
    }

    /// Why the definition `id` cannot be used, if it cannot.
    pub fn why_not(&self, id: &str) -> Option<String> {
        if self.defs.contains_key(id) {
            return None;
        }
        Some(
            self.broken
                .get(id)
                .cloned()
                .unwrap_or_else(|| format!("no service definition {id}")),
        )
    }

    pub fn list(&self) -> impl Iterator<Item = &Definition> {
        self.defs.values()
    }

    pub fn broken(&self) -> impl Iterator<Item = (&String, &String)> {
        self.broken.iter()
    }
}

fn stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}
