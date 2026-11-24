//! Places outside the working directory where the secretary's shell
//! commands may write, as the owner adds them on the console. None at first.
//! Reading is not widened or narrowed by them, and a sealed place stays
//! closed whatever is added.

use std::path::{Path, PathBuf};

use crate::paths::under;
use crate::store::Store;

const KEY: &str = "secretary.writable";

/// The places, in the order they were added.
pub fn places(store: &Store) -> anyhow::Result<Vec<PathBuf>> {
    Ok(match store.state(KEY)? {
        Some(json) => serde_json::from_str(&json)?,
        None => Vec::new(),
    })
}

/// Add `path`: absolute, or under `home` when it starts with `~/`. Adding a
/// place already there changes nothing. A place under one already there is
/// refused, since it is writable already; places under the one added are
/// dropped for the same reason, so removing a place never leaves part of it
/// writable.
pub fn add(store: &Store, path: &str, home: Option<&Path>) -> anyhow::Result<Vec<PathBuf>> {
    let path = checked(path, home)?;
    let mut places = places(store)?;
    if places.iter().any(|p| under(p, &path) && under(&path, p)) {
        return Ok(places);
    }
    if let Some(above) = places.iter().find(|p| under(&path, p)) {
        anyhow::bail!(
            "{} is writable already, as it is under {}",
            path.display(),
            above.display()
        );
    }
    places.retain(|p| !under(p, &path));
    places.push(path);
    save(store, &places)?;
    Ok(places)
}

/// Remove `path`, written as it is listed.
pub fn remove(store: &Store, path: &str) -> anyhow::Result<Vec<PathBuf>> {
    let mut places = places(store)?;
    places.retain(|p| p != Path::new(path.trim()));
    save(store, &places)?;
    Ok(places)
}

fn save(store: &Store, places: &[PathBuf]) -> anyhow::Result<()> {
    store.set_state(KEY, serde_json::to_string(places)?)
}

/// A folder's place, its parent and the folders directly inside it, for
/// the owner to walk to a place on the machine Famulen runs on.
#[derive(Debug, PartialEq, Eq)]
pub struct Folder {
    pub path: PathBuf,
    pub parent: Option<PathBuf>,
    /// By name, sorted; hidden ones are left out.
    pub children: Vec<String>,
}

/// The folder at `path` (empty for `home`), written as [`add`] takes it.
pub fn folder(path: &str, home: Option<&Path>) -> anyhow::Result<Folder> {
    let path = match path.trim() {
        "" => home
            .ok_or_else(|| anyhow::anyhow!("no home to start from; write the whole path"))?
            .to_owned(),
        path => whole(path, home)?,
    };
    let mut children: Vec<String> = std::fs::read_dir(&path)?
        .filter_map(Result::ok)
        .filter(|e| std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    children.sort();
    Ok(Folder {
        parent: path.parent().map(Path::to_owned),
        path,
        children,
    })
}

/// Folders below `root` named `name`, nearest first. A folder dropped on
/// the console from the owner's file manager arrives as its name alone,
/// so this is how its whole path is found again. Hidden folders, symbolic
/// links and folders that hold builds or packages are not walked into,
/// and the walk gives up after `budget` folders.
pub fn find(root: &Path, name: &str, budget: usize) -> Vec<PathBuf> {
    const DEPTH: usize = 6;
    const FOUND: usize = 20;
    const SKIPPED: [&str; 4] = ["node_modules", "target", "Library", "vendor"];
    let mut found = Vec::new();
    let mut queue = std::collections::VecDeque::from([(root.to_owned(), 0)]);
    let mut walked = 0;
    while let Some((dir, depth)) = queue.pop_front() {
        walked += 1;
        if walked > budget || found.len() >= FOUND {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
            .filter(|(n, _)| !n.starts_with('.'))
            .collect();
        children.sort();
        for (child, path) in children {
            if child == name {
                found.push(path.clone());
            }
            if depth + 1 < DEPTH && !SKIPPED.contains(&child.as_str()) {
                queue.push_back((path, depth + 1));
            }
        }
    }
    found.truncate(FOUND);
    found
}

fn checked(path: &str, home: Option<&Path>) -> anyhow::Result<PathBuf> {
    let path = whole(path, home)?;
    anyhow::ensure!(
        path.parent().is_some(),
        "the whole disk cannot be made writable"
    );
    Ok(path)
}

const WHOLE: &str = if cfg!(windows) {
    r"write a whole path, from a drive such as C:\ (or ~\)"
} else {
    "write a whole path, from / (or ~/)"
};

pub(crate) fn whole(path: &str, home: Option<&Path>) -> anyhow::Result<PathBuf> {
    let path = path.trim();
    let from_home =
        || home.ok_or_else(|| anyhow::anyhow!("no home to read ~ from; write the whole path"));
    let under_home = path
        .strip_prefix('~')
        .and_then(|rest| rest.strip_prefix(std::path::is_separator));
    let path = match under_home {
        Some(rest) => from_home()?.join(rest),
        None if path == "~" => from_home()?.to_owned(),
        None => PathBuf::from(path),
    };
    anyhow::ensure!(path.is_absolute(), "{WHOLE}");
    // Split by hand: `components` drops a `.` in the middle without a word.
    anyhow::ensure!(
        !path
            .to_string_lossy()
            .split(std::path::is_separator)
            .any(|part| part == "." || part == ".."),
        "write the path without . or .."
    );
    Ok(crate::paths::plain(path).components().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    /// `path` as a whole path of this system.
    fn at(path: &str) -> String {
        if cfg!(windows) {
            format!("C:{}", path.replace('/', "\\"))
        } else {
            path.to_owned()
        }
    }

    #[test]
    fn there_are_none_at_first() {
        assert!(places(&store()).unwrap().is_empty());
    }

    #[test]
    fn places_are_kept_in_the_order_added_and_once_each() {
        let s = store();
        add(&s, &at("/home/me/notes"), None).unwrap();
        add(&s, &at("/srv/out/"), None).unwrap();
        add(&s, &format!(" {} ", at("/home/me/notes")), None).unwrap();
        assert_eq!(
            places(&s).unwrap(),
            [
                PathBuf::from(at("/home/me/notes")),
                PathBuf::from(at("/srv/out"))
            ]
        );
    }

    #[test]
    fn a_path_from_home_is_written_out_in_full() {
        let s = store();
        add(&s, "~/notes", Some(Path::new(&at("/home/me")))).unwrap();
        assert_eq!(places(&s).unwrap(), [PathBuf::from(at("/home/me/notes"))]);
        assert!(add(&s, "~/x", None).is_err());
    }

    #[test]
    fn only_whole_plain_paths_below_the_root_are_taken() {
        let s = store();
        for bad in ["notes", ""] {
            assert!(add(&s, bad, None).is_err(), "{bad:?}");
        }
        for bad in ["/", "/home/me/../etc", "/home/./me"] {
            assert!(add(&s, &at(bad), None).is_err(), "{bad:?}");
        }
        assert!(places(&s).unwrap().is_empty());
    }

    #[test]
    fn a_place_under_one_already_there_is_refused_with_the_one_that_covers_it() {
        let s = store();
        add(&s, &at("/home/me/sandbox"), None).unwrap();
        let error = add(&s, &at("/home/me/sandbox/a"), None).unwrap_err();
        assert!(
            error.to_string().contains(&at("/home/me/sandbox")),
            "{error}"
        );
        add(&s, &at("/home/me/sandboxes"), None).unwrap();
        assert_eq!(
            places(&s).unwrap(),
            [
                PathBuf::from(at("/home/me/sandbox")),
                PathBuf::from(at("/home/me/sandboxes"))
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_place_is_known_in_every_spelling_windows_has_for_it() {
        let s = store();
        add(&s, r"C:\Srv\Out", None).unwrap();
        add(&s, "c:/srv/out/", None).unwrap();
        add(&s, r"\\?\C:\SRV\out", None).unwrap();
        assert_eq!(places(&s).unwrap(), [PathBuf::from(r"C:\Srv\Out")]);
        assert!(add(&s, "c:/srv/out/a", None).is_err());
        add(&s, "c:/srv", None).unwrap();
        assert_eq!(places(&s).unwrap(), [PathBuf::from(r"c:\srv")]);
        for bad in ["C:/home/../etc", "C:/home/./me", "C:/", r"\home", "C:home"] {
            assert!(add(&s, bad, None).is_err(), "{bad:?}");
        }
        add(&s, r"~\notes", Some(Path::new(r"C:\home"))).unwrap();
        assert!(
            places(&s)
                .unwrap()
                .contains(&PathBuf::from(r"C:\home\notes"))
        );
    }

    #[test]
    fn a_place_above_some_already_there_takes_their_place() {
        let s = store();
        add(&s, &at("/srv/out"), None).unwrap();
        add(&s, &at("/home/me/sandbox/a"), None).unwrap();
        add(&s, &at("/home/me/sandbox/b/c"), None).unwrap();
        add(&s, &at("/home/me/sandbox"), None).unwrap();
        assert_eq!(
            places(&s).unwrap(),
            [
                PathBuf::from(at("/srv/out")),
                PathBuf::from(at("/home/me/sandbox"))
            ]
        );
    }

    #[test]
    fn a_place_is_removed_as_listed() {
        let s = store();
        add(&s, &at("/a"), None).unwrap();
        add(&s, &at("/b"), None).unwrap();
        assert_eq!(remove(&s, &at("/a")).unwrap(), [PathBuf::from(at("/b"))]);
        assert_eq!(places(&s).unwrap(), [PathBuf::from(at("/b"))]);
    }

    fn tree(name: &str, dirs: &[&str]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("famulen-writable-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        for dir in dirs {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        root
    }

    #[test]
    fn a_folder_lists_the_folders_inside_but_not_hidden_ones_or_files() {
        let root = tree("folder", &["b", "a/x", ".hidden"]);
        std::fs::write(root.join("file"), "").unwrap();

        let f = folder(&root.display().to_string(), None).unwrap();
        assert_eq!(f.path, root);
        assert_eq!(f.parent.as_deref(), root.parent());
        assert_eq!(f.children, ["a", "b"]);
    }

    #[test]
    fn a_folder_starts_at_home_and_the_root_has_no_parent() {
        let root = tree("home", &["notes"]);
        assert_eq!(folder("", Some(&root)).unwrap().children, ["notes"]);
        assert_eq!(folder("~", Some(&root)).unwrap().path, root);
        assert_eq!(folder(&at("/"), None).unwrap().parent, None);
        assert!(folder("notes", Some(&root)).is_err());
        assert!(folder("", None).is_err());
    }

    #[test]
    fn a_folder_is_found_by_name_nearest_first_without_walking_into_hidden_or_build_folders() {
        let root = tree(
            "find",
            &["a/b/notes", "notes", ".cache/notes", "x/node_modules/notes"],
        );
        assert_eq!(
            find(&root, "notes", 1000),
            [root.join("notes"), root.join("a/b/notes")]
        );
        assert!(find(&root, "nothing", 1000).is_empty());
        assert_eq!(find(&root, "notes", 1), [root.join("notes")]);
    }
}
