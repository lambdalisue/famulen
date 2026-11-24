//! Working memory that may have outlived its work. It is told of, never
//! closed: only the secretary knows whether the work went on elsewhere.

use std::path::Path;
use std::time::{Duration, SystemTime};

use famulen_notebook::records::record_of;

use super::{self as layers, WorkingMemory};
use crate::records::{Record, Records, TASKS, is_over};

/// How long work in progress may go without being written before it is asked
/// about.
const QUIET_DAYS: u64 = 7;

/// A working memory that may be closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leftover {
    /// Its file name without `.md`.
    pub name: String,
    pub why: String,
}

/// Whether a working memory speaks of a task: it links to it, or has one of
/// `names`, such as its code, in its text.
pub fn speaks_of(memory: &WorkingMemory, task: &str, names: &[&str]) -> bool {
    let text = memory.render();
    layers::links(&text)
        .iter()
        .any(|target| record_of(target) == Some((TASKS, task)))
        || names.iter().any(|name| has_word(&text, name))
}

fn has_word(text: &str, word: &str) -> bool {
    let inside = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    !word.is_empty()
        && text.match_indices(word).any(|(at, _)| {
            !inside(text[..at].chars().next_back())
                && !inside(text[at + word.len()..].chars().next())
        })
}

/// Every working memory whose tasks all came to their end after it was last
/// written, or that was not written for long. One written after its tasks
/// ended was looked at then, so it is left alone until it is quiet again.
pub fn leftovers(records: &Records, dir: &Path, now: SystemTime) -> anyhow::Result<Vec<Leftover>> {
    let memories = layers::working_memories(dir)?;
    if memories.is_empty() {
        return Ok(Vec::new());
    }
    let tasks = records.list(TASKS)?;
    let codes = records.codes(TASKS)?;
    let mut found = Vec::new();
    for memory in memories {
        let file = dir
            .join(layers::WORKING)
            .join(format!("{}.md", memory.name));
        let written = std::fs::metadata(file)?.modified()?;
        let its: Vec<&Record> = tasks
            .iter()
            .filter(|t| {
                let code = codes.get(&t.id).map_or("", String::as_str);
                speaks_of(&memory, &t.id, &[code])
            })
            .collect();
        let ended = !its.is_empty()
            && its.iter().all(|t| is_over(t))
            && its.iter().any(|t| changed_since(records, t, written));
        let quiet = now.duration_since(written).unwrap_or_default();
        let why = if ended {
            let its: Vec<&str> = its
                .iter()
                .map(|t| codes.get(&t.id).unwrap_or(&t.id).as_str())
                .collect();
            format!("every task it speaks of is over ({})", its.join(", "))
        } else if quiet >= Duration::from_secs(QUIET_DAYS * 86_400) {
            format!("not written for {} days", quiet.as_secs() / 86_400)
        } else {
            continue;
        };
        found.push(Leftover {
            name: memory.name,
            why,
        });
    }
    Ok(found)
}

fn changed_since(records: &Records, task: &Record, since: SystemTime) -> bool {
    let path = records.notebook().root().join(task.rel());
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|at| at > since)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{ensure, write_working};
    use crate::notebook::Writer;
    use crate::notebook::testing::TempDir;
    use crate::records::Patch;
    use std::path::PathBuf;

    const DAY: Duration = Duration::from_secs(86_400);

    struct Bench {
        records: Records,
        dir: PathBuf,
        _tmp: TempDir,
    }

    fn bench(name: &str) -> Bench {
        let (tmp, records) = crate::records::testing::records(name);
        let dir = records.notebook().memory_dir();
        ensure(&dir).unwrap();
        Bench {
            records,
            dir,
            _tmp: tmp,
        }
    }

    impl Bench {
        fn task(&self, title: &str) -> Record {
            self.records
                .add(Writer::Secretary, TASKS, Patch::titled(title))
                .unwrap()
        }

        fn end(&self, task: &Record, status: &str) {
            let patch = Patch::default().with("Status", status);
            self.records
                .change(Writer::Owner, TASKS, &task.id, patch)
                .unwrap();
        }

        /// Write a working memory, as if that long ago.
        fn working(&self, waiting: &str, ago: Duration) {
            let memory = WorkingMemory {
                name: "x".into(),
                title: "x".into(),
                waiting: waiting.into(),
                ..WorkingMemory::default()
            };
            let rel = write_working(&self.dir, &memory).unwrap();
            std::fs::File::options()
                .write(true)
                .open(self.dir.join(rel))
                .unwrap()
                .set_modified(SystemTime::now() - ago)
                .unwrap();
        }

        fn why(&self) -> Vec<String> {
            leftovers(&self.records, &self.dir, SystemTime::now())
                .unwrap()
                .into_iter()
                .map(|l| format!("{}: {}", l.name, l.why))
                .collect()
        }
    }

    #[test]
    fn working_memory_is_left_over_once_every_task_it_speaks_of_is_over() {
        let b = bench("leftover-tasks");
        let (one, two) = (b.task("一"), b.task("二"));
        let code = b.records.code(TASKS, &two.id).unwrap();
        b.working(&format!("{} と {code} を待つ", one.link()), DAY);
        b.end(&one, "done");
        assert!(b.why().is_empty(), "one task goes on");

        b.end(&two, "cancelled");

        let why = b.why();
        assert_eq!(why.len(), 1);
        assert!(why[0].starts_with("x: every task it speaks of is over ("));
        assert!(why[0].contains(&code), "{why:?}");
    }

    #[test]
    fn working_memory_written_after_its_tasks_ended_is_left_alone_until_it_is_quiet() {
        let b = bench("leftover-written");
        let task = b.task("一");
        b.end(&task, "done");
        // Ahead, as a file system may keep whole seconds only.
        let memory = WorkingMemory {
            name: "x".into(),
            clues: task.link(),
            ..WorkingMemory::default()
        };
        let rel = write_working(&b.dir, &memory).unwrap();
        let at = SystemTime::now() + Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(b.dir.join(rel))
            .unwrap()
            .set_modified(at)
            .unwrap();

        assert!(b.why().is_empty());
        let later = leftovers(&b.records, &b.dir, at + 7 * DAY).unwrap();
        assert_eq!(later[0].why, "not written for 7 days");
    }

    #[test]
    fn working_memory_without_a_task_is_left_over_when_not_written_for_long() {
        let b = bench("leftover-quiet");
        b.working("Adroit のセッション", 6 * DAY);
        assert!(b.why().is_empty());

        b.working("Adroit のセッション", 8 * DAY);
        assert_eq!(b.why(), ["x: not written for 8 days"]);
        assert!(b.dir.join("working/x.md").is_file(), "it is never closed");
    }

    #[test]
    fn a_task_is_spoken_of_by_link_or_by_a_whole_name() {
        let memory = |clues: &str| WorkingMemory {
            clues: clues.into(),
            ..WorkingMemory::default()
        };
        let speaks = |clues: &str| speaks_of(&memory(clues), "t1", &["task-abc", ""]);
        assert!(speaks("[[records/tasks/t1|題]]"));
        assert!(speaks("委託task-abcの終わり"));
        assert!(speaks("task-abc-run4 のセッション"));
        assert!(!speaks("[[records/memos/t1|題]]"));
        assert!(!speaks("task-abcd"));
        assert!(!speaks("subtask-abc"));
        assert!(!speaks("なし"));
    }
}
