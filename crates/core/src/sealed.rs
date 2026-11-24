//! Places no tool of the secretary's model may reach: the gateway's own
//! data (the vault, its database) and the file the secrets are started
//! from. The gateway is the only reader of secrets, so a file tool that could
//! open them would undo it.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::paths;

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
        let readings = |p: &str| read_from(&self.workdir, p);
        let refused = |place: PathBuf| {
            format!(
                "{tool} would reach {}, where no tool may look: the gateway alone keeps secrets. Do not try another way.",
                place.display()
            )
        };
        let (paths, searches) = match tool {
            "Read" | "Write" | "Edit" | "MultiEdit" => (readings(field("file_path")?), false),
            "NotebookEdit" => (readings(field("notebook_path")?), false),
            "LS" => (readings(field("path")?), false),
            "Grep" => (readings(field("path").unwrap_or(".")), true),
            "Glob" => {
                let pattern = field("pattern").map(literal_prefix).unwrap_or_default();
                let roots = readings(field("path").unwrap_or("."))
                    .and_then(|bases| bases.iter().map(|base| read_from(base, &pattern)).collect())
                    .map(|roots: Vec<Vec<PathBuf>>| roots.concat());
                (roots, true)
            }
            "Bash" | "PowerShell" => {
                return self.mentioned(field("command")?).map(refused);
            }
            _ => return None,
        };
        let Some(paths) = paths else {
            return Some(format!(
                "{tool} names a path whose place cannot be told: write the whole path, from a drive's letter."
            ));
        };
        let reached = if searches {
            self.searched(&paths)
        } else {
            self.inside(&paths)
        };
        reached.map(refused)
    }

    /// The sealed place one of `paths` is in.
    fn inside(&self, paths: &[PathBuf]) -> Option<PathBuf> {
        let written = forms_of(paths);
        self.places
            .iter()
            .find(|place| {
                forms(place)
                    .iter()
                    .any(|p| written.iter().any(|f| f.starts_with(p)))
            })
            .cloned()
    }

    /// The sealed place a search from one of `paths` would reach: it is
    /// inside one, or one is inside it.
    fn searched(&self, paths: &[PathBuf]) -> Option<PathBuf> {
        if let Some(place) = self.inside(paths) {
            return Some(place);
        }
        let written = forms_of(paths);
        self.places
            .iter()
            .find(|place| {
                forms(place)
                    .iter()
                    .any(|p| written.iter().any(|f| p.starts_with(f)))
            })
            .cloned()
    }

    /// The sealed place a shell command names as a whole path, absolutely or
    /// from the working directory. Only a quick first refusal: a command
    /// that gets there another way is stopped by the sandbox it runs in.
    fn mentioned(&self, command: &str) -> Option<PathBuf> {
        let command = in_shell(command);
        for place in &self.places {
            let mut spellings: Vec<String> = forms(place)
                .iter()
                .flat_map(|p| shell_spellings(p))
                .collect();
            if let Some(relative) = relative(&self.workdir, place) {
                spellings.push(in_shell(&relative.display().to_string()));
            }
            if let Some(name) = place.file_name().and_then(|n| n.to_str())
                && name.starts_with('.')
            {
                spellings.push(in_shell(name));
            }
            if spellings.iter().any(|s| names_path(&command, s)) {
                return Some(place.clone());
            }
        }
        NAMES
            .iter()
            .find(|name| command.contains(&in_shell(name)))
            .map(|name| PathBuf::from(format!("${name}…")))
    }
}

/// A shell command's text as places are looked for in it.
fn in_shell(text: &str) -> String {
    if cfg!(windows) {
        paths::windows::command(text)
    } else {
        text.to_owned()
    }
}

fn shell_spellings(form: &Path) -> Vec<String> {
    let form = form.display().to_string();
    if cfg!(windows) {
        paths::windows::spellings(&form)
    } else {
        vec![form]
    }
}

/// Every place `path` may mean when read from `base`, or `None` when that
/// cannot be told.
fn read_from(base: &Path, path: &str) -> Option<Vec<PathBuf>> {
    if cfg!(windows) {
        let readings = paths::windows::readings(&base.to_string_lossy(), path)?;
        Some(readings.into_iter().map(PathBuf::from).collect())
    } else {
        Some(vec![normal(base, Path::new(path))])
    }
}

fn forms_of(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths.iter().flat_map(|p| forms(p)).collect()
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
/// link does not lead around a sealed place; each in the spelling places are
/// compared by.
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
    forms.iter().map(|f| paths::key(f)).collect()
}

/// The part of a glob pattern before its first wildcard.
fn literal_prefix(pattern: &str) -> String {
    let wild = |s: &str| s.contains(['*', '?', '[', '{']);
    Path::new(pattern)
        .components()
        // The `?` of a Windows `\\?\` prefix is no wildcard.
        .take_while(|c| {
            matches!(c, Component::Prefix(_)) || !wild(&c.as_os_str().to_string_lossy())
        })
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
