//! Places no tool of the secretary's model may reach: the gateway's own
//! data (the vault, its database) and the file the secrets are started
//! from. The gateway is the only reader of secrets, so a file tool that could
//! open them would undo it.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

/// The places, and the directory relative paths are read against.
#[derive(Debug, Clone, Default)]
pub struct Sealed {
    places: Vec<PathBuf>,
    workdir: PathBuf,
}

/// Names a shell command must not mention, whatever the path around them:
/// the variables the vault key and the apps' secrets come in.
const NAMES: &[&str] = &["FAMULEN_"];

impl Sealed {
    pub fn new(
        places: impl IntoIterator<Item = impl Into<PathBuf>>,
        workdir: impl Into<PathBuf>,
    ) -> Self {
        let workdir = workdir.into();
        Self {
            places: places
                .into_iter()
                .map(|p| normal(&workdir, &p.into()))
                .collect(),
            workdir,
        }
    }

    /// Why `tool` called with `input` must not run, when it reaches a sealed
    /// place.
    pub fn refusal(&self, tool: &str, input: &Value) -> Option<String> {
        if self.places.is_empty() {
            return None;
        }
        let field = |name: &str| input.get(name).and_then(Value::as_str);
        let reached = match tool {
            "Read" | "Write" | "Edit" | "MultiEdit" => {
                field("file_path").and_then(|p| self.inside(p))
            }
            "NotebookEdit" => field("notebook_path").and_then(|p| self.inside(p)),
            "LS" => field("path").and_then(|p| self.inside(p)),
            "Grep" => self.searched(field("path").unwrap_or(".")),
            "Glob" => {
                let base = field("path").unwrap_or(".");
                let pattern = field("pattern").map(literal_prefix).unwrap_or_default();
                let root = self.resolve(base).join(pattern);
                self.searched(&root.to_string_lossy())
            }
            "Bash" => field("command").and_then(|c| self.mentioned(c)),
            _ => None,
        }?;
        Some(format!(
            "{tool} would reach {}, where no tool may look: the gateway alone keeps secrets. Do not try another way.",
            reached.display()
        ))
    }

    fn resolve(&self, path: &str) -> PathBuf {
        normal(&self.workdir, Path::new(path))
    }

    /// The sealed place `path` is in.
    fn inside(&self, path: &str) -> Option<PathBuf> {
        let path = self.resolve(path);
        let written = forms(&path);
        self.places
            .iter()
            .find(|place| {
                forms(place)
                    .iter()
                    .any(|p| written.iter().any(|f| f.starts_with(p)))
            })
            .cloned()
    }

    /// The sealed place a search from `path` would reach: it is inside one,
    /// or one is inside it.
    fn searched(&self, path: &str) -> Option<PathBuf> {
        if let Some(place) = self.inside(path) {
            return Some(place);
        }
        let path = self.resolve(path);
        let forms_of_path = forms(&path);
        self.places
            .iter()
            .find(|place| {
                forms(place)
                    .iter()
                    .any(|p| forms_of_path.iter().any(|f| p.starts_with(f)))
            })
            .cloned()
    }

    /// The sealed place a shell command names as a whole path, absolutely or
    /// from the working directory. Only a quick first refusal: a command
    /// that gets there another way is stopped by the sandbox it runs in.
    fn mentioned(&self, command: &str) -> Option<PathBuf> {
        for place in &self.places {
            let mut spellings: Vec<String> = forms(place)
                .iter()
                .map(|p| p.display().to_string())
                .collect();
            if let Some(relative) = relative(&self.workdir, place) {
                spellings.push(relative.display().to_string());
            }
            if let Some(name) = place.file_name().and_then(|n| n.to_str())
                && name.starts_with('.')
            {
                spellings.push(name.to_owned());
            }
            if spellings.iter().any(|s| names_path(command, s)) {
                return Some(place.clone());
            }
        }
        NAMES
            .iter()
            .find(|name| command.contains(**name))
            .map(|name| PathBuf::from(format!("${name}…")))
    }
}

/// Whether `command` names `path` as a whole word: not as the start of a
/// longer name (`/data/gateway-notes`, `.env.sample`) or the end of one.
fn names_path(command: &str, path: &str) -> bool {
    let ends =
        |c: Option<char>| c.is_none_or(|c| c.is_whitespace() || "/'\"`;|&()<>=:".contains(c));
    command.match_indices(path).any(|(at, _)| {
        ends(command[..at].chars().next_back()) && ends(command[at + path.len()..].chars().next())
    })
}

/// `path` read against `base` when relative, with `.` and `..` worked out
/// without touching the disk.
fn normal(base: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `path` as written, and with the part that exists on disk resolved, so a
/// link does not lead around a sealed place.
fn forms(path: &Path) -> Vec<PathBuf> {
    let mut forms = vec![path.to_path_buf()];
    let mut existing = path;
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = existing.canonicalize() {
            let resolved = rest.iter().rev().fold(real, |p: PathBuf, c| p.join(c));
            if resolved != path {
                forms.push(resolved);
            }
            break;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                existing = parent;
            }
            _ => break,
        }
    }
    forms
}

/// The part of a glob pattern before its first wildcard.
fn literal_prefix(pattern: &str) -> String {
    let wild = |s: &str| s.contains(['*', '?', '[', '{']);
    Path::new(pattern)
        .components()
        .take_while(|c| !wild(&c.as_os_str().to_string_lossy()))
        .collect::<PathBuf>()
        .display()
        .to_string()
}

/// `path` written from `base` with `..`, when they share a root.
fn relative(base: &Path, path: &Path) -> Option<PathBuf> {
    let common = base
        .components()
        .zip(path.components())
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return None;
    }
    let ups = base.components().count() - common;
    let mut out = PathBuf::new();
    for _ in 0..ups {
        out.push("..");
    }
    for c in path.components().skip(common) {
        out.push(c);
    }
    Some(out)
}
