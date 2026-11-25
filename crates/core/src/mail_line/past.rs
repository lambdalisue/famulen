//! Going back over a mailbox for what came before. Mail that may tell of what
//! a line reads is put in it a month at a time, oldest first, so that an
//! order stands before its delivery and return.

use chrono::{Datelike, Months, NaiveDate};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use super::{Line, SCREENED};
use crate::app::App;
use crate::arrivals;
use crate::models::{CapUnit, Role};
use crate::store::{Store, now};

const SEARCH: &str = "search_messages";
/// The most pages one month is read in.
const PAGES: usize = 50;
/// Mail before this has no shop to tell of.
const EARLIEST: i32 = 1995;

/// Mail of a connection found going back.
const PAST: &str = "id IN (SELECT mail FROM {p}past_mails) AND connection = ?1";

/// How going back over a mailbox stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    Going,
    Stopped,
    Done,
}

impl Standing {
    fn parse(s: &str) -> Self {
        match s {
            "going" => Self::Going,
            "done" => Self::Done,
            _ => Self::Stopped,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Past {
    pub connection: String,
    pub since: NaiveDate,
    /// The month that is searched next.
    pub month: NaiveDate,
    /// The day it was started, up to which is searched.
    pub until: NaiveDate,
    pub standing: Standing,
    /// Every month was searched; what was found may still be read.
    pub searched: bool,
    /// Mails found, and those still to be read.
    pub found: i64,
    pub left: i64,
    /// Things added from them.
    pub added: i64,
    /// Why it does not get on, or empty.
    pub problem: String,
}

fn first_of(day: NaiveDate) -> NaiveDate {
    day.with_day(1).unwrap_or(day)
}

/// Go back over a mailbox, from `since` on, or go on where it was stopped.
/// Mail read before is not read again, so it may be started anew with an
/// earlier day; mail that could not be read is tried anew.
pub fn start(
    store: &Store,
    line: Line,
    connection: &str,
    since: NaiveDate,
    today: NaiveDate,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        since < today && since.year() >= EARLIEST,
        "mail is gone back over from {EARLIEST} on, from a day before today"
    );
    let mut conn = store.conn();
    let tx = conn.transaction()?;
    let resumed = tx.execute(
        &line.sql(
            "UPDATE {p}past SET state = 'going', problem = '', next_at = 0
         WHERE connection = ?1 AND since = ?2 AND state = 'stopped'",
        ),
        params![connection, since.to_string()],
    )?;
    if resumed == 0 {
        tx.execute(
            &line.sql("INSERT INTO {p}past (connection, since, month, until, state) VALUES (?1, ?2, ?3, ?4, 'going')
             ON CONFLICT (connection) DO UPDATE SET
                 since = ?2, month = ?3, until = ?4, state = 'going', problem = '', next_at = 0"),
            params![
                connection,
                since.to_string(),
                first_of(since).to_string(),
                today.to_string()
            ],
        )?;
    }
    tx.execute(
        &line.sql(&format!(
            "DELETE FROM {{p}}away WHERE mail IN (SELECT id FROM {{p}}mails WHERE {PAST})"
        )),
        [connection],
    )?;
    // What is screened has no title any more.
    tx.execute(
        &line.sql(&format!(
            "UPDATE {{p}}mails SET tries = 0, next_at = 0,
                 state = CASE WHEN title = '' THEN 'waiting' ELSE 'unscreened' END
             WHERE {PAST} AND state = 'failed'"
        )),
        [connection],
    )?;
    tx.commit()?;
    Ok(())
}

/// Stop. What was found and not read waits until it is started again.
pub fn stop(store: &Store, line: Line, connection: &str) -> anyhow::Result<()> {
    store.conn().execute(
        &line.sql(
            "UPDATE {p}past SET state = 'stopped', problem = ''
         WHERE connection = ?1 AND state = 'going'",
        ),
        [connection],
    )?;
    Ok(())
}

pub fn of(store: &Store, line: Line, connection: &str) -> anyhow::Result<Option<Past>> {
    let conn = store.conn();
    let row = conn
        .query_row(
            &line.sql(
                "SELECT since, month, until, state, problem,
                 (SELECT COUNT(*) FROM {p}past_mails p JOIN {p}mails m ON m.id = p.mail
                      WHERE m.connection = ?1),
                 (SELECT COUNT(*) FROM {p}past_mails p JOIN {p}mails m ON m.id = p.mail
                      WHERE m.connection = ?1 AND m.state IN ('unscreened', 'waiting')),
                 (SELECT COUNT(*) FROM {added} a JOIN {p}past_mails p ON p.mail = a.mail
                      JOIN {p}mails m ON m.id = p.mail WHERE m.connection = ?1)
             FROM {p}past WHERE connection = ?1",
            ),
            [connection],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    Past {
                        connection: connection.to_owned(),
                        since: NaiveDate::MIN,
                        month: NaiveDate::MIN,
                        until: NaiveDate::MIN,
                        standing: Standing::Stopped,
                        searched: false,
                        problem: r.get(4)?,
                        found: r.get(5)?,
                        left: r.get(6)?,
                        added: r.get(7)?,
                    },
                ))
            },
        )
        .optional()?;
    let Some((since, month, until, state, past)) = row else {
        return Ok(None);
    };
    let (month, until): (NaiveDate, NaiveDate) = (month.parse()?, until.parse()?);
    Ok(Some(Past {
        since: since.parse()?,
        month,
        until,
        searched: month > until,
        standing: Standing::parse(&state),
        ..past
    }))
}

/// Whether going back may use the model now. It leaves half of a day's cap to
/// what arrives.
pub(crate) fn may_ask(app: &App) -> anyhow::Result<bool> {
    let Some(assignment) = app.models.assignment(Role::Lookout)? else {
        return Ok(false);
    };
    let Some(cap) = assignment.cap else {
        return Ok(true);
    };
    let (cost, calls) = app
        .models
        .used_today(Role::Lookout, app.tz(), chrono::Utc::now())?;
    let used = match assignment.cap_unit {
        CapUnit::Yen => cost,
        CapUnit::Calls => calls as f64,
    };
    Ok(used < cap / 2.0)
}

/// Search the next month of each mailbox that is gone back over, once what
/// was found before is screened.
pub async fn go_on(app: &App, line: Line) -> anyhow::Result<()> {
    let at = now();
    let going: Vec<String> = {
        let conn = app.store.conn();
        let mut stmt = conn.prepare(
            &line.sql("SELECT connection FROM {p}past WHERE state = 'going' AND next_at <= ?1"),
        )?;
        stmt.query_map([at], |r| r.get(0))?
            .collect::<Result<_, _>>()?
    };
    for connection in going {
        if app.connections.get(&connection)?.is_none() {
            stop(&app.store, line, &connection)?;
            continue;
        }
        let Some(past) = of(&app.store, line, &connection)? else {
            continue;
        };
        if past.searched {
            if past.left == 0 {
                done(&app.store, line, &past)?;
                app.console.notify(crate::console_log::Event::Records);
            }
            continue;
        }
        if unscreened(&app.store, line, &connection)? >= SCREENED as i64 {
            continue;
        }
        let (problem, wait) = match month(app, line, &past).await {
            Ok(()) => (String::new(), 0),
            Err(error) => {
                tracing::warn!(%connection, %error, "could not search past mail");
                (error.to_string(), super::AGAIN_AFTER)
            }
        };
        app.store.conn().execute(
            &line.sql(
                "UPDATE {p}past SET problem = ?2, next_at = ?3
             WHERE connection = ?1 AND since = ?4 AND state = 'going'",
            ),
            params![connection, problem, at + wait, past.since.to_string()],
        )?;
        app.console.notify(crate::console_log::Event::Records);
    }
    Ok(())
}

/// All was searched and read: returns of what was not known then are read
/// again, as it may be known now.
fn done(store: &Store, line: Line, past: &Past) -> anyhow::Result<()> {
    let mut conn = store.conn();
    let tx = conn.transaction()?;
    tx.execute(
        &line.sql("UPDATE {p}past SET state = 'done' WHERE connection = ?1 AND since = ?2 AND state = 'going'"),
        params![past.connection, past.since.to_string()],
    )?;
    tx.execute(
        &line.sql(
            "UPDATE {p}mails SET state = 'waiting', tries = 0, next_at = 0
         WHERE connection = ?1 AND state = 'unmatched'",
        ),
        [&past.connection],
    )?;
    tx.commit()?;
    Ok(())
}

fn unscreened(store: &Store, line: Line, connection: &str) -> anyhow::Result<i64> {
    Ok(store.conn().query_row(
        &line.sql(
            "SELECT COUNT(*) FROM {p}past_mails p JOIN {p}mails m ON m.id = p.mail
         WHERE m.connection = ?1 AND m.state = 'unscreened'",
        ),
        [connection],
        |r| r.get(0),
    )?)
}

/// Put the mail of a month in line, some at a time; the month is left once
/// all of it is.
async fn month(app: &App, line: Line, past: &Past) -> anyhow::Result<()> {
    let next = past.month + Months::new(1);
    let from = past.month.max(past.since);
    // What came since it was started is the lookout's.
    let before = next.min(past.until);
    let query = format!(
        "{} -in:chats after:{} before:{}",
        line.likely(),
        from.format("%Y/%m/%d"),
        before.format("%Y/%m/%d")
    );
    let mut ids = Vec::new();
    let mut page: Option<String> = None;
    let pages = if from < before { PAGES } else { 0 };
    for _ in 0..pages {
        let mut args = json!({ "query": query, "max": 500 });
        if let Some(p) = &page {
            args["page"] = json!(p);
        }
        let answer = app.tool_json(&past.connection, SEARCH, args).await?;
        ids.extend(
            answer["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m["id"].as_str().map(str::to_owned)),
        );
        page = answer["nextPageToken"].as_str().map(str::to_owned);
        if page.is_none() {
            break;
        }
    }
    let mut put = 0;
    // Gmail lists the newest first.
    for id in ids.iter().rev() {
        if known(&app.store, line, &past.connection, id)? {
            continue;
        }
        if put == SCREENED {
            return Ok(());
        }
        let message = match app
            .tool_json(&past.connection, "get_message", json!({ "id": id }))
            .await
        {
            Ok(message) => message,
            Err(error) if arrivals::is_gone(&error) => continue,
            Err(error) => return Err(error),
        };
        add(&app.store, line, &past.connection, id, &message)?;
        put += 1;
    }
    // The owner may have stopped it or started it anew meanwhile.
    app.store.conn().execute(
        &line.sql(
            "UPDATE {p}past SET month = ?3 WHERE connection = ?1 AND month = ?2 AND since = ?4",
        ),
        params![
            past.connection,
            past.month.to_string(),
            next.to_string(),
            past.since.to_string()
        ],
    )?;
    Ok(())
}

fn known(store: &Store, line: Line, connection: &str, message: &str) -> anyhow::Result<bool> {
    Ok(store.conn().query_row(
        &line.sql("SELECT EXISTS (SELECT 1 FROM {p}mails WHERE connection = ?1 AND message = ?2)"),
        [connection, message],
        |r| r.get(0),
    )?)
}

fn add(
    store: &Store,
    line: Line,
    connection: &str,
    id: &str,
    message: &Value,
) -> anyhow::Result<()> {
    let item = arrivals::mail_item(message);
    let mut conn = store.conn();
    let tx = conn.transaction()?;
    let added = tx.execute(
        &line.sql(
            "INSERT OR IGNORE INTO {p}mails (connection, message, at, state, title, body)
         VALUES (?1, ?2, ?3, 'unscreened', ?4, ?5)",
        ),
        params![connection, id, now(), item.title, item.body],
    )?;
    if added > 0 {
        tx.execute(
            &line.sql("INSERT INTO {p}past_mails (mail) VALUES (last_insert_rowid())"),
            [],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_line::tests::with_model;
    use crate::mail_line::{read_waiting, screen_left, waiting};

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn mail(subject: &str) -> Value {
        json!({ "id": subject, "snippet": "s", "payload": { "headers": [{ "name": "Subject", "value": subject }] } })
    }

    async fn mailbox(dir: &std::path::Path) -> (App, String) {
        arrivals::tests::with_google(
            dir,
            vec![
                (
                    "/gmail/v1/users/me/messages",
                    json!({ "messages": [{ "id": "new" }, { "id": "old" }] }),
                ),
                ("/gmail/v1/users/me/messages/new", mail("new")),
                ("/gmail/v1/users/me/messages/old", mail("old")),
            ],
        )
        .await
    }

    fn lined_up(app: &App) -> Vec<String> {
        let conn = app.store.conn();
        let mut stmt = conn
            .prepare("SELECT message FROM purchase_mails ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[tokio::test]
    async fn a_mailbox_is_gone_over_a_month_at_a_time_oldest_first_and_each_mail_once() {
        let dir = crate::notebook::testing::TempDir::new("past-months");
        let (app, id) = mailbox(dir.path()).await;
        start(
            &app.store,
            Line::Purchases,
            &id,
            day("2026-10-20"),
            day("2026-11-25"),
        )
        .unwrap();

        go_on(&app, Line::Purchases).await.unwrap();

        assert_eq!(lined_up(&app), ["old", "new"]);
        let past = of(&app.store, Line::Purchases, &id).unwrap().unwrap();
        assert_eq!(
            (past.month, past.standing),
            (day("2026-11-01"), Standing::Going)
        );
        assert_eq!((past.found, past.left), (2, 2));

        go_on(&app, Line::Purchases).await.unwrap();
        go_on(&app, Line::Purchases).await.unwrap();

        assert_eq!(lined_up(&app).len(), 2);
        let past = of(&app.store, Line::Purchases, &id).unwrap().unwrap();
        assert!(past.searched);
        assert_eq!(past.standing, Standing::Going, "two are still to be read");
    }

    #[tokio::test]
    async fn one_goes_on_from_the_month_it_was_stopped_at() {
        let dir = crate::notebook::testing::TempDir::new("past-resume");
        let (app, id) = mailbox(dir.path()).await;
        let (since, today) = (day("2026-08-10"), day("2026-11-25"));
        start(&app.store, Line::Purchases, &id, since, today).unwrap();
        go_on(&app, Line::Purchases).await.unwrap();
        stop(&app.store, Line::Purchases, &id).unwrap();

        start(&app.store, Line::Purchases, &id, since, today).unwrap();

        let past = of(&app.store, Line::Purchases, &id).unwrap().unwrap();
        assert_eq!(past.month, day("2026-09-01"));
        start(&app.store, Line::Purchases, &id, day("2026-07-10"), today).unwrap();
        let past = of(&app.store, Line::Purchases, &id).unwrap().unwrap();
        assert_eq!(past.month, day("2026-07-01"));
    }

    #[tokio::test]
    async fn what_is_still_to_be_read_can_be_stopped_after_all_was_searched() {
        let dir = crate::notebook::testing::TempDir::new("past-stop-reading");
        let (app, id) = mailbox(dir.path()).await;
        with_model(&app, json!({ "purchases": [0, 1] })).await;
        start(
            &app.store,
            Line::Purchases,
            &id,
            day("2026-11-01"),
            day("2026-11-25"),
        )
        .unwrap();
        go_on(&app, Line::Purchases).await.unwrap();
        screen_left(&app, Line::Purchases).await.unwrap();
        go_on(&app, Line::Purchases).await.unwrap();
        stop(&app.store, Line::Purchases, &id).unwrap();
        assert!(
            waiting(&app.store, Line::Purchases, i64::MAX, true)
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_return_that_came_before_its_order_was_found_is_read_again_at_the_end() {
        let dir = crate::notebook::testing::TempDir::new("past-return");
        let (app, id) = mailbox(dir.path()).await;
        with_model(&app, json!({ "purchases": [] })).await;
        super::super::enqueue(&app.store, Line::Purchases, &id, "return").unwrap();
        let mail = waiting(&app.store, Line::Purchases, i64::MAX, true)
            .unwrap()
            .remove(0);
        super::super::unmatched(&app.store, Line::Purchases, mail.id).unwrap();
        assert!(
            waiting(&app.store, Line::Purchases, i64::MAX, true)
                .unwrap()
                .is_empty()
        );

        start(
            &app.store,
            Line::Purchases,
            &id,
            day("2026-11-01"),
            day("2026-11-25"),
        )
        .unwrap();
        go_on(&app, Line::Purchases).await.unwrap();
        screen_left(&app, Line::Purchases).await.unwrap();
        go_on(&app, Line::Purchases).await.unwrap();

        let w = waiting(&app.store, Line::Purchases, i64::MAX, true).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].message, "return");
        let past = of(&app.store, Line::Purchases, &id).unwrap().unwrap();
        assert_eq!(past.standing, Standing::Done);
    }

    #[tokio::test]
    async fn one_that_arrives_after_it_was_come_upon_is_no_longer_past() {
        let dir = crate::notebook::testing::TempDir::new("past-arrived");
        let (app, id) = mailbox(dir.path()).await;
        start(
            &app.store,
            Line::Purchases,
            &id,
            day("2026-11-01"),
            day("2026-11-25"),
        )
        .unwrap();
        go_on(&app, Line::Purchases).await.unwrap();
        stop(&app.store, Line::Purchases, &id).unwrap();

        super::super::enqueue(&app.store, Line::Purchases, &id, "new").unwrap();

        let w = waiting(&app.store, Line::Purchases, i64::MAX, false).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].message, "new");
    }

    #[tokio::test]
    async fn what_could_not_be_read_is_tried_anew_when_started_again() {
        let dir = crate::notebook::testing::TempDir::new("past-anew");
        let (app, id) = mailbox(dir.path()).await;
        start(
            &app.store,
            Line::Purchases,
            &id,
            day("2026-11-01"),
            day("2026-11-25"),
        )
        .unwrap();
        go_on(&app, Line::Purchases).await.unwrap();
        app.store
            .conn()
            .execute("UPDATE purchase_mails SET state = 'failed', tries = 6", [])
            .unwrap();
        start(
            &app.store,
            Line::Purchases,
            &id,
            day("2026-11-01"),
            day("2026-11-25"),
        )
        .unwrap();
        assert_eq!(
            of(&app.store, Line::Purchases, &id).unwrap().unwrap().left,
            2
        );
    }

    #[test]
    fn what_arrived_does_not_wait_for_past_mail() {
        let store = Store::open_in_memory().unwrap();
        add(&store, Line::Purchases, "g", "old", &mail("old")).unwrap();
        super::super::enqueue(&store, Line::Purchases, "g", "return").unwrap();
        let arrived = waiting(&store, Line::Purchases, i64::MAX, false)
            .unwrap()
            .remove(0);
        assert!(!super::super::earlier_waits(&store, Line::Purchases, &arrived).unwrap());
        let past = super::super::Waiting {
            past: true,
            ..arrived
        };
        assert!(super::super::earlier_waits(&store, Line::Purchases, &past).unwrap());
    }

    #[tokio::test]
    async fn one_stopped_goes_no_further_and_goes_on_once_started_again() {
        let dir = crate::notebook::testing::TempDir::new("past-stop");
        let (app, id) = mailbox(dir.path()).await;
        with_model(&app, json!({ "purchases": [0, 1] })).await;
        start(
            &app.store,
            Line::Purchases,
            &id,
            day("2026-11-01"),
            day("2026-11-25"),
        )
        .unwrap();
        go_on(&app, Line::Purchases).await.unwrap();
        start(
            &app.store,
            Line::Purchases,
            &id,
            day("2026-09-01"),
            day("2026-11-25"),
        )
        .unwrap();
        stop(&app.store, Line::Purchases, &id).unwrap();

        go_on(&app, Line::Purchases).await.unwrap();
        screen_left(&app, Line::Purchases).await.unwrap();

        let past = of(&app.store, Line::Purchases, &id).unwrap().unwrap();
        assert_eq!(
            (past.month, past.standing),
            (day("2026-09-01"), Standing::Stopped)
        );
        assert!(
            waiting(&app.store, Line::Purchases, i64::MAX, true)
                .unwrap()
                .is_empty()
        );

        start(
            &app.store,
            Line::Purchases,
            &id,
            day("2026-09-01"),
            day("2026-11-25"),
        )
        .unwrap();
        screen_left(&app, Line::Purchases).await.unwrap();
        assert_eq!(
            waiting(&app.store, Line::Purchases, i64::MAX, true)
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn what_arrives_is_read_before_past_mail() {
        let dir = crate::notebook::testing::TempDir::new("past-after");
        let (app, id) = mailbox(dir.path()).await;
        with_model(&app, json!({ "purchases": [0, 1] })).await;
        start(
            &app.store,
            Line::Purchases,
            &id,
            day("2026-11-01"),
            day("2026-11-25"),
        )
        .unwrap();
        go_on(&app, Line::Purchases).await.unwrap();
        screen_left(&app, Line::Purchases).await.unwrap();
        super::super::enqueue(&app.store, Line::Purchases, &id, "just now").unwrap();

        let first = &waiting(&app.store, Line::Purchases, i64::MAX, true).unwrap()[0];

        assert_eq!(first.message, "just now");
    }

    #[tokio::test]
    async fn past_mail_leaves_half_of_the_cap_to_what_arrives() {
        let dir = crate::notebook::testing::TempDir::new("past-cap");
        let (app, id) = mailbox(dir.path()).await;
        with_model(&app, json!({ "purchases": [0, 1] })).await;
        app.models
            .assign(Role::Lookout, "m", Some(2.0), None)
            .unwrap();
        start(
            &app.store,
            Line::Purchases,
            &id,
            day("2026-11-01"),
            day("2026-11-25"),
        )
        .unwrap();
        go_on(&app, Line::Purchases).await.unwrap();

        screen_left(&app, Line::Purchases).await.unwrap();
        read_waiting(&app, Line::Purchases).await.unwrap();

        assert_eq!(
            waiting(&app.store, Line::Purchases, i64::MAX, true)
                .unwrap()
                .len(),
            2,
            "screening used its half"
        );
        assert!(super::super::may_ask(&app).unwrap());
    }

    #[tokio::test]
    async fn a_mailbox_that_cannot_be_searched_says_why_and_is_tried_again() {
        let dir = crate::notebook::testing::TempDir::new("past-problem");
        let (app, id) = arrivals::tests::with_google(dir.path(), vec![]).await;
        start(
            &app.store,
            Line::Purchases,
            &id,
            day("2026-11-01"),
            day("2026-11-25"),
        )
        .unwrap();
        go_on(&app, Line::Purchases).await.unwrap();
        let past = of(&app.store, Line::Purchases, &id).unwrap().unwrap();
        assert!(!past.problem.is_empty());
        assert_eq!(
            (past.month, past.standing),
            (day("2026-11-01"), Standing::Going)
        );
    }

    #[test]
    fn today_a_day_to_come_and_one_before_there_were_shops_are_refused() {
        let store = Store::open_in_memory().unwrap();
        let today = day("2026-11-25");
        assert!(start(&store, Line::Purchases, "g", day("2026-11-26"), today).is_err());
        assert!(start(&store, Line::Purchases, "g", today, today).is_err());
        assert!(start(&store, Line::Purchases, "g", day("1994-01-01"), today).is_err());
    }
}
