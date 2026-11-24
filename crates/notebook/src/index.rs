//! The tools' own state in `.notebook/`: an index of every memory and record
//! to search, rebuilt from the files whenever one changed, and how often
//! each memory was used. Nothing here is committed; thrown away, only the uses
//! not yet written in the notebook are lost.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use chrono::NaiveDate;
use rusqlite::{Connection, TransactionBehavior, params};

use crate::link::links;
use crate::memory::{self, Layer};
use crate::records::{Backlink, Records};
use crate::search::{FullText, Hit, Query, Searcher};
use crate::{MEMORY, Notebook, RECORDS, slash};

const FILE: &str = "index.sqlite3";

/// Raised whenever the tables below change, so that an index made before is
/// read again from the files.
const SHAPE: i64 = 2;

/// What the files can give again. `uses` is not here: the files do not hold
/// it.
const DROP: &str = "
DROP TABLE IF EXISTS files;
DROP TABLE IF EXISTS items;
DROP TABLE IF EXISTS items_text;
DROP TABLE IF EXISTS links;
";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS files (
    path  TEXT PRIMARY KEY,
    stamp TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS items (
    rowid  INTEGER PRIMARY KEY,
    path   TEXT NOT NULL,
    scope  TEXT NOT NULL,
    id     TEXT NOT NULL,
    title  TEXT NOT NULL,
    body   TEXT NOT NULL,
    workgroup    TEXT NOT NULL,
    date   TEXT,
    fields TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS items_path ON items (path);
CREATE VIRTUAL TABLE IF NOT EXISTS items_text USING fts5 (title, body, tokenize = 'trigram');
CREATE TABLE IF NOT EXISTS links (
    path   TEXT NOT NULL,
    item   INTEGER NOT NULL,
    place  TEXT NOT NULL,
    target TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS links_target ON links (target);
CREATE INDEX IF NOT EXISTS links_path ON links (path);
CREATE TABLE IF NOT EXISTS uses (
    id   TEXT PRIMARY KEY,
    uses INTEGER NOT NULL,
    last TEXT NOT NULL
);
";

/// How much one memory has been used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Use {
    pub uses: u32,
    pub last: NaiveDate,
}

/// One thing the index holds: a memory item, a long-term memory, a working
/// memory or a record.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Item {
    /// Relative to the notebook.
    pub path: String,
    /// Where it is: `memory/<layer>`, `memory/long/<group>` or
    /// `records/<kind>`.
    pub scope: String,
    pub id: String,
    pub title: String,
    pub body: String,
    pub workgroup: Vec<String>,
    pub date: Option<String>,
    pub fields: BTreeMap<String, String>,
    /// The field or section each link is in, and what it leads to.
    pub links: Vec<(String, String)>,
}

/// What links to a record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Backlinks {
    pub records: Vec<Backlink>,
    /// Forgotten memories are left out.
    pub memories: Vec<MemoryLink>,
}

/// A memory that links to a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryLink {
    /// The file it is in, relative to the notebook.
    pub path: String,
    pub id: String,
    pub title: String,
}

/// The index of a notebook, in its `.notebook/`.
#[derive(Clone)]
pub struct Index {
    notebook: Notebook,
    conn: Arc<Mutex<Connection>>,
}

impl Index {
    pub fn open(notebook: &Notebook) -> anyhow::Result<Self> {
        let dir = notebook.state_dir();
        std::fs::create_dir_all(&dir)?;
        let mut conn = Connection::open(dir.join(FILE))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let shape: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if shape != SHAPE {
            tx.execute_batch(DROP)?;
            tx.pragma_update(None, "user_version", SHAPE)?;
        }
        tx.execute_batch(SCHEMA)?;
        tx.commit()?;
        Ok(Self {
            notebook: notebook.clone(),
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Every file the index reads, relative to the notebook.
    fn indexed_files(&self) -> anyhow::Result<Vec<String>> {
        let memory_dir = self.notebook.memory_dir();
        let mut files: Vec<String> = memory::entry_files(&memory_dir)?
            .into_iter()
            .chain(memory::working_files(&memory_dir)?)
            .map(|rel| format!("{MEMORY}/{}", slash(&rel)))
            .collect();
        let records = Records::new(self.notebook.clone());
        for kind in records.kind_names()? {
            for id in records.files(&kind)?.keys() {
                files.push(format!("{RECORDS}/{kind}/{id}.md"));
            }
        }
        Ok(files)
    }

    /// Bring the index up to date with the files: read what changed since
    /// it last read it, and drop what is gone.
    pub fn refresh(&self) -> anyhow::Result<()> {
        let files = self.indexed_files()?;
        let known: BTreeMap<String, String> = {
            let conn = self.conn();
            let mut stmt = conn.prepare("SELECT path, stamp FROM files")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?
        };
        let present: BTreeSet<&String> = files.iter().collect();
        for path in known.keys().filter(|p| !present.contains(p)) {
            self.forget(path)?;
        }
        for path in &files {
            let stamp = stamp(&self.notebook.root().join(path))?;
            if known.get(path) != Some(&stamp) {
                self.read(path, &stamp)?;
            }
        }
        Ok(())
    }

    fn forget(&self, path: &str) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM items_text WHERE rowid IN (SELECT rowid FROM items WHERE path = ?1)",
            [path],
        )?;
        tx.execute("DELETE FROM items WHERE path = ?1", [path])?;
        tx.execute("DELETE FROM links WHERE path = ?1", [path])?;
        tx.execute("DELETE FROM files WHERE path = ?1", [path])?;
        tx.commit()?;
        Ok(())
    }

    /// Read one file into the index. A file that cannot be read is left out
    /// until it is fixed.
    fn read(&self, path: &str, stamp: &str) -> anyhow::Result<()> {
        let items = match self.items_of(path) {
            Ok(items) => items,
            Err(error) => {
                tracing::debug!(%error, path, "a notebook file is not readable");
                Vec::new()
            }
        };
        self.forget(path)?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for item in items {
            tx.execute(
                "INSERT INTO items (path, scope, id, title, body, workgroup, date, fields)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    item.path,
                    item.scope,
                    item.id,
                    item.title,
                    item.body,
                    serde_json::to_string(&item.workgroup)?,
                    item.date,
                    serde_json::to_string(&item.fields)?,
                ],
            )?;
            let rowid = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO items_text (rowid, title, body) VALUES (?1, ?2, ?3)",
                params![rowid, item.title, item.body],
            )?;
            for (place, target) in &item.links {
                tx.execute(
                    "INSERT INTO links (path, item, place, target) VALUES (?1, ?2, ?3, ?4)",
                    params![item.path, rowid, place, target],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO files (path, stamp) VALUES (?1, ?2)
             ON CONFLICT (path) DO UPDATE SET stamp = excluded.stamp",
            params![path, stamp],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn items_of(&self, path: &str) -> anyhow::Result<Vec<Item>> {
        let full = self.notebook.root().join(path);
        if let Some(rest) = path.strip_prefix(&format!("{RECORDS}/")) {
            let (kind, file) = rest.split_once('/').context("not a record path")?;
            let id = file.trim_end_matches(".md");
            let record = Records::new(self.notebook.clone())
                .get(kind, id)?
                .context("no such record")?;
            // A field is found by its name in lower case, as it is in the
            // frontmatter.
            let mut fields: BTreeMap<String, String> = record
                .meta
                .fields()
                .filter_map(|(k, v)| Some((k.to_lowercase(), plain(v)?)))
                .collect();
            for item in record.body.fields() {
                if let Some((name, _)) = item.field() {
                    fields.insert(name.to_lowercase(), item.values().join(","));
                }
            }
            let date = ["due", "start", "updated"]
                .iter()
                .find_map(|k| fields.get(*k).cloned());
            return Ok(vec![Item {
                path: path.to_owned(),
                scope: format!("{RECORDS}/{kind}"),
                id: id.to_owned(),
                title: record.title(),
                body: record
                    .body
                    .text()
                    .split_once('\n')
                    .map(|(_, rest)| rest.trim().to_owned())
                    .unwrap_or_default(),
                workgroup: record.workgroups(),
                date,
                fields,
                links: record.links(),
            }]);
        }
        let rel = path
            .strip_prefix(&format!("{MEMORY}/"))
            .context("not a notebook file")?;
        let layer = Layer::of(rel).context("not a memory file")?;
        let scope = format!("{MEMORY}/{}", layer.scope());
        let name = Path::new(rel)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if layer == Layer::Working {
            let text = std::fs::read_to_string(&full)?;
            let working = memory::WorkingMemory::parse(&name, &text);
            return Ok(vec![Item {
                path: path.to_owned(),
                scope,
                id: name,
                title: working.title.clone(),
                links: outward(&text),
                body: text,
                workgroup: Vec::new(),
                date: None,
                fields: BTreeMap::new(),
            }]);
        }
        let doc = memory::read_doc(&self.notebook.memory_dir(), rel)?;
        let date = match layer {
            Layer::Episodic | Layer::Forgotten => Some(name.clone()),
            _ => None,
        };
        let prose = doc.long_text().map(str::trim).unwrap_or_default();
        Ok(doc
            .entries()
            .filter_map(|e| {
                let id = e.id()?.to_owned();
                let fields: BTreeMap<String, String> = e
                    .attrs()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_text()))
                    .collect();
                let body = if prose.is_empty() {
                    e.body().to_owned()
                } else {
                    format!("{}\n{prose}", e.body())
                };
                Some(Item {
                    path: path.to_owned(),
                    scope: scope.clone(),
                    title: e.body().to_owned(),
                    links: outward(&body),
                    body,
                    workgroup: e.workgroup(),
                    date: e.get("from").map(str::to_owned).or_else(|| date.clone()),
                    fields,
                    id,
                })
            })
            .collect())
    }

    /// What links to a record, on an index brought up to date.
    pub fn backlinks(
        &self,
        kind: impl AsRef<str>,
        id: impl AsRef<str>,
    ) -> anyhow::Result<Backlinks> {
        self.refresh()?;
        let target = format!("{RECORDS}/{}/{}", kind.as_ref(), id.as_ref());
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT i.path, i.scope, i.id, i.title, l.place
             FROM links l JOIN items i ON i.rowid = l.item
             WHERE l.target = ?1 ORDER BY i.path, l.rowid",
        )?;
        let rows = stmt.query_map([&target], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let forgotten = format!("{MEMORY}/{}", Layer::Forgotten.scope());
        let own = format!("{target}.md");
        let mut found = Backlinks::default();
        for row in rows {
            let (path, scope, id, title, from) = row?;
            if path == own || scope == forgotten {
                continue;
            }
            match scope.strip_prefix(&format!("{RECORDS}/")) {
                Some(kind) => found.records.push(Backlink {
                    kind: kind.to_owned(),
                    id,
                    title,
                    from,
                }),
                None => found.memories.push(MemoryLink { path, id, title }),
            }
        }
        Ok(found)
    }

    /// Count one use of a memory on `day`.
    pub fn record_use(&self, id: impl AsRef<str>, day: NaiveDate) -> anyhow::Result<()> {
        self.conn().execute(
            "INSERT INTO uses (id, uses, last) VALUES (?1, 1, ?2)
             ON CONFLICT (id) DO UPDATE SET uses = uses + 1, last = MAX(last, excluded.last)",
            params![id.as_ref(), day.format("%Y-%m-%d").to_string()],
        )?;
        Ok(())
    }

    /// Forget the uses of these memories, once they are written in the
    /// notebook.
    pub fn clear_uses<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> anyhow::Result<()> {
        let conn = self.conn();
        for id in ids {
            conn.execute("DELETE FROM uses WHERE id = ?1", [id])?;
        }
        Ok(())
    }

    /// Every memory used since its uses were last written in the notebook,
    /// by id.
    pub fn uses(&self) -> anyhow::Result<BTreeMap<String, Use>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, uses, last FROM uses")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut all = BTreeMap::new();
        for row in rows {
            let (id, uses, last) = row?;
            if let Ok(last) = NaiveDate::parse_from_str(&last, "%Y-%m-%d") {
                all.insert(id, Use { uses, last });
            }
        }
        Ok(all)
    }

    /// Search with the full-text search.
    pub fn search(&self, query: &Query) -> anyhow::Result<Vec<Hit>> {
        self.search_with(&FullText, query)
    }

    /// Search with a given way of searching, on an index brought up to date.
    pub fn search_with(&self, searcher: &dyn Searcher, query: &Query) -> anyhow::Result<Vec<Hit>> {
        self.refresh()?;
        searcher.search(self, query)
    }
}

fn outward(text: &str) -> Vec<(String, String)> {
    links(text)
        .into_iter()
        .map(|target| (String::new(), target))
        .collect()
}

fn plain(value: &serde_norway::Value) -> Option<String> {
    use serde_norway::Value;
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Sequence(items) => {
            Some(items.iter().filter_map(plain).collect::<Vec<_>>().join(","))
        }
        _ => None,
    }
}

/// When a file was last written and how long it is, to tell a changed file
/// from one the index already read.
fn stamp(path: &Path) -> anyhow::Result<String> {
    let meta = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    let modified = meta
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    Ok(format!("{}:{modified}", meta.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn an_index_of_an_older_shape_is_read_again_and_keeps_the_uses() {
        let tmp = TempDir::new("index-older");
        let notebook = Notebook::new(tmp.path());
        std::fs::create_dir_all(notebook.memory_dir()).unwrap();
        std::fs::write(
            notebook.memory_dir().join("core.md"),
            "# 心得\n\n- 返事は短く {id: m1, workgroup: [work]}\n",
        )
        .unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
        {
            let index = Index::open(&notebook).unwrap();
            index.refresh().unwrap();
            index.record_use("m1", day).unwrap();
            index
                .conn()
                .execute_batch(
                    "DROP TABLE items;
                     CREATE TABLE items (path TEXT);
                     PRAGMA user_version = 0;",
                )
                .unwrap();
        }

        let index = Index::open(&notebook).unwrap();

        let hits = index
            .search(&Query {
                text: "返事は短く".into(),
                workgroup: Some("work".into()),
                limit: 10,
                ..Query::default()
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(index.uses().unwrap()["m1"], Use { uses: 1, last: day });
    }
}
