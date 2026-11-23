//! Where each file belongs in the notebook's shape.

use std::path::{Path, PathBuf};

use crate::memory::{KEEP, Layer};
use crate::records::check_id;
use crate::workgroups::WORKGROUPS;
use crate::{GITIGNORE, MEMORY, Notebook, README, RECORDS, STATE, slash};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Place {
    Readme,
    Memory(Layer),
    Record {
        kind: String,
        id: String,
    },
    Workgroups,
    /// A file the shape allows that holds no memory or record.
    Other,
    Stray,
}

/// Where a file, relative to the notebook, is by the notebook's shape.
pub(super) fn place_of(path: &str) -> Place {
    if path == WORKGROUPS {
        return Place::Workgroups;
    }
    let parts: Vec<&str> = path.split('/').collect();
    match parts[..] {
        [README] => Place::Readme,
        [GITIGNORE] => Place::Other,
        [MEMORY, README] => Place::Readme,
        [MEMORY, .., KEEP] => Place::Other,
        [MEMORY, ..] => match Layer::of(&path[MEMORY.len() + 1..]) {
            Some(layer) => Place::Memory(layer),
            None => Place::Stray,
        },
        [RECORDS, _, README] => Place::Readme,
        [RECORDS, _, KEEP] => Place::Other,
        [RECORDS, kind, file] if !kind.starts_with('.') => match file.strip_suffix(".md") {
            Some(id) if check_id(id).is_ok() => Place::Record {
                kind: kind.to_owned(),
                id: id.to_owned(),
            },
            _ => Place::Stray,
        },
        _ => Place::Stray,
    }
}

/// Every file in the notebook but Git's and the tools' own, relative to it.
pub(super) fn all_files(notebook: &Notebook) -> anyhow::Result<Vec<String>> {
    let mut files = Vec::new();
    walk(notebook.root(), Path::new(""), &mut files)?;
    Ok(files.into_iter().map(|rel| slash(&rel)).collect())
}

/// Every Markdown file of the notebook, relative to it.
pub(crate) fn markdown_files(notebook: &Notebook) -> anyhow::Result<Vec<String>> {
    let mut files = all_files(notebook)?;
    files.retain(|f| f.ends_with(".md"));
    Ok(files)
}

fn walk(root: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for item in std::fs::read_dir(root.join(rel))? {
        let item = item?;
        let name = item.file_name();
        if rel.as_os_str().is_empty() && (name == ".git" || name == STATE) {
            continue;
        }
        let path = rel.join(&name);
        if item.file_type()?.is_dir() {
            walk(root, &path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}
