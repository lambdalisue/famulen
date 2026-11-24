//! Keeping a notebook from other writers, in this process and in others.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::marker::PhantomData;
use std::path::PathBuf;

use anyhow::Context as _;

use crate::Notebook;

const FILE: &str = "writing.lock";

thread_local! {
    /// The notebooks this thread holds, and how many times over.
    static HELD: RefCell<BTreeMap<PathBuf, (File, usize)>> = const { RefCell::new(BTreeMap::new()) };
}

/// A notebook kept from other writers until this is dropped.
#[must_use]
pub struct Hold {
    root: PathBuf,
    /// What a thread holds only that thread can let go.
    _this_thread: PhantomData<*const ()>,
}

impl Notebook {
    /// Wait until no one else writes the notebook, and keep it so while more
    /// than one file of it is read, changed and committed. A thread that
    /// holds it may hold it again.
    pub fn hold(&self) -> anyhow::Result<Hold> {
        let dir = self.state_dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
        let root = self
            .root()
            .canonicalize()
            .with_context(|| format!("finding {}", self.root().display()))?;
        HELD.with_borrow_mut(|held| {
            if let Some((_, times)) = held.get_mut(&root) {
                *times += 1;
                return Ok(());
            }
            let path = dir.join(FILE);
            let file = File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path)
                .with_context(|| format!("opening {}", path.display()))?;
            file.lock()
                .with_context(|| format!("locking {}", path.display()))?;
            held.insert(root.clone(), (file, 1));
            anyhow::Ok(())
        })?;
        Ok(Hold {
            root,
            _this_thread: PhantomData,
        })
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        HELD.with_borrow_mut(|held| {
            if let Some((_, times)) = held.get_mut(&self.root) {
                *times -= 1;
                if *times == 0 {
                    held.remove(&self.root);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::notebook;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn one_writer_waits_for_another_and_may_hold_again_what_it_holds() {
        let (_tmp, notebook) = notebook("hold");
        let first = notebook.hold().unwrap();
        let again = notebook.hold().unwrap();
        let (told, heard) = mpsc::channel();
        let other = std::thread::spawn({
            let notebook = notebook.clone();
            move || {
                let _hold = notebook.hold().unwrap();
                told.send(()).unwrap();
            }
        });

        drop(again);
        assert!(heard.recv_timeout(Duration::from_millis(100)).is_err());
        drop(first);
        heard.recv_timeout(Duration::from_secs(30)).unwrap();
        other.join().unwrap();
    }
}
