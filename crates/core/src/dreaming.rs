//! The nightly dream: the model proposes item-by-item edits to memory as
//! JSON, and Famulen applies them, refuses a dream that breaks the guards,
//! and commits what is left as one commit.
//!
//! The model never rewrites a file whole. A whole-file rewrite by a model
//! loses items silently; item edits can be counted, shown and checked.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::future::Future;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use famulen_notebook::index::Index;

use crate::console_log::{ConsoleLog, Who};
use crate::dream_settings::DreamSettings;
use crate::memory::{self, DreamDiff, DreamSummary, Entry, EpisodeState, Layer, Snapshot};
use crate::notebook::{self, Writer};
use crate::records::{Held, MEMOS, OWNER, Patch, Record, Records, TASKS, log_lines};
use crate::store::{Store, now};

/// One edit the model asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edit {
    /// `add`, `edit`, `move`, `confirm`, `forget` or `to_record`.
    pub op: String,
    #[serde(default)]
    pub id: Option<String>,
    /// For `add` and `move`: the file relative to the memory directory.
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub attrs: BTreeMap<String, String>,
    #[serde(default)]
    pub reason: Option<String>,
    /// For `to_record`: the record the item becomes.
    #[serde(default)]
    pub record: Option<AsRecord>,
}

/// The record a memory becomes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsRecord {
    /// `tasks` or `memos`. The other kinds are not offered: the dream is not
    /// shown the records, so it would add a second one of what is there.
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    /// For a task, `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`.
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
}

/// The kinds of record a memory may become.
const AS_RECORD: [&str; 2] = [TASKS, MEMOS];

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Plan {
    #[serde(default)]
    edits: Vec<Edit>,
    /// The episodic notes the dream dealt with, whether or not they changed
    /// anything. The others are read again next time.
    #[serde(default)]
    settled: Vec<String>,
}

pub fn plan_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "edits": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "op": { "type": "string", "enum": ["add", "edit", "confirm", "move", "forget", "to_record"] },
                        "id": { "type": "string" },
                        "file": { "type": "string" },
                        "text": { "type": "string" },
                        "attrs": { "type": "object", "additionalProperties": { "type": "string" } },
                        "reason": { "type": "string" },
                        "record": {
                            "type": "object",
                            "properties": {
                                "kind": { "type": "string", "enum": AS_RECORD },
                                "title": { "type": "string" },
                                "start": { "type": "string" },
                                "due": { "type": "string" }
                            },
                            "required": ["kind", "title"]
                        }
                    },
                    "required": ["op"]
                }
            },
            "settled": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["edits", "settled"]
    })
}

/// What the dreaming model is told, with `{core_lines}` for the core limit.
const SYSTEM: &str = "\
You are the dreaming mind of a personal secretary. Each night you reorganize its memory.

Input: the episodic notes waiting to be consolidated (raw noticings of the day), then the memory as it is: core (read in every conversation, at most {core_lines} lines), \
pending judgments, and long-term memories, one memory per file, grouped as semantic (facts: people, likes, how things are), procedural (how things are done) and prospective (promises and what is to be done). \
Each item is shown as one line, its attributes a YAML flow mapping at the end: \
`- text {id: …, src: said|confirmed|inferred, from: …, to: …, ev: …, workgroup: [...], party: …, need: …, supersedes: …}`, with how often it was used and when last (uses, last) where it has been.

Answer with small edits only:
- add: a new item in core.md or pending.md, or a new long-term memory in a file of its own, long/<group>/<name>.md with a short new name (give file, text, attrs; src is said when the owner said it, inferred when you concluded it).
- edit: change an item's text or attrs by id (raise ev when a note backs it again).
- move: move an item by id to another file (promote a well-backed item to core, demote an unused one from core, settle a pending judgment into long-term memory).
- confirm: an item a note backs again, by id (its evidence and last use go up).
- forget: move an item by id to forgotten memory, with a reason (outdated, replaced, wrong).
- to_record: an item that is really a record, by id: it leaves memory and becomes the record you give in `record`. \
  Choose its kind by what the owner would do with it. tasks: something to be done, by anyone, at any time (an action, an errand, \
  a promise to keep, something to ask, check, buy, send, decide or follow up), with a title that says what to do, and start and due (YYYY-MM-DD or YYYY-MM-DD HH:MM) only when they are known. \
  memos: a fact to look up with nothing to do about it (a device, a contract, an account, how something stands). When in doubt whether there is something to do, it is a task, not a memo.

Rules:
- Records (calendar events, tasks, people's accounts, devices, contracts) are not memory: do not add them, and move any already in memory out with to_record.
- A procedure is kept only when the owner confirmed it or it worked once; otherwise leave it pending.
- An item the owner said is never edited away or forgotten unless 3 or more notes contradict it; an inferred item needs 2. \
  With fewer, add or update a pending judgment instead (need=2 or need=3, ev counting what backs it).
- A replacement is an add whose attrs name the old id in supersedes, plus a forget of the old item with reason replaced.
- Keep core within {core_lines} lines; move what is least used out of core first.
- Never shrink a file, or a long-term group, to less than half its items in one night.
- The lookout judges what concerns the owner by semantic memory and records alone. Notes about lookout decisions the owner marked 要らなかった or 拾ってほしかった, \
  and what the owner said does or does not concern them (\"これは関係ない\"), become or back semantic memories of what they care about and what not to bring them, \
  so the lookout picks up and passes over the same things next time.
- Who a person is to the owner (their company, role, how to deal with them) is memory: keep it in long/semantic/<name>.md and link their record ([[records/parties/<id>]]) in the prose. Link the other records a memory speaks of the same way ([[records/<kind>/<id>|title]]), and keep the links a note has. Their contact accounts are records.
- Deal with every episodic note: file it, back an item with it, or decide it needs nothing. List every note id you dealt with in settled; a note left out is shown to you again next night.
- Write in the language the notes are written in. Do nothing when nothing needs to change.";

/// The system prompt for a dream that keeps core within `core_lines`.
pub fn system(core_lines: u32) -> String {
    SYSTEM.replace("{core_lines}", &core_lines.to_string())
}

/// The episodic notes the dream has not consolidated yet, and the memory.
pub fn prompt(dir: &Path, usage: &Index) -> anyhow::Result<String> {
    let used = usage.uses()?;
    let shown = |e: &Entry| {
        let mut e = e.clone();
        if let Some(u) = e.id().and_then(|id| used.get(id)) {
            add_use(&mut e, u);
        }
        e.render()
    };
    let mut text = String::from("Episodic notes:\n\n");
    let notes = waiting_notes(dir)?;
    for (date, e) in &notes {
        let _ = writeln!(text, "{date} {}", e.render());
    }
    if notes.is_empty() {
        text.push_str("(none)\n");
    }
    for (rel, label) in [
        (memory::CORE, "Core"),
        (memory::PENDING, "Pending judgments"),
    ] {
        let doc = memory::read_doc(dir, rel)?;
        let entries: Vec<&Entry> = doc.entries().collect();
        if rel == memory::CORE {
            let _ = writeln!(text, "\n{label} ({rel}, {} lines):\n", entries.len());
        } else {
            let _ = writeln!(text, "\n{label} ({rel}):\n");
        }
        for e in entries {
            let _ = writeln!(text, "{}", shown(e));
        }
    }
    text.push_str("\nLong-term memory:\n");
    for group in memory::long_term(dir)? {
        for file in group.files {
            let _ = writeln!(
                text,
                "\n{}:\n",
                file.path.to_string_lossy().replace('\\', "/")
            );
            for e in &file.entries {
                let _ = writeln!(text, "{}", shown(e));
            }
        }
    }
    Ok(text)
}

/// Add uses to what an item says of its own.
fn add_use(e: &mut Entry, used: &famulen_notebook::index::Use) {
    let before = e.get("uses").and_then(|u| u.parse::<u32>().ok());
    let last = used.last.format("%Y-%m-%d").to_string();
    let last = e
        .get("last")
        .map_or(last.clone(), |l| last.max(l.to_owned()));
    let _ = e.set("uses", (before.unwrap_or(0) + used.uses).to_string());
    let _ = e.set("last", last);
}

/// Write how often each kept memory was used into the notebook, and forget
/// the counts kept beside it.
fn settle_uses(dir: &Path, usage: &Index) -> anyhow::Result<()> {
    let used = usage.uses()?;
    let mut written = Vec::new();
    for rel in memory::entry_files(dir)? {
        if !Layer::of(&rel).is_some_and(Layer::is_kept) {
            continue;
        }
        let mut doc = memory::read_doc(dir, &rel)?;
        let ids: Vec<String> = doc
            .entries()
            .filter_map(|e| e.id().map(str::to_owned))
            .filter(|id| used.contains_key(id))
            .collect();
        if ids.is_empty() {
            continue;
        }
        for id in &ids {
            doc.update(id, |e| add_use(e, &used[id]));
        }
        doc.save(&dir.join(&rel))?;
        written.extend(ids);
    }
    usage.clear_uses(written.iter().map(String::as_str))
}

fn kept_file(rel: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(rel);
    anyhow::ensure!(
        Layer::of(&path).is_some_and(Layer::is_kept),
        "a dream may write only core, pending and long-term files, not {rel}"
    );
    Ok(path)
}

fn set_attrs(entry: &mut Entry, attrs: &BTreeMap<String, String>) -> anyhow::Result<()> {
    for (k, v) in attrs {
        if k == "id" {
            continue;
        }
        if v.is_empty() {
            entry.remove_attr(k);
        } else {
            entry.set(k, v.clone())?;
        }
    }
    Ok(())
}

/// Apply the edits. An edit that names a missing item or a file outside the
/// kept layers is skipped and reported.
pub fn apply(dir: &Path, edits: &[Edit], today: NaiveDate) -> anyhow::Result<Vec<String>> {
    let mut skipped = Vec::new();
    for edit in edits {
        if let Err(error) = apply_one(dir, edit, today) {
            skipped.push(format!(
                "{} {}: {error}",
                edit.op,
                edit.id.as_deref().unwrap_or("")
            ));
        }
    }
    Ok(skipped)
}

fn locate(dir: &Path, id: &str) -> anyhow::Result<PathBuf> {
    Snapshot::take(dir)?
        .entries
        .get(id)
        .filter(|l| l.layer.is_kept())
        .map(|l| l.file.clone())
        .ok_or_else(|| anyhow::anyhow!("no kept item {id}"))
}

fn apply_one(dir: &Path, edit: &Edit, today: NaiveDate) -> anyhow::Result<()> {
    match edit.op.as_str() {
        "add" => {
            let rel = kept_file(edit.file.as_deref().unwrap_or_default())?;
            let text = edit
                .text
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("an add needs text"))?;
            let path = dir.join(&rel);
            let mut doc = memory::doc_to_add_to(dir, &rel)?;
            let mut entry = Entry::new(text.trim()).with("id", memory::fresh_id(dir)?)?;
            set_attrs(&mut entry, &edit.attrs)?;
            if entry.src().is_none() {
                entry.set("src", "inferred")?;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            doc.push(entry)?;
            doc.save(&path)?;
        }
        "edit" => {
            let id = edit
                .id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("an edit needs an id"))?;
            let rel = locate(dir, id)?;
            let mut doc = memory::read_doc(dir, &rel)?;
            doc.update(id, |e| -> anyhow::Result<()> {
                if let Some(text) = edit.text.as_deref().filter(|t| !t.trim().is_empty()) {
                    e.set_text(text.trim());
                }
                set_attrs(e, &edit.attrs)
            })
            .ok_or_else(|| anyhow::anyhow!("no item {id}"))??;
            doc.save(&dir.join(&rel))?;
        }
        "move" => {
            let id = edit
                .id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("a move needs an id"))?;
            let to = kept_file(edit.file.as_deref().unwrap_or_default())?;
            let from = locate(dir, id)?;
            if from == to {
                return Ok(());
            }
            let mut source = memory::read_doc(dir, &from)?;
            let mut entry = source
                .remove(id)
                .ok_or_else(|| anyhow::anyhow!("no item {id}"))?;
            set_attrs(&mut entry, &edit.attrs)?;
            let path = dir.join(&to);
            let mut target = memory::doc_to_add_to(dir, &to)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            target.push(entry)?;
            target.save(&path)?;
            source.save(&dir.join(&from))?;
        }
        "confirm" => {
            let id = edit
                .id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("a confirm needs an id"))?;
            let rel = locate(dir, id)?;
            let mut doc = memory::read_doc(dir, &rel)?;
            doc.update(id, |e| -> anyhow::Result<()> {
                let ev = e.ev().unwrap_or(0) + 1;
                e.set("ev", ev.to_string())
            })
            .ok_or_else(|| anyhow::anyhow!("no item {id}"))??;
            doc.save(&dir.join(&rel))?;
        }
        "forget" => {
            let id = edit
                .id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("a forget needs an id"))?;
            let rel = locate(dir, id)?;
            memory::forget(
                dir,
                &rel,
                id,
                edit.reason.as_deref().unwrap_or("outdated"),
                today,
            )?;
        }
        other => anyhow::bail!("unknown op {other}"),
    }
    Ok(())
}

/// The episodic notes a dream reads, with their days: written and not yet
/// dreamt of.
pub fn waiting_notes(dir: &Path) -> anyhow::Result<Vec<(NaiveDate, Entry)>> {
    let mut notes = Vec::new();
    for date in memory::episodic_dates(dir)? {
        for e in memory::episodes(dir, date)? {
            if e.get("dreamt").is_none() && e.state() == Some(EpisodeState::Waiting) {
                notes.push((date, e));
            }
        }
    }
    Ok(notes)
}

/// Mark the notes the dream settled. A note it read but left is tried again,
/// until it has been left `MAX_TRIES` times; a note written while the model
/// was thinking was not read, and waits for the next dream.
fn mark_dreamt(
    dir: &Path,
    today: NaiveDate,
    read: &BTreeSet<String>,
    settled: &BTreeSet<String>,
) -> anyhow::Result<()> {
    const MAX_TRIES: u32 = 3;
    for date in memory::episodic_dates(dir)? {
        let rel = format!("{}/{date}.md", memory::EPISODIC);
        let mut doc = memory::read_doc(dir, &rel)?;
        let ids: Vec<String> = doc
            .entries()
            .filter_map(|e| e.id().map(str::to_owned))
            .filter(|id| read.contains(id))
            .collect();
        if ids.is_empty() {
            continue;
        }
        for id in ids {
            doc.update(&id, |e| {
                let tries = e
                    .get("tries")
                    .and_then(|t| t.parse::<u32>().ok())
                    .unwrap_or(0)
                    + 1;
                if settled.contains(&id) || tries >= MAX_TRIES {
                    e.set("dreamt", today.format("%Y-%m-%d").to_string())
                } else {
                    e.set("tries", tries.to_string())
                }
            });
        }
        doc.save(&dir.join(rel))?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub status: String,
    /// Whether the model's answer was applied, changing something or not.
    pub applied: bool,
    /// The night it was recorded as.
    pub night: i64,
    pub commit: Option<String>,
    pub summary: DreamSummary,
}

impl Outcome {
    /// A night that changed nothing, with why.
    fn nothing(status: String) -> Self {
        Self {
            status,
            applied: false,
            night: 0,
            commit: None,
            summary: DreamSummary::default(),
        }
    }
}

/// One recorded night.
#[derive(Debug, Clone, PartialEq)]
pub struct Night {
    pub id: i64,
    /// Unix seconds.
    pub at: i64,
    pub status: String,
    /// The memory commit the dream made; none when nothing was applied.
    pub commit: Option<String>,
    pub summary: DreamSummary,
}

/// The latest dreams, oldest first.
pub fn recent_dreams(store: &Store, limit: usize) -> anyhow::Result<Vec<Night>> {
    let mut nights = dreams_before(store, 0, limit)?;
    nights.reverse();
    Ok(nights)
}

/// The dreams older than the one with id `before`, newest first; `0` starts
/// from the newest.
pub fn dreams_before(store: &Store, before: i64, limit: usize) -> anyhow::Result<Vec<Night>> {
    let conn = store.conn();
    let mut stmt = conn.prepare(
        "SELECT id, at, status, commit_id, summary FROM dreams
         WHERE ?1 = 0 OR id < ?1 ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![before, limit as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, at, status, commit, summary) = row?;
        out.push(Night {
            id,
            at,
            status,
            commit,
            summary: serde_json::from_str(&summary).unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Dream once. `ask` puts the system prompt and the memory to the model and
/// returns its JSON answer. The night is recorded in the `dreams` table.
/// The memory is applied on its own: what follows a dream reads
/// [`Outcome::applied`] and the night it was recorded as.
pub async fn dream<F, Fut>(
    store: &Store,
    records: &Records,
    dir: &Path,
    today: NaiveDate,
    ask: F,
) -> anyhow::Result<Outcome>
where
    F: FnOnce(String, String) -> Fut,
    Fut: Future<Output = anyhow::Result<Value>>,
{
    commit_day(dir)?;
    let read = ids_of(waiting_notes(dir)?);
    let usage = Index::open(records.notebook())?;
    let core_lines = DreamSettings::load(store)?.core_lines;
    let mut prompt = prompt(dir, &usage)?;
    prompt.push_str(&talk(store, 1)?);
    let _ = writeln!(prompt, "\nToday is {today}.");
    let mut outcome = match ask(system(core_lines), prompt).await {
        Err(error) => Outcome::nothing(format!("見られなかった（{error}）")),
        Ok(answer) => {
            // What was written while the model thought is the day's, not the
            // dream's: a refused dream must not take it back.
            let _hold = records.notebook().hold()?;
            commit_day(dir)?;
            apply_answer(records, dir, &usage, today, &read, answer)?
        }
    };
    outcome.night = record_night(store, &outcome)?;
    Ok(outcome)
}

fn commit_day(dir: &Path) -> anyhow::Result<()> {
    notebook::commit_all(dir, &notebook::famulen(), "その日の記憶")?;
    Ok(())
}

fn ids_of(notes: Vec<(NaiveDate, Entry)>) -> BTreeSet<String> {
    notes
        .iter()
        .filter_map(|(_, e)| e.id().map(str::to_owned))
        .collect()
}

/// What was said on the console in the last days, to check against.
pub(crate) fn talk(store: &Store, days: i64) -> anyhow::Result<String> {
    let since = now() - days * 86_400;
    let talk: Vec<String> = ConsoleLog::new(store.clone())
        .recent(200)?
        .into_iter()
        .filter(|m| m.at >= since)
        .map(|m| {
            let who = match m.who {
                Who::Owner => "owner",
                Who::Aide => "secretary",
            };
            format!("{who}: {}", m.text.replace('\n', " "))
        })
        .collect();
    Ok(if talk.is_empty() {
        String::new()
    } else {
        format!(
            "\nWhat was said lately on the console, to check against:\n\n{}\n",
            talk.join("\n")
        )
    })
}

/// Apply the model's answer and commit it, or leave memory as it was when it
/// breaks a guard.
fn apply_answer(
    records: &Records,
    dir: &Path,
    usage: &Index,
    today: NaiveDate,
    read: &BTreeSet<String>,
    answer: Value,
) -> anyhow::Result<Outcome> {
    let mut plan: Plan = serde_json::from_value(answer)?;
    let before = Snapshot::take(dir)?;
    let proposed = plan.edits.len();
    let (moving, mut skipped) = take_records(records, dir, &mut plan.edits, &before)?;
    skipped.extend(apply(dir, &plan.edits, today)?);
    for reason in &skipped {
        tracing::debug!(%reason, "a dream edit was not applied");
    }
    let after = Snapshot::take(dir)?;
    if let Err(violation) = memory::check(&before, &after) {
        notebook::revert_to(dir, "HEAD")?;
        return Ok(Outcome::nothing(format!("当てなかった（{violation}）")));
    }
    // What one says may name another that is gone by now, and a record once
    // written stays though the memory comes back: nothing is written unless
    // all can be.
    for moved in &moving {
        if let Err(error) = records.check_add(Writer::Secretary, moved.kind, moved.patch.clone()) {
            notebook::revert_to(dir, "HEAD")?;
            return Err(error);
        }
    }
    if let Err(error) = write_all(records, moving) {
        notebook::revert_to(dir, "HEAD")?;
        return Err(error);
    }
    let settled: BTreeSet<String> = plan
        .settled
        .iter()
        .filter(|id| read.contains(*id))
        .cloned()
        .collect();
    mark_dreamt(dir, today, read, &settled)?;
    for edit in plan.edits.iter().filter(|e| e.op == "confirm") {
        if let Some(id) = edit
            .id
            .as_deref()
            .filter(|id| after.entries.contains_key(*id))
        {
            usage.record_use(id, today)?;
        }
    }
    settle_uses(dir, usage)?;
    let summary = DreamDiff::between(&before, &after).summary();
    let c = &summary.counts;
    let message = format!(
        "夢: 足した {} · 直した {} · 置き換えた {} · 忘れた {}",
        c.added, c.edited, c.replaced, c.archived
    );
    let commit = notebook::commit_all(dir, &notebook::famulen(), &message)?;
    let status = if skipped.is_empty() {
        format!("見た（変更の案 {proposed} 件）")
    } else {
        format!(
            "見た（変更の案 {proposed} 件のうち {} 件は当てなかった: {}）",
            skipped.len(),
            skipped.join("; ")
        )
    };
    Ok(Outcome {
        status,
        applied: true,
        night: 0,
        commit,
        summary,
    })
}

/// A memory on its way to the records.
struct Moving {
    kind: &'static str,
    title: String,
    text: String,
    patch: Patch,
}

impl Moving {
    /// Add the record, or log it at the open task that already says it, which
    /// may be one this dream made. The record added, if one was.
    fn write(self, held: &Held<'_>) -> anyhow::Result<Option<Record>> {
        let records = held.records();
        let workgroups = self.patch.workgroups.clone().unwrap_or_default();
        let known = match self.kind {
            TASKS => records.open_task_titled(&self.title, &workgroups)?,
            _ => None,
        };
        match known {
            Some(task) => {
                let line = format!("記憶にもあった：{}", self.text);
                // Logged already by a dream that did not stand after all.
                if log_lines(&task).iter().any(|l| l.text == line) {
                    return Ok(None);
                }
                let patch = Patch {
                    log: vec![line],
                    ..Patch::default()
                };
                held.change(Writer::Secretary, TASKS, &task.id, patch)?;
                Ok(None)
            }
            None => held.add(Writer::Secretary, self.kind, self.patch).map(Some),
        }
    }
}

/// Write every record or none: when one cannot be written, those added before
/// it are taken away, as their memories come back. Nobody else writes
/// records meanwhile.
fn write_all(records: &Records, moving: Vec<Moving>) -> anyhow::Result<()> {
    let held = records.hold()?;
    let mut added: Vec<Record> = Vec::new();
    for moved in moving {
        match moved.write(&held) {
            Ok(record) => added.extend(record),
            Err(error) => {
                for record in added.iter().rev() {
                    if let Err(error) = held.remove(Writer::Secretary, &record.kind, &record.id) {
                        tracing::warn!(%error, record = record.target(), "could not take back a record");
                    }
                }
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Turn every `to_record` edit into forgetting its item, and give the records
/// those items become. Records are committed as they are written, so they are
/// added only once the dream stands; what could not be added is refused now,
/// and its item stays in memory.
fn take_records(
    records: &Records,
    dir: &Path,
    edits: &mut Vec<Edit>,
    before: &Snapshot,
) -> anyhow::Result<(Vec<Moving>, Vec<String>)> {
    let mut moving = Vec::new();
    let mut skipped = Vec::new();
    let mut kept = Vec::new();
    let mut asked = BTreeSet::new();
    // What the records say is taken before any edit is applied.
    let edited: BTreeSet<Option<String>> = edits
        .iter()
        .filter(|e| e.op == "edit")
        .map(|e| e.id.clone())
        .collect();
    for mut edit in std::mem::take(edits) {
        if edit.op != "to_record" {
            kept.push(edit);
            continue;
        }
        if edited.contains(&edit.id) {
            skipped.push(format!(
                "to_record {}: edited in the same dream",
                edit.id.as_deref().unwrap_or("")
            ));
            continue;
        }
        if !asked.insert(edit.id.clone()) {
            skipped.push(format!(
                "to_record {}: asked twice",
                edit.id.as_deref().unwrap_or("")
            ));
            continue;
        }
        match to_record(records, dir, &edit, before) {
            Ok(moved) => {
                let kind = moved.kind;
                moving.push(moved);
                edit.op = "forget".into();
                edit.reason = Some(
                    if kind == TASKS {
                        "タスクに移した"
                    } else {
                        "メモに移した"
                    }
                    .into(),
                );
                kept.push(edit);
            }
            Err(error) => skipped.push(format!(
                "to_record {}: {error:#}",
                edit.id.as_deref().unwrap_or("")
            )),
        }
    }
    *edits = kept;
    Ok((moving, skipped))
}

/// The record one `to_record` edit asks for.
fn to_record(
    records: &Records,
    dir: &Path,
    edit: &Edit,
    before: &Snapshot,
) -> anyhow::Result<Moving> {
    let id = edit.id.as_deref().unwrap_or_default();
    let item = before
        .entries
        .get(id)
        .filter(|item| item.layer.is_kept())
        .ok_or_else(|| anyhow::anyhow!("no kept item"))?;
    let asked = edit
        .record
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("it does not say which record"))?;
    let kind = *AS_RECORD
        .iter()
        .find(|k| **k == asked.kind)
        .ok_or_else(|| anyhow::anyhow!("a memory cannot become one of {}", asked.kind))?;
    let text = whole_text(dir, item)?;
    let title = asked
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned);
    if kind == MEMOS {
        let title = title.unwrap_or_else(|| item.entry.body().chars().take(30).collect());
        let patch = Patch {
            notes: Some(text.clone()),
            workgroups: Some(item.entry.workgroup()),
            source: "夢で記憶から移した".into(),
            ..Patch::titled(&title)
        };
        records.check_add(Writer::Secretary, kind, patch.clone())?;
        return Ok(Moving {
            kind,
            title,
            text,
            patch,
        });
    }
    let title = title.ok_or_else(|| anyhow::anyhow!("a task needs a title"))?;
    let from = item
        .entry
        .from()
        .map(|d| format!("、{d} から"))
        .unwrap_or_default();
    let place = item.file.to_string_lossy().replace('\\', "/");
    let patch = task_patch(
        title.clone(),
        format!("{text}\n\n元は記憶の {place}（{id}{from}）"),
        asked.start.as_deref(),
        asked.due.as_deref(),
    );
    let patch = Patch {
        workgroups: Some(item.entry.workgroup()),
        source: "夢で記憶から移した".into(),
        ..patch
    };
    records.check_add(Writer::Secretary, kind, patch.clone())?;
    Ok(Moving {
        kind,
        title,
        text,
        patch,
    })
}

/// All an item says: for a long-term memory its prose too, which the dream is
/// not shown and the forgotten line does not keep.
fn whole_text(dir: &Path, item: &memory::Located) -> anyhow::Result<String> {
    let summary = item.entry.body();
    let doc = memory::read_doc(dir, &item.file)?;
    let prose = doc.long_text().unwrap_or_default().trim();
    Ok(if prose.is_empty() || prose == summary {
        summary.to_owned()
    } else if prose.contains(summary) {
        prose.to_owned()
    } else {
        format!("{summary}\n\n{prose}")
    })
}

/// A task of the owner's, as the dream makes one.
pub(crate) fn task_patch(
    title: String,
    notes: String,
    start: Option<&str>,
    due: Option<&str>,
) -> Patch {
    let mut patch = Patch {
        notes: Some(notes),
        ..Patch::titled(title)
    }
    .with("Assignee", OWNER);
    for (name, value) in [("Start", start), ("Due", due)] {
        if let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) {
            patch = patch.with(name, value);
        }
    }
    patch
}

fn record_night(store: &Store, outcome: &Outcome) -> anyhow::Result<i64> {
    let conn = store.conn();
    conn.execute(
        "INSERT INTO dreams (at, commit_id, status, summary) VALUES (?1, ?2, ?3, ?4)",
        params![
            now(),
            outcome.commit,
            outcome.status,
            serde_json::to_string(&outcome.summary)?
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Add what was done after the dream of a night to its record.
pub(crate) fn add_to_night(
    store: &Store,
    night: i64,
    changes: Vec<memory::Change>,
    status: &str,
) -> anyhow::Result<()> {
    let Some(mut was) = dreams_before(store, night + 1, 1)?
        .into_iter()
        .find(|n| n.id == night)
    else {
        return Ok(());
    };
    was.summary.changes.extend(changes);
    was.summary = DreamDiff {
        changes: was.summary.changes,
    }
    .summary();
    store.conn().execute(
        "UPDATE dreams SET status = ?2, summary = ?3 WHERE id = ?1",
        params![
            night,
            format!("{}。{status}", was.status),
            serde_json::to_string(&was.summary)?
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::testing::TempDir;

    /// A notebook's memory directory, with the records beside it.
    struct Memory {
        _tmp: TempDir,
        dir: PathBuf,
        records: crate::records::Records,
    }

    impl Memory {
        fn path(&self) -> &Path {
            &self.dir
        }
    }

    fn setup() -> (Memory, Store) {
        let tmp = TempDir::new("dreaming");
        let notebook = crate::notebook::open(tmp.path());
        crate::notebook::ensure(&notebook).unwrap();
        let store = Store::open_in_memory().unwrap();
        let memory = Memory {
            dir: notebook.memory_dir(),
            records: crate::records::Records::new(notebook),
            _tmp: tmp,
        };
        (memory, store)
    }

    /// A long-term memory file holding one item.
    fn long(id: &str, text: &str, extra: &str) -> String {
        format!("---\nid: {id}\nsummary: {text}\n{extra}---\n\n{text}\n")
    }

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 21).unwrap()
    }

    fn note(dir: &Path, text: &str) -> String {
        memory::append_episode(
            dir,
            day().and_hms_opt(10, 0, 0).unwrap(),
            text,
            EpisodeState::Waiting,
        )
        .unwrap()
        .id()
        .unwrap()
        .to_owned()
    }

    #[tokio::test]
    async fn a_dream_applies_the_edits_and_commits_them_once() {
        let (dir, store) = setup();
        let noted = note(dir.path(), "会議は午前がいいと言った");
        let outcome = dream(&store, &dir.records, dir.path(), day(), |_, prompt| async move {
            assert!(prompt.contains("会議は午前がいい"));
            Ok(json!({ "edits": [{ "op": "add", "file": "long/semantic/meetings.md", "text": "会議は午前に入れる", "attrs": { "src": "said" } }] , "settled": [noted] }))
        })
        .await
        .unwrap();
        assert_eq!(outcome.summary.counts.added, 1);
        assert!(outcome.commit.is_some());
        assert!(
            memory::read_file(dir.path(), "long/semantic/meetings.md")
                .unwrap()
                .contains("会議は午前に入れる")
        );
        let second = dream(
            &store,
            &dir.records,
            dir.path(),
            day(),
            |_, prompt| async move {
                assert!(prompt.contains("Episodic notes:\n\n(none)"), "{prompt}");
                Ok(json!({ "edits": [] }))
            },
        )
        .await
        .unwrap();
        assert_eq!(second.summary.counts.added, 0);
    }

    #[tokio::test]
    async fn the_dream_is_told_the_core_limit_the_owner_set() {
        let (dir, store) = setup();
        crate::dream_settings::DreamSettings {
            core_lines: 2,
            ..Default::default()
        }
        .save(&store)
        .unwrap();
        memory::write_file(
            dir.path(),
            "core.md",
            "# 心得\n- 一 {id: c1}\n- 二 {id: c2}\n- 三 {id: c3}\n".as_bytes(),
        )
        .unwrap();
        dream(
            &store,
            &dir.records,
            dir.path(),
            day(),
            |system, prompt| async move {
                assert!(system.contains("at most 2 lines"), "{system}");
                assert!(system.contains("Keep core within 2 lines"), "{system}");
                assert!(!system.contains("150"), "{system}");
                assert!(prompt.contains("Core (core.md, 3 lines):"), "{prompt}");
                Ok(json!({ "edits": [], "settled": [] }))
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_dream_that_shrinks_a_file_below_half_is_rolled_back() {
        let (dir, store) = setup();
        memory::write_file(dir.path(), "core.md", "# 心得\n- 一 {id: c1, src: inferred}\n- 二 {id: c2, src: inferred}\n- 三 {id: c3, src: inferred}\n".as_bytes()).unwrap();
        let outcome = dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [{ "op": "forget", "id": "c1" }, { "op": "forget", "id": "c2" }] }))
        })
        .await
        .unwrap();
        assert!(outcome.commit.is_none());
        assert!(
            outcome.status.contains("当てなかった"),
            "{}",
            outcome.status
        );
        assert_eq!(memory::core_entries(dir.path()).unwrap().len(), 3);
        assert!(memory::forgotten_months(dir.path()).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refused_dream_leaves_no_memo_and_keeps_what_was_noted_meanwhile() {
        let (dir, store) = setup();
        memory::write_file(dir.path(), "core.md", "# 心得\n- 一 {id: c1, src: inferred}\n- 二 {id: c2, src: inferred}\n- 三 {id: c3, src: inferred}\n".as_bytes()).unwrap();
        let path = dir.path().to_owned();
        let outcome = dream(&store, &dir.records, dir.path(), day(), |_, _| async move {
            note(&path, "考えている間の気づき");
            Ok(json!({ "edits": [{ "op": "to_record", "id": "c1", "record": { "kind": "memos", "title": "一" } }, { "op": "forget", "id": "c2" }] }))
        })
        .await
        .unwrap();
        assert!(
            outcome.status.contains("当てなかった"),
            "{}",
            outcome.status
        );
        assert!(dir.records.list(MEMOS).unwrap().is_empty());
        assert_eq!(waiting_notes(dir.path()).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn forgetting_moves_an_item_and_a_failed_model_records_the_night() {
        let (dir, store) = setup();
        for (rel, id, text) in [
            ("long/prospective/call.md", "p1", "来週電話する"),
            ("long/prospective/papers.md", "p2", "資料を送る"),
        ] {
            memory::write_file(
                dir.path(),
                rel,
                long(id, text, "src: inferred\n").as_bytes(),
            )
            .unwrap();
        }
        let outcome = dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [{ "op": "forget", "id": "p1", "reason": "済んだ" }] }))
        })
        .await
        .unwrap();
        assert_eq!(outcome.summary.counts.archived, 1);
        assert_eq!(memory::forgotten(dir.path(), "2026-11").unwrap().len(), 1);
        let failed = dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            anyhow::bail!("no model")
        })
        .await
        .unwrap();
        assert!(failed.status.contains("見られなかった"));
        let nights: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM dreams", [], |r| r.get(0))
            .unwrap();
        assert_eq!(nights, 2);
    }

    #[tokio::test]
    async fn a_reason_written_in_words_forgets_the_item_and_is_kept_as_written() {
        let (dir, store) = setup();
        for (rel, id, text) in [
            ("long/prospective/call.md", "p1", "来週電話する"),
            ("long/prospective/papers.md", "p2", "資料を送る"),
        ] {
            memory::write_file(
                dir.path(),
                rel,
                long(id, text, "src: inferred\n").as_bytes(),
            )
            .unwrap();
        }
        let outcome = dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [{ "op": "forget", "id": "p1", "reason": "e2e で {済んだ} done" }] }))
        })
        .await
        .unwrap();
        assert_eq!(outcome.summary.counts.archived, 1);
        let gone = memory::forgotten(dir.path(), "2026-11").unwrap();
        assert_eq!(gone[0].get("reason"), Some("e2e で {済んだ} done"));
    }

    #[tokio::test]
    async fn recent_dreams_tell_each_nights_commit_and_why_an_item_changed() {
        let (dir, store) = setup();
        for (rel, id, text) in [
            ("long/prospective/call.md", "p1", "来週電話する"),
            ("long/prospective/papers.md", "p2", "資料を送る"),
        ] {
            memory::write_file(
                dir.path(),
                rel,
                long(id, text, "src: inferred\n").as_bytes(),
            )
            .unwrap();
        }
        let kept = dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [{ "op": "forget", "id": "p1", "reason": "済んだ" }] }))
        })
        .await
        .unwrap();
        dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            anyhow::bail!("no model")
        })
        .await
        .unwrap();

        let nights = recent_dreams(&store, 14).unwrap();

        assert_eq!(nights.len(), 2);
        assert_eq!(nights[0].commit, kept.commit);
        assert!(nights[0].commit.is_some());
        assert_eq!(
            nights[0].summary.changes[0].reason.as_deref(),
            Some("済んだ")
        );
        assert_eq!(nights[1].commit, None);
        assert!(nights[0].id < nights[1].id);
    }

    #[tokio::test]
    async fn dreams_before_a_cursor_are_only_the_older_ones() {
        let (dir, store) = setup();
        for n in 1..=3 {
            dream(
                &store,
                &dir.records,
                dir.path(),
                day(),
                move |_, _| async move { anyhow::bail!("night {n}") },
            )
            .await
            .unwrap();
        }
        let newest = dreams_before(&store, 0, 1).unwrap();
        let older = dreams_before(&store, newest[0].id, 10).unwrap();
        assert_eq!(newest[0].status, "見られなかった（night 3）");
        assert_eq!(
            older.iter().map(|n| n.status.as_str()).collect::<Vec<_>>(),
            ["見られなかった（night 2）", "見られなかった（night 1）"]
        );
    }

    #[tokio::test]
    async fn a_record_in_memory_moves_to_a_memo_and_a_confirm_raises_evidence() {
        let (dir, store) = setup();
        for (rel, id, text, extra) in [
            (
                "long/semantic/yamada-phone.md",
                "p1",
                "山田さんの社用電話は 03-0000-0000",
                "src: said\n",
            ),
            (
                "long/semantic/yamada-early.md",
                "p2",
                "山田さんは朝が早い",
                "src: inferred\nev: 1\n",
            ),
            (
                "long/semantic/yamada-tea.md",
                "p3",
                "山田さんは紅茶が好き",
                "src: inferred\n",
            ),
        ] {
            memory::write_file(dir.path(), rel, long(id, text, extra).as_bytes()).unwrap();
        }
        let outcome = dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [{ "op": "to_record", "id": "p1", "record": { "kind": "memos", "title": "山田さんの社用電話" } }, { "op": "confirm", "id": "p2" }] }))
        })
        .await
        .unwrap();
        assert!(outcome.commit.is_some(), "{}", outcome.status);
        let memos = dir.records.list(crate::records::MEMOS).unwrap();
        assert!(memos[0].notes().contains("03-0000-0000"));
        assert!(!dir.path().join("long/semantic/yamada-phone.md").exists());
        let kept = memory::read_file(dir.path(), "long/semantic/yamada-early.md").unwrap();
        assert!(kept.contains("ev: 2"), "{kept}");
        assert!(
            kept.contains("uses: ") && kept.contains(&format!("last: {}", day())),
            "{kept}"
        );
        let index = famulen_notebook::index::Index::open(dir.records.notebook()).unwrap();
        assert!(index.uses().unwrap().is_empty());
        index.record_use("p2", day()).unwrap();
        settle_uses(dir.path(), &index).unwrap();
        let kept = memory::read_file(dir.path(), "long/semantic/yamada-early.md").unwrap();
        assert!(
            kept.contains("uses: 2") || kept.contains("uses: '2'"),
            "{kept}"
        );
    }

    fn prospective(dir: &Path, items: &[(&str, &str)]) {
        for (id, text) in items {
            memory::write_file(
                dir,
                format!("long/prospective/{id}.md"),
                long(id, text, "src: said\nfrom: 2026-11-20\nworkgroup: [work]\n").as_bytes(),
            )
            .unwrap();
        }
    }

    #[tokio::test]
    async fn something_to_do_in_memory_becomes_a_task_of_the_owners_with_where_it_came_from() {
        let (dir, store) = setup();
        prospective(
            dir.path(),
            &[("p1", "来週田中さんに電話する"), ("p2", "残る")],
        );
        let outcome = dream(
            &store,
            &dir.records,
            dir.path(),
            day(),
            |system, _| async move {
                assert!(system.contains("it is a task, not a memo"), "{system}");
                Ok(json!({ "edits": [{ "op": "to_record", "id": "p1", "record":
                { "kind": "tasks", "title": "田中さんに電話する", "due": "2026-11-28" } }] }))
            },
        )
        .await
        .unwrap();

        assert!(outcome.applied, "{}", outcome.status);
        assert!(dir.records.list(MEMOS).unwrap().is_empty());
        let tasks = dir.records.list(TASKS).unwrap();
        assert_eq!(tasks[0].title(), "田中さんに電話する");
        assert_eq!(tasks[0].body.value("Due").as_deref(), Some("2026-11-28"));
        assert_eq!(tasks[0].body.value("Assignee").as_deref(), Some(OWNER));
        assert_eq!(tasks[0].workgroups(), ["work"]);
        assert!(tasks[0].notes().contains("来週田中さんに電話する"));
        assert!(tasks[0].notes().contains("long/prospective/p1.md"));
        assert_eq!(
            outcome.summary.changes[0].reason.as_deref(),
            Some("タスクに移した")
        );
    }

    #[tokio::test]
    async fn a_memory_that_becomes_a_record_takes_its_prose_along() {
        let (dir, store) = setup();
        prospective(dir.path(), &[("p2", "残る")]);
        memory::write_file(
            dir.path(),
            "long/prospective/p1.md",
            "---\nid: p1\nsummary: 棚を直す\n---\n\nねじは 4 mm。\n\n- 道具箱にある\n".as_bytes(),
        )
        .unwrap();
        dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [{ "op": "to_record", "id": "p1", "record": { "kind": "tasks", "title": "棚を直す" } }] }))
        })
        .await
        .unwrap();
        let notes = dir.records.list(TASKS).unwrap()[0].notes();
        assert!(
            notes.starts_with("棚を直す\n\nねじは 4 mm。\n\n- 道具箱にある"),
            "{notes}"
        );
    }

    #[tokio::test]
    async fn a_record_of_no_kind_offered_or_with_a_bad_date_is_refused_and_stays_in_memory() {
        let (dir, store) = setup();
        prospective(
            dir.path(),
            &[("p1", "一"), ("p2", "二"), ("p3", "三"), ("p4", "四")],
        );
        let outcome = dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [
                { "op": "to_record", "id": "p1", "record": { "kind": "events", "title": "x" } },
                { "op": "to_record", "id": "p2", "record": { "kind": "tasks", "title": "x", "due": "来週" } },
                { "op": "to_record", "id": "p3", "record": { "kind": "tasks", "title": " " } },
                { "op": "to_record", "id": "p4" },
            ] }))
        })
        .await
        .unwrap();

        assert!(
            outcome.status.contains("4 件は当てなかった"),
            "{}",
            outcome.status
        );
        assert!(dir.records.list(TASKS).unwrap().is_empty());
        let kept = Snapshot::take(dir.path()).unwrap().entries;
        assert_eq!(kept.len(), 4);
        assert!(kept.values().all(|l| l.layer.is_kept()));
    }

    #[tokio::test]
    async fn a_memory_an_open_task_already_says_goes_to_its_log_instead_of_a_second_task() {
        let (dir, store) = setup();
        prospective(
            dir.path(),
            &[("p1", "請求書を送る。月末まで"), ("p2", "残る")],
        );
        dir.records
            .add(
                Writer::Owner,
                TASKS,
                Patch {
                    workgroups: Some(vec!["work".into()]),
                    ..Patch::titled("請求書を送る")
                },
            )
            .unwrap();
        dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [{ "op": "to_record", "id": "p1", "record": { "kind": "tasks", "title": "請求書を 送る" } }] }))
        })
        .await
        .unwrap();

        let tasks = dir.records.list(TASKS).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(
            crate::records::log_lines(&tasks[0])[0]
                .text
                .contains("月末まで")
        );
    }

    #[tokio::test]
    async fn memories_that_cannot_all_become_records_leave_none_behind() {
        let (dir, store) = setup();
        prospective(
            dir.path(),
            &[
                ("p1", "山田さんの電話"),
                ("p2", "番号は [[memory/long/prospective/p1]] にある"),
                ("p3", "残る"),
                ("p4", "残る二"),
            ],
        );
        let tried = dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [
                { "op": "to_record", "id": "p1", "record": { "kind": "memos", "title": "一" } },
                { "op": "to_record", "id": "p2", "record": { "kind": "memos", "title": "二" } },
            ] }))
        })
        .await;
        assert!(tried.is_err());
        assert!(dir.records.list(MEMOS).unwrap().is_empty());
        assert!(dir.path().join("long/prospective/p1.md").exists());
    }

    #[tokio::test]
    async fn a_record_already_added_is_taken_back_when_a_later_one_cannot_be() {
        let (dir, store) = setup();
        prospective(
            dir.path(),
            &[
                ("p1", "田中さんの電話"),
                ("p2", "[[田中]] に電話する"),
                ("p3", "残る"),
                ("p4", "残る二"),
            ],
        );
        dir.records
            .add(
                Writer::Owner,
                crate::records::PARTIES,
                Patch::titled("田中"),
            )
            .unwrap();
        let tried = dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [
                { "op": "to_record", "id": "p1", "record": { "kind": "memos", "title": "田中" } },
                { "op": "to_record", "id": "p2", "record": { "kind": "memos", "title": "二" } },
            ] }))
        })
        .await;
        assert!(tried.is_err());
        assert!(dir.records.list(MEMOS).unwrap().is_empty());
        assert!(dir.path().join("long/prospective/p1.md").exists());
    }

    #[tokio::test]
    async fn a_memory_edited_in_the_same_dream_stays_and_does_not_become_what_it_said_before() {
        let (dir, store) = setup();
        prospective(dir.path(), &[("p1", "送付先は旧住所"), ("p2", "残る")]);
        dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [
                { "op": "edit", "id": "p1", "text": "送付先は新住所" },
                { "op": "to_record", "id": "p1", "record": { "kind": "memos", "title": "送付先" } },
            ] }))
        })
        .await
        .unwrap();
        assert!(dir.records.list(MEMOS).unwrap().is_empty());
        let kept = memory::read_file(dir.path(), "long/prospective/p1.md").unwrap();
        assert!(kept.contains("新住所"), "{kept}");
    }

    #[tokio::test]
    async fn memories_of_one_title_become_one_task_that_holds_them_all() {
        let (dir, store) = setup();
        prospective(
            dir.path(),
            &[
                ("p1", "請求書を送る。月末まで"),
                ("p2", "請求書は田中さん宛"),
                ("p3", "残る"),
                ("p4", "残る二"),
            ],
        );
        dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [
                { "op": "to_record", "id": "p1", "record": { "kind": "tasks", "title": "請求書を送る" } },
                { "op": "to_record", "id": "p2", "record": { "kind": "tasks", "title": "請求書を 送る" } },
            ] }))
        })
        .await
        .unwrap();

        let tasks = dir.records.list(TASKS).unwrap();
        assert_eq!(tasks.len(), 1);
        for said in ["月末まで", "田中さん宛"] {
            assert!(tasks[0].body.text().contains(said), "{said}");
        }
    }

    #[tokio::test]
    async fn a_night_says_what_was_done_after_its_dream() {
        let (dir, store) = setup();
        let first = dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            Ok(json!({ "edits": [], "settled": [] }))
        })
        .await
        .unwrap();
        dream(&store, &dir.records, dir.path(), day(), |_, _| async {
            anyhow::bail!("later")
        })
        .await
        .unwrap();
        let change = memory::Change {
            op: memory::Op::Doubted,
            id: "x".into(),
            file: "records/memos/x.md".into(),
            text: "x".into(),
            previous: None,
            from_file: None,
            replaces: None,
            reason: Some("古い".into()),
        };

        add_to_night(&store, first.night, vec![change], "記録も見た").unwrap();

        let nights = recent_dreams(&store, 5).unwrap();
        assert!(
            nights[0].status.ends_with("。記録も見た"),
            "{}",
            nights[0].status
        );
        assert_eq!(nights[0].summary.counts.doubted, 1);
        assert!(nights[1].summary.changes.is_empty());
    }

    #[tokio::test]
    async fn a_note_written_while_the_model_thinks_waits_for_the_next_dream() {
        let (dir, store) = setup();
        note(dir.path(), "前からある気づき");
        let path = dir.path().to_owned();
        dream(&store, &dir.records, dir.path(), day(), |_, _| async move {
            let settled = ids_of(waiting_notes(&path).unwrap());
            note(&path, "考えている間の気づき");
            Ok(json!({ "edits": [], "settled": settled }))
        })
        .await
        .unwrap();
        dream(
            &store,
            &dir.records,
            dir.path(),
            day(),
            |_, prompt| async move {
                assert!(prompt.contains("考えている間の気づき"), "{prompt}");
                assert!(!prompt.contains("前からある気づき"), "{prompt}");
                Ok(json!({ "edits": [] }))
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_note_the_dream_left_is_shown_again_until_it_was_left_three_times() {
        let (dir, store) = setup();
        note(dir.path(), "山田さんはアクメ商事の人");
        for night in 0..3 {
            dream(
                &store,
                &dir.records,
                dir.path(),
                day(),
                |_, prompt| async move {
                    assert!(
                        prompt.contains("山田さんはアクメ商事の人"),
                        "night {night}: {prompt}"
                    );
                    Ok(json!({ "edits": [], "settled": [] }))
                },
            )
            .await
            .unwrap();
        }
        assert!(waiting_notes(dir.path()).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_dream_folds_the_owners_word_on_the_lookout_into_semantic_memory() {
        let (dir, store) = setup();
        note(
            dir.path(),
            "見張りの判断「週末セール」（秘書へ · 知らせる）はあなたにとって要らなかった",
        );
        dream(
            &store,
            &dir.records,
            dir.path(),
            day(),
            |system, prompt| async move {
                assert!(system.contains("The lookout judges what concerns the owner by semantic memory and records alone"));
                assert!(prompt.contains("週末セール"), "{prompt}");
                assert!(!prompt.contains("The lookout's list of interests"), "{prompt}");
                Ok(json!({
                    "edits": [{ "op": "add", "file": "long/semantic/sales.md", "text": "セールの知らせは要らない", "attrs": { "src": "inferred" } }],
                    "interests": "手で書いた関心",
                    "settled": ["m1"]
                }))
            },
        )
        .await
        .unwrap();
        let semantic: Vec<String> = memory::long_term(dir.path())
            .unwrap()
            .into_iter()
            .filter(|g| g.group == memory::LongGroup::Semantic)
            .flat_map(|g| g.files)
            .flat_map(|f| f.entries)
            .map(|e| e.body().to_owned())
            .collect();
        assert!(
            semantic
                .iter()
                .any(|b| b.contains("セールの知らせは要らない")),
            "{semantic:?}"
        );
        assert_eq!(store.state("lookout.interests").unwrap(), None);
    }

    #[test]
    fn a_dream_cannot_write_outside_the_kept_layers() {
        let (dir, _) = setup();
        let skipped = apply(
            dir.path(),
            &[Edit {
                op: "add".into(),
                id: None,
                file: Some("../x.md".into()),
                text: Some("x".into()),
                attrs: BTreeMap::new(),
                reason: None,
                record: None,
            }],
            day(),
        )
        .unwrap();
        assert_eq!(skipped.len(), 1);
    }
}
