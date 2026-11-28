//! Marking what a turn leaves in the notebook, however it wrote it: the
//! notebook as it was when the turn began is compared with what it is when the
//! turn ends.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::NaiveDate;
use famulen_notebook::{Doc, Entry, FileChange, Line, Page, changes, snapshot};
use serde::{Deserialize, Serialize};

use super::taint::{self, Holder, Taint};
use super::{Gate, carried, sources};
use crate::input::Input;
use crate::records::{ROUTINES, TASKS};
use crate::slack::web::Slack;
use crate::store::Store;

/// The state key of the turn whose writing is not yet marked.
const WATCH: &str = "disclosure.notebook";

/// Whether an attribute is what counting an item's uses rewrites, telling
/// nothing. Anything else written under such a name is told like any other.
fn counts(name: &str, value: &str) -> bool {
    match name {
        "uses" | "ev" => value.parse::<u16>().is_ok(),
        "last" => value.parse::<NaiveDate>().is_ok(),
        _ => false,
    }
}

#[derive(Serialize, Deserialize)]
struct Watch {
    input: i64,
    tree: String,
    /// What the turn was given when it began.
    begun: BTreeSet<Taint>,
    /// What its session holds of memory.
    handed: BTreeSet<String>,
}

impl<S: Slack> Gate<'_, S> {
    /// Remember the notebook as a turn about to begin finds it. What an earlier
    /// turn left unmarked, having been cut off, is marked first.
    pub(crate) async fn watch_notebook(&self, id: i64, input: &Input) -> anyhow::Result<()> {
        settle_notebook(self.store, self.memory_dir)?;
        carried::keep_handed(self.store, self.memory_dir, self.tz, id)?;
        let watch = Watch {
            input: id,
            tree: snapshot(sources::root(self.memory_dir))?,
            begun: self.taints(id, input).await?,
            handed: carried::handed(self.store, self.memory_dir, self.tz, id)?,
        };
        self.store
            .set_state(WATCH, serde_json::to_string(&watch)?)?;
        Ok(())
    }

    /// What was written into whatever an own tool's call reads.
    pub(crate) fn of_marked_read(
        &self,
        tool: &str,
        arguments: &serde_json::Value,
        answer: &str,
    ) -> anyhow::Result<Vec<Taint>> {
        let named = (tool == "read_file")
            .then(|| arguments["path"].as_str())
            .flatten()
            .and_then(|path| sources::in_notebook(self.memory_dir, path));
        let mut taints = match named {
            Some(path) => self.of_marked_in(&format!("{path}\n{answer}"))?,
            None => self.of_marked_in(answer)?,
        };
        if matches!(tool, "get_layer" | "set_working") {
            taints.extend(carried::named_in_memory(self.store, answer)?);
        }
        Ok(taints)
    }
}

/// Mark what changed in the notebook while the watched turn ran with what that
/// turn read in confidence. What was written in confidence among the memory it
/// was handed is marked too, until [`carried::compare_written`] clears it:
/// nothing records which of it the turn wrote out again.
pub(crate) fn settle_notebook(store: &Store, memory_dir: &Path) -> anyhow::Result<()> {
    let Some(watch) = store.state(WATCH)?.filter(|w| !w.is_empty()) else {
        return Ok(());
    };
    let watch: Watch = serde_json::from_str(&watch)?;
    let mut taints = watch.begun;
    taints.extend(taint::held_by(store, &Holder::Input(watch.input))?);
    let mut handed = watch.handed;
    if let Some(conversation) = taint::conversation_of(store, watch.input)? {
        taints.extend(taint::held_by(store, &Holder::Conversation(conversation))?);
        handed.extend(carried::handed_before(store, conversation)?);
    }
    let (confided, handed) = carried::confided_among(store, &handed, |_| true)?;
    if !taints.is_empty() || !handed.is_empty() {
        let root = sources::root(memory_dir);
        let taints: Vec<Taint> = taints.into_iter().collect();
        let mut text = String::new();
        let mut holders = Vec::new();
        for change in changes(root, &watch.tree, &snapshot(root)?)? {
            let mut held: Vec<Holder> = written(&change).into_iter().map(Holder::Record).collect();
            held.extend(run_from(&change));
            if held.is_empty() {
                continue;
            }
            for holder in &held {
                taint::mark(store, holder, &taints)?;
            }
            text.push_str(&added(&change));
            holders.extend(held);
        }
        let doubts = carried::provisionally(store, vec![(text, holders)], &confided, &handed)?;
        carried::keep_doubts(store, doubts)?;
    }
    store.set_state(WATCH, "")?;
    Ok(())
}

/// Where a change wrote, and the lines it added there.
fn added(change: &FileChange) -> String {
    let before: BTreeSet<&str> = change
        .before
        .as_deref()
        .unwrap_or_default()
        .lines()
        .collect();
    let mut text = format!("{}\n", change.path);
    for line in change.after.as_deref().unwrap_or_default().lines() {
        if !before.contains(line) {
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

/// What is run from a record the change wrote, which reads its marks apart
/// from those of the record.
fn run_from(change: &FileChange) -> Option<Holder> {
    change.after.as_ref()?;
    let (kind, file) = change.path.strip_prefix("records/")?.split_once('/')?;
    let id = file.strip_suffix(".md")?.to_owned();
    match kind {
        TASKS => Some(Holder::Task(id)),
        ROUTINES => Some(Holder::Routine(id)),
        _ => None,
    }
}

/// The names under which what a change wrote is read again: the ids of the
/// items it wrote, or the file's path where no id tells it.
fn written(change: &FileChange) -> Vec<String> {
    let Some(after) = &change.after else {
        return Vec::new();
    };
    let (path, before) = (
        change.path.as_str(),
        change.before.as_deref().unwrap_or_default(),
    );
    let whole = || vec![path.to_owned()];
    let Some(name) = path.strip_suffix(".md") else {
        return whole();
    };
    if path.starts_with("records/") {
        let stem = name.rsplit('/').next().unwrap_or(name);
        return vec![path.to_owned(), stem.to_owned()];
    }
    if path.starts_with("memory/long/") {
        return written_long(path, before, after);
    }
    if !path.starts_with("memory/") || path.starts_with("memory/working/") {
        return whole();
    }
    written_items(path, before, after)
}

fn written_long(path: &str, before: &str, after: &str) -> Vec<String> {
    let (Ok(before), Ok(after)) = (Page::parse(before), Page::parse(after)) else {
        return vec![path.to_owned()];
    };
    if told_by_page(&before) == told_by_page(&after) {
        return Vec::new();
    }
    std::iter::once(path.to_owned())
        .chain(after.text("id"))
        .collect()
}

fn told_by_page(page: &Page) -> (Vec<String>, &str) {
    let mut fields: Vec<String> = page
        .fields()
        .filter(|(name, _)| !page.text(name).is_some_and(|value| counts(name, &value)))
        .map(|(name, value)| format!("{name}: {value:?}"))
        .collect();
    fields.sort();
    (fields, page.body().trim())
}

pub(super) fn told_by_item(entry: &Entry) -> (&str, Vec<String>) {
    let mut attrs: Vec<String> = entry
        .attrs()
        .iter()
        .map(|(name, value)| (name, value.to_text()))
        .filter(|(name, value)| !counts(name, value))
        .map(|(name, value)| format!("{name}: {value}"))
        .collect();
    attrs.sort();
    (entry.text(), attrs)
}

/// A file of items. Counting an item's uses again writes nothing of it.
fn written_items(path: &str, before: &str, after: &str) -> Vec<String> {
    let (before, after) = (Doc::parse(before), Doc::parse(after));
    let mut keys = Vec::new();
    let mut untold = false;
    for line in after.lines() {
        match line {
            Line::Entry(entry) => {
                let was = match entry.id() {
                    Some(id) => before.find(id),
                    None => before.entries().find(|e| e.body() == entry.body()),
                };
                if was.is_none_or(|was| told_by_item(was) != told_by_item(entry)) {
                    match entry.id() {
                        Some(id) => keys.push(id.to_owned()),
                        None => untold = true,
                    }
                }
            }
            Line::Other(text) => {
                let said = text.trim();
                untold |=
                    !said.is_empty() && !said.starts_with('#') && !before.lines().contains(line);
            }
        }
    }
    if untold {
        keys.push(path.to_owned());
    }
    keys
}
