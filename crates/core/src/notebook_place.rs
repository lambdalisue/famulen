//! Where the notebook is kept, and moving it when the owner chooses another
//! place on the console.
//!
//! A move is only asked for while Famulen runs. It is carried out the next
//! time Famulen starts, before anything opens the notebook, so nothing can
//! write to the place it is moved from; `run` starts Famulen again for it.

use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::config::Config;
use crate::notebook::{MEMORY, RECORDS};
use crate::store::Store;

const PLACE_KEY: &str = "notebook.place";
const MOVING_KEY: &str = "notebook.moving";
const FAILED_KEY: &str = "notebook.move_failed";
const LEFTOVER_KEY: &str = "notebook.leftover";

const STAGING: &str = ".famulen-notebook-moving";
/// What a notebook keeps as whole directories; its other files are the ones
/// its history holds.
const DIRECTORIES: [&str; 4] = [".git", ".notebook", MEMORY, RECORDS];

/// The notebook's place, as the console shows and changes it.
#[derive(Clone)]
pub struct NotebookPlace {
    store: Store,
    current: PathBuf,
    default: PathBuf,
    gateway: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub current: PathBuf,
    pub default: PathBuf,
    /// Where it is about to be moved.
    pub moving: Option<PathBuf>,
    /// Why the last move did not happen.
    pub failed: Option<String>,
    /// Where files of the notebook could not be removed after a move.
    pub leftover: Option<PathBuf>,
}

impl NotebookPlace {
    pub fn new(store: Store, config: &Config) -> Self {
        Self {
            store,
            current: config.notebook_dir(),
            default: config.default_notebook_dir(),
            gateway: config.gateway_dir(),
        }
    }

    pub fn status(&self) -> anyhow::Result<Status> {
        Ok(Status {
            current: self.current.clone(),
            default: self.default.clone(),
            moving: moving(&self.store)?,
            failed: self.store.state(FAILED_KEY)?,
            leftover: self.store.state(LEFTOVER_KEY)?.map(PathBuf::from),
        })
    }

    /// Ask for the notebook to be moved to `path`, or to the default when it
    /// is empty. What can be refused already is refused here.
    pub fn request(&self, path: &str, home: Option<&Path>) -> anyhow::Result<()> {
        anyhow::ensure!(
            moving(&self.store)?.is_none(),
            "the notebook is already being moved"
        );
        let target = match path.trim() {
            "" => self.default.clone(),
            path => crate::writable::whole(path, home)?,
        };
        let target = real(&target)?;
        let current = real(&self.current)?;
        anyhow::ensure!(
            target != current,
            "the notebook is already at {}",
            target.display()
        );
        check(&current, &target, &self.gateway)?;
        conflicts(&current, &target, &entries(&current)?)?;
        self.store.clear_state(FAILED_KEY)?;
        self.store.clear_state(LEFTOVER_KEY)?;
        self.store.set_state(MOVING_KEY, text(&target)?)
    }
}

/// Where the notebook is about to be moved, if a move was asked for.
pub fn moving(store: &Store) -> anyhow::Result<Option<PathBuf>> {
    Ok(store.state(MOVING_KEY)?.map(PathBuf::from))
}

/// Carry out a move that was asked for, and give the configuration with the
/// place the notebook is at. Only for when nothing has the notebook open.
pub fn settle(config: Config, store: &Store) -> anyhow::Result<Config> {
    let default = config.default_notebook_dir();
    let chosen = |store: &Store| -> anyhow::Result<Option<PathBuf>> {
        Ok(store.state(PLACE_KEY)?.map(PathBuf::from))
    };
    if let Some(target) = moving(store)? {
        let current = chosen(store)?.unwrap_or_else(|| default.clone());
        if let Err(error) = carry_out(store, &current, &target, &default, &config.gateway_dir()) {
            tracing::error!(error = format!("{error:#}"), target = %target.display(), "could not move the notebook");
            store.set_state(FAILED_KEY, format!("{error:#}"))?;
        }
        store.clear_state(MOVING_KEY)?;
    }
    let place = match chosen(store)? {
        Some(place) => {
            // Making a new notebook here would hide that the owner's is
            // missing, such as when its disk is not mounted.
            anyhow::ensure!(
                place.join(MEMORY).is_dir(),
                "the notebook is not at {}, where the console says it is; put it back there",
                place.display()
            );
            place
        }
        None => default,
    };
    std::fs::create_dir_all(&place).with_context(|| format!("making {}", place.display()))?;
    let place = real(&place)?;
    apart(&place, &config.gateway_dir())?;
    Ok(config.with_notebook_at(place))
}

fn carry_out(
    store: &Store,
    current: &Path,
    target: &Path,
    default: &Path,
    gateway: &Path,
) -> anyhow::Result<()> {
    let (current, target) = (real(current)?, real(target)?);
    if current == target {
        return Ok(());
    }
    check(&current, &target, gateway)?;
    let entries = entries(&current)?;
    copy_notebook(&current, &target, &entries)?;
    store.set_state(LEFTOVER_KEY, text(&current)?)?;
    if target == real(default)? {
        store.clear_state(PLACE_KEY)?;
    } else {
        store.set_state(PLACE_KEY, text(&target)?)?;
    }
    store.clear_state(MOVING_KEY)?;
    match remove_notebook(&current, &target, &entries) {
        Ok(()) => store.clear_state(LEFTOVER_KEY)?,
        Err(error) => {
            tracing::warn!(%error, from = %current.display(), "the notebook was moved, but not all of it could be removed from where it was");
        }
    }
    tracing::info!(from = %current.display(), to = %target.display(), "moved the notebook");
    Ok(())
}

fn check(current: &Path, target: &Path, gateway: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        !target.starts_with(current),
        "{} is inside the notebook itself",
        target.display()
    );
    apart(target, gateway)
}

fn apart(notebook: &Path, gateway: &Path) -> anyhow::Result<()> {
    let gateway = real(gateway)?;
    anyhow::ensure!(
        !notebook.starts_with(&gateway) && !gateway.starts_with(notebook),
        "the notebook cannot be kept at {}: the gateway keeps its secrets in {}",
        notebook.display(),
        gateway.display()
    );
    Ok(())
}

/// What the notebook at `root` consists of, relative to it. Files that only
/// happen to lie in the same directory are not among them.
fn entries(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    anyhow::ensure!(
        root.join(".git").is_dir(),
        "{} does not hold a notebook with its own history",
        root.display()
    );
    let mut entries: Vec<PathBuf> = DIRECTORIES
        .into_iter()
        .map(PathBuf::from)
        .filter(|dir| exists(&root.join(dir)))
        .collect();
    let tracked = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .context("running git ls-files")?;
    anyhow::ensure!(
        tracked.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&tracked.stderr).trim()
    );
    entries.extend(
        String::from_utf8(tracked.stdout)
            .context("a file of the notebook has a name that is not UTF-8")?
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .filter(|path| !DIRECTORIES.iter().any(|dir| path.starts_with(dir)))
            .filter(|path| exists(&root.join(path))),
    );
    Ok(entries)
}

/// Refuse a place that already holds something else by the name of a part of
/// the notebook. The same thing there is a move that was cut off.
fn conflicts(from: &Path, to: &Path, entries: &[PathBuf]) -> anyhow::Result<()> {
    anyhow::ensure!(
        !exists(to) || to.is_dir(),
        "{} is not a directory",
        to.display()
    );
    for entry in entries {
        let there = to.join(entry);
        anyhow::ensure!(
            !exists(&there) || same(&from.join(entry), &there)?,
            "{} is already there",
            there.display()
        );
    }
    Ok(())
}

fn copy_notebook(from: &Path, to: &Path, entries: &[PathBuf]) -> anyhow::Result<()> {
    conflicts(from, to, entries)?;
    std::fs::create_dir_all(to).with_context(|| format!("making {}", to.display()))?;
    let staging = to.join(STAGING);
    if exists(&staging) {
        std::fs::remove_dir_all(&staging)?;
    }
    let mut placed = Vec::new();
    let copied = place_copies(from, to, &staging, entries, &mut placed);
    let cleared = if exists(&staging) {
        std::fs::remove_dir_all(&staging)
    } else {
        Ok(())
    };
    if copied.is_err() {
        for entry in &placed {
            let _ = remove(&to.join(entry));
        }
    }
    copied?;
    Ok(cleared?)
}

/// Copy everything beside its destination and compare it before any of it is
/// put in place, so a copy that fails leaves the destination as it was.
fn place_copies<'a>(
    from: &Path,
    to: &Path,
    staging: &Path,
    entries: &'a [PathBuf],
    placed: &mut Vec<&'a PathBuf>,
) -> anyhow::Result<()> {
    let mut missing = Vec::new();
    for entry in entries {
        if exists(&to.join(entry)) {
            continue;
        }
        let (source, copy) = (from.join(entry), staging.join(entry));
        copy_tree(&source, &copy).with_context(|| format!("copying {}", source.display()))?;
        anyhow::ensure!(
            same(&source, &copy)?,
            "{} was not copied faithfully",
            source.display()
        );
        missing.push(entry);
    }
    for entry in missing {
        let there = to.join(entry);
        if let Some(parent) = there.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(staging.join(entry), &there)
            .with_context(|| format!("placing {}", there.display()))?;
        placed.push(entry);
    }
    Ok(())
}

fn remove_notebook(root: &Path, moved: &Path, entries: &[PathBuf]) -> anyhow::Result<()> {
    for entry in entries {
        let path = root.join(entry);
        // The owner's editor may have saved into it since it was copied.
        anyhow::ensure!(
            same(&path, &moved.join(entry))?,
            "{} changed while it was moved",
            path.display()
        );
        remove(&path).with_context(|| format!("removing {}", path.display()))?;
        let mut parent = entry.parent();
        while let Some(dir) = parent.filter(|p| !p.as_os_str().is_empty()) {
            if std::fs::remove_dir(root.join(dir)).is_err() {
                break;
            }
            parent = dir.parent();
        }
    }
    // Fails when the owner keeps something else there, which stays.
    let _ = std::fs::remove_dir(root);
    Ok(())
}

fn remove(path: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path)?.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn copy_tree(from: &Path, to: &Path) -> anyhow::Result<()> {
    let kind = std::fs::symlink_metadata(from)?.file_type();
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if kind.is_dir() {
        std::fs::create_dir_all(to)?;
        for child in std::fs::read_dir(from)? {
            let name = child?.file_name();
            copy_tree(&from.join(&name), &to.join(&name))?;
        }
    } else if kind.is_symlink() {
        link(&std::fs::read_link(from)?, to)?;
    } else {
        anyhow::ensure!(kind.is_file(), "{} is not a plain file", from.display());
        std::fs::copy(from, to)?;
        // The original is removed next, so the copy must be on the disk.
        #[cfg(unix)]
        std::fs::File::open(to)?.sync_all()?;
    }
    Ok(())
}

#[cfg(unix)]
fn link(to: &Path, at: &Path) -> anyhow::Result<()> {
    Ok(std::os::unix::fs::symlink(to, at)?)
}

#[cfg(not(unix))]
fn link(_to: &Path, at: &Path) -> anyhow::Result<()> {
    anyhow::bail!("{} is a link, which cannot be moved here", at.display())
}

fn same(a: &Path, b: &Path) -> anyhow::Result<bool> {
    let (of_a, of_b) = (std::fs::symlink_metadata(a)?, std::fs::symlink_metadata(b)?);
    if of_a.file_type() != of_b.file_type() {
        return Ok(false);
    }
    if of_a.is_symlink() {
        return Ok(std::fs::read_link(a)? == std::fs::read_link(b)?);
    }
    if !of_a.is_dir() {
        return Ok(of_a.len() == of_b.len() && std::fs::read(a)? == std::fs::read(b)?);
    }
    let names = |dir: &Path| -> std::io::Result<std::collections::BTreeSet<_>> {
        std::fs::read_dir(dir)?
            .map(|e| e.map(|e| e.file_name()))
            .collect()
    };
    let children = names(a)?;
    if children != names(b)? {
        return Ok(false);
    }
    for name in children {
        if !same(&a.join(&name), &b.join(&name))? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// `path` without links on the way, even when the end of it is not made yet.
fn real(path: &Path) -> anyhow::Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let mut above = path.as_path();
    loop {
        match above.canonicalize() {
            Ok(found) => return Ok(found.join(path.strip_prefix(above)?).components().collect()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                above = above
                    .parent()
                    .with_context(|| format!("nothing of {} exists", path.display()))?;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", above.display()));
            }
        }
    }
}

fn text(path: &Path) -> anyhow::Result<&str> {
    path.to_str()
        .with_context(|| format!("{} is not UTF-8", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DATA_DIR_ENV, Defaults, NOTEBOOK_DIR_ENV};
    use crate::notebook::testing::TempDir;

    struct Home {
        tmp: TempDir,
        store: Store,
    }

    impl Home {
        /// A data directory with a notebook at its default place.
        fn new(name: &str) -> Self {
            let home = Self {
                tmp: TempDir::new(name),
                store: Store::open_in_memory().unwrap(),
            };
            crate::notebook::ensure(&crate::notebook::open(home.settle().notebook_dir())).unwrap();
            home
        }

        fn root(&self) -> PathBuf {
            self.tmp.path().canonicalize().unwrap()
        }

        fn config(&self) -> Config {
            let data = self.root().join("data");
            Config::load(
                None,
                |name| match name {
                    DATA_DIR_ENV => Some(data.display().to_string()),
                    _ => None,
                },
                Defaults {
                    data_dir: data.clone(),
                    timezone: chrono_tz::UTC,
                },
            )
            .unwrap()
            .prepare()
            .unwrap()
        }

        fn try_settle(&self) -> anyhow::Result<Config> {
            settle(self.config(), &self.store)
        }

        fn settle(&self) -> Config {
            self.try_settle().unwrap()
        }

        fn place(&self) -> NotebookPlace {
            NotebookPlace::new(self.store.clone(), &self.settle())
        }

        fn request(&self, path: &Path) -> anyhow::Result<()> {
            self.place().request(path.to_str().unwrap(), None)
        }
    }

    #[test]
    fn the_notebook_is_at_its_default_until_the_owner_chooses() {
        let home = Home::new("place-default");
        let status = home.place().status().unwrap();
        assert_eq!(status.current, home.root().join("data/notebook"));
        assert_eq!(status.default, status.current);
        assert_eq!(status.moving, None);
    }

    #[test]
    fn a_move_takes_the_memories_and_the_history_and_leaves_nothing_behind() {
        let home = Home::new("place-move");
        let old = home.root().join("data/notebook");
        std::fs::write(old.join("memory/core.md"), "- not committed yet\n").unwrap();
        let target = home.root().join("notes/famulen");

        home.request(&target).unwrap();
        assert_eq!(home.place().status().unwrap().current, target);

        assert_eq!(
            std::fs::read_to_string(target.join("memory/core.md")).unwrap(),
            "- not committed yet\n"
        );
        assert_eq!(crate::notebook::log(&target, 10).unwrap().len(), 1);
        assert!(!old.exists());
        assert_eq!(home.place().status().unwrap().leftover, None);
        assert!(!target.join(STAGING).exists());
    }

    #[test]
    fn nothing_moves_while_famulen_runs() {
        let home = Home::new("place-asked");
        let place = home.place();
        let target = home.root().join("notes");

        place.request(target.to_str().unwrap(), None).unwrap();

        assert_eq!(place.status().unwrap().moving, Some(target.clone()));
        assert!(!target.exists());
        assert!(place.request(target.to_str().unwrap(), None).is_err());
    }

    #[test]
    fn the_owners_other_files_stay_where_they_are_on_both_sides() {
        let home = Home::new("place-others");
        let old = home.root().join("data/notebook");
        std::fs::write(old.join("own.md"), "old side").unwrap();
        let target = home.root().join("notes");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("INDEX.md"), "new side").unwrap();

        home.request(&target).unwrap();
        home.settle();

        assert_eq!(std::fs::read_dir(&old).unwrap().count(), 1);
        assert!(old.join("own.md").exists());
        assert!(!target.join("own.md").exists());
        assert_eq!(
            std::fs::read_to_string(target.join("INDEX.md")).unwrap(),
            "new side"
        );
        assert!(target.join("README.md").exists());
    }

    #[test]
    fn a_place_holding_something_by_a_name_the_notebook_uses_is_refused() {
        let home = Home::new("place-taken");
        for name in [".git", "README.md", "memory"] {
            let target = home.root().join(format!("notes-{name}"));
            std::fs::create_dir_all(target.join(name)).unwrap();
            let error = home.request(&target).unwrap_err().to_string();
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn a_move_that_cannot_be_made_leaves_the_notebook_and_says_why() {
        let home = Home::new("place-failed");
        let target = home.root().join("notes");
        home.request(&target).unwrap();
        std::fs::create_dir_all(target.join("records")).unwrap();

        let status = home.place().status().unwrap();

        assert_eq!(status.current, home.root().join("data/notebook"));
        assert!(status.current.join("memory/core.md").exists());
        assert_eq!(status.moving, None);
        assert!(status.failed.unwrap().contains("records"));
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 1);
    }

    #[test]
    fn what_changed_while_it_was_moved_is_not_removed() {
        let home = Home::new("place-changed");
        let old = home.root().join("data/notebook");
        let target = home.root().join("notes");
        let entries = entries(&old).unwrap();
        copy_notebook(&old, &target, &entries).unwrap();
        std::fs::write(old.join("memory/core.md"), "- saved just now\n").unwrap();

        assert!(remove_notebook(&old, &target, &entries).is_err());

        assert!(old.join("memory/core.md").exists());
    }

    #[test]
    fn a_move_cut_off_on_the_way_is_finished_at_the_next_start() {
        let home = Home::new("place-resume");
        let old = home.root().join("data/notebook");
        let target = home.root().join("notes");
        home.request(&target).unwrap();
        copy_tree(&old.join("memory"), &target.join("memory")).unwrap();
        std::fs::create_dir_all(target.join(STAGING).join("records")).unwrap();

        assert_eq!(home.settle().notebook_dir(), target);
        assert!(target.join("records/workgroups.md").exists());
        assert!(!target.join(STAGING).exists());
        assert!(!old.exists());
    }

    #[test]
    fn an_empty_path_moves_it_back_to_the_default() {
        let home = Home::new("place-back");
        home.request(&home.root().join("notes")).unwrap();
        home.settle();

        home.place().request("", None).unwrap();

        let default = home.root().join("data/notebook");
        assert_eq!(home.settle().notebook_dir(), default);
        assert!(default.join("memory/core.md").exists());
        assert!(!home.root().join("notes").exists());
        assert_eq!(home.store.state(PLACE_KEY).unwrap(), None);
    }

    #[test]
    fn a_chosen_place_wins_over_a_default_set_later() {
        let home = Home::new("place-wins");
        let target = home.root().join("notes");
        home.request(&target).unwrap();
        home.settle();

        let env = home.root().join("elsewhere");
        let data = home.root().join("data");
        let config = Config::load(
            None,
            |name| (name == NOTEBOOK_DIR_ENV).then(|| env.display().to_string()),
            Defaults {
                data_dir: data,
                timezone: chrono_tz::UTC,
            },
        )
        .unwrap();

        let config = settle(config, &home.store).unwrap();
        assert_eq!(config.notebook_dir(), target);
        assert_eq!(config.default_notebook_dir(), env);
        assert!(!env.exists());
    }

    #[test]
    fn a_chosen_place_that_is_gone_is_not_replaced_by_an_empty_notebook() {
        let home = Home::new("place-gone");
        let target = home.root().join("notes");
        home.request(&target).unwrap();
        home.settle();
        std::fs::rename(&target, home.root().join("away")).unwrap();

        assert!(home.try_settle().is_err());
        assert!(!target.exists());
    }

    #[test]
    fn the_notebook_is_kept_apart_from_the_gateways_secrets_and_from_itself() {
        let home = Home::new("place-apart");
        let data = home.root().join("data");
        for place in [
            data.clone(),
            data.join("gateway"),
            data.join("gateway/x"),
            data.join("notebook"),
            data.join("notebook/memory/x"),
        ] {
            assert!(home.request(&place).is_err(), "{}", place.display());
        }
        assert!(home.place().request("notes", None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_place_behind_a_link_is_known_by_its_real_path() {
        let home = Home::new("place-link");
        let real = home.root().join("vault");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, home.root().join("link")).unwrap();

        home.request(&home.root().join("link/notebook")).unwrap();

        assert_eq!(home.settle().notebook_dir(), real.join("notebook"));
    }
}
