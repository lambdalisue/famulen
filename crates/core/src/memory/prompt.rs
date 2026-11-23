//! What of memory every new session is handed.

use std::fmt::Write as _;
use std::path::Path;

use chrono::NaiveDate;

use super::{self as layers, CORE};

/// Core, the long-term index and the work in progress, as handed to a new
/// session, core cut at `core_lines`. Core items not yet or no longer valid on
/// `today` are left out.
pub fn prompt_context(dir: &Path, today: NaiveDate, core_lines: usize) -> String {
    let mut text = String::new();

    let core: Vec<String> = layers::core_entries(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.from().is_none_or(|from| from <= today))
        .filter(|e| e.to().is_none_or(|to| to >= today))
        .map(|e| e.render())
        .collect();
    let _ = writeln!(text, "Core ({CORE}):\n");
    if core.is_empty() {
        text.push_str("(none)\n");
    }
    for line in core.iter().take(core_lines) {
        let _ = writeln!(text, "{line}");
    }
    if core.len() > core_lines {
        let _ = writeln!(
            text,
            "… (cut at {core_lines} lines; read {CORE} for the rest)"
        );
    }

    text.push_str("\nLong-term memory (read a file when its topic comes up):\n\n");
    let files: Vec<_> = layers::long_term(dir)
        .unwrap_or_default()
        .into_iter()
        .flat_map(|group| group.files)
        .collect();
    if files.is_empty() {
        text.push_str("(none)\n");
    }
    for file in files {
        let _ = writeln!(
            text,
            "- {}: {}",
            file.path.to_string_lossy().replace('\\', "/"),
            file.summary
        );
    }

    text.push_str("\nWork in progress:\n\n");
    let working = layers::working_memories(dir).unwrap_or_default();
    if working.is_empty() {
        text.push_str("(none)\n");
    }
    for memory in working {
        let _ = writeln!(
            text,
            "- {}/{}.md: {}",
            layers::WORKING,
            memory.name,
            memory.title
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{WorkingMemory, ensure, write_file, write_working};
    use crate::notebook::testing::TempDir;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 21).unwrap()
    }

    #[test]
    fn the_context_holds_core_the_long_term_index_and_work_titles() {
        let tmp = TempDir::new("prompt");
        ensure(tmp.path()).unwrap();
        write_file(
            tmp.path(),
            CORE,
            "# 心得\n\n- 返事は短く {id: m1, src: said}\n".as_bytes(),
        )
        .unwrap();
        write_file(
            tmp.path(),
            "long/semantic/tanaka.md",
            "---\nid: m2\nsummary: 田中さんは甘いものが好き\n---\n\n本文\n".as_bytes(),
        )
        .unwrap();
        write_working(
            tmp.path(),
            &WorkingMemory {
                name: "trip".into(),
                title: "出張の手配".into(),
                ..WorkingMemory::default()
            },
        )
        .unwrap();

        let text = prompt_context(tmp.path(), today(), 150);

        assert!(
            text.contains("- 返事は短く {id: m1, src: said}\n"),
            "{text}"
        );
        assert!(
            text.contains("- long/semantic/tanaka.md: 田中さんは甘いものが好き\n"),
            "{text}"
        );
        assert!(text.contains("- working/trip.md: 出張の手配\n"), "{text}");
        assert!(!text.contains("# 心得"), "{text}");
    }

    #[test]
    fn core_longer_than_the_limit_is_cut_with_a_marker() {
        let tmp = TempDir::new("prompt-cut");
        ensure(tmp.path()).unwrap();
        let core: String = (1..=25)
            .map(|n| format!("- rule {n} {{id: m{n}}}\n"))
            .collect();
        write_file(tmp.path(), CORE, core.as_bytes()).unwrap();

        let text = prompt_context(tmp.path(), today(), 20);

        assert!(text.contains("- rule 20 {id: m20}\n"));
        assert!(!text.contains("- rule 21 "));
        assert!(text.contains("… (cut at 20 lines; read core.md for the rest)"));
    }

    #[test]
    fn core_items_outside_their_period_are_left_out() {
        let tmp = TempDir::new("prompt-period");
        ensure(tmp.path()).unwrap();
        write_file(
            tmp.path(),
            CORE,
            b"- ended {id: m1, to: 2026-11-20}\n- later {id: m2, from: 2026-11-22}\n- now {id: m3, from: 2026-11-21, to: 2026-11-21}\n",
        )
        .unwrap();

        let text = prompt_context(tmp.path(), today(), 150);

        assert!(!text.contains("ended"));
        assert!(!text.contains("later"));
        assert!(text.contains("- now "));
    }

    #[test]
    fn an_empty_memory_says_so_in_each_part() {
        let tmp = TempDir::new("prompt-empty");
        ensure(tmp.path()).unwrap();

        let text = prompt_context(tmp.path(), today(), 150);

        assert_eq!(text.matches("(none)").count(), 3, "{text}");
    }
}
