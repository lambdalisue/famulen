#![doc = include_str!("../SPEC.md")]

pub mod body;
pub mod check;
mod doc;
mod entry;
mod git;
mod link;
pub mod memory;
mod page;
pub mod records;
#[cfg(test)]
pub(crate) mod testing;
pub mod workgroups;

pub use doc::*;
pub use entry::*;
pub use git::*;
pub use link::link_target;
pub use page::*;

use std::path::{Path, PathBuf};

/// The memory layers, relative to the notebook.
pub const MEMORY: &str = "memory";
/// The records, one directory per kind, relative to the notebook.
pub const RECORDS: &str = "records";
pub const README: &str = "README.md";
/// The tools' own state, relative to the notebook. Never committed.
pub const STATE: &str = ".notebook";
/// The version of the notebook's shape, as its README's frontmatter says.
pub const SHAPE: u32 = 1;

pub(crate) const GITIGNORE: &str = ".gitignore";

const README_TEMPLATE: &str = include_str!("../templates/README.md");

/// The kinds of record a new notebook starts with, and their READMEs.
pub const STANDARD_KINDS: [(&str, &str); 8] = [
    ("tasks", include_str!("../templates/records/tasks.md")),
    ("routines", include_str!("../templates/records/routines.md")),
    ("memos", include_str!("../templates/records/memos.md")),
    ("parties", include_str!("../templates/records/parties.md")),
    ("holdings", include_str!("../templates/records/holdings.md")),
    (
        "contracts",
        include_str!("../templates/records/contracts.md"),
    ),
    (
        "calendars",
        include_str!("../templates/records/calendars.md"),
    ),
    ("events", include_str!("../templates/records/events.md")),
];

/// A path as the notebook writes it, whatever the platform separates with.
pub(crate) fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The notebook in a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notebook {
    root: PathBuf,
    tool: String,
}

impl Notebook {
    /// A notebook whose commits name this library as their tool.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            tool: format!("famulen-notebook {}", env!("CARGO_PKG_VERSION")),
        }
    }

    /// Name the tool its commits are made with (`Notebook-Tool`).
    pub fn with_tool(mut self, name: impl AsRef<str>, version: impl AsRef<str>) -> Self {
        self.tool = format!("{} {}", name.as_ref(), version.as_ref());
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn memory_dir(&self) -> PathBuf {
        self.root.join(MEMORY)
    }

    pub fn records_dir(&self) -> PathBuf {
        self.root.join(RECORDS)
    }

    /// Where the tools keep their own state: indexes and counts.
    pub fn state_dir(&self) -> PathBuf {
        self.root.join(STATE)
    }

    /// Make the directory a Git repository holding the README, the memory
    /// layers and the standard kinds of record, creating what is missing
    /// and committing only what this call created.
    pub fn ensure(&self, signature: &Signature) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        git::init(&self.root)?;
        let mut made: Vec<String> = memory::ensure(&self.memory_dir())?
            .into_iter()
            .map(|rel| format!("{MEMORY}/{}", slash(&rel)))
            .collect();
        for (rel, text) in [(README, README_TEMPLATE), (GITIGNORE, "/.notebook/\n")] {
            let path = self.root.join(rel);
            if !path.exists() {
                std::fs::write(&path, text)?;
                made.push(rel.to_owned());
            }
        }
        for (kind, readme) in STANDARD_KINDS {
            let dir = self.records_dir().join(kind);
            let path = dir.join(README);
            if !path.exists() {
                std::fs::create_dir_all(&dir)?;
                std::fs::write(path, readme)?;
                made.push(format!("{RECORDS}/{kind}/{README}"));
            }
        }
        let workgroups = self.root.join(workgroups::WORKGROUPS);
        if !workgroups.exists() {
            std::fs::write(&workgroups, workgroups::TEMPLATE)?;
            made.push(workgroups::WORKGROUPS.to_owned());
        }
        if !made.is_empty() {
            // Only what this call made goes in, so edits not yet committed
            // stay with whoever made them.
            self.commit(signature, "手帳を用意した", &made)?;
        }
        Ok(())
    }

    /// Commit the changes to `paths`, given relative to the notebook, in the
    /// signer's name, with the tool and the signature's details as trailers.
    pub fn commit(
        &self,
        signature: &Signature,
        subject: &str,
        paths: &[impl AsRef<str>],
    ) -> anyhow::Result<Option<String>> {
        git::commit_paths(
            &self.root,
            &signature.author,
            &signature.message(subject, &self.tool),
            paths,
        )
    }
}

/// Who writes a change, and what with: the author of the commit and what
/// the commit's trailers say.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Signature {
    pub author: Author,
    /// Others who wrote it with the author (`Co-authored-by`).
    pub co_authors: Vec<Author>,
    /// The application or agent an AI writer ran in (`Notebook-Agent`).
    pub agent: Option<String>,
    /// The model of an AI writer (`Notebook-Model`).
    pub model: Option<String>,
    /// What the change was written from (`Notebook-Source`).
    pub sources: Vec<String>,
}

impl Signature {
    pub fn new(author: Author) -> Self {
        Self {
            author,
            ..Self::default()
        }
    }

    /// The commit message: the subject, then a trailer per detail.
    pub fn message(&self, subject: &str, tool: &str) -> String {
        let mut trailers: Vec<String> = self
            .co_authors
            .iter()
            .map(|a| format!("Co-authored-by: {} <{}>", a.name, a.email))
            .collect();
        trailers.push(format!("Notebook-Tool: {tool}"));
        trailers.extend(self.agent.iter().map(|a| format!("Notebook-Agent: {a}")));
        trailers.extend(self.model.iter().map(|m| format!("Notebook-Model: {m}")));
        trailers.extend(self.sources.iter().map(|s| format!("Notebook-Source: {s}")));
        let trailers: Vec<String> = trailers
            .into_iter()
            .map(|t| t.split(['\r', '\n']).collect::<Vec<_>>().join(" "))
            .collect();
        format!("{}\n\n{}", subject.trim(), trailers.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::{TempDir, me};

    #[test]
    fn a_missing_notebook_becomes_a_repository_with_its_readmes_layers_and_kinds() {
        let tmp = TempDir::new("notebook-new");
        let notebook = Notebook::new(tmp.path().join("notebook"));

        notebook.ensure(&me()).unwrap();

        let root = notebook.root();
        assert!(root.join(".git").is_dir());
        let readme = Page::load(&root.join(README)).unwrap();
        assert_eq!(readme.text("notebook").as_deref(), Some("1"));
        for path in [
            "memory/README.md",
            "memory/core.md",
            "memory/episodic",
            "memory/long/semantic",
            "memory/long/procedural",
            "memory/long/prospective",
            "records/tasks/README.md",
            "records/memos/README.md",
            "records/parties/README.md",
            "records/holdings/README.md",
            "records/contracts/README.md",
            "records/events/README.md",
            "records/workgroups.md",
        ] {
            assert!(root.join(path).exists(), "{path}");
        }
        let log = log(root, 10).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].author, me().author);
        assert_eq!(
            std::fs::read_to_string(root.join(".gitignore")).unwrap(),
            "/.notebook/\n"
        );
    }

    #[test]
    fn ensuring_twice_makes_no_second_commit() {
        let tmp = TempDir::new("notebook-twice");
        let notebook = Notebook::new(tmp.path());

        notebook.ensure(&me()).unwrap();
        notebook.ensure(&me()).unwrap();

        assert_eq!(log(tmp.path(), 10).unwrap().len(), 1);
    }

    #[test]
    fn ensuring_does_not_commit_edits_not_yet_committed() {
        let tmp = TempDir::new("notebook-dirty");
        let notebook = Notebook::new(tmp.path());
        notebook.ensure(&me()).unwrap();
        std::fs::remove_dir_all(tmp.path().join("records/events")).unwrap();
        notebook.commit(&me(), "drop events", &["records"]).unwrap();
        std::fs::write(tmp.path().join("memory/core.md"), "# 心得\n\n- y\n").unwrap();

        notebook.ensure(&me()).unwrap();

        assert!(tmp.path().join("records/events/README.md").is_file());
        let log = log(tmp.path(), 10).unwrap();
        assert_eq!(log.len(), 3, "the missing kind is put back and committed");
        let touched: Vec<String> = diff_stat(tmp.path(), &log[0].id)
            .unwrap()
            .into_iter()
            .map(|s| s.path)
            .collect();
        assert_eq!(touched, ["records/events/README.md"]);
    }

    #[test]
    fn ensuring_puts_back_the_readme_of_a_kind_that_already_holds_records() {
        let tmp = TempDir::new("notebook-kind-readme");
        let notebook = Notebook::new(tmp.path());
        let record = tmp.path().join("records/tasks/t1.md");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, "---\nid: t1\n---\n\n# x\n").unwrap();

        notebook.ensure(&me()).unwrap();

        assert!(tmp.path().join("records/tasks/README.md").is_file());
        assert!(record.is_file());
    }

    #[test]
    fn a_commit_names_its_tool_and_whatever_else_the_signature_says() {
        let tmp = TempDir::new("notebook-trailers");
        let notebook = Notebook::new(tmp.path()).with_tool("famulen-notebook-cli", "0.1.0");
        let signature = Signature {
            author: Author::new("Claude", "noreply@anthropic.com"),
            co_authors: vec![Author::new("Alice", "alice@example.com")],
            agent: Some("Claude Code 2.1.0".into()),
            model: Some("claude-opus-5-5".into()),
            sources: vec!["slack:C1/1.0".into(), "line\nbreak".into()],
        };

        notebook.ensure(&signature).unwrap();

        let commit = &log(tmp.path(), 1).unwrap()[0];
        assert_eq!(commit.author, signature.author);
        assert_eq!(
            commit.message,
            "手帳を用意した\n\n\
             Co-authored-by: Alice <alice@example.com>\n\
             Notebook-Tool: famulen-notebook-cli 0.1.0\n\
             Notebook-Agent: Claude Code 2.1.0\n\
             Notebook-Model: claude-opus-5-5\n\
             Notebook-Source: slack:C1/1.0\n\
             Notebook-Source: line break"
        );
    }

    #[test]
    fn a_commit_without_details_still_names_the_library() {
        let message = me().message("x", "famulen-notebook 0.0.0");

        assert_eq!(message, "x\n\nNotebook-Tool: famulen-notebook 0.0.0");
    }
}
