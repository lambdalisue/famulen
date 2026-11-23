//! The sources the lookout reads, and what arrived for them.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Item, Lookout, Route};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LookoutSource {
    pub id: i64,
    pub name: String,
    pub connection: String,
    pub tool: String,
    pub args: Value,
    /// Seconds between reads: its own, or else the default for its feed.
    pub interval: i64,
    /// The interval the owner chose for this source alone.
    pub own_interval: Option<i64>,
    pub feed: Feed,
    /// Where the owner sends what concerns them; the model decides when none.
    pub route: Option<Route>,
    pub enabled: bool,
    pub last_run: Option<i64>,
}

/// The tool of a source that reads what arrived for its connection, as the
/// gateway's receivers handed it over, instead of calling a tool.
pub const ARRIVED: &str = "arrived";

/// Where something arrives within a connection: a mail label, a chat channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub id: String,
    pub name: String,
}

/// The arguments of a source that reads what arrives in `places`.
pub fn arrived_in(places: &[Place]) -> Value {
    if places.is_empty() {
        json!({})
    } else {
        json!({ "places": places })
    }
}

impl LookoutSource {
    /// The places a source reading what arrives is narrowed to; none reads
    /// whatever no other source is narrowed to.
    pub fn places(&self) -> Vec<Place> {
        places_in(&self.args)
    }
}

fn places_in(args: &Value) -> Vec<Place> {
    serde_json::from_value(args["places"].clone()).unwrap_or_default()
}

/// What a source reads, which decides how often it is read until the owner
/// says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Feed {
    Mail,
    Chat,
    Other,
}

impl Feed {
    pub const ALL: [Feed; 3] = [Feed::Mail, Feed::Chat, Feed::Other];

    pub fn as_str(self) -> &'static str {
        match self {
            Feed::Mail => "mail",
            Feed::Chat => "chat",
            Feed::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Feed {
        Feed::ALL
            .into_iter()
            .find(|f| f.as_str() == s)
            .unwrap_or(Feed::Other)
    }

    fn key(self) -> String {
        format!("lookout.every.{}", self.as_str())
    }

    /// Seconds between reads until the owner changes the default.
    pub fn every(self) -> i64 {
        match self {
            Feed::Mail => 30 * 60,
            Feed::Chat => 5 * 60,
            Feed::Other => 15 * 60,
        }
    }
}

impl Lookout {
    pub fn add_source(
        &self,
        name: impl AsRef<str>,
        connection: impl AsRef<str>,
        tool: impl AsRef<str>,
        args: &Value,
        feed: Feed,
        interval: Option<i64>,
    ) -> anyhow::Result<i64> {
        anyhow::ensure!(!name.as_ref().trim().is_empty(), "a lookout needs a name");
        anyhow::ensure!(
            interval.is_none_or(|i| i >= 60),
            "a lookout runs at most once a minute"
        );
        // An event goes to one source, so a second one would never be given any.
        anyhow::ensure!(
            tool.as_ref() != ARRIVED
                || !places_in(args).is_empty()
                || self.rest_of(connection.as_ref())?.is_none(),
            "what arrives on this connection is already watched"
        );
        let conn = self.store.conn();
        conn.execute(
            "INSERT INTO lookout_sources (name, connection, tool, args, feed, interval) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![name.as_ref().trim(), connection.as_ref(), tool.as_ref(), args.to_string(), feed.as_str(), interval],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn sources(&self) -> anyhow::Result<Vec<LookoutSource>> {
        let defaults = self.defaults()?;
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, connection, tool, args, interval, enabled, last_run, feed, route FROM lookout_sources ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            let args: String = r.get(4)?;
            let feed = Feed::parse(&r.get::<_, String>(8)?);
            let own_interval: Option<i64> = r.get(5)?;
            let default = defaults
                .iter()
                .find(|(f, _)| *f == feed)
                .map_or(feed.every(), |(_, every)| *every);
            Ok(LookoutSource {
                id: r.get(0)?,
                name: r.get(1)?,
                connection: r.get(2)?,
                tool: r.get(3)?,
                args: serde_json::from_str(&args).unwrap_or(Value::Null),
                interval: own_interval.unwrap_or(default),
                own_interval,
                feed,
                route: r
                    .get::<_, Option<String>>(9)?
                    .as_deref()
                    .and_then(Route::parse),
                enabled: r.get::<_, i64>(6)? != 0,
                last_run: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// How often each feed is read by a source with no interval of its own.
    pub fn defaults(&self) -> anyhow::Result<Vec<(Feed, i64)>> {
        Feed::ALL
            .into_iter()
            .map(|f| {
                let every = self
                    .store
                    .state(f.key())?
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(f.every());
                Ok((f, every))
            })
            .collect()
    }

    /// Change how often `feed` is read by sources with no interval of their
    /// own, in seconds; a minute at least.
    pub fn set_default(&self, feed: Feed, every: i64) -> anyhow::Result<()> {
        anyhow::ensure!(every >= 60, "a lookout runs at most once a minute");
        self.store.set_state(feed.key(), every.to_string())
    }

    pub fn set_enabled(&self, id: i64, enabled: bool) -> anyhow::Result<()> {
        self.store.conn().execute(
            "UPDATE lookout_sources SET enabled = ?2 WHERE id = ?1",
            params![id, enabled as i64],
        )?;
        Ok(())
    }

    /// How often a source is read, in seconds and a minute at least, or
    /// `None` to follow the default for its feed again.
    pub fn set_interval(&self, id: i64, interval: Option<i64>) -> anyhow::Result<()> {
        anyhow::ensure!(
            interval.is_none_or(|i| i >= 60),
            "a lookout runs at most once a minute"
        );
        let changed = self.store.conn().execute(
            "UPDATE lookout_sources SET interval = ?2 WHERE id = ?1",
            params![id, interval],
        )?;
        anyhow::ensure!(changed == 1, "no lookout {id}");
        Ok(())
    }

    pub fn remove_source(&self, id: i64) -> anyhow::Result<()> {
        let conn = self.store.conn();
        conn.execute("DELETE FROM lookout_sources WHERE id = ?1", [id])?;
        conn.execute(
            "DELETE FROM lookout_heard WHERE source = ?1",
            [id.to_string()],
        )?;
        Ok(())
    }

    /// Where a source sends what concerns the owner; none leaves it to the
    /// model.
    pub fn set_route(&self, id: i64, route: Option<Route>) -> anyhow::Result<()> {
        self.store.conn().execute(
            "UPDATE lookout_sources SET route = ?2 WHERE id = ?1",
            params![id, route.map(Route::as_str)],
        )?;
        Ok(())
    }

    pub fn rename(&self, id: i64, name: &str) -> anyhow::Result<()> {
        anyhow::ensure!(!name.trim().is_empty(), "a lookout needs a name");
        self.store.conn().execute(
            "UPDATE lookout_sources SET name = ?2 WHERE id = ?1",
            params![id, name.trim()],
        )?;
        Ok(())
    }

    /// The sources whose interval has passed.
    pub fn due(&self, at: i64) -> anyhow::Result<Vec<LookoutSource>> {
        Ok(self
            .sources()?
            .into_iter()
            .filter(|s| s.enabled && s.last_run.is_none_or(|last| at - last >= s.interval))
            .collect())
    }

    pub fn mark_run(&self, id: i64, at: i64) -> anyhow::Result<()> {
        self.store.conn().execute(
            "UPDATE lookout_sources SET last_run = ?2 WHERE id = ?1",
            params![id, at],
        )?;
        Ok(())
    }

    /// The sources that read what arrives on `connection`.
    fn arrived_sources(&self, connection: &str) -> anyhow::Result<Vec<LookoutSource>> {
        Ok(self
            .sources()?
            .into_iter()
            .filter(|s| s.connection == connection && s.tool == ARRIVED)
            .collect())
    }

    /// The source that reads whatever no other is narrowed to.
    fn rest_of(&self, connection: &str) -> anyhow::Result<Option<LookoutSource>> {
        Ok(self
            .arrived_sources(connection)?
            .into_iter()
            .find(|s| s.places().is_empty()))
    }

    /// Change the tool a source calls and its arguments. A source reading
    /// what arrives stays one.
    pub fn set_reading(&self, id: i64, tool: &str, args: &Value) -> anyhow::Result<()> {
        anyhow::ensure!(tool != ARRIVED, "what arrives is read without a tool");
        let changed = self.store.conn().execute(
            "UPDATE lookout_sources SET tool = ?2, args = ?3 WHERE id = ?1 AND tool != ?4",
            params![id, tool, args.to_string(), ARRIVED],
        )?;
        anyhow::ensure!(changed == 1, "no lookout {id} that calls a tool");
        Ok(())
    }

    /// Change the places a source reading what arrives is narrowed to.
    pub fn set_places(&self, id: i64, places: &[Place]) -> anyhow::Result<()> {
        let source = self
            .sources()?
            .into_iter()
            .find(|s| s.id == id && s.tool == ARRIVED)
            .ok_or_else(|| anyhow::anyhow!("no lookout {id} that reads what arrives"))?;
        if places.is_empty() {
            anyhow::ensure!(
                self.rest_of(&source.connection)?
                    .is_none_or(|rest| rest.id == id),
                "what arrives on this connection is already watched"
            );
        }
        self.store.conn().execute(
            "UPDATE lookout_sources SET args = ?2 WHERE id = ?1",
            params![id, arrived_in(places).to_string()],
        )?;
        Ok(())
    }

    /// Keep what arrived on `connection` until the source that reads it is
    /// read. Nothing is kept when no source reads it.
    pub fn hear(&self, connection: impl AsRef<str>, item: &Item) -> anyhow::Result<bool> {
        self.hear_at(connection, item, &[])
    }

    /// [`Self::hear`] for something that arrived in `places`: the first
    /// source narrowed to one of them takes it, else the one narrowed to none.
    pub fn hear_at(
        &self,
        connection: impl AsRef<str>,
        item: &Item,
        places: &[String],
    ) -> anyhow::Result<bool> {
        let sources = self.arrived_sources(connection.as_ref())?;
        let narrowed = sources
            .iter()
            .find(|s| s.places().iter().any(|p| places.contains(&p.id)));
        let Some(source) = narrowed
            .or_else(|| sources.iter().find(|s| s.places().is_empty()))
            .filter(|s| s.enabled)
        else {
            return Ok(false);
        };
        self.store.conn().execute(
            "INSERT INTO lookout_heard (source, title, body) VALUES (?1, ?2, ?3)",
            params![source.id.to_string(), item.title, item.body],
        )?;
        Ok(true)
    }

    /// What the source reading `connection` has waiting.
    #[cfg(test)]
    pub fn heard_on(&self, connection: impl AsRef<str>) -> anyhow::Result<Vec<Item>> {
        match self.rest_of(connection.as_ref())? {
            Some(source) => self.take_heard(source.id),
            None => Ok(Vec::new()),
        }
    }

    /// Everything heard for a source since the last read.
    pub fn take_heard(&self, source: i64) -> anyhow::Result<Vec<Item>> {
        let source = source.to_string();
        let conn = self.store.conn();
        let items: Vec<(i64, Item)> = {
            let mut stmt = conn.prepare(
                "SELECT id, title, body FROM lookout_heard WHERE source = ?1 ORDER BY id",
            )?;
            stmt.query_map([&source], |r| {
                Ok((
                    r.get(0)?,
                    Item {
                        title: r.get(1)?,
                        body: r.get(2)?,
                    },
                ))
            })?
            .collect::<Result<_, _>>()?
        };
        if let Some((last, _)) = items.last() {
            conn.execute(
                "DELETE FROM lookout_heard WHERE source = ?1 AND id <= ?2",
                params![source, last],
            )?;
        }
        Ok(items.into_iter().map(|(_, i)| i).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookout::tests::{item, lookout};

    #[test]
    fn a_route_and_a_name_are_changed_later_and_a_route_goes_back_to_the_model() {
        let w = lookout();
        let id = w
            .add_source("x", "c", "t", &json!({}), Feed::Other, None)
            .unwrap();
        w.set_route(id, Some(Route::Digest)).unwrap();
        w.rename(id, "新しい名前").unwrap();
        let s = &w.sources().unwrap()[0];
        assert_eq!(
            (s.name.as_str(), s.route),
            ("新しい名前", Some(Route::Digest))
        );
        w.set_route(id, None).unwrap();
        assert_eq!(w.sources().unwrap()[0].route, None);
        assert!(w.rename(id, " ").is_err());
    }

    #[test]
    fn what_arrives_for_a_watched_connection_waits_for_the_next_read() {
        let w = lookout();
        let item = Item {
            title: "障害".into(),
            body: "b".into(),
        };
        assert!(
            !w.hear("slack", &item).unwrap(),
            "nothing is kept unwatched"
        );
        w.add_source("Slack", "slack", ARRIVED, &json!({}), Feed::Other, Some(60))
            .unwrap();
        assert!(w.hear("slack", &item).unwrap());
        assert_eq!(w.heard_on("slack").unwrap(), vec![item]);
        assert!(w.heard_on("slack").unwrap().is_empty());
    }

    #[test]
    fn what_arrives_is_kept_only_for_a_source_that_reads_what_arrives() {
        let w = lookout();
        let item = Item {
            title: "x".into(),
            body: "x".into(),
        };
        w.add_source("検索", "c", "search", &json!({}), Feed::Other, None)
            .unwrap();
        assert!(!w.hear("c", &item).unwrap(), "a tool is called instead");
        let id = w
            .add_source("届いたもの", "c", ARRIVED, &json!({}), Feed::Other, None)
            .unwrap();
        assert!(w.hear("c", &item).unwrap());
        assert_eq!(w.take_heard(id).unwrap().len(), 1);
    }

    #[test]
    fn a_connection_has_one_source_that_reads_what_arrives() {
        let w = lookout();
        w.add_source("一つ目", "c", ARRIVED, &json!({}), Feed::Other, None)
            .unwrap();
        assert!(
            w.add_source("二つ目", "c", ARRIVED, &json!({}), Feed::Other, None)
                .is_err()
        );
        w.add_source("別の先", "d", ARRIVED, &json!({}), Feed::Other, None)
            .unwrap();
        w.add_source("検索", "c", "search", &json!({}), Feed::Other, None)
            .unwrap();
    }

    fn placed(places: &[&str]) -> Value {
        let places: Vec<Value> = places
            .iter()
            .map(|p| json!({ "id": p, "name": p }))
            .collect();
        json!({ "places": places })
    }

    #[test]
    fn an_event_goes_to_the_source_that_names_its_place_and_else_to_the_one_naming_none() {
        let w = lookout();
        let add = |name: &str, places: &[&str]| {
            w.add_source(name, "c", ARRIVED, &placed(places), Feed::Other, None)
                .unwrap()
        };
        let rest = add("残り", &[]);
        let work = add("仕事", &["work", "boss"]);
        let late = add("あと", &["work"]);
        w.hear_at("c", &item("a"), &["work".into()]).unwrap();
        w.hear_at("c", &item("b"), &["home".into()]).unwrap();
        w.hear("c", &item("c")).unwrap();
        let heard = |id| -> Vec<String> {
            w.take_heard(id)
                .unwrap()
                .into_iter()
                .map(|i| i.title)
                .collect()
        };
        assert_eq!(heard(work), ["a"]);
        assert_eq!(heard(late), Vec::<String>::new(), "each goes to one source");
        assert_eq!(heard(rest), ["b", "c"]);
    }

    #[test]
    fn what_a_stopped_source_would_read_is_not_handed_to_the_rest() {
        let w = lookout();
        let rest = w
            .add_source("残り", "c", ARRIVED, &json!({}), Feed::Other, None)
            .unwrap();
        let quiet = w
            .add_source(
                "止めた",
                "c",
                ARRIVED,
                &placed(&["noise"]),
                Feed::Other,
                None,
            )
            .unwrap();
        w.set_enabled(quiet, false).unwrap();
        assert!(!w.hear_at("c", &item("a"), &["noise".into()]).unwrap());
        assert!(w.take_heard(rest).unwrap().is_empty());
    }

    #[test]
    fn places_are_changed_later_but_never_to_a_second_source_naming_none() {
        let w = lookout();
        w.add_source("残り", "c", ARRIVED, &json!({}), Feed::Other, None)
            .unwrap();
        let id = w
            .add_source("仕事", "c", ARRIVED, &placed(&["work"]), Feed::Other, None)
            .unwrap();
        let home = Place {
            id: "home".into(),
            name: "家".into(),
        };
        w.set_places(id, std::slice::from_ref(&home)).unwrap();
        let s = w.sources().unwrap().into_iter().find(|s| s.id == id);
        assert_eq!(s.unwrap().places(), [home]);
        assert!(w.set_places(id, &[]).is_err());
    }

    #[test]
    fn what_a_removed_source_had_not_read_is_gone() {
        let w = lookout();
        let item = Item {
            title: "x".into(),
            body: "x".into(),
        };
        let add = || {
            w.add_source("届いたもの", "c", ARRIVED, &json!({}), Feed::Other, None)
                .unwrap()
        };
        let first = add();
        w.hear("c", &item).unwrap();
        w.remove_source(first).unwrap();
        let second = add();
        assert!(w.take_heard(second).unwrap().is_empty());
    }

    #[test]
    fn sources_come_due_by_their_interval() {
        let w = lookout();
        let id = w
            .add_source(
                "Gmail",
                "gmail-work",
                "search_threads",
                &json!({}),
                Feed::Other,
                Some(900),
            )
            .unwrap();
        assert_eq!(w.due(1000).unwrap().len(), 1);
        w.mark_run(id, 1000).unwrap();
        assert!(w.due(1500).unwrap().is_empty());
        assert_eq!(w.due(1900).unwrap().len(), 1);
        assert!(
            w.add_source("x", "c", "t", &json!({}), Feed::Other, Some(10))
                .is_err()
        );
    }

    #[test]
    fn a_sources_interval_is_changed_later_and_not_below_a_minute() {
        let w = lookout();
        let id = w
            .add_source("Slack", "slack", ARRIVED, &json!({}), Feed::Other, Some(60))
            .unwrap();
        w.mark_run(id, 1000).unwrap();

        w.set_interval(id, Some(600)).unwrap();

        assert!(w.due(1100).unwrap().is_empty());
        assert_eq!(w.due(1600).unwrap().len(), 1);
        assert!(w.set_interval(id, Some(30)).is_err());
        assert_eq!(w.sources().unwrap()[0].interval, 600);
    }

    #[test]
    fn a_source_without_its_own_interval_follows_the_default_for_what_it_reads() {
        let w = lookout();
        let mail = w
            .add_source("Gmail", "g", ARRIVED, &json!({}), Feed::Mail, None)
            .unwrap();
        let chat = w
            .add_source("Slack", "s", ARRIVED, &json!({}), Feed::Chat, None)
            .unwrap();
        let every = |id| {
            w.sources()
                .unwrap()
                .into_iter()
                .find(|s| s.id == id)
                .unwrap()
        };
        assert_eq!(every(mail).interval, 1800);
        assert_eq!(every(chat).interval, 300);
        assert_eq!(every(chat).own_interval, None);

        w.set_default(Feed::Chat, 120).unwrap();
        assert_eq!(every(chat).interval, 120);
        assert_eq!(every(mail).interval, 1800);
        assert!(w.set_default(Feed::Chat, 30).is_err());
        assert_eq!(
            w.defaults().unwrap(),
            [(Feed::Mail, 1800), (Feed::Chat, 120), (Feed::Other, 900)]
        );
    }

    #[test]
    fn a_source_with_its_own_interval_keeps_it_until_it_goes_back_to_the_default() {
        let w = lookout();
        let id = w
            .add_source("Slack", "s", ARRIVED, &json!({}), Feed::Chat, Some(600))
            .unwrap();
        w.set_default(Feed::Chat, 120).unwrap();
        let source = || w.sources().unwrap().remove(0);
        assert_eq!(source().interval, 600);
        assert_eq!(source().own_interval, Some(600));

        w.set_interval(id, None).unwrap();
        assert_eq!(source().interval, 120);
        assert_eq!(source().own_interval, None);
    }
}
