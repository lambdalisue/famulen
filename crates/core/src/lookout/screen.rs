//! Screening what was read: which of it concerns the owner, and where that
//! goes.

use std::future::Future;

use rusqlite::params;

use super::{Item, Lookout, More, Outcome, Reading, Route, Screened, Urgency, Verdict, dedup_key};
use crate::input::{Input, Sender, Source};
use crate::store::now;

/// A feed's first read may pick far more than one site should be asked for.
const PAGES_AT_ONCE: usize = 20;

/// Where what no one decided goes.
fn undecided(route: Option<Route>) -> Outcome {
    match route {
        Some(Route::Log) => Outcome::Logged,
        _ => Outcome::Digest,
    }
}

impl Lookout {
    /// Screen the new items of a batch with one call of `model`, which reads
    /// them all at once and answers with a [`Verdict`]. Before it, `skim`
    /// sees those with more to open by their title and abstract, and answers
    /// which to open; the rest of those go no further. Either answers `None`
    /// when the lookout is over its daily cap. `open` reads a page; one it
    /// cannot read is judged by its abstract. Returns the inputs for the
    /// secretary: one per group to act on or to tell.
    pub async fn read<S, FS, O, FO, M, F>(
        &self,
        source: &str,
        batch: Vec<Reading>,
        skim: S,
        open: O,
        model: M,
    ) -> anyhow::Result<Vec<Input>>
    where
        S: FnOnce(Vec<Item>) -> FS,
        FS: Future<Output = anyhow::Result<Option<Vec<usize>>>>,
        O: Fn(String) -> FO,
        FO: Future<Output = Option<String>>,
        M: FnOnce(Vec<Item>) -> F,
        F: Future<Output = anyhow::Result<Option<Verdict>>>,
    {
        let fresh = self.record_new(source, batch)?;
        if fresh.is_empty() {
            return Ok(Vec::new());
        }
        let route = self
            .sources()?
            .into_iter()
            .find(|s| s.id.to_string() == source)
            .and_then(|s| s.route);
        // An item is recorded before the model reads it, so it is seen once;
        // when the call fails it goes to the digest rather than staying
        // undecided, which a later read would skip as already seen.
        let unread = |ids: Vec<i64>, error: Option<anyhow::Error>| -> anyhow::Result<Vec<Input>> {
            let why = match error {
                Some(error) => format!("見張りのモデルが失敗した（{error}）"),
                None => "見張りのモデルが上限に達した".to_owned(),
            };
            for id in ids {
                self.decide(id, undecided(route), &why)?;
            }
            Ok(Vec::new())
        };
        let skimmed: Vec<Item> = fresh
            .iter()
            .filter(|(_, r)| r.more.is_some())
            .map(|(_, r)| r.item.clone())
            .collect();
        let picked = if skimmed.is_empty() {
            Vec::new()
        } else {
            match skim(skimmed).await {
                Ok(Some(picked)) => picked,
                result => return unread(fresh.iter().map(|(id, _)| *id).collect(), result.err()),
            }
        };
        let fresh = self.open_picked(fresh, &picked, open).await?;
        if fresh.is_empty() {
            return Ok(Vec::new());
        }
        let verdict = match model(fresh.iter().map(|(_, i)| i.clone()).collect()).await {
            Ok(Some(verdict)) => verdict,
            result => return unread(fresh.iter().map(|(id, _)| *id).collect(), result.err()),
        };
        self.settle(source, route, &fresh, verdict)
    }

    /// Record the items of a batch not seen before, so each is seen once.
    fn record_new(&self, source: &str, batch: Vec<Reading>) -> anyhow::Result<Vec<(i64, Reading)>> {
        let mut fresh = Vec::new();
        for reading in batch {
            if let Some(id) = self.record(source, &reading.item)? {
                fresh.push((id, reading));
            }
        }
        Ok(fresh)
    }

    /// Open what the skim picked, by its place among those with more to open.
    /// Those it passed over are decided here; those with nothing to open stay
    /// as they are.
    async fn open_picked<O, FO>(
        &self,
        fresh: Vec<(i64, Reading)>,
        picked: &[usize],
        open: O,
    ) -> anyhow::Result<Vec<(i64, Item)>>
    where
        O: Fn(String) -> FO,
        FO: Future<Output = Option<String>>,
    {
        let mut kept = Vec::new();
        let mut index = 0;
        let mut pages = 0;
        for (id, Reading { mut item, more }) in fresh {
            let Some(more) = more else {
                kept.push((id, item));
                continue;
            };
            let skimmed_as = index;
            index += 1;
            if !picked.contains(&skimmed_as) {
                self.decide(id, Outcome::Unrelated, "題と要約から、関わりがない")?;
                continue;
            }
            match more {
                More::Text(text) => item.body = text,
                More::Page(url) if pages < PAGES_AT_ONCE => {
                    pages += 1;
                    if let Some(text) = open(url).await {
                        item.body = format!("{}\n{text}", item.body);
                    }
                }
                More::Page(_) => {}
            }
            kept.push((id, item));
        }
        Ok(kept)
    }

    /// Decide every item as the model judged it, with `route` in place of
    /// where the model sends it. Returns the inputs for the secretary.
    fn settle(
        &self,
        source: &str,
        route: Option<Route>,
        fresh: &[(i64, Item)],
        verdict: Verdict,
    ) -> anyhow::Result<Vec<Input>> {
        let screened: Vec<Screened> = (0..fresh.len())
            .map(|i| verdict.items.get(i).copied().unwrap_or(Screened::UNJUDGED))
            .collect();
        let mut decided = vec![false; fresh.len()];
        for (i, s) in screened.iter().enumerate() {
            if !s.related {
                decided[i] = true;
                self.decide(fresh[i].0, Outcome::Unrelated, "関わりがない")?;
            }
        }
        let mut inputs = Vec::new();
        for group in verdict.groups {
            let members: Vec<usize> = group
                .items
                .iter()
                .copied()
                .filter(|i| *i < fresh.len() && !decided[*i])
                .collect();
            if members.is_empty() {
                continue;
            }
            let urgency = members
                .iter()
                .map(|i| screened[*i].urgency)
                .min_by_key(|u| *u as u8)
                .unwrap_or(Urgency::Later);
            let outcome = match group.outcome {
                Outcome::Unrelated => Outcome::Digest,
                outcome => outcome,
            };
            let outcome = route.map_or(outcome, |r| r.instead_of(outcome));
            let together = if members.len() > 1 {
                format!(" · {} 件をまとめた", members.len())
            } else {
                String::new()
            };
            for i in &members {
                decided[*i] = true;
                self.decide(
                    fresh[*i].0,
                    outcome,
                    &format!("{}{together} · {}", group.reason, urgency.label()),
                )?;
            }
            if matches!(outcome, Outcome::Act | Outcome::Notify) {
                let title = if group.title.is_empty() {
                    fresh[members[0]].1.title.clone()
                } else {
                    group.title.clone()
                };
                let details = members
                    .iter()
                    .map(|i| {
                        format!(
                            "- #{} {}\n{}",
                            fresh[*i].0, fresh[*i].1.title, fresh[*i].1.body
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                inputs.push(Input {
                    sender: Sender::System,
                    source: Source::Lookout {
                        item: fresh[members[0]].0,
                        source: source.to_owned(),
                    },
                    text: format!(
                        "The lookout passed this on to {} ({}, urgency: {:?}).\nSubject: {title}\nWhy: {}\nItems:\n{details}",
                        if outcome == Outcome::Act { "act on" } else { "tell the owner about" },
                        outcome.as_str(),
                        urgency,
                        group.reason,
                    ),
                });
            }
        }
        for (i, (id, _)) in fresh.iter().enumerate() {
            if !decided[i] {
                self.decide(
                    *id,
                    undecided(route),
                    &format!(
                        "見張りが行き先を決めなかった · {}",
                        screened[i].urgency.label()
                    ),
                )?;
            }
        }
        Ok(inputs)
    }

    /// Record a new item; returns its id, or none when it was seen before.
    fn record(&self, source: &str, item: &Item) -> anyhow::Result<Option<i64>> {
        let key = dedup_key(source, item);
        let conn = self.store.conn();
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO lookout_items (source, dedup_key, at, title, body, outcome)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
            params![source, key, now(), item.title, item.body],
        )?;
        Ok((inserted > 0).then(|| conn.last_insert_rowid()))
    }

    fn decide(&self, id: i64, outcome: Outcome, reason: &str) -> anyhow::Result<()> {
        self.store.conn().execute(
            "UPDATE lookout_items SET outcome = ?2, reason = ?3 WHERE id = ?1",
            params![id, outcome.as_str(), reason],
        )?;
        Ok(())
    }

    /// [`Self::read`] for items with nothing more to open.
    #[cfg(test)]
    pub async fn screen<M, F>(
        &self,
        source: &str,
        items: Vec<Item>,
        model: M,
    ) -> anyhow::Result<Vec<Input>>
    where
        M: FnOnce(Vec<Item>) -> F,
        F: Future<Output = anyhow::Result<Option<Verdict>>>,
    {
        let batch = items.into_iter().map(Reading::from).collect();
        let skim = |_| std::future::ready(Ok(Some(Vec::new())));
        let open = |_| std::future::ready(None);
        self.read(source, batch, skim, open, model).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookout::feed::tests::{RSS, answer};
    use crate::lookout::tests::{Answer, each_alone, group, item, lookout, related, unrelated};
    use crate::lookout::{Feed, items_from};
    use serde_json::json;

    #[tokio::test]
    async fn the_same_item_goes_through_once() {
        let w = lookout();
        assert_eq!(
            w.screen("x", vec![item("a")], each_alone(Outcome::Act))
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            w.screen("x", vec![item("a")], each_alone(Outcome::Act))
                .await
                .unwrap()
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn one_model_call_decides_relation_urgency_grouping_and_destination_for_a_batch() {
        let w = lookout();
        let mut calls = 0;
        let inputs = w
            .screen(
                "x",
                vec![
                    item("昼ごはん"),
                    item("脆弱性 A"),
                    item("脆弱性 A の続報"),
                    item("セール"),
                ],
                |items: Vec<Item>| {
                    calls += 1;
                    assert_eq!(items.len(), 4, "the whole batch is read at once");
                    std::future::ready(Ok(Some(Verdict {
                        items: vec![
                            unrelated(),
                            related(Urgency::Later),
                            related(Urgency::Now),
                            related(Urgency::Today),
                        ],
                        groups: vec![
                            group(&[1, 2], Outcome::Act, "脆弱性 A", "使っている"),
                            group(&[3], Outcome::Digest, "", "欲しいもの"),
                        ],
                    })))
                },
            )
            .await
            .unwrap();

        assert_eq!(calls, 1);
        assert_eq!(inputs.len(), 1, "one input per group passed on");
        assert!(inputs[0].text.contains("脆弱性 A の続報"));
        assert!(
            inputs[0].text.contains("urgency: Now"),
            "{}",
            inputs[0].text
        );
        let recent = w.before(0, 4).unwrap();
        let outcome_of = |title: &str| recent.iter().find(|d| d.title == title).unwrap();
        assert_eq!(outcome_of("昼ごはん").outcome, Outcome::Unrelated);
        assert_eq!(outcome_of("脆弱性 A").outcome, Outcome::Act);
        assert!(
            outcome_of("脆弱性 A")
                .reason
                .contains("2 件をまとめた · 急ぐ")
        );
        assert_eq!(outcome_of("セール").outcome, Outcome::Digest);
        assert!(outcome_of("セール").reason.contains("今日のうち"));
        let digest = w.digest().unwrap().unwrap();
        assert!(digest.text.contains("] セール ("), "{}", digest.text);
        assert!(w.digest().unwrap().is_none());
    }

    #[tokio::test]
    async fn an_unrelated_item_stays_dropped_even_when_a_group_names_it() {
        let w = lookout();
        let inputs = w
            .screen("x", vec![item("広告")], |_| {
                std::future::ready(Ok(Some(Verdict {
                    items: vec![unrelated()],
                    groups: vec![group(&[0], Outcome::Notify, "", "r")],
                })))
            })
            .await
            .unwrap();
        assert!(inputs.is_empty());
        assert_eq!(w.before(0, 1).unwrap()[0].outcome, Outcome::Unrelated);
    }

    #[tokio::test]
    async fn over_the_daily_cap_the_batch_goes_to_the_morning_digest() {
        let w = lookout();
        let inputs = w
            .screen("x", vec![item("a"), item("b")], |_| {
                std::future::ready(Ok(None))
            })
            .await
            .unwrap();
        assert!(inputs.is_empty());
        let recent = w.before(0, 2).unwrap();
        assert!(recent.iter().all(|d| d.outcome == Outcome::Digest));
        assert!(recent[0].reason.contains("上限"), "{}", recent[0].reason);
        assert_eq!(
            w.digest().unwrap().unwrap().text.matches("\n- #").count(),
            2
        );
    }

    #[tokio::test]
    async fn a_related_item_no_group_holds_goes_to_the_digest() {
        let w = lookout();
        w.screen("x", vec![item("a")], |_| {
            std::future::ready(Ok(Some(Verdict {
                items: Vec::new(),
                groups: Vec::new(),
            })))
        })
        .await
        .unwrap();
        assert_eq!(w.before(0, 1).unwrap()[0].outcome, Outcome::Digest);
    }

    async fn routed(route: Route, model: Outcome) -> (usize, Outcome) {
        let w = lookout();
        let id = w
            .add_source("x", "c", "t", &json!({}), Feed::Other, None)
            .unwrap();
        w.set_route(id, Some(route)).unwrap();
        let inputs = w
            .screen(&id.to_string(), vec![item("a")], each_alone(model))
            .await
            .unwrap();
        (inputs.len(), w.before(0, 1).unwrap()[0].outcome)
    }

    #[tokio::test]
    async fn a_source_with_its_own_route_sends_what_concerns_the_owner_there() {
        use Outcome::{Act, Digest, Logged, Notify};
        assert_eq!(routed(Route::Secretary, Digest).await, (1, Notify));
        assert_eq!(routed(Route::Secretary, Act).await, (1, Act));
        assert_eq!(routed(Route::Digest, Act).await, (0, Digest));
        assert_eq!(routed(Route::Log, Act).await, (0, Logged));
    }

    #[tokio::test]
    async fn what_does_not_concern_the_owner_goes_nowhere_whatever_the_route() {
        let w = lookout();
        let id = w
            .add_source("x", "c", "t", &json!({}), Feed::Other, None)
            .unwrap();
        w.set_route(id, Some(Route::Secretary)).unwrap();
        let inputs = w
            .screen(&id.to_string(), vec![item("広告")], |_| {
                std::future::ready(Ok(Some(Verdict {
                    items: vec![unrelated()],
                    groups: vec![group(&[0], Outcome::Notify, "", "r")],
                })))
            })
            .await
            .unwrap();
        assert!(inputs.is_empty());
        assert_eq!(w.before(0, 1).unwrap()[0].outcome, Outcome::Unrelated);
    }

    #[tokio::test]
    async fn a_source_only_logged_stays_out_of_the_digest_when_the_model_fails() {
        let w = lookout();
        let id = w
            .add_source("x", "c", "t", &json!({}), Feed::Other, None)
            .unwrap();
        w.set_route(id, Some(Route::Log)).unwrap();
        w.screen(&id.to_string(), vec![item("a")], |_| {
            std::future::ready(Ok(None))
        })
        .await
        .unwrap();
        assert!(w.digest().unwrap().is_none());
    }

    #[tokio::test]
    async fn a_failing_model_sends_its_batch_to_the_digest_instead_of_losing_it() {
        let w = lookout();
        w.screen("x", vec![item("a")], |_| {
            std::future::ready(Err(anyhow::anyhow!("timeout")))
        })
        .await
        .unwrap();
        let d = &w.before(0, 1).unwrap()[0];
        assert_eq!(d.outcome, Outcome::Digest);
        assert!(d.reason.contains("timeout"));
    }

    fn article(title: &str) -> Reading {
        Reading {
            item: item(title),
            more: Some(More::Text(format!("{title} 全文"))),
        }
    }

    fn linked(title: &str) -> Reading {
        Reading {
            item: item(title),
            more: Some(More::Page(format!("https://example.com/{title}"))),
        }
    }

    /// For batches no page of which is opened.
    fn no_page(_: String) -> std::future::Ready<Option<String>> {
        panic!("opened a page")
    }

    #[tokio::test]
    async fn the_page_of_what_the_skim_picks_is_opened_and_no_other() {
        let w = lookout();
        let opened = std::cell::RefCell::new(Vec::new());
        let mut read = Vec::new();

        w.read(
            "x",
            vec![linked("無関係"), linked("脆弱性"), linked("消えた")],
            |_| std::future::ready(Ok(Some(vec![1, 2]))),
            |url: String| {
                opened.borrow_mut().push(url.clone());
                std::future::ready(url.ends_with("脆弱性").then(|| "頁の本文".to_owned()))
            },
            |items| {
                read = items.clone();
                each_alone(Outcome::Digest)(items)
            },
        )
        .await
        .unwrap();

        assert_eq!(
            opened.into_inner(),
            ["https://example.com/脆弱性", "https://example.com/消えた"]
        );
        let read: Vec<_> = read.iter().map(|i| i.body.as_str()).collect();
        assert_eq!(
            read,
            ["脆弱性 body\n頁の本文", "消えた body"],
            "what cannot be opened is judged by its abstract"
        );
    }

    #[tokio::test]
    async fn only_so_many_pages_are_opened_at_once() {
        let w = lookout();
        let opened = std::cell::Cell::new(0);
        let batch: Vec<_> = (0..PAGES_AT_ONCE + 5)
            .map(|i| linked(&i.to_string()))
            .collect();
        let all = (0..batch.len()).collect::<Vec<_>>();
        let mut read = 0;

        w.read(
            "x",
            batch,
            |_| std::future::ready(Ok(Some(all))),
            |_| {
                opened.set(opened.get() + 1);
                std::future::ready(None)
            },
            |items| {
                read = items.len();
                each_alone(Outcome::Digest)(items)
            },
        )
        .await
        .unwrap();

        assert_eq!(opened.get(), PAGES_AT_ONCE);
        assert_eq!(read, PAGES_AT_ONCE + 5);
    }

    #[tokio::test]
    async fn only_what_the_skim_picks_is_read_in_full() {
        let w = lookout();
        let mut skimmed = Vec::new();
        let mut read = Vec::new();
        let batch = vec![article("無関係"), item("メール").into(), article("脆弱性")];

        w.read(
            "x",
            batch,
            |items| {
                skimmed = items;
                std::future::ready(Ok(Some(vec![1, 9])))
            },
            no_page,
            |items| {
                read = items.clone();
                each_alone(Outcome::Digest)(items)
            },
        )
        .await
        .unwrap();

        assert_eq!(skimmed, [item("無関係"), item("脆弱性")]);
        let read: Vec<_> = read.iter().map(|i| i.body.as_str()).collect();
        assert_eq!(read, ["メール body", "脆弱性 全文"]);
        let outcome = |title: &str| {
            w.before(0, 10)
                .unwrap()
                .into_iter()
                .find(|d| d.title == title)
                .unwrap()
                .outcome
        };
        assert_eq!(outcome("無関係"), Outcome::Unrelated);
        assert_eq!(outcome("メール"), Outcome::Digest);
        assert_eq!(outcome("脆弱性"), Outcome::Digest);
    }

    #[tokio::test]
    async fn what_has_nothing_to_open_is_not_skimmed_and_nothing_picked_is_not_read() {
        let w = lookout();
        w.read(
            "x",
            vec![item("a").into()],
            |_| -> std::future::Ready<anyhow::Result<Option<Vec<usize>>>> { panic!("skimmed") },
            no_page,
            each_alone(Outcome::Digest),
        )
        .await
        .unwrap();

        w.read(
            "x",
            vec![article("b")],
            |_| std::future::ready(Ok(Some(vec![]))),
            no_page,
            |_| -> Answer { panic!("read") },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_skim_that_fails_or_is_over_the_cap_sends_its_batch_to_the_digest() {
        let w = lookout();
        for (title, skim) in [("a", Ok(None)), ("b", Err(anyhow::anyhow!("down")))] {
            w.read(
                "x",
                vec![article(title)],
                |_| std::future::ready(skim),
                no_page,
                |_| -> Answer { panic!("read") },
            )
            .await
            .unwrap();
        }

        let outcomes: Vec<_> = w.before(0, 10).unwrap().iter().map(|d| d.outcome).collect();
        assert_eq!(outcomes, [Outcome::Digest, Outcome::Digest]);
    }

    #[tokio::test]
    async fn only_the_entries_a_feed_gained_are_read_again() {
        let w = lookout();
        let skim = |items: Vec<Item>| std::future::ready(Ok(Some((0..items.len()).collect())));
        let unopened = |_: String| std::future::ready(None);
        let first = items_from(&answer(RSS));
        let more = RSS.replace(
            "<item><title>A",
            "<item><title>新しい</title><link>https://example.com/3</link></item><item><title>A",
        );
        assert_eq!(first.len(), 2);
        let none = |items: Vec<Item>| async move {
            Ok(Some(Verdict {
                items: vec![
                    Screened {
                        related: false,
                        urgency: Urgency::Later
                    };
                    items.len()
                ],
                groups: vec![],
            }))
        };
        w.read("feed", first, skim, unopened, none).await.unwrap();

        let mut read = Vec::new();
        w.read(
            "feed",
            items_from(&answer(&more)),
            skim,
            unopened,
            |items| {
                read = items.clone();
                none(items)
            },
        )
        .await
        .unwrap();

        let titles: Vec<_> = read.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["新しい"]);
    }
}
