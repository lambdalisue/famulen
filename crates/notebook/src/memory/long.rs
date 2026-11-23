use std::path::{Path, PathBuf};

use super::{LONG, LongGroup, links, markdown_files, read_doc, stem};
use crate::Entry;

/// One long-term memory: a file holding one item.
#[derive(Debug, Clone, PartialEq)]
pub struct LongTermFile {
    /// Relative to the memory directory.
    pub path: PathBuf,
    /// The file name without `.md`.
    pub name: String,
    /// The line shown in the long-term index.
    pub summary: String,
    /// What its prose links to: `[[records/parties/p1|田中さん]]` gives
    /// `records/parties/p1`.
    pub links: Vec<String>,
    /// Its one item; none when the file says nothing yet.
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LongTermGroup {
    pub group: LongGroup,
    pub files: Vec<LongTermFile>,
}

/// Every long-term group, in a fixed order, with its files by name.
pub fn long_term(dir: &Path) -> anyhow::Result<Vec<LongTermGroup>> {
    LongGroup::ALL
        .into_iter()
        .map(|group| {
            let files = markdown_files(dir, &Path::new(LONG).join(group.dir_name()))?
                .into_iter()
                .map(|path| {
                    let doc = read_doc(dir, &path)?;
                    let entries: Vec<Entry> = doc.entries().cloned().collect();
                    Ok(LongTermFile {
                        name: stem(&path),
                        summary: entries
                            .first()
                            .map(|e| e.text().to_owned())
                            .unwrap_or_default(),
                        links: links(doc.long_text().unwrap_or_default()),
                        entries,
                        path,
                    })
                })
                .collect::<anyhow::Result<_>>()?;
            Ok(LongTermGroup { group, files })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{ensure, write_file};
    use crate::testing::TempDir;

    fn long(id: &str, summary: &str, extra: &str, body: &str) -> String {
        format!("---\nid: {id}\nsummary: {summary}\n{extra}---\n\n{body}\n")
    }

    #[test]
    fn long_term_lists_each_group_with_its_files_summaries_and_links() {
        let tmp = TempDir::new("long-term");
        ensure(tmp.path()).unwrap();
        write_file(
            tmp.path(),
            "long/semantic/tanaka-sweets.md",
            long(
                "m1",
                "田中さんは甘いものが好き",
                "src: said\n",
                "[[records/parties/p1|田中さん]] は甘いものが好き。",
            )
            .as_bytes(),
        )
        .unwrap();

        let groups = long_term(tmp.path()).unwrap();

        let order: Vec<LongGroup> = groups.iter().map(|g| g.group).collect();
        assert_eq!(order, LongGroup::ALL);
        let people = &groups[0].files;
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].path, Path::new("long/semantic/tanaka-sweets.md"));
        assert_eq!(people[0].name, "tanaka-sweets");
        assert_eq!(people[0].summary, "田中さんは甘いものが好き");
        assert_eq!(people[0].links, ["records/parties/p1"]);
        assert_eq!(people[0].entries.len(), 1);
        assert_eq!(people[0].entries[0].id(), Some("m1"));
        assert!(groups[1].files.is_empty());
    }
}
