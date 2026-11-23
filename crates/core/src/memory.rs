//! What the secretary keeps between conversations, in the notebook's
//! `memory/` layers. The day writes episodic and working memory; the nightly
//! dream consolidates the rest. The layers themselves are the notebook
//! library's; here is what Famulen does with them.

mod dream;
mod prompt;

pub use dream::*;
pub use prompt::*;

pub use famulen_notebook::memory::*;
pub use famulen_notebook::{Entry, EpisodeState, Src};

use std::path::{Path, PathBuf};

/// What long-term memory holds about a party, with the file each item is in:
/// every memory whose prose or summary links to the party's record, and every one
/// tagged with the party.
pub fn party_memories(dir: &Path, party: &str) -> anyhow::Result<Vec<(PathBuf, Entry)>> {
    let target = format!("{}/parties/{party}", famulen_notebook::RECORDS);
    Ok(long_term(dir)?
        .into_iter()
        .flat_map(|g| g.files)
        .flat_map(|f| {
            let linked = f.links.contains(&target);
            let path = f.path;
            f.entries
                .into_iter()
                .filter({
                    let target = target.clone();
                    move |e| {
                        linked || e.get("party") == Some(party) || links(e.text()).contains(&target)
                    }
                })
                .map(move |e| (path.clone(), e))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::testing::TempDir;

    fn long(id: &str, summary: &str, extra: &str, body: &str) -> String {
        format!("---\nid: {id}\nsummary: {summary}\n{extra}---\n\n{body}\n")
    }

    #[test]
    fn party_memories_gather_memories_linking_to_the_party_and_those_tagged_with_it() {
        let tmp = TempDir::new("party-memories");
        ensure(tmp.path()).unwrap();
        for (rel, text) in [
            (
                "long/semantic/tanaka-sweets.md",
                long(
                    "m1",
                    "甘いものが好き",
                    "",
                    "[[records/parties/p7]] は甘いものが好き。",
                ),
            ),
            (
                "long/semantic/suzuki-tea.md",
                long(
                    "m3",
                    "紅茶が好き",
                    "",
                    "[[records/parties/p8]] は紅茶が好き。",
                ),
            ),
            (
                "long/semantic/tanaka-work.md",
                long("m5", "\"[[records/parties/p7]] は経理\"", "", "経理の人。"),
            ),
            (
                "long/prospective/send-papers.md",
                long("m4", "田中さんに資料を送る", "party: p7\n", "資料を送る。"),
            ),
        ] {
            write_file(tmp.path(), rel, text.as_bytes()).unwrap();
        }

        let found = party_memories(tmp.path(), "p7").unwrap();

        let ids: Vec<(&Path, &str)> = found
            .iter()
            .map(|(file, e)| (file.as_path(), e.id().unwrap_or_default()))
            .collect();
        assert_eq!(
            ids,
            [
                (Path::new("long/semantic/tanaka-sweets.md"), "m1"),
                (Path::new("long/semantic/tanaka-work.md"), "m5"),
                (Path::new("long/prospective/send-papers.md"), "m4"),
            ]
        );
        assert!(party_memories(tmp.path(), "p9").unwrap().is_empty());
    }
}
