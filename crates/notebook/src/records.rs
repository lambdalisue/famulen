//! Records: one Markdown file per record in `records/<kind>/<id>.md`. The
//! frontmatter holds metadata, the body holds what the record says, as
//! Markdown structure. Each kind is a directory, and the kind's README
//! declares how its bodies are laid out.

mod kind;
mod names;

pub use kind::*;
pub use names::*;

use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use anyhow::Context as _;

use crate::body::Body;
use crate::index::Index;
use crate::link::found;
use crate::{Notebook, Page, README, RECORDS, Signature, link_target};

/// The metadata every kind of record has, in its frontmatter.
pub const METADATA: &[&str] = &[
    "id",
    "labels",
    "workgroup",
    "source",
    "created",
    "updated",
    "google",
    "url",
    "obsolete",
    "obsolete_reason",
    "moved",
];

/// How much of an id its code takes at least.
const CODE_LENGTH: usize = 6;

/// The mark of a record that may no longer be true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obsolete {
    /// The day it was marked, `YYYY-MM-DD`.
    pub on: String,
    /// Why it looks out of date.
    pub reason: String,
}

/// One record: its kind, its id (the file name), its metadata and its body.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub kind: String,
    pub id: String,
    /// The frontmatter. Its own body is not used.
    pub meta: Page,
    pub body: Body,
}

impl Record {
    /// Read a record from its file's text.
    pub fn parse(kind: &Kind, id: &str, text: &str) -> anyhow::Result<Self> {
        let mut meta = Page::parse(text)?;
        let body = Body::parse(meta.body(), &kind.layout)?;
        meta.set_body("");
        Ok(Self {
            kind: kind.name.clone(),
            id: id.to_owned(),
            meta,
            body,
        })
    }

    pub fn render(&self) -> String {
        let mut page = self.meta.clone();
        page.set_body(self.body.text());
        page.render()
    }

    /// The title, or the id when there is none.
    pub fn title(&self) -> String {
        self.body.title().unwrap_or(&self.id).to_owned()
    }

    pub fn notes(&self) -> String {
        self.body.notes()
    }

    pub fn workgroups(&self) -> Vec<String> {
        self.meta.list("workgroup")
    }

    pub fn labels(&self) -> Vec<String> {
        self.meta.list("labels")
    }

    /// The mark that it may no longer be true, when it carries one.
    pub fn obsolete(&self) -> Option<Obsolete> {
        Some(Obsolete {
            on: self.meta.text("obsolete")?,
            reason: self.meta.text("obsolete_reason").unwrap_or_default(),
        })
    }

    /// Mark it as doubtful, or take the mark off.
    pub fn set_obsolete(&mut self, mark: Option<&Obsolete>) {
        self.meta.set_opt("obsolete", mark.map(|m| m.on.as_str()));
        self.meta.set_opt(
            "obsolete_reason",
            mark.map(|m| m.reason.as_str()).filter(|r| !r.is_empty()),
        );
    }

    /// The kind and id of the record it was made into.
    pub fn moved(&self) -> Option<(String, String)> {
        let target = self.meta.text("moved")?;
        record_of(target.trim()).map(|(k, i)| (k.to_owned(), i.to_owned()))
    }

    /// Where it is in the notebook, without `.md`: what a wiki link to it
    /// names.
    pub fn target(&self) -> String {
        format!("{RECORDS}/{}/{}", self.kind, self.id)
    }

    /// Where it is in the notebook.
    pub fn rel(&self) -> String {
        format!("{}.md", self.target())
    }

    /// Every link it holds, once for each field or section it stands in, which
    /// is nothing for free text outside them.
    pub fn links(&self) -> Vec<(String, String)> {
        let body = &self.body;
        let place = |at: usize| -> &str {
            let field = body.fields().iter().find(|f| f.span.contains(&at));
            let section = body.sections().iter().find(|s| s.content.contains(&at));
            match (field.and_then(|f| f.field()), section) {
                (Some((name, _)), _) => name,
                (None, Some(section)) => &section.name,
                (None, None) => "",
            }
        };
        let mut all = Vec::new();
        for link in found(body.text()) {
            let one = (place(link.at).to_owned(), link.target.to_owned());
            if !all.contains(&one) {
                all.push(one);
            }
        }
        all
    }

    /// A wiki link to it, with its title.
    pub fn link(&self) -> String {
        titled_link(&self.kind, &self.id, &self.title())
    }
}

/// A record id is its file name, so it may not leave its directory.
pub fn check_id(id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !id.trim().is_empty()
            && !id.starts_with('.')
            && !id.contains(['/', '\\', '\0'])
            && id.trim() == id
            && id != README.trim_end_matches(".md"),
        "invalid record id: {id:?}"
    );
    Ok(())
}

/// Whether an id is a ULID in lower case.
pub fn is_ulid(id: &str) -> bool {
    id.len() == 26 && id.to_lowercase() == id && ulid::Ulid::from_string(&id.to_uppercase()).is_ok()
}

/// A new id. The random part is drawn anew each time, so that the codes of
/// records made together differ.
pub fn fresh_id() -> String {
    ulid::Ulid::generate().to_string().to_lowercase()
}

fn check_kind(kind: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !kind.is_empty() && !kind.starts_with('.') && !kind.contains(['/', '\\', '\0']),
        "invalid kind of record: {kind:?}"
    );
    Ok(())
}

/// A code or id that fits several records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguous {
    pub text: String,
    /// The code and title of each.
    pub candidates: Vec<(String, String)>,
}

impl std::fmt::Display for Ambiguous {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} could be", self.text)?;
        for (code, title) in &self.candidates {
            write!(f, " {code} ({title})")?;
        }
        Ok(())
    }
}

impl std::error::Error for Ambiguous {}

/// The records of a notebook.
#[derive(Debug, Clone)]
pub struct Records {
    notebook: Notebook,
}

impl Records {
    pub fn new(notebook: Notebook) -> Self {
        Self { notebook }
    }

    pub fn notebook(&self) -> &Notebook {
        &self.notebook
    }

    fn dir(&self, kind: &str) -> PathBuf {
        self.notebook.records_dir().join(kind)
    }

    pub fn path(&self, kind: &str, id: &str) -> PathBuf {
        self.dir(kind).join(format!("{id}.md"))
    }

    /// The name of every kind: each directory under `records/`.
    pub fn kind_names(&self) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::new();
        let read = match std::fs::read_dir(self.notebook.records_dir()) {
            Ok(read) => read,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(names),
            Err(e) => return Err(e.into()),
        };
        for item in read {
            let item = item?;
            let name = item.file_name().to_string_lossy().into_owned();
            if item.file_type()?.is_dir() && check_kind(&name).is_ok() {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    /// A kind as its README declares it.
    pub fn kind(&self, name: &str) -> anyhow::Result<Kind> {
        check_kind(name)?;
        let readme = self.dir(name).join(README);
        let page = if readme.is_file() {
            Some(Page::load(&readme)?)
        } else {
            None
        };
        Kind::declared(name, page.as_ref()).with_context(|| format!("reading {}", readme.display()))
    }

    /// Every record file of a kind, by id.
    pub fn files(&self, kind: &str) -> anyhow::Result<BTreeMap<String, PathBuf>> {
        check_kind(kind)?;
        let read = match std::fs::read_dir(self.dir(kind)) {
            Ok(read) => read,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(e.into()),
        };
        let mut files = BTreeMap::new();
        for item in read {
            let path = item?.path();
            if path.extension().is_none_or(|e| e != "md") || !path.is_file() {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if check_id(id).is_ok() {
                files.insert(id.to_owned(), path);
            }
        }
        Ok(files)
    }

    pub fn get(&self, kind: &str, id: &str) -> anyhow::Result<Option<Record>> {
        check_kind(kind)?;
        check_id(id)?;
        let path = self.path(kind, id);
        if !path.is_file() {
            forget(&path);
            return Ok(None);
        }
        load(&self.kind(kind)?, id, &path).map(Some)
    }

    /// Every record of a kind that can be read, by id.
    pub fn list(&self, kind: &str) -> anyhow::Result<Vec<Record>> {
        let files = self.files(kind)?;
        forget_gone(&self.dir(kind), &files);
        let kind = self.kind(kind)?;
        let mut records = Vec::new();
        for (id, path) in files {
            match load(&kind, &id, &path) {
                Ok(record) => records.push(record),
                Err(error) => {
                    tracing::debug!(%error, path = %path.display(), "a record file is not readable")
                }
            }
        }
        Ok(records)
    }

    /// A record not yet written.
    pub fn draft(&self, kind: &str, title: &str) -> anyhow::Result<Record> {
        let layout = self.kind(kind)?.layout;
        let id = fresh_id();
        let mut meta = Page::new();
        meta.set("id", id.as_str());
        let mut body = Body::parse("", &layout)?;
        body.set_title(title)?;
        Ok(Record {
            kind: kind.to_owned(),
            id,
            meta,
            body,
        })
    }

    /// What people call a record: its kind's word and as much of the id's
    /// end as tells it from the kind's other records.
    pub fn code(&self, kind: &str, id: &str) -> anyhow::Result<String> {
        let ids: Vec<String> = self.files(kind)?.into_keys().collect();
        Ok(format!("{}-{}", self.kind(kind)?.code, tail(id, &ids)))
    }

    /// The codes of every record of a kind, by id.
    pub fn codes(&self, kind: &str) -> anyhow::Result<BTreeMap<String, String>> {
        let ids: Vec<String> = self.files(kind)?.into_keys().collect();
        let word = self.kind(kind)?.code;
        Ok(ids
            .iter()
            .map(|id| (id.clone(), format!("{word}-{}", tail(id, &ids))))
            .collect())
    }

    /// The record of a kind that an id, a code or a wiki link names.
    pub fn resolve(&self, kind: &str, text: &str) -> anyhow::Result<Option<String>> {
        let text = link_target(text);
        let text = match record_of(text) {
            Some((k, id)) if k == kind => id,
            Some(_) => return Ok(None),
            None => text.trim(),
        };
        let files = self.files(kind)?;
        if files.contains_key(text) {
            return Ok(Some(text.to_owned()));
        }
        let word = self.kind(kind)?.code;
        let lower = text.to_lowercase();
        let end = match lower.rsplit_once('-') {
            Some((w, end)) if w == word => end,
            Some(_) => return Ok(None),
            None => &lower,
        };
        if end.len() < CODE_LENGTH {
            return Ok(None);
        }
        let hits: Vec<&String> = files.keys().filter(|id| id.ends_with(end)).collect();
        match &hits[..] {
            [] => Ok(None),
            [one] => Ok(Some((*one).clone())),
            many => Err(Ambiguous {
                text: text.to_owned(),
                candidates: many
                    .iter()
                    .map(|id| {
                        let title = self.get(kind, id)?.map(|r| r.title()).unwrap_or_default();
                        Ok((self.code(kind, id)?, title))
                    })
                    .collect::<anyhow::Result<_>>()?,
            }
            .into()),
        }
    }

    /// Write a record without committing it. What would make the record
    /// wrong by the notebook's rules is refused; what was wrong before and
    /// is not touched is not.
    pub fn save(&self, record: &Record) -> anyhow::Result<()> {
        check_kind(&record.kind)?;
        check_id(&record.id)?;
        let path = self.path(&record.kind, &record.id);
        let before = std::fs::read_to_string(&path).unwrap_or_default();
        let mut record = Names::new(self.clone()).record(
            &self.kind(&record.kind)?,
            record,
            &before,
            Unsure::Refuse,
        )?;
        let now = now();
        if record.meta.text("created").is_none() {
            record.meta.set("created", now.as_str());
        }
        if record.meta.text("id").as_deref() != Some(&record.id) {
            record.meta.set("id", record.id.as_str());
        }
        record.meta.set("updated", now);
        let text = record.render();
        crate::check::refuse_new_errors(self, &record, &text)?;
        if path.exists() {
            std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Another writer may be making a record of the same id.
        let mut file = std::fs::File::create_new(&path)
            .with_context(|| format!("making {}", path.display()))?;
        file.write_all(text.as_bytes())?;
        Ok(())
    }

    /// Refuse what [`Self::save`] would refuse, writing nothing.
    pub fn verify(&self, record: &Record) -> anyhow::Result<()> {
        crate::check::refuse_new_errors(self, record, &record.render())
    }

    /// Write a record and commit it. When its title changed, the links that
    /// carried the old one are brought along.
    pub fn write(
        &self,
        signature: &Signature,
        record: &Record,
        subject: &str,
    ) -> anyhow::Result<()> {
        let _hold = self.notebook.hold()?;
        let before = self
            .get(&record.kind, &record.id)
            .ok()
            .flatten()
            .map(|r| r.title());
        self.save(record)?;
        let mut touched = vec![record.rel()];
        if let Some(before) = before.filter(|b| *b != record.title()) {
            touched.extend(self.retitle_links(record, &before)?);
        }
        self.notebook.commit(signature, subject, &touched)?;
        Ok(())
    }

    /// Give the links to a record its new title where they carry the old.
    fn retitle_links(&self, record: &Record, before: &str) -> anyhow::Result<Vec<String>> {
        let old = titled_link(&record.kind, &record.id, before);
        self.replace_everywhere(&[(old, record.link())])
    }

    /// Lead every link to one record to another, in records and memory alike,
    /// and commit it.
    pub fn relink(
        &self,
        signature: &Signature,
        kind: &str,
        from: impl AsRef<str>,
        to: impl AsRef<str>,
        subject: &str,
    ) -> anyhow::Result<()> {
        let (from, to) = (from.as_ref(), to.as_ref());
        check_kind(kind)?;
        check_id(from)?;
        check_id(to)?;
        let _hold = self.notebook.hold()?;
        let replaced = ["]", "|", "#", ".md]", ".md|", ".md#"].map(|end| {
            (
                format!("[[{RECORDS}/{kind}/{from}{end}"),
                format!("[[{RECORDS}/{kind}/{to}{end}"),
            )
        });
        let touched = self.replace_everywhere(&replaced)?;
        if !touched.is_empty() {
            self.notebook.commit(signature, subject, &touched)?;
        }
        Ok(())
    }

    /// The files that were changed.
    fn replace_everywhere(&self, replaced: &[(String, String)]) -> anyhow::Result<Vec<String>> {
        let mut touched = Vec::new();
        for rel in crate::check::markdown_files(&self.notebook)? {
            let path = self.notebook.root().join(&rel);
            let Ok(before) = std::fs::read_to_string(&path) else {
                continue;
            };
            let after = replaced
                .iter()
                .fold(before.clone(), |text, (old, new)| text.replace(old, new));
            if after != before {
                std::fs::write(&path, after)?;
                touched.push(rel);
            }
        }
        Ok(touched)
    }

    /// Remove a record and commit its removal. A missing record is no change.
    pub fn remove(
        &self,
        signature: &Signature,
        kind: &str,
        id: &str,
        subject: &str,
    ) -> anyhow::Result<bool> {
        check_kind(kind)?;
        check_id(id)?;
        let _hold = self.notebook.hold()?;
        match std::fs::remove_file(self.path(kind, id)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        }
        self.notebook
            .commit(signature, subject, &[format!("{RECORDS}/{kind}/{id}.md")])?;
        Ok(true)
    }

    /// The records that link to one, with what they link from.
    pub fn backlinks(&self, kind: &str, id: &str) -> anyhow::Result<Vec<Backlink>> {
        Ok(Index::open(&self.notebook)?.backlinks(kind, id)?.records)
    }
}

/// A record that links to another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    pub kind: String,
    pub id: String,
    pub title: String,
    /// The field or section the link is in; empty for free text.
    pub from: String,
}

/// A record as it was last parsed, with the text and kind it was parsed from.
struct Parsed {
    text: String,
    kind: Kind,
    record: Record,
}

/// Records as they were last parsed, by path. Parsing is what reading a whole
/// kind costs, and the background chores and the console read whole kinds
/// over and over while few records change. The text is compared rather than
/// the file's time and size, as a file can change without either moving, and
/// rather than a hash of it, as hashing every file on every read costs more
/// than comparing it. The lock is held only to take an entry out or put one in.
static PARSED: LazyLock<Mutex<HashMap<PathBuf, Arc<Parsed>>>> = LazyLock::new(Default::default);

fn parsed_all() -> MutexGuard<'static, HashMap<PathBuf, Arc<Parsed>>> {
    PARSED.lock().unwrap_or_else(PoisonError::into_inner)
}

fn load(kind: &Kind, id: &str, path: &Path) -> anyhow::Result<Record> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let had = parsed_all().get(path).cloned();
    if let Some(p) = had
        && p.text == text
        && p.kind == *kind
        && p.record.id == id
    {
        return Ok(p.record.clone());
    }
    #[cfg(test)]
    PARSES.with(|n| n.set(n.get() + 1));
    let record =
        Record::parse(kind, id, &text).with_context(|| format!("reading {}", path.display()))?;
    let parsed = Parsed {
        text,
        kind: kind.clone(),
        record: record.clone(),
    };
    parsed_all().insert(path.to_owned(), Arc::new(parsed));
    Ok(record)
}

/// Let go of what was parsed from a file that is gone.
fn forget(path: &Path) {
    parsed_all().remove(path);
}

/// Let go of what was parsed from files of a kind that are gone.
fn forget_gone(dir: &Path, files: &BTreeMap<String, PathBuf>) {
    parsed_all().retain(|path, _| {
        path.parent() != Some(dir)
            || path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|id| files.get(id) == Some(path))
    });
}

#[cfg(test)]
thread_local! {
    static PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many records this thread has parsed.
#[cfg(test)]
fn parsed() -> usize {
    PARSES.with(std::cell::Cell::get)
}

fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

fn tail<'a>(id: &'a str, among: &[String]) -> &'a str {
    let longest = id.len();
    (CODE_LENGTH.min(longest)..=longest)
        .filter_map(|n| id.get(longest - n..))
        .find(|end| !among.iter().any(|o| o != id && o.ends_with(end)))
        .unwrap_or(id)
}

/// The path of a record relative to the notebook from a wiki link target,
/// when it names one: `records/parties/p1` gives `("parties", "p1")`.
pub fn record_of(target: &str) -> Option<(&str, &str)> {
    let rest = target.strip_prefix(RECORDS)?.strip_prefix('/')?;
    let (kind, id) = rest.split_once('/')?;
    (!id.contains('/')).then_some((kind, id))
}

/// A record id from a field that may be a wiki link or a bare id.
pub fn id_of(text: &str) -> String {
    let target = link_target(text);
    target.rsplit('/').next().unwrap_or(target).to_owned()
}

/// A wiki link to a record.
pub fn link(kind: &str, id: &str) -> String {
    format!("[[{RECORDS}/{kind}/{id}]]")
}

/// A wiki link to a record, with its title.
pub fn titled_link(kind: &str, id: &str, title: &str) -> String {
    let title = title.replace(['[', ']', '|', '\n'], " ");
    format!("[[{RECORDS}/{kind}/{id}|{}]]", title.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log;
    use crate::testing::{TempDir, me, notebook};

    fn records(name: &str) -> (TempDir, Records) {
        let (tmp, notebook) = notebook(name);
        (tmp, Records::new(notebook))
    }

    fn task(records: &Records, title: &str) -> Record {
        let mut record = records.draft("tasks", title).unwrap();
        record.body.set("Status", "pending").unwrap();
        record.body.set_notes("覚え書き").unwrap();
        record
    }

    #[test]
    fn a_standard_kind_has_its_standard_fields_whatever_its_readme_leaves_out() {
        let (_tmp, r) = records("records-kinds-older");
        std::fs::write(
            r.notebook().records_dir().join("tasks/README.md"),
            "---\ncode: todo\nfields:\n  Status: [open, done]\n  Where: text\n---\n",
        )
        .unwrap();

        let tasks = r.kind("tasks").unwrap();

        assert_eq!(tasks.code, "todo");
        assert!(matches!(
            tasks.field("Status"),
            Some((_, FieldType::OneOf(c))) if c.contains(&"open".to_owned())
        ));
        assert!(tasks.field("Where").is_some());
        assert!(tasks.field("Due").is_some());
        assert!(matches!(
            tasks.section("delegation"),
            Some((_, SectionType::Fields(_)))
        ));
        assert!(tasks.layout.sections.contains(&"Delegation".to_owned()));
    }

    #[test]
    fn the_standard_kinds_declare_their_fields_sections_and_code_words() {
        let (_tmp, r) = records("records-kinds");

        assert_eq!(
            r.kind_names().unwrap(),
            [
                "calendars",
                "contracts",
                "events",
                "holdings",
                "memos",
                "parties",
                "routines",
                "tasks"
            ]
        );
        let tasks = r.kind("tasks").unwrap();
        assert_eq!(tasks.code, "task");
        assert!(matches!(
            tasks.field("status"),
            Some((_, FieldType::OneOf(c))) if c.contains(&"in_progress".to_owned())
        ));
        assert!(matches!(
            r.kind("holdings").unwrap().section("acquired"),
            Some((_, SectionType::Fields(_)))
        ));
        assert!(matches!(
            r.kind("events").unwrap().section("Attendees"),
            Some((_, SectionType::Items(_)))
        ));
        assert!(r.kind("memos").unwrap().layout.free);
    }

    #[test]
    fn a_kind_added_as_a_directory_is_used_without_a_declaration() {
        let (tmp, r) = records("records-new-kind");
        std::fs::create_dir_all(tmp.path().join("records/machines")).unwrap();

        let machine = r.draft("machines", "MacBook").unwrap();
        r.write(&me(), &machine, "機器を足した").unwrap();

        let read = r.get("machines", &machine.id).unwrap().unwrap();
        assert_eq!(read.title(), "MacBook");
        assert!(
            r.code("machines", &machine.id)
                .unwrap()
                .starts_with("machines-")
        );
    }

    #[test]
    fn a_record_is_one_file_named_by_its_id_and_committed_in_the_signers_name() {
        let (tmp, r) = records("records-write");
        let record = task(&r, "見積もりの返事");
        assert!(is_ulid(&record.id), "{}", record.id);

        r.write(&me(), &record, "タスクを足した").unwrap();

        assert!(tmp.path().join(record.rel()).is_file());
        let commit = &log(tmp.path(), 1).unwrap()[0];
        assert_eq!(commit.author, me().author);
        let read = r.get("tasks", &record.id).unwrap().unwrap();
        assert_eq!(
            read.meta.fields().next().map(|(k, _)| k.to_owned()),
            Some("id".into())
        );
        assert!(read.meta.text("created").is_some());
        assert_eq!(read.title(), "見積もりの返事");
        assert_eq!(read.body.value("Status").as_deref(), Some("pending"));
        assert_eq!(read.notes(), "覚え書き");
    }

    #[test]
    fn ids_made_in_the_same_moment_differ_to_their_ends() {
        let ends: std::collections::BTreeSet<String> =
            (0..200).map(|_| fresh_id()[18..].to_owned()).collect();
        assert_eq!(ends.len(), 200);
    }

    #[test]
    fn a_code_is_as_long_as_it_takes_and_one_that_fits_several_names_them() {
        let (tmp, r) = records("records-codes");
        let write = |id: &str, title: &str| {
            std::fs::write(
                tmp.path().join(format!("records/tasks/{id}.md")),
                format!("---\nid: {id}\n---\n\n# {title}\n"),
            )
            .unwrap();
        };
        write("01kat3v8q4zj6m2x9r5c7w1hbn", "見積もり");
        write("01kat3v8q4zj6m2x9r5cdw1hbn", "請求");
        write("01kat3v8q4zj6m2x9r5c7w1hbz", "別");

        assert_eq!(
            r.code("tasks", "01kat3v8q4zj6m2x9r5c7w1hbz").unwrap(),
            "task-7w1hbz"
        );
        assert_eq!(
            r.code("tasks", "01kat3v8q4zj6m2x9r5c7w1hbn").unwrap(),
            "task-7w1hbn"
        );
        assert_eq!(
            r.resolve("tasks", "TASK-7w1hbz").unwrap().as_deref(),
            Some("01kat3v8q4zj6m2x9r5c7w1hbz")
        );
        assert_eq!(r.resolve("tasks", "task-zzzzzz").unwrap(), None);
        assert_eq!(r.resolve("tasks", "memo-7w1hbz").unwrap(), None);
        write("01kat3v8q4zj6m2x9r5d7w1hbn", "三つ目");
        assert_eq!(
            r.code("tasks", "01kat3v8q4zj6m2x9r5c7w1hbn").unwrap(),
            "task-c7w1hbn"
        );
        let error = r.resolve("tasks", "task-7w1hbn").unwrap_err();
        assert_eq!(
            error.downcast_ref::<Ambiguous>().unwrap().candidates.len(),
            2
        );
    }

    #[test]
    fn what_the_kind_does_not_declare_survives_a_rewrite() {
        let (tmp, r) = records("records-unknown");
        let record = task(&r, "請求書");
        r.write(&me(), &record, "足した").unwrap();
        let path = tmp.path().join(record.rel());
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("- Status: pending\n", "- Status: pending\n- 気分: よい\n")
            .replace("---\n\n#", "mine: x\n---\n\n#");
        std::fs::write(&path, text).unwrap();

        let mut read = r.get("tasks", &record.id).unwrap().unwrap();
        read.body.set("Status", "done").unwrap();
        r.write(&me(), &read, "済にした").unwrap();

        let again = r.get("tasks", &record.id).unwrap().unwrap();
        assert_eq!(again.body.value("気分").as_deref(), Some("よい"));
        assert_eq!(again.meta.text("mine").as_deref(), Some("x"));
        assert_eq!(again.body.value("Status").as_deref(), Some("done"));
    }

    #[test]
    fn a_write_that_would_be_wrong_is_refused_but_not_for_what_was_wrong_before() {
        let (tmp, r) = records("records-refuse");
        let mut record = task(&r, "x");
        r.write(&me(), &record, "足した").unwrap();
        record.body.set("Due", "tomorrow").unwrap();

        let error = r.write(&me(), &record, "直した").unwrap_err();
        assert!(format!("{error:#}").contains("Due"), "{error:#}");

        let path = tmp.path().join(record.rel());
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("pending", "someday")).unwrap();
        let mut read = r.get("tasks", &record.id).unwrap().unwrap();
        read.body.set("Priority", "high").unwrap();
        r.write(&me(), &read, "直した").unwrap();
    }

    #[test]
    fn a_mark_of_doubt_is_read_back_and_taken_off_leaving_the_rest_as_it_was() {
        let (tmp, r) = records("records-obsolete");
        for kind in ["memos", "contracts"] {
            let record = r.draft(kind, "回線").unwrap();
            r.write(&me(), &record, "足した").unwrap();
            let before = r.get(kind, &record.id).unwrap().unwrap();
            let mark = Obsolete {
                on: "2026-11-24".into(),
                reason: "料金: 2 年前のまま".into(),
            };

            let mut marked = before.clone();
            marked.set_obsolete(Some(&mark));
            r.write(&me(), &marked, "印を付けた").unwrap();

            let mut read = r.get(kind, &record.id).unwrap().unwrap();
            assert_eq!(read.obsolete(), Some(mark));
            assert_eq!(read.body, before.body);
            let findings = crate::check::check(r.notebook()).unwrap();
            assert_eq!(findings, [], "{kind}");
            read.set_obsolete(None);
            assert_eq!(read.obsolete(), None);
            let text = read.render();
            assert!(!text.contains("obsolete"), "{text}");
        }
        drop(tmp);
    }

    #[test]
    fn a_record_says_what_it_was_made_into() {
        let (_tmp, r) = records("records-moved");
        let mut memo = r.draft("memos", "電話する").unwrap();
        assert_eq!(memo.moved(), None);
        memo.meta
            .set("moved", "records/tasks/01kat3v8q4zj6m2x9r5c7w1hbn");
        assert_eq!(
            memo.moved(),
            Some(("tasks".into(), "01kat3v8q4zj6m2x9r5c7w1hbn".into()))
        );
    }

    #[test]
    fn verifying_refuses_what_writing_would_and_writes_nothing() {
        let (_tmp, r) = records("records-verify");
        let mut record = task(&r, "x");
        r.verify(&record).unwrap();
        record.body.set("Due", "tomorrow").unwrap();
        assert!(r.verify(&record).is_err());
        assert!(r.list("tasks").unwrap().is_empty());
    }

    #[test]
    fn a_new_title_goes_into_the_links_that_carried_the_old_one() {
        let (tmp, r) = records("records-retitle");
        let mut party = r.draft("parties", "田中").unwrap();
        r.write(&me(), &party, "足した").unwrap();
        let mut memo = r.draft("memos", "メモ").unwrap();
        memo.body
            .set_notes(&format!(
                "{} と [[{}|たなっち]]",
                party.link(),
                party.target()
            ))
            .unwrap();
        r.write(&me(), &memo, "足した").unwrap();

        party.body.set_title("田中太郎").unwrap();
        r.write(&me(), &party, "直した").unwrap();

        let text = std::fs::read_to_string(tmp.path().join(memo.rel())).unwrap();
        assert!(text.contains(&format!("[[{}|田中太郎]]", party.target())));
        assert!(text.contains("|たなっち]]"));
    }

    #[test]
    fn links_to_a_record_are_found_with_where_they_are() {
        let (_tmp, r) = records("records-backlinks");
        let a = task(&r, "先");
        r.write(&me(), &a, "足した").unwrap();
        let mut b = task(&r, "後");
        b.body.set("Blocked by", vec![a.link()]).unwrap();
        r.write(&me(), &b, "足した").unwrap();

        let found = r.backlinks("tasks", &a.id).unwrap();

        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].id.as_str(), found[0].from.as_str()),
            (b.id.as_str(), "Blocked by")
        );
    }

    #[test]
    fn an_id_or_kind_cannot_leave_its_directory() {
        let (_tmp, r) = records("records-ids");
        for bad in ["../x", "a/b", ".hidden", "", " t1", "README"] {
            assert!(r.get("tasks", bad).is_err(), "{bad:?}");
        }
        assert!(r.get("../memory", "core").is_err());
    }

    #[test]
    fn a_removed_record_is_gone_and_its_removal_committed() {
        let (tmp, r) = records("records-remove");
        let record = task(&r, "x");
        r.write(&me(), &record, "足した").unwrap();

        assert!(r.remove(&me(), "tasks", &record.id, "消した").unwrap());
        assert!(!r.remove(&me(), "tasks", &record.id, "消した").unwrap());

        assert!(r.get("tasks", &record.id).unwrap().is_none());
        assert_eq!(
            log(tmp.path(), 1).unwrap()[0].message.lines().next(),
            Some("消した")
        );
    }

    #[test]
    fn an_unreadable_record_is_left_out_of_the_list() {
        let (tmp, r) = records("records-broken");
        std::fs::write(
            tmp.path().join("records/tasks/broken.md"),
            "---\na: [\n---\n",
        )
        .unwrap();
        let record = task(&r, "読める");
        r.write(&me(), &record, "足した").unwrap();

        let ids: Vec<String> = r.list("tasks").unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, [record.id]);
    }

    #[test]
    fn a_record_read_again_unchanged_is_not_parsed_again() {
        let (_tmp, r) = records("records-parsed-once");
        let record = task(&r, "一度だけ");
        r.write(&me(), &record, "足した").unwrap();
        r.list("tasks").unwrap();

        let before = parsed();
        r.list("tasks").unwrap();
        r.get("tasks", &record.id).unwrap();

        assert_eq!(parsed(), before);
    }

    #[test]
    fn a_record_changed_on_disk_is_read_as_it_now_is_whatever_its_size_and_time() {
        let (tmp, r) = records("records-parsed-anew");
        let record = task(&r, "前の題");
        r.write(&me(), &record, "足した").unwrap();
        let path = tmp.path().join(record.rel());
        let file = std::fs::File::open(&path).unwrap();
        let written = file.metadata().unwrap().modified().unwrap();
        assert_eq!(r.list("tasks").unwrap()[0].title(), "前の題");

        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("前の題", "後の題")).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(written)
            .unwrap();

        assert_eq!(r.list("tasks").unwrap()[0].title(), "後の題");
        assert_eq!(
            r.get("tasks", &record.id).unwrap().unwrap().title(),
            "後の題"
        );
    }

    #[test]
    fn a_record_is_read_by_its_kind_as_the_readme_now_lays_it_out() {
        let (tmp, r) = records("records-parsed-relaid");
        let mut record = task(&r, "置き場");
        record.body.set_notes("## Where\n\n棚").unwrap();
        r.write(&me(), &record, "足した").unwrap();
        let sections = |r: &Records| r.list("tasks").unwrap()[0].body.sections().len();
        let had = sections(&r);

        let readme = tmp.path().join("records/tasks/README.md");
        let text = std::fs::read_to_string(&readme).unwrap();
        std::fs::write(
            &readme,
            text.replacen("sections:\n", "sections:\n  Where: {Shelf: text}\n", 1),
        )
        .unwrap();

        assert_ne!(sections(&r), had);
    }

    #[test]
    fn a_link_names_a_record_and_back() {
        assert_eq!(link("parties", "p1"), "[[records/parties/p1]]");
        assert_eq!(record_of("records/parties/p1"), Some(("parties", "p1")));
        assert_eq!(record_of("memory/core"), None);
        assert_eq!(id_of("[[records/parties/p1|田中さん]]"), "p1");
        assert_eq!(id_of("p1"), "p1");
    }
}
