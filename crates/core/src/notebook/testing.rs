use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// Lay a notebook as [`super::ensure`] makes it, copied from one made once:
/// making one starts Git six times. One that is there already is left as it is.
pub(crate) fn lay(root: &Path) {
    static MADE: OnceLock<Vec<(PathBuf, Option<Vec<u8>>)>> = OnceLock::new();
    if root.exists() && std::fs::read_dir(root).unwrap().next().is_some() {
        return;
    }
    let made = MADE.get_or_init(|| {
        let tmp = TempDir::new("made");
        super::ensure(&super::open(tmp.path())).unwrap();
        let mut found = Vec::new();
        gather(tmp.path(), Path::new(""), &mut found);
        found
    });
    for (rel, bytes) in made {
        let path = root.join(rel);
        match bytes {
            Some(bytes) => std::fs::write(path, bytes).unwrap(),
            None => std::fs::create_dir_all(path).unwrap(),
        }
    }
}

/// What lies in `root`, each directory before what is in it and with no content.
fn gather(root: &Path, at: &Path, found: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
    found.push((at.to_path_buf(), None));
    for entry in std::fs::read_dir(root.join(at)).unwrap() {
        let rel = at.join(entry.unwrap().file_name());
        if root.join(&rel).is_dir() {
            gather(root, &rel, found);
        } else {
            let bytes = std::fs::read(root.join(&rel)).unwrap();
            found.push((rel, Some(bytes)));
        }
    }
}
