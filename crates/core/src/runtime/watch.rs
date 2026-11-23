//! Reading the lookout's sources when they come due, with its model.

use chrono::Utc;

use super::keep;
use crate::app::App;
use crate::lookout::{self, Item, LookoutSource, More, Reading};
use crate::memory;
use crate::models::Role;
use crate::records;

/// What the lookout's model is told to pick what to read in full. What the
/// owner cares about follows.
const SKIM: &str = "\
You skim articles for a personal secretary by their title and abstract, \
the way a person goes down a feed. \
Answer the indices of those that may well concern the owner, to be read in full next; \
when unsure, include it. Nothing that plainly does not concern them.
What the owner cares about, and what they said does not concern them:
";

/// What the lookout's model is told to judge a batch. What the owner cares
/// about follows.
const SCREEN: &str = "\
You screen items for a personal secretary, all of them in one answer.
For every item by its index: whether it concerns the owner, and how soon it needs them: \
now (within the hour), today, or later.
Then put the items that concern the owner into groups, one subject each \
(a thread and its replies, the same vulnerability reported twice), checking them against each other, \
and decide for each group: act (the secretary should do something: a task given to the owner, a request), \
notify (the owner should know today: a vulnerability in what they use, a sale on what they want), \
or digest (worth a line in tomorrow morning's summary). \
Give each group a short title and say why in one short sentence, both in Japanese. \
Every item that concerns the owner belongs to one group.
What the owner cares about, and what they said does not concern them:
";

/// What the lookout judges relevance by: records (open tasks, people,
/// holdings) and semantic memory, where the owner's word that something does
/// not concern them lands too. Nothing in it is written by hand.
pub fn relevance_profile(app: &App) -> String {
    let mut profile = String::new();
    let tasks: Vec<String> = app
        .records
        .list(records::TASKS)
        .unwrap_or_default()
        .iter()
        .filter(|t| !records::is_over(t))
        .map(|t| t.title())
        .take(40)
        .collect();
    if !tasks.is_empty() {
        profile.push_str(&format!("\nOpen task: {}", tasks.join(", ")));
    }
    let prefs: Vec<String> = memory::long_term(&app.memory_dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|g| g.group == memory::LongGroup::Semantic)
        .flat_map(|g| g.files)
        .flat_map(|f| f.entries)
        .map(|e| e.body().to_owned())
        .take(40)
        .collect();
    if !prefs.is_empty() {
        profile.push_str(&format!(
            "\nWhat memory says they like, want, care about or do not: {}",
            prefs.join("; ")
        ));
    }
    let parties: Vec<String> = app
        .records
        .list(records::PARTIES)
        .unwrap_or_default()
        .iter()
        .map(|p| {
            let aliases = p.body.values("Aliases");
            if aliases.is_empty() {
                p.title()
            } else {
                format!("{} ({})", p.title(), aliases.join(", "))
            }
        })
        .take(80)
        .collect();
    if !parties.is_empty() {
        profile.push_str(&format!(
            "\nPeople and organizations: {}",
            parties.join(", ")
        ));
    }
    let mut holdings: Vec<String> = Vec::new();
    for kind in [records::HOLDINGS, records::CONTRACTS] {
        holdings.extend(app.records.list(kind).unwrap_or_default().iter().map(
            |h| match h.obsolete() {
                Some(_) => format!("{} (may be out of date)", h.title()),
                None => h.title(),
            },
        ));
    }
    if !holdings.is_empty() {
        profile.push_str(&format!("\nOwned and subscribed: {}", holdings.join(", ")));
    }
    profile.trim_start().to_owned()
}

/// What a lookout source has to say now: what arrived for its connection, or
/// what its tool answers. `None` when it could not be read.
async fn read_source(app: &App, source: &LookoutSource) -> anyhow::Result<Option<Vec<Reading>>> {
    if source.tool == lookout::ARRIVED {
        let heard = app.lookout.take_heard(source.id)?;
        return Ok(Some(heard.into_iter().map(Reading::from).collect()));
    }
    let Some(connection) = app.connections.get(&source.connection)? else {
        return Ok(None);
    };
    // It is called as the owner, whom nothing is asked of, while the secretary
    // can add a source: without this, adding one would run any tool unasked.
    if !app
        .permissions
        .only_reads(&source.connection, &source.tool)?
    {
        tracing::warn!(source = %source.name, tool = %source.tool, "the lookout calls only tools that say they only read");
        return Ok(None);
    }
    let result = app
        .gateway
        .call_as_owner(&connection, &source.tool, source.args.clone())
        .await;
    Ok(match result {
        Ok(result) if !result.is_error => {
            let mut batch = lookout::items_from(&result);
            for reading in &mut batch {
                let opens = match &reading.more {
                    Some(More::Page(url)) => {
                        app.gateway.opener(&connection, &source.tool, url).is_some()
                    }
                    _ => true,
                };
                if !opens {
                    reading.more = None;
                }
            }
            Some(batch)
        }
        Ok(result) => {
            tracing::warn!(source = %source.name, error = %result.text, "the lookout tool failed");
            None
        }
        Err(error) => {
            tracing::warn!(source = %source.name, %error, "could not read the lookout");
            None
        }
    })
}

/// The text of a page something a source read links to, when it may be opened.
async fn open_page(app: &App, source: &LookoutSource, url: &str) -> Option<String> {
    let read = async {
        let connection = app.connections.get(&source.connection)?;
        let Some(connection) = connection else {
            return Ok(None);
        };
        let Some(tool) = app.gateway.opener(&connection, &source.tool, url) else {
            return Ok(None);
        };
        if !app.permissions.only_reads(&source.connection, &tool)? {
            return Ok(None);
        }
        let result = app
            .gateway
            .call_as_owner(&connection, &tool, serde_json::json!({ "url": url }))
            .await?;
        anyhow::ensure!(!result.is_error, "the page answered an error");
        anyhow::Ok(lookout::page_text(&result.text))
    };
    match read.await {
        Ok(text) => text,
        Err(error) => {
            tracing::warn!(source = %source.name, url, %error, "could not open a page for the lookout");
            None
        }
    }
}

/// Ask the lookout's model about `items`, or nothing over its daily cap.
async fn ask_lookout(
    app: &App,
    tz: chrono_tz::Tz,
    system: &str,
    items: &[Item],
    schema: &serde_json::Value,
) -> anyhow::Result<Option<serde_json::Value>> {
    if app.models.assignment(Role::Lookout)?.is_none() {
        anyhow::bail!("no model is assigned to the lookout");
    }
    if !app.models.within_cap(Role::Lookout, tz, Utc::now())? {
        return Ok(None);
    }
    let completion = app
        .models
        .complete(
            Role::Lookout,
            tz,
            system,
            &lookout::numbered(items),
            Some(schema),
        )
        .await?;
    let answer = completion
        .json
        .ok_or_else(|| anyhow::anyhow!("the lookout answered without JSON"))?;
    Ok(Some(answer))
}

pub(super) async fn poll_lookout(app: &App) -> anyhow::Result<()> {
    let at = Utc::now().timestamp();
    let due = app.lookout.due(at)?;
    if due.is_empty() {
        return Ok(());
    }
    let tz = app.tz();
    let profile = relevance_profile(app);
    for source in due {
        app.lookout.mark_run(source.id, at)?;
        let Some(batch) = read_source(app, &source).await? else {
            continue;
        };
        if batch.is_empty() {
            continue;
        }
        let skim = async |items: Vec<Item>| {
            let system = format!("{SKIM}{profile}");
            let schema = lookout::skim_schema();
            let answer = ask_lookout(app, tz, &system, &items, &schema).await?;
            answer.as_ref().map(lookout::picked_from).transpose()
        };
        let source_ref = &source;
        let open = move |url: String| async move { open_page(app, source_ref, &url).await };
        let model = async |items: Vec<Item>| {
            let system = format!("{SCREEN}{profile}");
            let schema = lookout::verdict_schema();
            let answer = ask_lookout(app, tz, &system, &items, &schema).await?;
            Ok(answer.map(|a| lookout::verdict_from(&a, items.len())))
        };
        let inputs = app
            .lookout
            .read(&source.id.to_string(), batch, skim, open, model)
            .await?;
        for input in inputs {
            keep(&app.store, &app.inputs, &input)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::testing;

    #[tokio::test]
    async fn a_source_whose_tool_no_longer_only_reads_is_not_called() {
        let dir = crate::notebook::testing::TempDir::new("runtime-readers");
        let hit = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = hit.clone();
        let router = axum::Router::new().fallback(move || {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { "[]" }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let app = testing::app(dir.path());
        app.connections
            .add(
                "svc",
                "svc",
                "svc",
                crate::connections::Transport::Api {
                    base_url: base,
                    openapi_url: String::new(),
                },
                Default::default(),
                "",
            )
            .unwrap();
        app.gateway.tools_of("svc").await.unwrap();
        let args = serde_json::json!({ "path": "/" });
        for tool in ["http_get", "http_post"] {
            app.lookout
                .add_source(tool, "svc", tool, &args, lookout::Feed::Other, None)
                .unwrap();
        }
        for source in app.lookout.sources().unwrap() {
            let read = read_source(&app, &source).await.unwrap();
            assert_eq!(read.is_some(), source.tool == "http_get", "{}", source.tool);
        }
        assert_eq!(hit.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_feeds_entries_open_only_pages_on_the_feeds_host() {
        let dir = crate::notebook::testing::TempDir::new("runtime-feed");
        let app = testing::app(dir.path());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let feed = format!(
            r#"<rss version="2.0"><channel><title>t</title>
<item><title>中</title><link>{base}/a</link></item>
<item><title>外</title><link>http://127.0.0.1:1/b</link></item></channel></rss>"#
        );
        let router = axum::Router::new()
            .route("/feed", axum::routing::get(move || async move { feed }))
            .route("/a", axum::routing::get(|| async { "<h1>題</h1>本文" }));
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let values = std::collections::BTreeMap::from([("url".into(), format!("{base}/feed"))]);
        let c = app.gateway.add_defined("rss", values, None).unwrap();
        app.gateway.tools_of(&c.id).await.unwrap();
        let args = serde_json::json!({});
        app.lookout
            .add_source("f", &c.id, "get_feed", &args, lookout::Feed::Other, None)
            .unwrap();
        let source = app.lookout.sources().unwrap().remove(0);

        let batch = read_source(&app, &source).await.unwrap().unwrap();

        let more: Vec<_> = batch.into_iter().map(|r| r.more).collect();
        let page = format!("{base}/a");
        assert_eq!(more, [Some(lookout::More::Page(page.clone())), None]);
        assert_eq!(
            open_page(&app, &source, &page).await.as_deref(),
            Some("題 本文")
        );
        assert_eq!(
            open_page(&app, &source, &format!("{base}/none")).await,
            None
        );
        assert_eq!(open_page(&app, &source, "http://127.0.0.1:1/b").await, None);
    }

    #[test]
    fn the_lookout_judges_by_records_and_memory() {
        let dir = crate::notebook::testing::TempDir::new("runtime-profile");
        let app = testing::app(dir.path());
        app.records
            .add(
                crate::notebook::Writer::Secretary,
                records::TASKS,
                records::Patch::titled("請求書を送る"),
            )
            .unwrap();
        memory::write_file(
            &app.memory_dir,
            "long/semantic/sales.md",
            "---\nid: m1\nsummary: セールの知らせは要らない\n---\n\nセールの知らせは要らない。\n"
                .as_bytes(),
        )
        .unwrap();

        let profile = relevance_profile(&app);

        assert!(profile.contains("請求書を送る"), "{profile}");
        assert!(profile.contains("セールの知らせは要らない"), "{profile}");
    }
}
