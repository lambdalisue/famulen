//! Fetching the files of what the owner said on Slack. The message is kept
//! first and its files after it, so neither a slow file nor a restart loses
//! it; its turn starts once they are here.

use std::time::Duration;

use chrono::Utc;

use crate::app::App;
use crate::attachments::{Attached, MAX_BYTES, MAX_FILES, TOO_LARGE, TOO_MANY};
use crate::gateway::{Fetched, SENT_ELSEWHERE};
use crate::input::Source;
use crate::slack::web::{Refused, Slack};

/// How long the files of one message may take altogether.
const LIMIT: Duration = Duration::from_secs(90);

/// Fetch what the input `id` still waits for, if anything.
pub(super) async fn settle(app: &App, id: i64) -> anyhow::Result<()> {
    let waits = app
        .store
        .get(id)?
        .is_some_and(|row| waits(row.input.attached()));
    if !waits {
        return show(app, id);
    }
    let clients = app.slack.read().unwrap_or_else(|p| p.into_inner()).clone();
    settle_from(app, id, |workspace| {
        clients
            .as_ref()
            .and_then(|c| c.get(Some(workspace)).ok())
            .cloned()
    })
    .await
}

async fn settle_from<S: Slack>(
    app: &App,
    id: i64,
    client: impl Fn(&str) -> Option<S>,
) -> anyhow::Result<()> {
    let _one_at_a_time = app.attachments.fetching().await;
    let Some(mut input) = app.store.get(id)?.map(|row| row.input) else {
        return Ok(());
    };
    let Source::Slack {
        workspace,
        attached,
        ..
    } = &mut input.source
    else {
        return Ok(());
    };
    if waits(attached) {
        let slack = client(workspace);
        *attached = fetch(app, slack.as_ref(), std::mem::take(attached)).await;
        app.store.set_input(id, &input)?;
    }
    show(app, id)
}

fn waits(attached: &[Attached]) -> bool {
    attached
        .iter()
        .any(|a| matches!(a, Attached::OnSlack { .. }))
}

/// Show what came of the files of the input `id`. Done even when nothing was
/// fetched: the turn may have fetched them before the message was shown.
fn show(app: &App, id: i64) -> anyhow::Result<()> {
    match app.store.get(id)? {
        Some(row) if !row.input.attached().is_empty() => {
            app.console.set_attached(id, row.input.attached())
        }
        _ => Ok(()),
    }
}

/// Fetch the files of one message. One that does not arrive holds back
/// nothing else.
async fn fetch<S: Slack>(app: &App, slack: Option<&S>, files: Vec<Attached>) -> Vec<Attached> {
    let until = tokio::time::Instant::now() + LIMIT;
    let mut attached = Vec::new();
    for (i, file) in files.into_iter().enumerate() {
        let Attached::OnSlack {
            name,
            mime,
            size,
            path,
        } = file
        else {
            attached.push(file);
            continue;
        };
        let fetched = match slack {
            _ if i >= MAX_FILES => Err(TOO_MANY.to_owned()),
            _ if size > MAX_BYTES as u64 => Err(TOO_LARGE.to_owned()),
            None => Err("Slack につながっていない".to_owned()),
            Some(slack) => {
                let fetching = fetch_one(app, slack, &name, &mime, size, &path);
                tokio::time::timeout_at(until, fetching)
                    .await
                    .unwrap_or_else(|_| Err("時間内に取れなかった".to_owned()))
            }
        };
        attached.push(fetched.unwrap_or_else(|why| {
            tracing::warn!(%why, "an attached file did not arrive");
            Attached::missing(&name, why)
        }));
    }
    attached
}

const NEEDS_SCOPE: &str = "Slack がファイルを渡さない。アプリの権限に files:read があるか確かめる";

/// How long to wait before each new try, when Slack could not be reached.
const AGAIN_AFTER: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];

/// Whether asking again may end otherwise: Slack was busy or not reached.
fn may_pass(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Refused>().is_none_or(|r| {
        ["answered 5", "answered 429"]
            .iter()
            .any(|busy| r.why.starts_with(busy))
    })
}

async fn download<S: Slack>(slack: &S, path: &str) -> anyhow::Result<Fetched> {
    for wait in AGAIN_AFTER {
        match slack.download(path).await {
            Err(error) if may_pass(&error) => {
                tracing::debug!(%error, "could not fetch a file; trying again");
                tokio::time::sleep(wait).await;
            }
            done => return done,
        }
    }
    slack.download(path).await
}

async fn fetch_one<S: Slack>(
    app: &App,
    slack: &S,
    name: &str,
    mime: &str,
    size: u64,
    path: &str,
) -> Result<Attached, String> {
    let fetched = download(slack, path).await.map_err(|e| {
        let elsewhere = e
            .downcast_ref::<Refused>()
            .is_some_and(|r| r.why == SENT_ELSEWHERE);
        match elsewhere {
            true => NEEDS_SCOPE.to_owned(),
            false => format!("Slack から取れなかった（{e:#}）"),
        }
    })?;
    // Slack answers a page to log in at when the bot may not read the file.
    let a_page =
        fetched.mime == "text/html" && (mime != "text/html" || fetched.bytes.len() as u64 != size);
    if a_page {
        return Err(NEEDS_SCOPE.to_owned());
    }
    app.attachments
        .keep(name, &fetched.bytes, Utc::now().timestamp())
        .map(Attached::Kept)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Input, Sender};
    use crate::notebook::testing::TempDir;
    use crate::slack::fake::FakeSlack;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

    fn slack() -> FakeSlack {
        let slack = FakeSlack::default();
        *slack.files.lock().unwrap() = vec![
            ("ok".into(), fetched("image/png", PNG)),
            ("login".into(), fetched("text/html", b"<html>log in</html>")),
            ("zip".into(), fetched("application/zip", b"PK\x03\x04\0")),
        ];
        slack
    }

    fn fetched(mime: &str, bytes: &[u8]) -> Fetched {
        Fetched {
            mime: mime.into(),
            bytes: bytes.to_vec(),
        }
    }

    fn file(name: &str, mime: &str, path: &str) -> Attached {
        Attached::OnSlack {
            name: name.into(),
            mime: mime.into(),
            size: 12,
            path: path.into(),
        }
    }

    #[tokio::test]
    async fn a_kept_messages_files_are_fetched_once_and_shown_where_it_was_said() {
        let dir = TempDir::new("files-settle");
        let app = crate::runtime::testing::app(dir.path());
        let input = Input {
            sender: Sender::Owner,
            source: Source::Slack {
                workspace: "slack".into(),
                channel: "D1".into(),
                ts: "1.1".into(),
                thread_ts: None,
                user: "U1".into(),
                unmentioned: false,
                others_before: false,
                attached: vec![file("画面.png", "image/png", "ok")],
            },
            text: String::new(),
        };
        let id = app.store.enqueue(&input).unwrap().unwrap();
        crate::runtime::slack::show_on_console(&app, &input, "w");
        let mut events = app.console.subscribe();
        let slack = slack();

        settle_from(&app, id, |_| Some(&slack)).await.unwrap();
        let settled = app.store.get(id).unwrap().unwrap().input;
        slack.files.lock().unwrap().clear();
        settle_from(&app, id, |_| Some(&slack)).await.unwrap();

        let [Attached::Kept(kept)] = settled.attached() else {
            panic!("{:?}", settled.attached())
        };
        assert_eq!(std::fs::read(&kept.path).unwrap(), PNG);
        assert!(settled.render().contains(&kept.path.display().to_string()));
        assert_eq!(app.store.get(id).unwrap().unwrap().input, settled);
        let shown = app.console.recent(1).unwrap().remove(0);
        assert_eq!(shown.meta["attached"][0]["id"], kept.id);
        assert!(events.try_recv().is_ok(), "an open console is told");
    }

    #[tokio::test]
    async fn a_file_that_cannot_be_taken_says_why_and_keeps_back_nothing_else() {
        let dir = TempDir::new("files-missing");
        let app = crate::runtime::testing::app(dir.path());
        let mut large = file("large.png", "image/png", "ok");
        if let Attached::OnSlack { size, .. } = &mut large {
            *size = MAX_BYTES as u64 + 1;
        }
        let files = vec![
            file("login.png", "image/png", "login"),
            file("page.html", "text/html", "login"),
            file("gone.png", "image/png", "gone"),
            file("a.zip", "application/zip", "zip"),
            file("away.png", "image/png", "away"),
            large,
            file("ok.png", "image/png", "ok"),
        ];

        let mut attached = fetch(&app, Some(&&slack()), files).await;

        assert!(matches!(attached.pop(), Some(Attached::Kept(a)) if a.name == "ok.png"));
        let missing: Vec<(&str, &str)> = attached
            .iter()
            .map(|a| match a {
                Attached::Missing { name, why } => (name.as_str(), why.as_str()),
                other => panic!("{other:?}"),
            })
            .collect();
        let names: Vec<&str> = missing.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "login.png",
                "page.html",
                "gone.png",
                "a.zip",
                "away.png",
                "large.png"
            ]
        );
        assert!(missing[0].1.contains("files:read"));
        assert!(missing[4].1.contains("files:read"));
        assert!(!missing[2].1.contains("files:read"));
    }

    #[tokio::test]
    async fn files_past_the_tenth_are_not_fetched() {
        let dir = TempDir::new("files-many");
        let app = crate::runtime::testing::app(dir.path());
        let files = (0..11)
            .map(|i| file(&format!("{i}.png"), "image/png", "ok"))
            .collect();

        let attached = fetch(&app, Some(&&slack()), files).await;

        assert!(matches!(attached[9], Attached::Kept(_)));
        assert!(matches!(&attached[10], Attached::Missing { why, .. } if why == TOO_MANY));
    }
}
