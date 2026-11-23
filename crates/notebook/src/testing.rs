use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{Author, Notebook, Signature};

pub(crate) fn me() -> Signature {
    Signature::new(Author::new("Alice", "alice@example.com"))
}

/// A notebook made anew in a directory of its own.
pub(crate) fn notebook(name: &str) -> (TempDir, Notebook) {
    let tmp = TempDir::new(name);
    let notebook = Notebook::new(tmp.path());
    notebook.ensure(&me()).unwrap();
    (tmp, notebook)
}

/// Write a file by hand, given relative to `root`.
pub(crate) fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Write a record by hand, and give where it is.
pub(crate) fn record(notebook: &Notebook, kind: &str, id: &str, body: &str) -> String {
    let rel = format!("records/{kind}/{id}.md");
    write(
        notebook.root(),
        &rel,
        &format!("---\nid: {id}\n---\n\n{body}"),
    );
    rel
}

/// A directory under the system temp dir, removed when dropped.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "famulen-notebook-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
