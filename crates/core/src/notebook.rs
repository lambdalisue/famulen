//! How Famulen keeps its notebook: everything it knows about the owner, kept
//! with the notebook library. Here are only Famulen's own rules: who may
//! write, in whose name, and what Famulen adds to a new notebook.

#[cfg(test)]
pub(crate) mod testing;

use crate::store::Store;

pub use famulen_notebook::{
    Author, Hold, MEMORY, Notebook, Page, README, RECORDS, Signature, commit_all, commit_paths,
    diff_stat, link_target, log, revert_to,
};

/// Famulen itself, the author of what the secretary writes.
pub fn famulen() -> Author {
    Author::new("Famulen", "famulen@localhost")
}

/// The owner, the author of what they change on the console.
pub fn owner() -> Author {
    Author::new("Owner", "owner@localhost")
}

/// The notebook at `root`, its commits naming Famulen as
/// their tool.
pub fn open(root: impl Into<std::path::PathBuf>) -> Notebook {
    Notebook::new(root).with_tool("famulen", env!("CARGO_PKG_VERSION"))
}

/// Make the notebook in Famulen's name, committing only what this call made.
pub fn ensure(notebook: &Notebook) -> anyhow::Result<()> {
    notebook.ensure(&Writer::Secretary.signature())
}

const SHARED_KEY: &str = "notebook.shared";

/// Whether the owner shares the notebook with copies on other machines.
pub fn is_shared(store: &Store) -> anyhow::Result<bool> {
    Ok(store.state(SHARED_KEY)?.is_some())
}

pub fn set_shared(store: &Store, shared: bool) -> anyhow::Result<()> {
    if shared {
        store.set_state(SHARED_KEY, "1")
    } else {
        store.clear_state(SHARED_KEY)
    }
}

/// Who writes to the notebook. Each change is committed in their name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Writer {
    /// The owner, on the console or by hand.
    Owner,
    /// The secretary, in a conversation or a chore.
    Secretary,
    /// The lookout, which screens what comes in from outside.
    Lookout,
    /// A worker the secretary started for one task.
    Worker,
}

impl Writer {
    pub fn author(self) -> Author {
        match self {
            Writer::Owner => owner(),
            Writer::Secretary | Writer::Lookout | Writer::Worker => famulen(),
        }
    }

    /// The signature a commit by this writer carries.
    pub fn signature(self) -> Signature {
        Signature::new(self.author())
    }

    /// Refuse a writer other than the owner and the secretary: what the
    /// lookout and workers read outside reaches the notebook only through the
    /// secretary.
    pub fn may_write(self) -> anyhow::Result<()> {
        match self {
            Writer::Owner | Writer::Secretary => Ok(()),
            Writer::Lookout | Writer::Worker => Err(Refused(self).into()),
        }
    }
}

/// A write to the notebook by someone who may only read what they are given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused(pub Writer);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let who = match self.0 {
            Writer::Lookout => "the lookout",
            Writer::Worker => "a worker",
            Writer::Owner => "the owner",
            Writer::Secretary => "the secretary",
        };
        write!(
            f,
            "{who} may not write the notebook; hand what should be kept to the secretary"
        )
    }
}

impl std::error::Error for Refused {}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::TempDir;

    #[test]
    fn a_new_notebook_is_made_once_in_famulens_name() {
        let tmp = TempDir::new("notebook-famulen");
        let notebook = open(tmp.path());

        ensure(&notebook).unwrap();
        ensure(&notebook).unwrap();

        assert!(tmp.path().join("records/workgroups.md").is_file());
        assert!(tmp.path().join("memory/core.md").is_file());
        let log = log(tmp.path(), 10).unwrap();
        assert_eq!(log.len(), 1, "made once, then nothing more");
        assert_eq!(log[0].author, famulen());
        assert!(log[0].message.contains("Notebook-Tool: famulen "));
    }

    #[test]
    fn only_the_owner_and_the_secretary_may_write() {
        assert!(Writer::Owner.may_write().is_ok());
        assert!(Writer::Secretary.may_write().is_ok());
        for writer in [Writer::Lookout, Writer::Worker] {
            let refused = writer.may_write().unwrap_err();
            assert_eq!(refused.downcast_ref::<Refused>(), Some(&Refused(writer)));
        }
    }
}
