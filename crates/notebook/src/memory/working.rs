use std::path::{Path, PathBuf};

use anyhow::Context as _;

use super::{WORKING, markdown_files, stem};

const SECTION_DONE: &str = "ここまで";
const SECTION_NEXT: &str = "次にやる";
const SECTION_WAITING: &str = "待っている";
const SECTION_CLUES: &str = "手がかり";

/// The progress of one piece of work, as free text per section.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkingMemory {
    /// The file name without `.md`.
    pub name: String,
    pub title: String,
    pub done: String,
    pub next: String,
    pub waiting: String,
    pub clues: String,
}

impl WorkingMemory {
    pub fn parse(name: impl Into<String>, text: &str) -> Self {
        let mut memory = WorkingMemory {
            name: name.into(),
            ..WorkingMemory::default()
        };
        let mut sections = [SECTION_DONE, SECTION_NEXT, SECTION_WAITING, SECTION_CLUES]
            .map(|heading| (heading, String::new()));
        let mut current = None;
        for line in text.lines() {
            if let Some(title) = line.strip_prefix("# ") {
                if memory.title.is_empty() {
                    memory.title = title.trim().to_string();
                }
                current = None;
            } else if let Some(heading) = line.strip_prefix("## ") {
                current = sections.iter().position(|(h, _)| *h == heading.trim());
            } else if let Some(at) = current {
                let body = &mut sections[at].1;
                body.push_str(line);
                body.push('\n');
            }
        }
        let [done, next, waiting, clues] = sections.map(|(_, body)| body.trim().to_string());
        if memory.title.is_empty() {
            memory.title = memory.name.clone();
        }
        WorkingMemory {
            done,
            next,
            waiting,
            clues,
            ..memory
        }
    }

    pub fn render(&self) -> String {
        let mut text = format!("# {}\n", self.title);
        for (heading, body) in [
            (SECTION_DONE, &self.done),
            (SECTION_NEXT, &self.next),
            (SECTION_WAITING, &self.waiting),
            (SECTION_CLUES, &self.clues),
        ] {
            text.push_str(&format!("\n## {heading}\n"));
            if !body.trim().is_empty() {
                text.push_str(&format!("\n{}\n", body.trim()));
            }
        }
        text
    }
}

fn working_path(name: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\']),
        "invalid working memory name: {name:?}"
    );
    Ok(Path::new(WORKING).join(format!("{name}.md")))
}

/// Every working memory file, relative to the directory.
pub fn working_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    markdown_files(dir, Path::new(WORKING))
}

/// Every working memory file, by name.
pub fn working_memories(dir: &Path) -> anyhow::Result<Vec<WorkingMemory>> {
    working_files(dir)?
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(dir.join(&path))?;
            Ok(WorkingMemory::parse(stem(&path), &text))
        })
        .collect()
}

/// Write a working memory file. Returns its path relative to the directory.
pub fn write_working(dir: &Path, memory: &WorkingMemory) -> anyhow::Result<PathBuf> {
    let rel = working_path(&memory.name)?;
    let path = dir.join(&rel);
    std::fs::create_dir_all(dir.join(WORKING))?;
    std::fs::write(&path, memory.render()).with_context(|| format!("writing {}", rel.display()))?;
    Ok(rel)
}

/// Remove a finished piece of work. Returns whether there was one.
pub fn remove_working(dir: &Path, name: &str) -> anyhow::Result<bool> {
    let path = dir.join(working_path(name)?);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::ensure;
    use crate::testing::TempDir;

    #[test]
    fn working_memory_round_trips_its_title_and_sections() {
        let tmp = TempDir::new("working");
        ensure(tmp.path()).unwrap();
        let memory = WorkingMemory {
            name: "trip".into(),
            title: "出張の手配".into(),
            done: "新幹線を押さえた".into(),
            next: "- ホテルを探す\n- 経費を申請する".into(),
            waiting: String::new(),
            clues: "前回は駅前".into(),
        };

        let rel = write_working(tmp.path(), &memory).unwrap();

        assert_eq!(rel, Path::new("working/trip.md"));
        assert_eq!(working_memories(tmp.path()).unwrap(), vec![memory]);
        assert!(remove_working(tmp.path(), "trip").unwrap());
        assert!(!remove_working(tmp.path(), "trip").unwrap());
        assert!(working_memories(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn a_working_memory_without_a_title_is_named_after_its_file() {
        let memory = WorkingMemory::parse("draft", "## 次にやる\n\n返事を書く\n## 他\n無視\n");

        assert_eq!(memory.title, "draft");
        assert_eq!(memory.next, "返事を書く");
        assert_eq!(memory.done, "");
    }

    #[test]
    fn a_working_memory_name_cannot_leave_its_directory() {
        let tmp = TempDir::new("working-name");
        for name in ["", "../x", "a/b", ".hidden"] {
            let memory = WorkingMemory {
                name: name.into(),
                ..WorkingMemory::default()
            };
            assert!(write_working(tmp.path(), &memory).is_err(), "{name:?}");
        }
    }
}
