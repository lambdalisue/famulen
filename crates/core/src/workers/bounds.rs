//! What an agent doing a delegated task may reach: the tools handed to it,
//! and with its file tools nothing but its own directory and the one it was
//! told to work in.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::Kept;
use super::order::may_hand;
use crate::paths::under;
use crate::permissions::{DELEGATE_INSTEAD, LAUNCHERS, split_tool};
use crate::sealed::{PATH_FIELDS, Sealed, literal_prefix, read_from, resolved};

#[derive(Debug, Clone)]
pub(super) struct Bounds {
    tools: Vec<String>,
    workdir: PathBuf,
    roots: Vec<PathBuf>,
    sealed: Sealed,
    hosts: Vec<String>,
    /// What its file tools may read but not write.
    unwritable: Vec<PathBuf>,
    /// Names its file tools write no file or directory by, at any depth.
    unwritable_names: Vec<String>,
}

impl Bounds {
    pub(super) fn new(
        tools: &[String],
        workdir: &Path,
        directory: Option<&Path>,
        kept: &Kept,
    ) -> Self {
        Self {
            tools: tools.to_vec(),
            workdir: workdir.to_owned(),
            roots: std::iter::once(workdir)
                .chain(directory)
                .map(Path::to_owned)
                .collect(),
            sealed: Sealed::new(&kept.places, workdir),
            hosts: kept.hosts.clone(),
            unwritable: Vec::new(),
            unwritable_names: Vec::new(),
        }
    }

    /// Keep its file tools from writing `places`, as its shell is kept.
    pub(super) fn with_unwritable(self, places: Vec<PathBuf>) -> Self {
        Self {
            unwritable: places,
            ..self
        }
    }

    /// Keep its file tools from writing anything called one of `names`, or
    /// inside such a directory, wherever it is.
    pub(super) fn with_unwritable_names(self, names: &[&str]) -> Self {
        Self {
            unwritable_names: names.iter().map(|n| (*n).to_owned()).collect(),
            ..self
        }
    }

    /// The built-in tools the model is offered.
    pub(super) fn builtins(&self) -> Vec<&str> {
        self.tools
            .iter()
            .map(String::as_str)
            .filter(|t| split_tool(t).is_none() && may_hand(t))
            .collect()
    }

    /// The connections some tool of which is handed over.
    pub(super) fn connections(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .tools
            .iter()
            .filter(|t| may_hand(t))
            .filter_map(|t| split_tool(t))
            .map(|(c, _)| c.to_owned())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    pub(super) fn hands(&self, tool: &str) -> bool {
        may_hand(tool) && self.tools.iter().any(|t| t == tool)
    }

    /// Why a call must not be made, whatever the permissions say.
    pub(super) fn refusal(&self, tool: &str, input: &Value) -> Option<String> {
        if LAUNCHERS.contains(&tool) {
            return Some(DELEGATE_INSTEAD.to_owned());
        }
        if !self.hands(tool) {
            return Some(format!("{tool} was not handed to you for this work."));
        }
        if let Some(refused) = self.sealed.refusal(tool, input) {
            return Some(refused);
        }
        if tool == "WebFetch" {
            return self.refused_host(input);
        }
        let mut fields = PATH_FIELDS.iter().filter(|(name, _)| *name == tool);
        let (_, field) = fields.next()?;
        // What is not read here as the model CLI would read it is not run.
        let text = match input.get(*field) {
            None | Some(Value::Null) => ".",
            Some(Value::String(text)) => text,
            Some(other) => return Some(format!("{tool}: {other} is no path.")),
        };
        let Some(mut places) = read_from(&self.workdir, text) else {
            return Some(format!("{tool}: {text:?} cannot be told where it leads."));
        };
        for name in ["pattern", "glob"] {
            let searches = matches!((tool, name), ("Glob", "pattern") | ("Grep", "glob"));
            let pattern = match input.get(name) {
                Some(Value::String(pattern)) if searches => pattern.trim(),
                _ => continue,
            };
            if pattern.contains("..") || pattern.starts_with('~') {
                return Some(format!("{tool}: {pattern:?} is not taken as a pattern."));
            }
            if tool == "Glob" {
                let prefix = literal_prefix(pattern);
                let read: Option<Vec<_>> = places.iter().map(|p| read_from(p, &prefix)).collect();
                let Some(read) = read else {
                    return Some(format!(
                        "{tool}: {pattern:?} cannot be told where it leads."
                    ));
                };
                places = read.concat();
            }
        }
        let writes = matches!(tool, "Write" | "Edit" | "MultiEdit" | "NotebookEdit");
        if writes
            && let Some(kept) = places.iter().find(|place| {
                let place = folded(&resolved(place));
                self.unwritable
                    .iter()
                    .any(|u| under(&place, &folded(&resolved(u))))
                    || self.names_unwritable(&place)
            })
        {
            return Some(format!("{tool}: {} is not to be written.", kept.display()));
        }
        // A path that may mean several places stays inside in each of them.
        let outside = places.iter().find(|place| !self.inside(place))?;
        Some(format!(
            "{tool}: {} is outside the directories of this work.",
            outside.display()
        ))
    }

    /// Why a page is not read: it is on this machine, or cannot be told where
    /// it is.
    fn refused_host(&self, input: &Value) -> Option<String> {
        let text = input.get("url").and_then(Value::as_str).unwrap_or_default();
        let Ok(url) = url::Url::parse(text.trim()) else {
            return Some(format!("WebFetch: {text:?} cannot be told where it leads."));
        };
        let near = match url.host() {
            None => true,
            Some(url::Host::Ipv4(ip)) => is_near(ip),
            Some(url::Host::Ipv6(ip)) => {
                ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
                    || ip.to_ipv4_mapped().is_some_and(is_near)
            }
            // A name without a dot is one of the network here.
            Some(url::Host::Domain(name)) => {
                let name = name.trim_end_matches('.');
                !name.contains('.') || name.ends_with(".localhost")
            }
        };
        let bare = |host: &str| host.trim_matches(['[', ']', '.']).to_ascii_lowercase();
        let host = bare(url.host_str().unwrap_or_default());
        (near || self.hosts.iter().any(|h| bare(h) == host))
            .then(|| format!("WebFetch: {host} is kept from you."))
    }

    /// Whether `place`, as it is folded, is or is inside something called
    /// one of the names nothing is written by, below the root it is in.
    fn names_unwritable(&self, place: &Path) -> bool {
        let names: Vec<PathBuf> = self
            .unwritable_names
            .iter()
            .map(|n| folded(Path::new(n)))
            .collect();
        self.roots
            .iter()
            .filter_map(|root| place.strip_prefix(folded(&resolved(root))).ok())
            .any(|below| {
                below
                    .components()
                    .any(|c| names.iter().any(|n| n.as_os_str() == c.as_os_str()))
            })
    }

    /// Whether `path`, its links followed, stays in a root.
    fn inside(&self, path: &Path) -> bool {
        // Told strictly: what may be inside under another name is not.
        let path = resolved(path);
        self.roots.iter().any(|root| under(&path, &resolved(root)))
    }
}

/// `path` as the file system tells names apart: macOS's does not tell
/// cases apart.
fn folded(path: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_owned()
    }
}

/// Whether an address is of this machine or the network it is in.
fn is_near(ip: std::net::Ipv4Addr) -> bool {
    let shared = ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]);
    ip.is_loopback() || ip.is_unspecified() || ip.is_private() || ip.is_link_local() || shared
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::testing::TempDir;
    use serde_json::json;

    fn bounds(dir: &TempDir, tools: &[&str]) -> Bounds {
        for sub in ["work", "repo", "notebook"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        let tools: Vec<String> = tools.iter().map(|t| (*t).to_owned()).collect();
        Bounds::new(
            &tools,
            &dir.path().join("work"),
            Some(&dir.path().join("repo")),
            &Kept {
                places: vec![dir.path().join("notebook")],
                hosts: vec!["famulen.example.ts.net".into()],
            },
        )
    }

    #[test]
    fn what_its_files_may_not_be_written_is_read_and_not_written() {
        let dir = TempDir::new("bounds-unwritable");
        let kept = dir.path().join("repo/.claude/settings.json");
        let b = bounds(&dir, &["Read", "Write", "Edit"]).with_unwritable(vec![kept.clone()]);
        let at = |tool: &str, path: &Path| b.refusal(tool, &json!({ "file_path": path }));
        assert_eq!(at("Read", &kept), None);
        for tool in ["Write", "Edit"] {
            let why = at(tool, &kept).unwrap_or_default();
            assert!(why.contains("not to be written"), "{tool}: {why}");
        }
        assert_eq!(at("Write", &dir.path().join("repo/a.rs")), None);
        let deep = b.clone().with_unwritable_names(&[".claude", ".mcp.json"]);
        for kept in ["repo/crates/x/.claude/settings.json", "repo/sub/.mcp.json"] {
            let why = deep.refusal("Write", &json!({ "file_path": dir.path().join(kept) }));
            assert!(why.is_some(), "{kept}");
        }
        assert_eq!(
            deep.refusal(
                "Write",
                &json!({ "file_path": dir.path().join("repo/claude.md") })
            ),
            None
        );
        if cfg!(target_os = "macos") {
            let shouted = dir.path().join("repo/.CLAUDE/settings.json");
            assert!(
                at("Write", &shouted).is_some(),
                "the same place by another case"
            );
        }
    }

    #[test]
    fn only_what_was_handed_over_is_offered() {
        let dir = TempDir::new("bounds-tools");
        let b = bounds(
            &dir,
            &[
                "Read",
                "Task",
                "KillShell",
                "default",
                "mcp__mail__search",
                "mcp__famulen__save_record",
            ],
        );
        assert_eq!(b.builtins(), ["Read"]);
        assert_eq!(b.connections(), ["mail"]);
        assert!(b.refusal("mcp__mail__search", &json!({})).is_none());
        for tool in [
            "Bash",
            "Task",
            "SandboxNetworkAccess",
            "mcp__mail__send",
            "mcp__famulen__save_record",
            "KillShell",
        ] {
            assert!(b.refusal(tool, &json!({})).is_some(), "{tool}");
        }
    }

    #[test]
    fn file_tools_stay_in_the_directories_of_the_work() {
        let dir = TempDir::new("bounds-files");
        let b = bounds(&dir, &["Read", "Glob", "Grep", "Write"]);
        let at = |rel: &str| dir.path().join(rel).display().to_string();
        let allowed = [
            ("Read", json!({ "file_path": at("repo/a.rs") })),
            ("Read", json!({ "file_path": "handed/a.md" })),
            ("Grep", json!({ "pattern": "x" })),
            ("Glob", json!({ "pattern": "**/*.md" })),
            ("Glob", json!({ "path": at("repo"), "pattern": "src/*" })),
            ("Write", json!({ "file_path": at("work/out.md") })),
        ];
        for (tool, input) in allowed {
            assert_eq!(b.refusal(tool, &input), None, "{tool} {input}");
        }
        let refused = [
            ("Read", json!({ "file_path": at("notebook/a.md") })),
            ("Read", json!({ "file_path": at("other") })),
            ("Read", json!({ "file_path": "../notebook/a.md" })),
            ("Read", json!({ "file_path": "/etc/passwd" })),
            ("Grep", json!({ "pattern": "x", "path": at("") })),
            ("Glob", json!({ "pattern": at("notebook/*") })),
            ("Glob", json!({ "pattern": "*/../../notebook/*" })),
            ("Glob", json!({ "pattern": "~/*" })),
            ("Grep", json!({ "pattern": "x", "path": "~/.ssh" })),
            ("Grep", json!({ "pattern": "x", "path": " /etc " })),
            ("Read", json!({ "file_path": "~root/x" })),
            ("Read", json!({ "file_path": ["/etc/passwd"] })),
            ("Write", json!({ "file_path": at("out.md") })),
        ];
        for (tool, input) in refused {
            assert!(b.refusal(tool, &input).is_some(), "{tool} {input}");
        }
    }

    #[test]
    fn no_page_of_this_machine_or_of_famulen_is_read() {
        let dir = TempDir::new("bounds-hosts");
        let b = bounds(&dir, &["WebFetch"]);
        let read = |url: &str| b.refusal("WebFetch", &json!({ "url": url }));
        for url in [
            "http://127.0.0.1:7790/health",
            "http://127.1:7790",
            "http://2130706433/",
            "http://localhost:3000",
            "http://a.localhost.",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://0.0.0.0/",
            "http://192.168.1.2:7790/",
            "http://100.100.1.2/",
            "http://[fd7a::1]:7790/",
            "http://console:3000/",
            "https://FAMULEN.example.ts.net/",
            "file:///etc/hosts",
            "",
        ] {
            assert!(read(url).is_some(), "{url}");
        }
        for url in ["https://example.com/a", "http://93.184.216.34/"] {
            assert_eq!(read(url), None, "{url}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_directory_is_told_by_windows_spellings_and_not_by_a_share_of_another_host() {
        let dir = TempDir::new("bounds-windows");
        let b = bounds(&dir, &["Read"]);
        let repo = dir.path().join("repo").display().to_string();
        let read = |path: String| b.refusal("Read", &json!({ "file_path": path }));
        for path in [
            format!("{repo}\\a.md"),
            format!("{}/a.md", repo.replace('\\', "/").to_uppercase()),
        ] {
            assert_eq!(read(path.clone()), None, "{path}");
        }
        let rest = repo.split_once(':').unwrap().1;
        assert!(read(format!(r"\\elsewhere\C${rest}\a.md")).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_does_not_lead_out() {
        let dir = TempDir::new("bounds-links");
        let b = bounds(&dir, &["Read"]);
        std::os::unix::fs::symlink(dir.path(), dir.path().join("repo/up")).unwrap();
        let through = dir.path().join("repo/up/secret.md");
        assert!(
            b.refusal(
                "Read",
                &json!({ "file_path": through.display().to_string() })
            )
            .is_some()
        );
    }
}
