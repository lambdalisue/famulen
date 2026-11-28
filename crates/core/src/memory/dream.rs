//! Telling what a dream changed, and refusing changes that lose memory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{self as layers, Entry, Layer};

/// An item and where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub file: PathBuf,
    pub layer: Layer,
    pub entry: Entry,
}

/// Every item with an id in core, pending, long-term and forgotten memory,
/// and how many items each of those files holds. The inbox and working
/// memory are the dream's input, not what it keeps, so they are left out.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub entries: BTreeMap<String, Located>,
    pub counts: BTreeMap<PathBuf, usize>,
}

impl Snapshot {
    pub fn take(dir: &Path) -> anyhow::Result<Self> {
        let mut snapshot = Snapshot::default();
        for file in layers::entry_files(dir)? {
            let Some(layer) = Layer::of(&file) else {
                continue;
            };
            if layer == Layer::Episodic {
                continue;
            }
            let doc = layers::read_doc(dir, &file)?;
            snapshot.counts.insert(file.clone(), doc.entries().count());
            for entry in doc.entries() {
                if let Some(id) = entry.id() {
                    snapshot.entries.insert(
                        id.to_string(),
                        Located {
                            file: file.clone(),
                            layer,
                            entry: entry.clone(),
                        },
                    );
                }
            }
        }
        Ok(snapshot)
    }
}

/// What a dream did to one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// A new item in core or long-term memory.
    Added,
    /// The same item with its text changed.
    Edited,
    /// A new item that names the item it replaces in `supersedes=`.
    Replaced,
    /// An item moved to forgotten memory.
    Archived,
    /// The same text, used or backed by more evidence than before.
    Confirmed,
    /// A new judgement left pending.
    PendingAdded,
    /// The same text moved between core, pending and long-term memory.
    Moved,
    /// An item gone from every layer.
    Deleted,
    /// A memo that became a task.
    TaskMade,
    /// A record marked as one that may no longer be true.
    Doubted,
    /// A note nothing was kept of.
    Discarded,
}

/// One change, with the text a person reads in the console.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub op: Op,
    pub id: String,
    /// Where the item is after the dream (before it, for a deletion).
    pub file: String,
    pub text: String,
    /// The text before an edit or a replacement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// Where the item was before a move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_file: Option<String>,
    /// The id a replacement supersedes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
    /// The item's `reason=` attribute, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The notes it rests on, as they were: the inbox lets go of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub basis: Vec<Basis>,
}

/// A note a change rests on.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Basis {
    pub id: String,
    /// The day it was written.
    pub date: String,
    pub time: String,
    pub text: String,
    pub src: String,
    /// Where it came from, as its `origin=` attribute says.
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DreamDiff {
    pub changes: Vec<Change>,
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl DreamDiff {
    /// Classify every id that differs between the two snapshots.
    ///
    /// A replacement is a new item in core or long-term memory whose
    /// `supersedes=` names an item that existed before; what became of the
    /// old item (a `to=` date, or a move to forgotten memory) is part of that
    /// replacement and is not counted again.
    pub fn between(before: &Snapshot, after: &Snapshot) -> Self {
        let mut changes = Vec::new();
        let mut superseded = Vec::new();
        for (id, now) in &after.entries {
            if before.entries.contains_key(id) {
                continue;
            }
            let change = |op| Change {
                op,
                id: id.clone(),
                file: path_string(&now.file),
                text: now.entry.text().to_string(),
                previous: None,
                from_file: None,
                replaces: None,
                reason: now.entry.get("reason").map(str::to_string),
                basis: Vec::new(),
            };
            match now.layer {
                Layer::Core | Layer::Long(_) => {
                    match now
                        .entry
                        .get("supersedes")
                        .and_then(|old| before.entries.get_key_value(old))
                    {
                        Some((old_id, old)) => {
                            superseded.push(old_id.clone());
                            changes.push(Change {
                                previous: Some(old.entry.text().to_string()),
                                replaces: Some(old_id.clone()),
                                ..change(Op::Replaced)
                            });
                        }
                        None => changes.push(change(Op::Added)),
                    }
                }
                Layer::Pending => changes.push(change(Op::PendingAdded)),
                Layer::Episodic | Layer::Working | Layer::Forgotten => {}
            }
        }
        for (id, was) in &before.entries {
            if superseded.contains(id) {
                continue;
            }
            let base = Change {
                op: Op::Deleted,
                id: id.clone(),
                file: path_string(&was.file),
                text: was.entry.text().to_string(),
                previous: None,
                from_file: None,
                replaces: None,
                reason: None,
                basis: Vec::new(),
            };
            let Some(now) = after.entries.get(id) else {
                changes.push(base);
                continue;
            };
            let change = Change {
                file: path_string(&now.file),
                text: now.entry.text().to_string(),
                reason: now.entry.get("reason").map(str::to_string),
                ..base
            };
            if now.layer == Layer::Forgotten {
                if was.layer != Layer::Forgotten {
                    changes.push(Change {
                        op: Op::Archived,
                        from_file: Some(path_string(&was.file)),
                        ..change
                    });
                }
            } else if now.entry.text() != was.entry.text() {
                changes.push(Change {
                    op: Op::Edited,
                    previous: Some(was.entry.text().to_string()),
                    ..change
                });
            } else if now.layer != was.layer {
                changes.push(Change {
                    op: Op::Moved,
                    from_file: Some(path_string(&was.file)),
                    ..change
                });
            } else if grew(&was.entry, &now.entry) {
                changes.push(Change {
                    op: Op::Confirmed,
                    ..change
                });
            }
        }
        changes.sort_by(|a, b| a.id.cmp(&b.id).then(a.op.cmp(&b.op)));
        DreamDiff { changes }
    }

    pub fn summary(&self) -> DreamSummary {
        let mut counts = Counts::default();
        for change in &self.changes {
            *counts.slot(change.op) += 1;
        }
        DreamSummary {
            counts,
            changes: self.changes.clone(),
        }
    }
}

fn grew(was: &Entry, now: &Entry) -> bool {
    now.ev().unwrap_or(0) > was.ev().unwrap_or(0)
}

/// How many changes of each kind one dream made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Counts {
    pub added: usize,
    pub edited: usize,
    pub replaced: usize,
    pub archived: usize,
    pub confirmed: usize,
    pub pending_added: usize,
    pub moved: usize,
    pub deleted: usize,
    pub task_made: usize,
    pub doubted: usize,
    pub discarded: usize,
}

impl Counts {
    fn slot(&mut self, op: Op) -> &mut usize {
        match op {
            Op::Added => &mut self.added,
            Op::Edited => &mut self.edited,
            Op::Replaced => &mut self.replaced,
            Op::Archived => &mut self.archived,
            Op::Confirmed => &mut self.confirmed,
            Op::PendingAdded => &mut self.pending_added,
            Op::Moved => &mut self.moved,
            Op::Deleted => &mut self.deleted,
            Op::TaskMade => &mut self.task_made,
            Op::Doubted => &mut self.doubted,
            Op::Discarded => &mut self.discarded,
        }
    }
}

/// One night's result, as kept for the console.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DreamSummary {
    pub counts: Counts,
    pub changes: Vec<Change>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{CORE, PENDING, ensure, forget, write_file};
    use crate::notebook::testing::TempDir;
    use chrono::NaiveDate;

    fn memory(name: &str, files: &[(&str, &str)]) -> TempDir {
        let tmp = TempDir::new(name);
        ensure(tmp.path()).unwrap();
        for (rel, text) in files {
            write_file(tmp.path(), rel, text.as_bytes()).unwrap();
        }
        tmp
    }

    /// A long-term memory file holding one item.
    fn long(id: &str, text: &str) -> String {
        format!("---\nid: {id}\nsummary: {text}\n---\n")
    }

    fn ops(diff: &DreamDiff) -> Vec<(&str, Op)> {
        diff.changes.iter().map(|c| (c.id.as_str(), c.op)).collect()
    }

    #[test]
    fn a_snapshot_leaves_episodic_and_working_memory_out() {
        let tmp = memory(
            "snapshot",
            &[
                (CORE, "- a {id: m1}\n"),
                (
                    "episodic/2026-11-21.md",
                    "- 09:00 b {id: m2, state: waiting}\n",
                ),
                ("working/x.md", "- c {id: m3}\n"),
                ("forgotten/2026-11.md", "- d {id: m4}\n"),
            ],
        );

        let snapshot = Snapshot::take(tmp.path()).unwrap();

        let ids: Vec<&str> = snapshot.entries.keys().map(String::as_str).collect();
        assert_eq!(ids, vec!["m1", "m4"]);
        assert_eq!(snapshot.counts.get(Path::new(CORE)), Some(&1));
        assert_eq!(snapshot.counts.get(Path::new(PENDING)), Some(&0));
    }

    #[test]
    fn new_items_are_added_or_pending_by_their_layer() {
        let tmp = memory("diff-added", &[]);
        let before = Snapshot::take(tmp.path()).unwrap();
        write_file(tmp.path(), CORE, b"- a {id: m1}\n").unwrap();
        write_file(
            tmp.path(),
            "long/semantic/food.md",
            long("m2", "b").as_bytes(),
        )
        .unwrap();
        write_file(tmp.path(), PENDING, b"- c {id: m3, need: 2}\n").unwrap();

        let diff = DreamDiff::between(&before, &Snapshot::take(tmp.path()).unwrap());

        assert_eq!(
            ops(&diff),
            vec![
                ("m1", Op::Added),
                ("m2", Op::Added),
                ("m3", Op::PendingAdded)
            ]
        );
        assert_eq!(diff.changes[1].file, "long/semantic/food.md");
    }

    #[test]
    fn a_changed_text_under_the_same_id_is_an_edit_with_the_previous_text() {
        let tmp = memory("diff-edited", &[(CORE, "- old {id: m1}\n")]);
        let before = Snapshot::take(tmp.path()).unwrap();
        write_file(tmp.path(), CORE, b"- new {id: m1}\n").unwrap();

        let diff = DreamDiff::between(&before, &Snapshot::take(tmp.path()).unwrap());

        assert_eq!(ops(&diff), vec![("m1", Op::Edited)]);
        assert_eq!(diff.changes[0].previous.as_deref(), Some("old"));
        assert_eq!(diff.changes[0].text, "new");
    }

    #[test]
    fn more_evidence_with_the_same_text_is_a_confirmation() {
        let tmp = memory(
            "diff-confirmed",
            &[(CORE, "- b {id: m2, ev: 1}\n- c {id: m3}\n")],
        );
        let before = Snapshot::take(tmp.path()).unwrap();
        write_file(
            tmp.path(),
            CORE,
            b"- b {id: m2, ev: 2}\n- c {id: m3, note: x}\n",
        )
        .unwrap();

        let diff = DreamDiff::between(&before, &Snapshot::take(tmp.path()).unwrap());

        assert_eq!(ops(&diff), vec![("m2", Op::Confirmed)]);
    }

    #[test]
    fn an_item_moved_to_forgotten_memory_is_archived_with_its_reason() {
        let tmp = memory("diff-archived", &[(CORE, "- a {id: m1}\n- b {id: m2}\n")]);
        let before = Snapshot::take(tmp.path()).unwrap();
        forget(
            tmp.path(),
            CORE,
            "m1",
            "unused",
            NaiveDate::from_ymd_opt(2026, 11, 21).unwrap(),
        )
        .unwrap();

        let diff = DreamDiff::between(&before, &Snapshot::take(tmp.path()).unwrap());

        assert_eq!(ops(&diff), vec![("m1", Op::Archived)]);
        assert_eq!(diff.changes[0].file, "forgotten/2026-11.md");
        assert_eq!(diff.changes[0].from_file.as_deref(), Some("core.md"));
        assert_eq!(diff.changes[0].reason.as_deref(), Some("unused"));
    }

    #[test]
    fn a_new_item_naming_an_old_one_in_supersedes_is_a_replacement_that_absorbs_the_old_ones_archival()
     {
        let tmp = memory(
            "diff-replaced",
            &[(CORE, "- 朝に連絡 {id: m1, src: said}\n- x {id: m2}\n")],
        );
        let before = Snapshot::take(tmp.path()).unwrap();
        forget(
            tmp.path(),
            CORE,
            "m1",
            "superseded",
            NaiveDate::from_ymd_opt(2026, 11, 21).unwrap(),
        )
        .unwrap();
        let core = std::fs::read_to_string(tmp.path().join(CORE)).unwrap();
        write_file(
            tmp.path(),
            CORE,
            format!("{core}- 夕方に連絡 {{id: m3, supersedes: m1}}\n").as_bytes(),
        )
        .unwrap();

        let diff = DreamDiff::between(&before, &Snapshot::take(tmp.path()).unwrap());

        assert_eq!(ops(&diff), vec![("m3", Op::Replaced)]);
        assert_eq!(diff.changes[0].replaces.as_deref(), Some("m1"));
        assert_eq!(diff.changes[0].previous.as_deref(), Some("朝に連絡"));
    }

    #[test]
    fn a_replacement_may_leave_the_old_item_in_place_with_an_end_date() {
        let tmp = memory("diff-replaced-to", &[(CORE, "- a {id: m1}\n")]);
        let before = Snapshot::take(tmp.path()).unwrap();
        write_file(
            tmp.path(),
            CORE,
            b"- a {id: m1, to: 2026-11-20}\n- b {id: m2, supersedes: m1}\n",
        )
        .unwrap();

        let diff = DreamDiff::between(&before, &Snapshot::take(tmp.path()).unwrap());

        assert_eq!(ops(&diff), vec![("m2", Op::Replaced)]);
    }

    #[test]
    fn the_same_text_in_another_layer_is_a_move_and_a_vanished_item_a_deletion() {
        let tmp = memory("diff-moved", &[(PENDING, "- a {id: m1}\n- b {id: m2}\n")]);
        let before = Snapshot::take(tmp.path()).unwrap();
        write_file(tmp.path(), PENDING, b"").unwrap();
        write_file(tmp.path(), "long/semantic/x.md", long("m1", "a").as_bytes()).unwrap();

        let diff = DreamDiff::between(&before, &Snapshot::take(tmp.path()).unwrap());

        assert_eq!(ops(&diff), vec![("m1", Op::Moved), ("m2", Op::Deleted)]);
        assert_eq!(diff.changes[0].from_file.as_deref(), Some("pending.md"));
    }

    #[test]
    fn the_summary_counts_each_op_and_round_trips_through_json() {
        let tmp = memory("summary", &[(CORE, "- a {id: m1}\n")]);
        let before = Snapshot::take(tmp.path()).unwrap();
        write_file(
            tmp.path(),
            CORE,
            b"- a2 {id: m1}\n- b {id: m2}\n- c {id: m3}\n",
        )
        .unwrap();

        let summary = DreamDiff::between(&before, &Snapshot::take(tmp.path()).unwrap()).summary();

        assert_eq!(summary.counts.added, 2);
        assert_eq!(summary.counts.edited, 1);
        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["counts"]["pending_added"], 0);
        assert_eq!(json["changes"][0]["op"], "edited");
        assert_eq!(
            serde_json::from_value::<DreamSummary>(json).unwrap(),
            summary
        );
    }
}
