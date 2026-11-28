//! Carrying marks along when what was written changes its name: a night's
//! dream and chore rewrite, merge, move and remove what is remembered.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chrono::Utc;
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use super::notebook::told_by_item;
use super::taint::{self, Holder, Kind, Taint};
use super::{Reader, Saying};
use crate::console_log::{ConsoleLog, Who};
use crate::conversations::Conversations;
use crate::dreaming::TALK_SHOWN;
use crate::memory::{self, Change, Layer, Located, Op, Snapshot};
use crate::records::{MEMOS, Records, TASKS};
use crate::store::Store;

/// A shorter id, written by hand, would be found in any text.
const SHORTEST_ID: usize = 8;

/// What is marked of any of the names `text` tells.
pub(super) fn marked_in(store: &Store, text: &str) -> anyhow::Result<BTreeSet<Taint>> {
    Ok(taint::marked_records(store)?
        .into_iter()
        .filter(|(id, _)| id.len() >= SHORTEST_ID && text.contains(id.as_str()))
        .map(|(_, taint)| taint)
        .collect())
}

fn handed_now(memory_dir: &Path, tz: Tz) -> BTreeSet<String> {
    let today = Utc::now().with_timezone(&tz).date_naive();
    memory::prompt_context(memory_dir, today, usize::MAX)
        .lines()
        .filter(|line| line.contains("- "))
        .map(str::to_owned)
        .collect()
}

fn handed_key(conversation: i64) -> String {
    format!("disclosure.handed.{conversation}")
}

pub(super) fn handed_before(store: &Store, conversation: i64) -> anyhow::Result<BTreeSet<String>> {
    Ok(match store.state(handed_key(conversation))? {
        Some(lines) => serde_json::from_str(&lines)?,
        None => BTreeSet::new(),
    })
}

/// Keep what a turn's conversation is handed of memory: its session goes on
/// holding it after memory has dropped or reworded it.
pub(super) fn keep_handed(
    store: &Store,
    memory_dir: &Path,
    tz: Tz,
    input: i64,
) -> anyhow::Result<()> {
    let Some(conversation) = taint::conversation_of(store, input)? else {
        return Ok(());
    };
    let mut lines = handed_before(store, conversation)?;
    lines.extend(handed_now(memory_dir, tz));
    store.set_state(handed_key(conversation), serde_json::to_string(&lines)?)?;
    Ok(())
}

/// What was handed to the conversation `from` goes with what is said there.
pub(super) fn carry_handed(store: &Store, from: i64, to: i64) -> anyhow::Result<()> {
    let mut lines = handed_before(store, to)?;
    lines.extend(handed_before(store, from)?);
    store.set_state(handed_key(to), serde_json::to_string(&lines)?)?;
    Ok(())
}

/// The lines written in confidence among what a session is handed of memory,
/// now or earlier in the turn's conversation. Nothing records that a session
/// read them, so the one who reads what is said is shown them.
pub(super) fn confided(
    store: &Store,
    memory_dir: &Path,
    tz: Tz,
    input: i64,
    counts: impl Fn(&Taint) -> bool,
) -> anyhow::Result<Vec<String>> {
    let lines = handed(store, memory_dir, tz, input)?;
    Ok(confided_among(store, &lines, counts)?.0)
}

/// What a turn's session holds of memory: what it is handed now, and what its
/// conversation was handed earlier.
pub(super) fn handed(
    store: &Store,
    memory_dir: &Path,
    tz: Tz,
    input: i64,
) -> anyhow::Result<BTreeSet<String>> {
    let mut lines = handed_now(memory_dir, tz);
    if let Some(conversation) = taint::conversation_of(store, input)? {
        lines.extend(handed_before(store, conversation)?);
    }
    Ok(lines)
}

/// The lines among those handed of memory that were written in confidence,
/// and what they were marked with.
pub(super) fn confided_among(
    store: &Store,
    lines: &BTreeSet<String>,
    counts: impl Fn(&Taint) -> bool,
) -> anyhow::Result<(Vec<String>, BTreeSet<Taint>)> {
    let mut marked = taint::marked_records(store)?;
    marked.retain(|(_, taint)| counts(taint));
    let mut confided = Vec::new();
    let mut taints = BTreeSet::new();
    for line in lines {
        let path = line
            .strip_prefix("- ")
            .and_then(|rest| rest.split_once(".md:"))
            .map(|(path, _)| format!("memory/{path}.md"));
        // A line written by hand without an id is told only by where it is.
        let unnamed = path.is_none() && !line.contains("id: ");
        let mut of_line = marked
            .iter()
            .filter(|(key, _)| {
                path.as_ref() == Some(key)
                    || (key.len() >= SHORTEST_ID && line.contains(key.as_str()))
                    || (unnamed && (key == "memory/core.md" || key.starts_with("memory/episodic/")))
            })
            .map(|(_, taint)| taint.clone())
            .peekable();
        if of_line.peek().is_some() {
            confided.push(line.clone());
            taints.extend(of_line);
        }
    }
    Ok((confided, taints))
}

/// What is marked of the working memories an own tool's answer names.
pub(super) fn named_in_memory(store: &Store, answer: &str) -> anyhow::Result<BTreeSet<Taint>> {
    Ok(taint::marked_records(store)?
        .into_iter()
        .filter(|(key, _)| {
            let Some(file) = key.strip_prefix("memory/") else {
                return false;
            };
            let told = match file.strip_prefix("working/") {
                Some(working) => working.strip_suffix(".md"),
                None => Some(file),
            };
            told.is_some_and(|name| answer.contains(&format!("\"{name}\"")))
        })
        .map(|(_, taint)| taint)
        .collect())
}

/// What a dream did to memory.
pub(crate) struct Reorganized<'a> {
    pub(crate) before: &'a Snapshot,
    pub(crate) after: &'a Snapshot,
    pub(crate) changes: &'a [Change],
    /// The notes each item's edits rest on, by the item's id.
    pub(crate) rests_on: &'a BTreeMap<String, BTreeSet<String>>,
    pub(crate) notes_shown: &'a BTreeSet<String>,
    /// Everything the model was shown.
    pub(crate) shown: &'a str,
    /// Since when the talk shown to the model was said.
    pub(crate) talk_since: i64,
    /// The records memories became, with the titles the model gave them.
    pub(crate) titled: &'a [(String, String)],
}

fn of_key(store: &Store, key: &str) -> anyhow::Result<BTreeSet<Taint>> {
    taint::held_by(store, &Holder::Record(key.to_owned()))
}

fn path_of(item: &Located) -> String {
    format!("memory/{}", item.file.to_string_lossy().replace('\\', "/"))
}

fn of_item(store: &Store, snapshot: &Snapshot, id: &str) -> anyhow::Result<BTreeSet<Taint>> {
    let mut taints = of_key(store, id)?;
    if let Some(item) = snapshot.entries.get(id) {
        taints.extend(of_key(store, &path_of(item))?);
    }
    Ok(taints)
}

fn of_notes(store: &Store, notes: Option<&BTreeSet<String>>) -> anyhow::Result<BTreeSet<Taint>> {
    let mut taints = BTreeSet::new();
    for note in notes.into_iter().flatten() {
        taints.extend(of_key(store, note)?);
    }
    Ok(taints)
}

/// What the talk on the console since then was marked with, whichever way it
/// came there, and what was said that is marked: a dream is shown what was
/// said on Slack there as well.
fn of_talk(store: &Store, since: i64) -> anyhow::Result<(BTreeSet<Taint>, Vec<String>)> {
    let conversations = Conversations::new(store.clone());
    let mut taints = BTreeSet::new();
    let mut lines = Vec::new();
    for message in ConsoleLog::new(store.clone()).recent(TALK_SHOWN)? {
        if message.at < since {
            continue;
        }
        let input = match message.input() {
            Some(input) => Some(input),
            None => store.id_of(&format!("console:{}", message.id))?,
        };
        let mut said = BTreeSet::new();
        if let Some(input) = input {
            said.extend(taint::held_by(store, &Holder::Input(input))?);
        }
        let asked_in = match input {
            Some(input) => taint::conversation_of(store, input)?,
            None => None,
        };
        match conversations.of_message(&message)?.or(asked_in) {
            Some(conversation) => {
                said.extend(taint::held_by(store, &Holder::Conversation(conversation))?);
            }
            // Which room it was said in is looked up when a turn takes it up.
            None if message.on_slack() && message.who == Who::Owner => {
                said.insert(Taint::new(Kind::Private, "確かめられなかったもの"));
            }
            None => {}
        }
        if !said.is_empty() {
            lines.push(message.text.replace('\n', " "));
            taints.extend(said);
        }
    }
    Ok((taints, lines))
}

/// The holders under which the item `id` is marked.
fn holders_of(after: &Snapshot, id: &str) -> Vec<Holder> {
    let mut holders = vec![Holder::Record(id.to_owned())];
    // A long-term memory is also told by where it is.
    if let Some(item) = after.entries.get(id)
        && matches!(item.layer, Layer::Long(_))
    {
        holders.push(Holder::Record(path_of(item)));
    }
    holders
}

fn mark_item(
    store: &Store,
    after: &Snapshot,
    id: &str,
    taints: BTreeSet<Taint>,
) -> anyhow::Result<()> {
    if taints.is_empty() {
        return Ok(());
    }
    let taints: Vec<Taint> = taints.into_iter().collect();
    for holder in holders_of(after, id) {
        taint::mark(store, &holder, &taints)?;
    }
    Ok(())
}

/// What is marked of the files `shown` tells by their path.
fn marked_by_path_in(store: &Store, shown: &str) -> anyhow::Result<BTreeSet<Taint>> {
    Ok(taint::marked_records(store)?
        .into_iter()
        .filter(|(key, _)| {
            key.strip_prefix("memory/")
                .is_some_and(|file| shown.contains(file))
        })
        .map(|(_, taint)| taint)
        .collect())
}

/// What of the dream's material was written in confidence.
fn in_confidence(
    store: &Store,
    dream: &Reorganized<'_>,
    talk: Vec<String>,
) -> anyhow::Result<Vec<String>> {
    let marked: BTreeSet<String> = taint::marked_records(store)?
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    // A line written by hand without an id is told only by where it is.
    let unnamed_marked = marked.iter().any(|key| key.starts_with("memory/"));
    let mut lines: BTreeSet<String> = dream
        .shown
        .lines()
        .filter(|line| {
            marked
                .iter()
                .any(|key| key.len() >= SHORTEST_ID && line.contains(key.as_str()))
                || (unnamed_marked && line.starts_with("- ") && !line.contains("id: "))
        })
        .map(str::to_owned)
        .collect();
    for (id, item) in &dream.before.entries {
        if !of_item(store, dream.before, id)?.is_empty() {
            lines.insert(item.entry.text().to_owned());
        }
    }
    lines.extend(talk);
    Ok(lines.into_iter().collect())
}

/// Marks given to something written only until it is compared with what its
/// writer was shown in confidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Doubt {
    id: i64,
    text: String,
    confided: Vec<String>,
    marks: Vec<(Holder, Taint)>,
}

/// Take back the marks of what turns out to tell nothing confidential. What
/// cannot be compared keeps them.
pub(crate) async fn settle_doubts(
    store: &Store,
    reader: Option<&dyn Reader>,
    doubts: &[Doubt],
) -> anyhow::Result<()> {
    let Some(reader) = reader else {
        return Ok(());
    };
    for doubt in doubts {
        let saying = Saying {
            text: doubt.text.clone(),
            room: "手帳の記憶".into(),
            read: Vec::new(),
            here: false,
            owner: String::new(),
            confided: doubt.confided.clone(),
            confided_only: true,
        };
        match reader.read(&saying).await {
            Ok(reading) if reading.discloses.is_none() => {
                for (holder, taint) in &doubt.marks {
                    taint::clear_doubt(store, doubt.id, holder, taint)?;
                }
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "what was written could not be compared"),
        }
    }
    Ok(())
}

/// The state key of what turns wrote that is yet to be compared.
const DOUBTS: &str = "disclosure.doubts";

fn pending(store: &Store) -> anyhow::Result<Vec<Doubt>> {
    Ok(match store.state(DOUBTS)?.filter(|d| !d.is_empty()) {
        Some(doubts) => serde_json::from_str(&doubts)?,
        None => Vec::new(),
    })
}

/// Keep what a turn wrote, to be compared once no turn is running.
pub(super) fn keep_doubts(store: &Store, doubts: Vec<Doubt>) -> anyhow::Result<()> {
    if doubts.is_empty() {
        return Ok(());
    }
    let mut kept = pending(store)?;
    kept.extend(doubts);
    store.set_state(DOUBTS, serde_json::to_string(&kept)?)
}

/// Compare what turns wrote with what they were handed in confidence.
pub(crate) async fn compare_written(
    store: &Store,
    reader: Option<&dyn Reader>,
) -> anyhow::Result<()> {
    let doubts = pending(store)?;
    if doubts.is_empty() {
        return Ok(());
    }
    settle_doubts(store, reader, &doubts).await?;
    store.set_state(DOUBTS, "")
}

/// Mark what was written with what its writer was shown in confidence, until
/// it is compared.
pub(super) fn provisionally(
    store: &Store,
    written: Vec<(String, Vec<Holder>)>,
    confided: &[String],
    taints: &BTreeSet<Taint>,
) -> anyhow::Result<Vec<Doubt>> {
    let mut doubts = Vec::new();
    for (text, holders) in written {
        let id = rand::random();
        let mut marks = Vec::new();
        for holder in holders {
            for taint in taints {
                if taint::mark_doubted(store, id, &holder, taint)? {
                    marks.push((holder.clone(), taint.clone()));
                }
            }
        }
        if !marks.is_empty() {
            doubts.push(Doubt {
                id,
                text,
                confided: confided.to_vec(),
                marks,
            });
        }
    }
    Ok(doubts)
}

/// Mark what a dream wrote with what it was written from. Where no note says
/// what that was, it is taken to be anything the model was shown. The notes a
/// model cites do not show that it wrote from nothing else, so what they
/// account for is marked the same until it is compared.
pub(crate) fn carry_dream(store: &Store, dream: &Reorganized<'_>) -> anyhow::Result<Vec<Doubt>> {
    let mut removed = BTreeSet::new();
    for change in dream.changes {
        let gone = match change.op {
            Op::Archived | Op::Deleted => Some(change.id.as_str()),
            Op::Replaced => change.replaces.as_deref(),
            _ => None,
        };
        if let Some(id) = gone {
            removed.extend(of_item(store, dream.before, id)?);
        }
    }
    let mut anything = marked_in(store, dream.shown)?;
    anything.extend(marked_by_path_in(store, dream.shown)?);
    let (talked, talk) = of_talk(store, dream.talk_since)?;
    anything.extend(talked);
    let confided = in_confidence(store, dream, talk)?;
    // A move within one layer is no change, yet puts a memory under a path
    // that tells nothing of it.
    for (id, now) in &dream.after.entries {
        if dream
            .before
            .entries
            .get(id)
            .is_some_and(|was| was.file != now.file)
        {
            mark_item(store, dream.after, id, of_item(store, dream.before, id)?)?;
        }
    }
    // What an item tells is also in its attributes, whose rewriting alone is
    // no change either.
    let mut written: BTreeSet<&str> = dream
        .after
        .entries
        .iter()
        .filter(|(id, now)| {
            let was = dream.before.entries.get(*id);
            was.is_some_and(|was| told_by_item(&was.entry) != told_by_item(&now.entry))
        })
        .map(|(id, _)| id.as_str())
        .collect();
    let mut replaces = BTreeMap::new();
    for change in dream.changes {
        let id = change.id.as_str();
        if matches!(
            change.op,
            Op::Added | Op::Replaced | Op::Edited | Op::PendingAdded
        ) {
            written.insert(id);
            if let Some(old) = &change.replaces {
                replaces.insert(id, old.as_str());
            }
        } else {
            mark_item(store, dream.after, id, of_item(store, dream.before, id)?)?;
        }
    }
    let mut doubted = Vec::new();
    for id in written {
        let mut taints = of_item(store, dream.before, id)?;
        let cited = dream.rests_on.get(id);
        taints.extend(of_notes(store, cited)?);
        // A made-up id would otherwise pass for grounds that hold nothing
        // confidential.
        let grounded =
            cited.is_some_and(|notes| !notes.is_empty() && notes.is_subset(dream.notes_shown));
        taints.extend(removed.iter().cloned());
        if let Some(old) = replaces.get(id) {
            taints.extend(of_item(store, dream.before, old)?);
        }
        // Without the lines to compare with, nothing can be cleared.
        if !grounded || confided.is_empty() {
            taints.extend(anything.iter().cloned());
        } else if let Some(item) = dream.after.entries.get(id) {
            let (text, attrs) = told_by_item(&item.entry);
            let told = format!("{text}\n{}", attrs.join("\n"));
            doubted.push((told, holders_of(dream.after, id)));
        }
        mark_item(store, dream.after, id, taints)?;
    }
    for (record, title) in dream.titled {
        if confided.is_empty() {
            mark_record(store, record, anything.clone())?;
        } else {
            let holders = vec![Holder::Record(record.clone()), Holder::Task(record.clone())];
            doubted.push((title.clone(), holders));
        }
    }
    provisionally(store, doubted, &confided, &anything)
}

/// Mark the record a memory became.
pub(crate) fn carry_to_record(
    store: &Store,
    before: &Snapshot,
    rests_on: &BTreeMap<String, BTreeSet<String>>,
    item: &str,
    record: &str,
) -> anyhow::Result<()> {
    let mut taints = of_item(store, before, item)?;
    taints.extend(of_notes(store, rests_on.get(item))?);
    mark_record(store, record, taints)
}

/// Mark a record rewritten from what a model was shown with anything marked of
/// it: the records and memories named in `shown`, and the talk since `since`.
pub(crate) fn carry_shown(store: &Store, shown: &str, since: i64, to: &str) -> anyhow::Result<()> {
    let mut taints = marked_in(store, shown)?;
    taints.extend(marked_by_path_in(store, shown)?);
    taints.extend(of_talk(store, since)?.0);
    mark_record(store, to, taints)
}

/// Mark what one record says with what is marked of another it was made from.
pub(crate) fn carry_record(
    store: &Store,
    from: impl IntoIterator<Item = impl AsRef<str>>,
    to: &str,
) -> anyhow::Result<()> {
    let mut taints = BTreeSet::new();
    for key in from {
        taints.extend(of_key(store, key.as_ref())?);
    }
    mark_record(store, to, taints)
}

fn mark_record(store: &Store, record: &str, taints: BTreeSet<Taint>) -> anyhow::Result<()> {
    let taints: Vec<Taint> = taints.into_iter().collect();
    taint::mark(store, &Holder::Record(record.to_owned()), &taints)?;
    taint::mark(store, &Holder::Task(record.to_owned()), &taints)
}

/// Mark a turn about to run with what is marked of the notes and files it is
/// asked about.
pub(crate) fn carry_to_input(
    store: &Store,
    from: impl IntoIterator<Item = impl AsRef<str>>,
    input: i64,
) -> anyhow::Result<()> {
    let mut taints = BTreeSet::new();
    for key in from {
        taints.extend(of_key(store, key.as_ref())?);
    }
    let taints: Vec<Taint> = taints.into_iter().collect();
    taint::mark(store, &Holder::Input(input), &taints)
}

/// The state key telling that what earlier dreams dropped was given back.
const RECOVERED: &str = "disclosure.recovered";

/// Give back, once, the marks that dreams before this dropped, from what each
/// night recorded of its changes. The oldest night comes first, so that what
/// one night wrote is a source of the next.
pub(crate) fn recover(
    store: &Store,
    records: &Records,
    nights: &[Vec<Change>],
) -> anyhow::Result<()> {
    if store.state(RECOVERED)?.is_some() {
        return Ok(());
    }
    let root = records.notebook().root();
    // A night does not tell which record a memory became: the record is told
    // by saying what the memory said.
    let mut written = records.list(TASKS)?;
    written.extend(records.list(MEMOS)?);
    for changes in nights {
        let mut removed = BTreeSet::new();
        let mut noted = BTreeSet::new();
        for change in changes {
            for basis in &change.basis {
                noted.extend(of_key(store, &basis.id)?);
            }
            let gone = match change.op {
                Op::Archived | Op::Deleted => Some(&change.id),
                Op::Replaced => change.replaces.as_ref(),
                _ => None,
            };
            if let Some(id) = gone {
                removed.extend(of_key(store, id)?);
            }
        }
        for change in changes {
            let mut taints = of_key(store, &change.id)?;
            if let Some(from) = &change.from_file {
                taints.extend(of_key(store, &in_notebook(from))?);
            }
            match change.op {
                Op::Added | Op::Replaced | Op::Edited | Op::PendingAdded => {
                    taints.extend(removed.iter().cloned());
                    if let Some(old) = &change.replaces {
                        taints.extend(of_key(store, old)?);
                    }
                    for basis in &change.basis {
                        taints.extend(of_key(store, &basis.id)?);
                    }
                    if change.basis.is_empty() {
                        taints.extend(noted.iter().cloned());
                    }
                }
                Op::TaskMade => {
                    let task = Path::new(&change.file).file_stem();
                    if let Some(task) = task.and_then(|t| t.to_str()) {
                        mark_record(store, task, taints.clone())?;
                    }
                    continue;
                }
                _ => {}
            }
            if taints.is_empty() {
                continue;
            }
            if change.op == Op::Archived && !change.text.is_empty() {
                for record in written
                    .iter()
                    .filter(|r| r.body.text().contains(&change.text))
                {
                    mark_record(store, &record.id, taints.clone())?;
                }
            }
            let taints: Vec<Taint> = taints.into_iter().collect();
            taint::mark(store, &Holder::Record(change.id.clone()), &taints)?;
            if change.file.starts_with("long/") && root.join(in_notebook(&change.file)).exists() {
                let path = in_notebook(&change.file);
                taint::mark(store, &Holder::Record(path), &taints)?;
            }
        }
    }
    store.set_state(RECOVERED, "done")?;
    Ok(())
}

fn in_notebook(memory_file: &str) -> String {
    format!("memory/{memory_file}")
}

#[cfg(test)]
pub(crate) mod marking {
    use super::*;

    /// Mark what is told by `key` as written in confidence.
    pub(crate) fn mark(store: &Store, key: &str) {
        let read = [Taint::of_connection("gh", "の内容")];
        taint::mark(store, &Holder::Record(key.to_owned()), &read).unwrap();
    }

    pub(crate) fn mark_input(store: &Store, input: i64) {
        let read = [Taint::of_connection("gh", "の内容")];
        taint::mark(store, &Holder::Input(input), &read).unwrap();
    }

    pub(crate) fn mark_conversation_of(store: &Store, input: i64) {
        let conversation = taint::conversation_of(store, input).unwrap().unwrap();
        let read = [Taint::of_connection("gh", "の内容")];
        taint::mark(store, &Holder::Conversation(conversation), &read).unwrap();
    }

    pub(crate) fn marked(store: &Store, key: &str) -> bool {
        !of_key(store, key).unwrap().is_empty()
    }

    pub(crate) fn marked_input(store: &Store, input: i64) -> bool {
        !taint::held_by(store, &Holder::Input(input))
            .unwrap()
            .is_empty()
    }
}
