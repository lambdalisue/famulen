//! Searching the notebook. Every way of searching takes the same query and
//! gives hits of the same shape, so a new way (by meaning, say) can be added
//! beside the full-text search without changing who searches.

use std::collections::BTreeMap;

use rusqlite::params_from_iter;

use crate::body::same;
use crate::index::Index;
use crate::{MEMORY, memory};

/// What to look for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Query {
    /// Words, separated by spaces; every word must appear. Empty finds
    /// everything the filters let through.
    pub text: String,
    /// Where to look, as path prefixes in the notebook: `memory`,
    /// `memory/long`, `records/tasks`. Empty is everywhere.
    pub scopes: Vec<String>,
    /// Only what applies to this workgroup.
    pub workgroup: Option<String>,
    /// Only what has these field values. Names are the same in any case.
    pub fields: Vec<(String, String)>,
    /// Only what is dated on or after this day (`YYYY-MM-DD`).
    pub from: Option<String>,
    /// Only what is dated on or before this day (`YYYY-MM-DD`).
    pub to: Option<String>,
    /// Look in forgotten memory too.
    pub forgotten: bool,
    /// At most this many hits; 0 is the default.
    pub limit: usize,
}

/// One thing found.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// Relative to the notebook.
    pub path: String,
    /// `memory/<layer>` or `records/<kind>`.
    pub scope: String,
    pub id: String,
    /// The title, a long-term memory's summary, or a memory item's text.
    pub title: String,
    /// Around where the words were found.
    pub snippet: String,
    /// Higher is a better match.
    pub score: f64,
}

/// A way of searching.
pub trait Searcher {
    fn search(&self, index: &Index, query: &Query) -> anyhow::Result<Vec<Hit>>;
}

const DEFAULT_LIMIT: usize = 20;
/// How many characters a snippet shows before and after where a word is
/// found.
const BEFORE: usize = 20;
const AFTER: usize = 60;

/// The words to look for. A word is any run of characters between spaces,
/// as it may stand anywhere in a Japanese sentence.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_lowercase).collect()
}

/// Full-text search: every word must be in the title or the text. Words of
/// three characters or more are looked up in the trigram index; shorter
/// ones are matched within what that finds.
pub struct FullText;

struct Row {
    path: String,
    scope: String,
    id: String,
    title: String,
    body: String,
    workgroup: Vec<String>,
    date: Option<String>,
    fields: BTreeMap<String, String>,
    rank: f64,
}

impl FullText {
    fn rows(&self, index: &Index, words: &[String]) -> anyhow::Result<Vec<Row>> {
        let long: Vec<&String> = words.iter().filter(|w| w.chars().count() >= 3).collect();
        let columns = "i.path, i.scope, i.id, i.title, i.body, i.workgroup, i.date, i.fields";
        let (sql, args): (String, Vec<String>) = if long.is_empty() {
            (format!("SELECT {columns}, 0.0 FROM items i"), Vec::new())
        } else {
            let matching = long
                .iter()
                .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" AND ");
            (
                format!(
                    "SELECT {columns}, bm25(items_text) FROM items_text
                     JOIN items i ON i.rowid = items_text.rowid
                     WHERE items_text MATCH ?1"
                ),
                vec![matching],
            )
        };
        let conn = index.conn();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params_from_iter(args.iter()), |r| {
                Ok(Row {
                    path: r.get(0)?,
                    scope: r.get(1)?,
                    id: r.get(2)?,
                    title: r.get(3)?,
                    body: r.get(4)?,
                    workgroup: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                    date: r.get(6)?,
                    fields: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
                    rank: r.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

fn in_scope(row: &Row, query: &Query) -> bool {
    let forgotten = format!("{MEMORY}/{}", memory::FORGOTTEN);
    if row.scope == forgotten && !query.forgotten {
        return false;
    }
    query.scopes.is_empty()
        || query
            .scopes
            .iter()
            .map(|s| s.trim_end_matches('/'))
            .any(|s| row.scope == s || row.scope.starts_with(&format!("{s}/")))
}

fn passes(row: &Row, query: &Query) -> bool {
    let date = row.date.as_deref().map(|d| d.get(..10).unwrap_or(d));
    in_scope(row, query)
        && query
            .workgroup
            .as_ref()
            .is_none_or(|c| row.workgroup.is_empty() || row.workgroup.contains(c))
        && query.fields.iter().all(|(k, v)| {
            row.fields
                .iter()
                .any(|(name, have)| same(name, k) && have == v)
        })
        && query
            .from
            .as_deref()
            .is_none_or(|from| date.is_some_and(|d| d >= from))
        && query
            .to
            .as_deref()
            .is_none_or(|to| date.is_some_and(|d| d.get(..to.len()).unwrap_or(d) <= to))
}

/// The text around the first word found, cut on character boundaries.
fn snippet(text: &str, words: &[String]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    let found = words.iter().find_map(|w| {
        let w: Vec<char> = w.chars().collect();
        (0..lower.len().saturating_sub(w.len() - 1))
            .find(|&i| lower.len() == chars.len() && lower[i..i + w.len()] == w[..])
    });
    let at = found.unwrap_or(0);
    let start = at.saturating_sub(BEFORE);
    let end = (at + AFTER).min(chars.len());
    let mut cut: String = chars[start..end].iter().collect();
    cut = cut.split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        cut.insert(0, '…');
    }
    if end < chars.len() {
        cut.push('…');
    }
    cut
}

impl Searcher for FullText {
    fn search(&self, index: &Index, query: &Query) -> anyhow::Result<Vec<Hit>> {
        let words = words(&query.text);
        let mut hits: Vec<Hit> = self
            .rows(index, &words)?
            .into_iter()
            .filter(|row| passes(row, query))
            .filter_map(|row| {
                let haystack = format!("{}\n{}", row.title, row.body).to_lowercase();
                if !words.iter().all(|w| haystack.contains(w)) {
                    return None;
                }
                let found: usize = words
                    .iter()
                    .map(|w| haystack.matches(w.as_str()).count())
                    .sum();
                let score = if row.rank != 0.0 {
                    -row.rank
                } else {
                    found as f64
                };
                let source = if row.body.is_empty() {
                    &row.title
                } else {
                    &row.body
                };
                Some(Hit {
                    snippet: snippet(source, &words),
                    path: row.path,
                    scope: row.scope,
                    id: row.id,
                    title: row.title,
                    score,
                })
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.id.cmp(&b.id))
        });
        hits.truncate(if query.limit == 0 {
            DEFAULT_LIMIT
        } else {
            query.limit
        });
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Notebook;
    use crate::records::Records;
    use crate::testing::{TempDir, notebook, write};

    fn indexed(name: &str) -> (TempDir, Notebook, Index) {
        let (tmp, notebook) = notebook(name);
        let index = Index::open(&notebook).unwrap();
        (tmp, notebook, index)
    }

    fn task(notebook: &Notebook, id: &str, title: &str, fields: &[(&str, &str)]) {
        let records = Records::new(notebook.clone());
        let mut record = records.draft("tasks", title).unwrap();
        record.id = id.into();
        for (k, v) in fields {
            if *k == "workgroup" {
                record.meta.set_list(k, &[(*v).to_owned()]);
            } else {
                record.body.set(k, *v).unwrap();
            }
        }
        record.meta.set("id", id);
        write(notebook.root(), &record.rel(), &record.render());
    }

    fn found(index: &Index, query: Query) -> Vec<String> {
        index
            .search(&query)
            .unwrap()
            .into_iter()
            .map(|h| format!("{}#{}", h.path, h.id))
            .collect()
    }

    fn text(words: &str) -> Query {
        Query {
            text: words.into(),
            ..Query::default()
        }
    }

    #[test]
    fn a_word_finds_the_memories_and_records_that_hold_it_best_first_with_a_snippet() {
        let (_tmp, nb, index) = indexed("search-words");
        write(
            nb.root(),
            "memory/core.md",
            "# 心得\n\n- 会議は午前に入れる {id: m1, src: said}\n- 返事は短く {id: m2}\n",
        );
        write(
            nb.root(),
            "memory/long/semantic/tanaka.md",
            "---\nid: m3\nsummary: 田中さんは甘いものが好き\n---\n\n田中さんは和菓子が好き。会議のあとに渡す。\n",
        );
        task(&nb, "t1", "会議の資料を作る", &[("status", "open")]);

        let hits = index.search(&text("会議")).unwrap();

        let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids.len(), 3, "{hits:?}");
        assert!(ids.contains(&"m1") && ids.contains(&"m3") && ids.contains(&"t1"));
        let tanaka = hits.iter().find(|h| h.id == "m3").unwrap();
        assert_eq!(tanaka.title, "田中さんは甘いものが好き");
        assert!(tanaka.snippet.contains("会議"), "{}", tanaka.snippet);
        assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));
    }

    #[test]
    fn a_japanese_word_inside_a_sentence_is_found_long_or_short() {
        let (_tmp, nb, index) = indexed("search-japanese");
        write(
            nb.root(),
            "memory/core.md",
            "# 心得\n\n- 打ち合わせは午後三時以降がよい {id: m1}\n",
        );

        assert_eq!(found(&index, text("午後三時")), ["memory/core.md#m1"]);
        assert_eq!(found(&index, text("午後")), ["memory/core.md#m1"]);
        assert_eq!(
            found(&index, text("午後 打ち合わせ")),
            ["memory/core.md#m1"]
        );
        assert!(found(&index, text("午前")).is_empty());
    }

    #[test]
    fn filters_let_through_only_what_matches_them() {
        let (_tmp, nb, index) = indexed("search-filters");
        task(
            &nb,
            "t1",
            "見積もり",
            &[("status", "open"), ("due", "2026-11-28")],
        );
        task(
            &nb,
            "t2",
            "見積もりの確認",
            &[("status", "done"), ("due", "2026-12-10")],
        );
        write(
            nb.root(),
            "memory/core.md",
            "# 心得\n\n- 見積もりは二日以内 {id: m1, workgroup: [work]}\n",
        );
        write(
            nb.root(),
            "memory/pending.md",
            "# 保留\n\n- 見積もりは安め {id: m2, workgroup: [personal]}\n",
        );

        let only = |q: Query| found(&index, q);
        assert_eq!(
            only(Query {
                scopes: vec!["records/tasks".into()],
                fields: vec![("status".into(), "open".into())],
                ..Query::default()
            }),
            ["records/tasks/t1.md#t1"]
        );
        assert_eq!(
            only(Query {
                text: "見積もり".into(),
                scopes: vec!["memory".into()],
                workgroup: Some("work".into()),
                ..Query::default()
            }),
            ["memory/core.md#m1"]
        );
        assert_eq!(
            only(Query {
                scopes: vec!["records".into()],
                from: Some("2026-12-01".into()),
                ..Query::default()
            }),
            ["records/tasks/t2.md#t2"]
        );
    }

    #[test]
    fn a_field_is_found_whatever_the_case_of_its_name() {
        let (_tmp, nb, index) = indexed("search-field-case");
        task(&nb, "t1", "見積もり", &[("Status", "open")]);

        for name in ["Status", "status", "STATUS"] {
            let query = Query {
                fields: vec![(name.into(), "open".into())],
                ..Query::default()
            };
            assert_eq!(found(&index, query), ["records/tasks/t1.md#t1"], "{name}");
        }
    }

    #[test]
    fn changes_made_outside_show_in_the_next_search() {
        let (_tmp, nb, index) = indexed("search-outside");
        write(
            nb.root(),
            "memory/core.md",
            "# 心得\n\n- 古い決まり {id: m1}\n",
        );
        assert_eq!(found(&index, text("古い決まり")).len(), 1);

        write(
            nb.root(),
            "memory/core.md",
            "# 心得\n\n- 新しい決まりに変わった {id: m1}\n",
        );
        task(&nb, "t9", "新しいタスク", &[]);
        assert!(found(&index, text("古い決まり")).is_empty());
        assert_eq!(found(&index, text("新しい")).len(), 2);

        std::fs::remove_file(nb.root().join("records/tasks/t9.md")).unwrap();
        assert_eq!(found(&index, text("新しい")), ["memory/core.md#m1"]);
    }

    #[test]
    fn forgotten_memory_is_searched_only_when_asked() {
        let (_tmp, nb, index) = indexed("search-forgotten");
        write(
            nb.root(),
            "memory/forgotten/2026-11.md",
            "# 2026-11\n\n- 昔の好み {id: m1, reason: replaced}\n",
        );

        assert!(found(&index, text("昔の好み")).is_empty());
        let asked = Query {
            forgotten: true,
            ..text("昔の好み")
        };
        assert_eq!(found(&index, asked), ["memory/forgotten/2026-11.md#m1"]);
    }

    #[test]
    fn another_way_of_searching_takes_the_same_query_and_gives_the_same_hits() {
        struct Everything;
        impl Searcher for Everything {
            fn search(&self, index: &Index, query: &Query) -> anyhow::Result<Vec<Hit>> {
                FullText.search(
                    index,
                    &Query {
                        text: String::new(),
                        ..query.clone()
                    },
                )
            }
        }
        let (_tmp, nb, index) = indexed("search-another");
        task(&nb, "t1", "どれでも", &[]);

        let hits = index
            .search_with(&Everything, &text("当たらない言葉"))
            .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "t1");
    }

    #[test]
    fn uses_are_counted_in_the_index_and_leave_the_notebook_as_it_was() {
        let (_tmp, nb, index) = indexed("search-uses");
        let day = |d| chrono::NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        write(nb.root(), "memory/core.md", "# 心得\n\n- 短く {id: m1}\n");
        let before = std::fs::read_to_string(nb.root().join("memory/core.md")).unwrap();

        index.record_use("m1", day(20)).unwrap();
        index.record_use("m1", day(22)).unwrap();
        index.record_use("m1", day(21)).unwrap();

        let used = index.uses().unwrap()["m1"];
        assert_eq!((used.uses, used.last), (3, day(22)));
        assert!(!index.uses().unwrap().contains_key("m2"));
        assert_eq!(
            std::fs::read_to_string(nb.root().join("memory/core.md")).unwrap(),
            before
        );
        assert!(nb.state_dir().join("index.sqlite3").is_file());
    }
}
