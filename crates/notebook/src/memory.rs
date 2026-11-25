//! Where each layer of memory lives in the directory, and reading and
//! writing them.

mod long;
mod working;

pub use crate::link::links;
pub use long::*;
pub use working::*;

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use anyhow::Context as _;
use chrono::{NaiveDate, NaiveDateTime};

use crate::{Doc, Entry, EpisodeState, README, Src, slash};

pub const CORE: &str = "core.md";
pub const PENDING: &str = "pending.md";
/// The inbox of notes. The directory's name is older than the layer's, and
/// kept: other programs that keep the notebook know it.
pub const EPISODIC: &str = "episodic";
pub const WORKING: &str = "working";
pub const LONG: &str = "long";
pub const FORGOTTEN: &str = "forgotten";

/// Git keeps no empty directory, so each layer directory holds this file.
pub(crate) const KEEP: &str = ".gitkeep";

const README_TEMPLATE: &str = include_str!("../templates/memory.md");
const CORE_TEMPLATE: &str = "# 心得\n";
const PENDING_TEMPLATE: &str = "# 保留中の判断\n";

/// What a long-term memory file is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LongGroup {
    /// Facts: people, likes, how things are.
    Semantic,
    /// How things are done.
    Procedural,
    /// Promises and what is to be done.
    Prospective,
}

impl LongGroup {
    pub const ALL: [LongGroup; 3] = [
        LongGroup::Semantic,
        LongGroup::Procedural,
        LongGroup::Prospective,
    ];

    pub fn dir_name(self) -> &'static str {
        match self {
            LongGroup::Semantic => "semantic",
            LongGroup::Procedural => "procedural",
            LongGroup::Prospective => "prospective",
        }
    }

    pub fn from_dir_name(name: &str) -> Option<Self> {
        LongGroup::ALL.into_iter().find(|g| g.dir_name() == name)
    }
}

/// The layer a file belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    Episodic,
    Working,
    Pending,
    Core,
    Long(LongGroup),
    Forgotten,
}

impl Layer {
    /// The layer of a Markdown file given relative to the memory directory.
    pub fn of(rel: impl AsRef<Path>) -> Option<Layer> {
        let rel = rel.as_ref();
        if rel.extension()? != "md" {
            return None;
        }
        let parts: Vec<&str> = rel
            .components()
            .map(|c| match c {
                Component::Normal(s) => s.to_str(),
                _ => None,
            })
            .collect::<Option<_>>()?;
        if parts.iter().any(|p| p.starts_with('.')) {
            return None;
        }
        match parts[..] {
            [CORE] => Some(Layer::Core),
            [PENDING] => Some(Layer::Pending),
            [EPISODIC, _] => Some(Layer::Episodic),
            [WORKING, _] => Some(Layer::Working),
            [FORGOTTEN, _] => Some(Layer::Forgotten),
            [LONG, group, _] => LongGroup::from_dir_name(group).map(Layer::Long),
            _ => None,
        }
    }

    /// Core, long-term and pending: what consolidation may not shrink.
    pub fn is_kept(self) -> bool {
        matches!(self, Layer::Core | Layer::Long(_) | Layer::Pending)
    }

    /// Where the layer is, relative to the memory directory, without `.md`:
    /// `core`, `episodic`, `long/semantic`.
    pub fn scope(self) -> String {
        match self {
            Layer::Core => "core".into(),
            Layer::Pending => "pending".into(),
            Layer::Episodic => EPISODIC.into(),
            Layer::Working => WORKING.into(),
            Layer::Long(group) => format!("{LONG}/{}", group.dir_name()),
            Layer::Forgotten => FORGOTTEN.into(),
        }
    }
}

/// Create every layer missing from the directory. Returns the files this call
/// made, relative to the directory, for the notebook to commit.
pub fn ensure(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut made = Vec::new();
    for (file, template) in [
        (README, README_TEMPLATE),
        (CORE, CORE_TEMPLATE),
        (PENDING, PENDING_TEMPLATE),
    ] {
        let path = dir.join(file);
        if !path.exists() {
            std::fs::write(&path, template)?;
            made.push(PathBuf::from(file));
        }
    }
    let long_dirs: Vec<PathBuf> = LongGroup::ALL
        .iter()
        .map(|g| Path::new(LONG).join(g.dir_name()))
        .collect();
    let dirs = [EPISODIC, WORKING, FORGOTTEN]
        .into_iter()
        .map(PathBuf::from)
        .chain(long_dirs);
    for sub in dirs {
        let path = dir.join(&sub);
        if !path.exists() {
            std::fs::create_dir_all(&path)?;
            std::fs::write(path.join(KEEP), "")?;
            made.push(sub.join(KEEP));
        }
    }
    Ok(made)
}

/// Every file that holds memory items, relative to the directory, sorted.
pub fn entry_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for file in [CORE, PENDING] {
        if dir.join(file).is_file() {
            files.push(PathBuf::from(file));
        }
    }
    for sub in [EPISODIC, FORGOTTEN] {
        files.extend(markdown_files(dir, Path::new(sub))?);
    }
    for group in LongGroup::ALL {
        files.extend(markdown_files(
            dir,
            &Path::new(LONG).join(group.dir_name()),
        )?);
    }
    files.sort();
    Ok(files)
}

fn markdown_files(dir: &Path, sub: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let read = match std::fs::read_dir(dir.join(sub)) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("listing {}", sub.display())),
    };
    let mut files = Vec::new();
    for item in read {
        let item = item?;
        let rel = sub.join(item.file_name());
        if item.file_type()?.is_file() && Layer::of(&rel).is_some() {
            files.push(rel);
        }
    }
    files.sort();
    Ok(files)
}

/// Read one file of memory, given relative to the directory. A long-term
/// memory file is read as its one item.
pub fn read_doc(dir: &Path, rel: impl AsRef<Path>) -> anyhow::Result<Doc> {
    let rel = rel.as_ref();
    match Layer::of(rel) {
        Some(Layer::Long(_)) => Doc::load_long(&dir.join(rel)),
        _ => Doc::load(&dir.join(rel)),
    }
}

/// A file to add an item to: the file as it is, or, when it does not exist
/// yet, an empty one headed by its name.
pub fn doc_to_add_to(dir: &Path, rel: impl AsRef<Path>) -> anyhow::Result<Doc> {
    let rel = rel.as_ref();
    let doc = read_doc(dir, rel)?;
    if doc.is_long() || !doc.render().is_empty() {
        return Ok(doc);
    }
    Ok(Doc::parse(&format!("# {}\n", stem(rel))))
}

fn entries(dir: &Path, rel: impl AsRef<Path>) -> anyhow::Result<Vec<Entry>> {
    Ok(read_doc(dir, rel)?.entries().cloned().collect())
}

pub fn core_entries(dir: &Path) -> anyhow::Result<Vec<Entry>> {
    entries(dir, CORE)
}

pub fn pending_entries(dir: &Path) -> anyhow::Result<Vec<Entry>> {
    entries(dir, PENDING)
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn episode_path(date: NaiveDate) -> PathBuf {
    Path::new(EPISODIC).join(format!("{}.md", date.format("%Y-%m-%d")))
}

/// The days that have a file of notes, oldest first.
pub fn episodic_dates(dir: &Path) -> anyhow::Result<Vec<NaiveDate>> {
    Ok(markdown_files(dir, Path::new(EPISODIC))?
        .iter()
        .filter_map(|path| NaiveDate::parse_from_str(&stem(path), "%Y-%m-%d").ok())
        .collect())
}

pub fn episodes(dir: &Path, date: NaiveDate) -> anyhow::Result<Vec<Entry>> {
    entries(dir, episode_path(date))
}

/// Write down a noticing in the day's file of the inbox, with a fresh id.
pub fn append_episode(
    dir: &Path,
    at: NaiveDateTime,
    text: &str,
    state: EpisodeState,
) -> anyhow::Result<Entry> {
    push_episode(dir, at, episode(dir, at, text, state)?)
}

/// Like [`append_episode`], marking where the noticing came from.
pub fn append_episode_from(
    dir: &Path,
    at: NaiveDateTime,
    text: &str,
    state: EpisodeState,
    src: Src,
) -> anyhow::Result<Entry> {
    let entry = episode(dir, at, text, state)?.with("src", src.as_str())?;
    push_episode(dir, at, entry)
}

fn episode(
    dir: &Path,
    at: NaiveDateTime,
    text: &str,
    state: EpisodeState,
) -> anyhow::Result<Entry> {
    Entry::new(&format!("{} {text}", at.format("%H:%M")))
        .with("id", fresh_id(dir)?)?
        .with("state", state.as_str())
}

fn push_episode(dir: &Path, at: NaiveDateTime, entry: Entry) -> anyhow::Result<Entry> {
    let rel = episode_path(at.date());
    let mut doc = doc_to_add_to(dir, &rel)?;
    doc.push(entry.clone())?;
    doc.save(&dir.join(rel))?;
    Ok(entry)
}

/// Set one attribute on a note.
pub fn tag_episode(
    dir: &Path,
    date: NaiveDate,
    id: &str,
    key: &str,
    value: &str,
) -> anyhow::Result<()> {
    let path = dir.join(episode_path(date));
    let mut doc = Doc::load(&path)?;
    doc.update(id, |e| e.set(key, value))
        .with_context(|| format!("no item {id}"))??;
    doc.save(&path)
}

/// How much of a ULID's random part a memory's id takes.
const RANDOM_LENGTH: usize = 10;

/// An id no item in the directory has yet, nor one written meanwhile in
/// another copy of the notebook.
pub fn fresh_id(dir: &Path) -> anyhow::Result<String> {
    let mut ids = BTreeSet::new();
    for file in entry_files(dir)? {
        for entry in read_doc(dir, &file)?.entries() {
            if let Some(id) = entry.id() {
                ids.insert(id.to_string());
            }
        }
    }
    loop {
        // Counting up would give two copies of a notebook, written apart, the
        // same id for different memories.
        let drawn = ulid::Ulid::generate().to_string().to_lowercase();
        let id = format!("m{}", &drawn[drawn.len() - RANDOM_LENGTH..]);
        if !ids.contains(&id) {
            return Ok(id);
        }
    }
}

/// Move an item to the month's forgotten file, noting why, when and from
/// where, so it can be put back.
pub fn forget(
    dir: &Path,
    rel: impl AsRef<Path>,
    id: &str,
    reason: &str,
    on: NaiveDate,
) -> anyhow::Result<Entry> {
    let rel = rel.as_ref();
    let layer = Layer::of(rel).with_context(|| format!("not a memory file: {}", rel.display()))?;
    anyhow::ensure!(
        layer != Layer::Forgotten,
        "{} is already forgotten",
        rel.display()
    );
    let mut doc = read_doc(dir, rel)?;
    let mut entry = doc
        .remove(id)
        .with_context(|| format!("no item {id} in {}", rel.display()))?;
    entry.set("forgot", on.format("%Y-%m-%d").to_string())?;
    entry.set("reason", reason)?;
    entry.set("was", slash(rel))?;

    let forgotten = forgotten_path(&on.format("%Y-%m").to_string());
    let mut target = doc_to_add_to(dir, &forgotten)?;
    target.push(entry.clone())?;
    target.save(&dir.join(forgotten))?;
    doc.save(&dir.join(rel))?;
    Ok(entry)
}

/// The months that have a forgotten file (`YYYY-MM`), oldest first.
pub fn forgotten_months(dir: &Path) -> anyhow::Result<Vec<String>> {
    Ok(markdown_files(dir, Path::new(FORGOTTEN))?
        .iter()
        .map(|path| stem(path))
        .filter(|month| is_month(month))
        .collect())
}

pub fn forgotten(dir: &Path, month: &str) -> anyhow::Result<Vec<Entry>> {
    anyhow::ensure!(is_month(month), "not a month: {month:?}");
    entries(dir, forgotten_path(month))
}

fn forgotten_path(month: &str) -> PathBuf {
    Path::new(FORGOTTEN).join(format!("{month}.md"))
}

fn is_month(month: &str) -> bool {
    NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").is_ok() && month.len() == 7
}

/// The text of a layer file, given relative to the directory.
pub fn read_file(dir: &Path, rel: impl AsRef<Path>) -> anyhow::Result<String> {
    let rel = rel.as_ref();
    anyhow::ensure!(
        Layer::of(rel).is_some(),
        "not a memory file: {}",
        rel.display()
    );
    std::fs::read_to_string(dir.join(rel)).with_context(|| format!("reading {}", rel.display()))
}

/// Replace a whole layer file with what the owner wrote. Only Markdown files
/// in a layer are accepted, and only UTF-8 text.
pub fn write_file(dir: &Path, rel: impl AsRef<Path>, content: &[u8]) -> anyhow::Result<()> {
    let rel = rel.as_ref();
    anyhow::ensure!(
        Layer::of(rel).is_some(),
        "not a memory file: {}",
        rel.display()
    );
    let text = std::str::from_utf8(content)
        .with_context(|| format!("{} is not UTF-8 text", rel.display()))?;
    anyhow::ensure!(!text.contains('\0'), "{} is not text", rel.display());
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, text).with_context(|| format!("writing {}", rel.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_missing_directory_gets_every_layer() {
        let tmp = TempDir::new("ensure-new");
        let dir = tmp.path().join("memory");

        let made = ensure(&dir).unwrap();

        assert!(made.contains(&PathBuf::from(CORE)));
        assert!(made.contains(&PathBuf::from("long/semantic/.gitkeep")));
        assert!(!dir.join(".git").exists(), "the notebook keeps the history");
        for path in [
            "core.md",
            "pending.md",
            "episodic",
            "working",
            "forgotten",
            "README.md",
            "long/semantic",
            "long/procedural",
            "long/prospective",
        ] {
            assert!(dir.join(path).exists(), "{path}");
        }
    }

    #[test]
    fn ensuring_again_makes_only_what_went_missing() {
        let tmp = TempDir::new("ensure-twice");
        ensure(tmp.path()).unwrap();
        std::fs::write(tmp.path().join(CORE), "# 心得\n\n- x\n").unwrap();
        std::fs::remove_dir_all(tmp.path().join("long/prospective")).unwrap();

        let made = ensure(tmp.path()).unwrap();

        assert_eq!(made, vec![PathBuf::from("long/prospective/.gitkeep")]);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join(CORE)).unwrap(),
            "# 心得\n\n- x\n"
        );
    }

    #[test]
    fn the_layer_of_a_file_follows_its_place() {
        assert_eq!(Layer::of("core.md"), Some(Layer::Core));
        assert_eq!(Layer::of("pending.md"), Some(Layer::Pending));
        assert_eq!(Layer::of("episodic/2026-11-21.md"), Some(Layer::Episodic));
        assert_eq!(Layer::of("working/x.md"), Some(Layer::Working));
        assert_eq!(
            Layer::of("long/semantic/tanaka.md"),
            Some(Layer::Long(LongGroup::Semantic))
        );
        assert_eq!(Layer::of("forgotten/2026-11.md"), Some(Layer::Forgotten));
        for outside in [
            "other.md",
            "long/other/x.md",
            "long/semantic/x.txt",
            "../core.md",
            "/core.md",
            "working/.hidden.md",
            ".git/config.md",
        ] {
            assert_eq!(Layer::of(outside), None, "{outside}");
        }
    }

    #[test]
    fn episodic_items_are_appended_to_the_days_file_with_time_and_state() {
        let tmp = TempDir::new("episodic");
        ensure(tmp.path()).unwrap();
        let at = date(2026, 11, 21).and_hms_opt(9, 5, 0).unwrap();

        let first =
            append_episode(tmp.path(), at, "午後の会議を好む", EpisodeState::Waiting).unwrap();
        let second = append_episode(tmp.path(), at, "契約は3月まで", EpisodeState::Record).unwrap();

        assert_ne!(first.id(), second.id());
        assert_eq!(
            episodic_dates(tmp.path()).unwrap(),
            vec![date(2026, 11, 21)]
        );
        let items = episodes(tmp.path(), date(2026, 11, 21)).unwrap();
        assert_eq!(items, vec![first, second]);
        assert_eq!(items[0].body(), "午後の会議を好む");
        assert_eq!(items[1].state(), Some(EpisodeState::Record));
        assert!(
            read_file(tmp.path(), "episodic/2026-11-21.md")
                .unwrap()
                .starts_with("# 2026-11-21\n\n- 09:05 午後の会議を好む {id: ")
        );
        assert!(episodes(tmp.path(), date(2026, 11, 20)).unwrap().is_empty());
    }

    #[test]
    fn copies_written_apart_give_their_memories_different_ids() {
        let tmp = TempDir::new("fresh-id");
        ensure(tmp.path()).unwrap();

        let ids: BTreeSet<_> = (0..100).map(|_| fresh_id(tmp.path()).unwrap()).collect();

        assert_eq!(ids.len(), 100);
    }

    #[test]
    fn forgetting_moves_an_item_to_the_months_file_with_why_when_and_where() {
        let tmp = TempDir::new("forget");
        ensure(tmp.path()).unwrap();
        write_file(
            tmp.path(),
            CORE,
            "# 心得\n\n- 古い決まり {id: m1, src: said}\n- 残る {id: m2}\n".as_bytes(),
        )
        .unwrap();

        let moved = forget(tmp.path(), CORE, "m1", "superseded", date(2026, 11, 21)).unwrap();

        assert_eq!(moved.get("forgot"), Some("2026-11-21"));
        assert_eq!(moved.get("reason"), Some("superseded"));
        assert_eq!(moved.get("was"), Some("core.md"));
        assert_eq!(moved.src(), Some(Src::Said));
        assert_eq!(forgotten_months(tmp.path()).unwrap(), vec!["2026-11"]);
        assert_eq!(forgotten(tmp.path(), "2026-11").unwrap(), vec![moved]);
        let core: Vec<_> = core_entries(tmp.path()).unwrap();
        assert_eq!(core.len(), 1);
        assert_eq!(core[0].id(), Some("m2"));
    }

    #[test]
    fn forgetting_a_missing_item_changes_nothing() {
        let tmp = TempDir::new("forget-missing");
        ensure(tmp.path()).unwrap();

        assert!(forget(tmp.path(), CORE, "m9", "x", date(2026, 11, 21)).is_err());
        assert!(forgotten_months(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn the_owner_may_write_only_utf8_text_to_layer_files() {
        let tmp = TempDir::new("owner-write");
        ensure(tmp.path()).unwrap();

        assert!(write_file(tmp.path(), CORE, &[0xff, 0xfe]).is_err());
        assert!(write_file(tmp.path(), CORE, b"a\0b").is_err());
        assert!(write_file(tmp.path(), "../outside.md", b"x").is_err());
        assert!(write_file(tmp.path(), "other.md", b"x").is_err());
        assert!(write_file(tmp.path(), "long/semantic/x.txt", b"x").is_err());

        write_file(tmp.path(), CORE, "# 心得\n\n- 新しい {id: m1}\n".as_bytes()).unwrap();
        assert_eq!(core_entries(tmp.path()).unwrap()[0].text(), "新しい");
    }
}
