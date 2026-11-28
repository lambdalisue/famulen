//! Taking what arrives at the channel from Slack.

use chrono::Utc;

use super::keep;
use super::session::workgroup_of;
use crate::app::App;
use crate::approvals;
use crate::attachments;
use crate::claude::session::Services;
use crate::console_log::Who;
use crate::gateway::Arrival;
use crate::input::{Input, Sender, Source};
use crate::lookout::{Item, title_of};
use crate::models::{Assignment, ModelKind, Models, Role};
use crate::slack::attention;
use crate::slack::clients::SlackClients;
use crate::slack::events::{self, Said};
use crate::slack::presence;
use crate::slack::web::{GatewaySlack, Slack};

/// Read one thing the gateway handed the channel from a Slack workspace: the
/// owner's words become an input, marked as received at once; what others
/// say goes to the lookout; a pressed button answers its approval.
pub(super) async fn read_channel(
    app: &App,
    services: &Services<GatewaySlack>,
    clients: Option<&SlackClients<GatewaySlack>>,
    arrival: &Arrival,
) -> anyhow::Result<()> {
    let received = std::time::Instant::now();
    anyhow::ensure!(
        app.connections
            .get(&arrival.connection)?
            .is_some_and(|c| crate::channel::is_slack(&c)),
        "the channel reads only Slack"
    );
    let workspace = arrival.connection.as_str();
    if let Err(error) = learn_bot_dm(app, clients, arrival).await {
        tracing::warn!(%error, "could not find the bot's DM with the owner");
    }
    let Taken {
        input: kept,
        actions,
        attended,
    } = take_from_channel(app, arrival)?;
    if let Some(said) = attended {
        let (models, tz, name) = (app.models.clone(), app.tz(), app.aide_name());
        let judge = |item: Item, before: Vec<String>| ask_triage(models, tz, name, item, before);
        let asked = triage_model(&app.models, app.tz())?.is_some();
        let slack = client_of(clients, workspace);
        if let Some(input) =
            take_attended(app, slack, workspace, said, asked, judge, received).await?
            && input.sender == Sender::Owner
        {
            show_on_console(app, &input, &app.workspace_label(workspace));
            fetch_files(app, &input);
        }
    }
    if let Some(input) = kept {
        if let Some(slack) = client_of(clients, workspace) {
            show_working(app, slack, &input, received);
        }
        show_on_console(app, &input, &app.workspace_label(workspace));
        fetch_files(app, &input);
    }
    for action in actions {
        match services.approvals.answer(&action).await {
            Ok(Some(input)) => {
                keep(&app.store, &app.inputs, &input)?;
            }
            Ok(None) => tracing::debug!(?action, "approval press ignored"),
            Err(error) => tracing::warn!(%error, "could not answer the approval"),
        }
    }
    Ok(())
}

/// Fetch the files of `input`, without holding back what arrives next. Its
/// turn waits for them all the same.
fn fetch_files(app: &App, input: &Input) {
    if input.attached().is_empty() {
        return;
    }
    let (app, key) = (app.clone(), input.dedup_key());
    tokio::spawn(async move {
        let settled = async {
            match app.store.id_of(&key)? {
                Some(id) => super::files::settle(&app, id).await,
                None => Ok(()),
            }
        };
        if let Err(error) = settled.await {
            tracing::warn!(%error, "could not fetch what was attached");
        }
    });
}

fn client_of(
    clients: Option<&SlackClients<GatewaySlack>>,
    workspace: &str,
) -> Option<GatewaySlack> {
    clients.and_then(|c| c.get(Some(workspace)).ok()).cloned()
}

/// Weigh a message in an attended channel. The owner's is shown as worked on
/// while triage, when `asked`, weighs it, and on when it becomes an input.
async fn take_attended<S, J, F>(
    app: &App,
    slack: Option<S>,
    workspace: &str,
    said: Said,
    asked: bool,
    judge: J,
    received: std::time::Instant,
) -> anyhow::Result<Option<Input>>
where
    S: Slack + Clone + 'static,
    J: FnOnce(Item, Vec<String>) -> F,
    F: std::future::Future<Output = anyhow::Result<Option<bool>>>,
{
    let slack = slack.filter(|_| said.by_owner);
    let (channel, thread) = (
        said.channel.clone(),
        said.thread_ts.clone().unwrap_or_else(|| said.ts.clone()),
    );
    let expect = async {
        match &slack {
            Some(slack) if asked => {
                presence::expect(slack, &app.store, workspace, &channel, &thread).await
            }
            _ => Ok(false),
        }
    };
    let (expected, weighed) =
        tokio::join!(expect, attend_to(app, workspace, said, judge, Utc::now()));
    let expected = expected.unwrap_or_else(|error| {
        tracing::debug!(%error, "could not show that the message is weighed");
        false
    });
    match (&slack, &weighed) {
        (Some(slack), Ok(Some(input))) => show_working(app, slack.clone(), input, received),
        (Some(slack), _) if expected => presence::withdraw(slack, &channel, &thread).await,
        _ => {}
    }
    weighed
}

/// Show on Slack that `input` is being worked on, without holding back what
/// arrives next.
fn show_working<S: Slack + 'static>(
    app: &App,
    slack: S,
    input: &Input,
    received: std::time::Instant,
) {
    let Source::Slack {
        workspace,
        channel,
        ts,
        thread_ts,
        unmentioned,
        ..
    } = &input.source
    else {
        return;
    };
    let at = presence::At {
        workspace: workspace.clone(),
        channel: channel.clone(),
        ts: ts.clone(),
        thread_ts: thread_ts.clone(),
        unmentioned: *unmentioned,
        received,
    };
    let (store, key) = (app.store.clone(), input.dedup_key());
    tokio::spawn(async move {
        let shown = async {
            let Some(id) = store.id_of(&key)? else {
                return anyhow::Ok(());
            };
            presence::show_working(&slack, &store, &at, id).await
        };
        if let Err(error) = shown.await {
            tracing::debug!(%error, "could not show that the message was received");
        }
    });
}

/// What a Slack arrival came to: the owner's mention or direct message kept
/// as an input, the approval buttons pressed, and a message in a channel the
/// channel is attending, still to be weighed.
struct Taken {
    input: Option<Input>,
    actions: Vec<events::Action>,
    attended: Option<Said>,
}

/// Keep what a Slack arrival says: the owner's mention or direct message as
/// an input, when new, and others' words as heard by the lookout, to be read
/// in its next batch. A mention starts attending its channel; a message in an
/// attended channel is handed back to be weighed at once.
fn take_from_channel(app: &App, arrival: &Arrival) -> anyhow::Result<Taken> {
    let workspace = arrival.connection.as_str();
    let now = Utc::now();
    let owner = owner_of(app, workspace)?;
    let bot_dm = bot_dm(app, workspace)?;
    let envelope = events::read_envelope(&arrival.payload, workspace, &owner, bot_dm.as_deref());
    let kept = match envelope.input {
        Some(input) if keep(&app.store, &app.inputs, &input)?.is_some() => Some(input),
        _ => None,
    };
    if let Some(Source::Slack { channel, ts, .. }) = kept.as_ref().map(|i| &i.source) {
        first_taken(app, workspace, channel, ts)?;
        if envelope.mention {
            attention::attend(&app.store, workspace, channel, now)?;
        }
    }
    let bot = app
        .gateway
        .account_values(workspace)
        .ok()
        .and_then(|values| values.get("bot_user").cloned())
        .unwrap_or_default();
    let attended = match envelope.said {
        Some(said)
            if kept.is_none()
                && !events::mentions(&said.text, &bot)
                && attention::attending(&app.store, workspace, &said.channel, now)? =>
        {
            // The owner's words are taken here and never heard; others' are
            // heard all the same, for the lookout's batch.
            let fresh = !said.by_owner || first_taken(app, workspace, &said.channel, &said.ts)?;
            fresh.then_some(said)
        }
        _ => None,
    };
    let heard = match envelope.heard {
        Some(h) if first_taken(app, workspace, &h.channel, &h.ts)? => Some(h),
        _ => None,
    };
    if let Some(heard) = heard {
        let item = item_of(&heard.channel, &heard.user, &heard.ts, &heard.text);
        app.lookout
            .hear_at(workspace, &item, std::slice::from_ref(&heard.channel))?;
    }
    let (approvals, others): (Vec<_>, Vec<_>) = envelope
        .actions
        .into_iter()
        .partition(approvals::is_approval_action);
    for action in others {
        tracing::debug!(?action, "unhandled slack action");
    }
    Ok(Taken {
        input: kept,
        actions: approvals,
        attended,
    })
}

/// A message as a model judges it.
fn item_of(channel: &str, user: &str, ts: &str, text: &str) -> Item {
    Item {
        title: title_of(text),
        body: format!("channel: {channel} · from: {user} · ts: {ts}\n{text}"),
    }
}

/// Weigh a message in an attended channel with triage's one narrow
/// question, at once. Said to the secretary, it becomes an input in the
/// sender's own name, with what was said there before and not to the
/// secretary, and the channel stays attended a while longer. Otherwise it is
/// kept as what was said before.
async fn attend_to<J, F>(
    app: &App,
    workspace: &str,
    said: Said,
    judge: J,
    now: chrono::DateTime<Utc>,
) -> anyhow::Result<Option<Input>>
where
    J: FnOnce(Item, Vec<String>) -> F,
    F: std::future::Future<Output = anyhow::Result<Option<bool>>>,
{
    let words = attachments::with_names(&said.text, &said.files);
    let item = item_of(&said.channel, &said.user, &said.ts, &words);
    let before = attention::context(&app.store, workspace, &said.channel)?;
    let addressed = app
        .triage
        .weigh(workspace, &item, |item| judge(item, before))
        .await?;
    match addressed {
        Some(true) => {
            let before = attention::take_context(&app.store, workspace, &said.channel)?;
            let text = if before.is_empty() {
                said.text.clone()
            } else {
                format!(
                    "Said before this in the channel, not to you (context, not requests):\n{}\n---\n{}",
                    before.join("\n"),
                    said.text
                )
            };
            let input = Input {
                sender: if said.by_owner {
                    Sender::Owner
                } else {
                    Sender::Other
                },
                source: Source::Slack {
                    workspace: workspace.to_owned(),
                    channel: said.channel.clone(),
                    ts: said.ts.clone(),
                    thread_ts: said.thread_ts.clone(),
                    user: said.user.clone(),
                    unmentioned: true,
                    others_before: from_others(&before, &owner_of(app, workspace)?),
                    attached: said.files,
                },
                text,
            };
            attention::attend(&app.store, workspace, &said.channel, now)?;
            Ok(keep(&app.store, &app.inputs, &input)?.map(|_| input))
        }
        Some(false) => {
            attention::remember(
                &app.store,
                workspace,
                &said.channel,
                &format!("<@{}>: {words}", said.user),
            )?;
            Ok(None)
        }
        None => Ok(None),
    }
}

/// Whether any of the lines kept as said before is not the owner's.
fn from_others(before: &[String], owner: &str) -> bool {
    let mark = format!("<@{owner}>: ");
    owner.is_empty() && !before.is_empty() || before.iter().any(|line| !line.starts_with(&mark))
}

/// Triage's model, unless there is none or it is over its cap.
fn triage_model(models: &Models, tz: chrono_tz::Tz) -> anyhow::Result<Option<Assignment>> {
    let assigned = models.assignment(Role::Triage)?;
    Ok(match assigned {
        Some(assigned) if models.within_cap(Role::Triage, tz, Utc::now())? => Some(assigned),
        _ => None,
    })
}

/// Triage's model, asked one narrow question of one message: is it said to
/// the secretary? `None` when triage has no model or is over its cap, so
/// the channel falls back to mentions only.
async fn ask_triage(
    models: Models,
    tz: chrono_tz::Tz,
    name: String,
    item: Item,
    before: Vec<String>,
) -> anyhow::Result<Option<bool>> {
    let Some(assigned) = triage_model(&models, tz)? else {
        return Ok(None);
    };
    let before = if before.is_empty() {
        "(nothing)".to_owned()
    } else {
        before.join("\n")
    };
    let instruction = format!(
        "{name} is an AI secretary who was mentioned in this Slack channel a little while ago. \
         Decide whether the message is said to {name} or continues a conversation with {name}, \
         rather than people talking among themselves. Said before it in the channel:\n{before}"
    );
    let message = format!("{}\n{}", item.title, item.body);
    let judge = models
        .model(&assigned.model)?
        .is_some_and(|m| m.spec.kind == ModelKind::Judge);
    if judge {
        let label = models
            .classify(
                Role::Triage,
                tz,
                &["addressed", "not_addressed"],
                &instruction,
                &message,
            )
            .await?;
        return Ok(Some(label == "addressed"));
    }
    let completion = models
        .complete_brief(
            Role::Triage,
            tz,
            &instruction,
            &message,
            Some(&crate::triage::schema()),
        )
        .await?;
    let answer = completion
        .json
        .ok_or_else(|| anyhow::anyhow!("triage answered without JSON"))?;
    Ok(Some(
        answer
            .get("addressed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    ))
}

fn owner_of(app: &App, workspace: &str) -> anyhow::Result<String> {
    Ok(app
        .workspaces()?
        .into_iter()
        .find(|w| w.id == workspace)
        .map(|w| w.owner)
        .unwrap_or_default())
}

fn bot_dm_key(workspace: &str) -> String {
    format!("channel.dm.{workspace}")
}

/// The channel of the bot's DM with the owner in `workspace`, once known.
fn bot_dm(app: &App, workspace: &str) -> anyhow::Result<Option<String>> {
    app.store.state(bot_dm_key(workspace))
}

fn remember_bot_dm(app: &App, workspace: &str, channel: &str) -> anyhow::Result<()> {
    app.store.set_state(bot_dm_key(workspace), channel)
}

/// Mark a Slack message as taken. Returns whether it was the first time:
/// Slack may send the same message once for the bot and once for the owner.
fn first_taken(app: &App, workspace: &str, channel: &str, ts: &str) -> anyhow::Result<bool> {
    let changed = app.store.conn().execute(
        "INSERT INTO slack_taken (workspace, channel, ts) VALUES (?1, ?2, ?3)
         ON CONFLICT DO NOTHING",
        rusqlite::params![workspace, channel, ts],
    )?;
    Ok(changed == 1)
}

/// Learn the bot's DM with the owner before reading a direct message the
/// owner wrote, so a message to the bot is told from one to anyone else.
async fn learn_bot_dm(
    app: &App,
    clients: Option<&SlackClients<GatewaySlack>>,
    arrival: &Arrival,
) -> anyhow::Result<()> {
    let workspace = arrival.connection.as_str();
    let owner = owner_of(app, workspace)?;
    if !events::needs_bot_dm(&arrival.payload, &owner) || bot_dm(app, workspace)?.is_some() {
        return Ok(());
    }
    let Some(slack) = clients.and_then(|c| c.get(Some(workspace)).ok()) else {
        return Ok(());
    };
    let channel = slack.open_dm(&owner).await?;
    remember_bot_dm(app, workspace, &channel)
}

/// Put what the owner said on Slack into the console's one conversation.
pub(super) fn show_on_console(app: &App, input: &Input, workspace: &str) {
    if !matches!(input.source, Source::Slack { .. }) {
        return;
    }
    let meta = serde_json::json!({
        "origin": format!("Slack · {workspace}"),
        "workgroup": workgroup_of(&app.workgroups, input),
        "input": app.store.id_of(&input.dedup_key()).ok().flatten(),
        "attached": input.attached(),
    });
    if let Err(error) = app.console.add(Who::Owner, &input.text, meta) {
        tracing::warn!(%error, "could not show the Slack message on the console");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack::fake::FakeSlack;

    fn slack_arrival(event: serde_json::Value) -> Arrival {
        Arrival {
            about: String::new(),
            over: false,
            connection: "slack".into(),
            receiver: "events".into(),
            received_at: 0,
            headers: Default::default(),
            payload: serde_json::json!({ "envelope_id": "e1", "type": "events_api", "payload": { "event": event } }),
        }
    }

    #[test]
    fn what_was_said_before_is_others_unless_every_line_is_the_owners() {
        let lines = |of: &[&str]| of.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        assert!(!from_others(&[], "U1"));
        assert!(!from_others(&[], ""));
        assert!(!from_others(&lines(&["<@U1>: a", "<@U1>: b"]), "U1"));
        assert!(from_others(&lines(&["<@U1>: a", "<@U2>: <@U1>: b"]), "U1"));
        assert!(from_others(&lines(&["<@U1>: a"]), ""));
    }

    #[test]
    fn the_owners_words_on_slack_become_an_input_and_others_are_heard_by_the_lookout() {
        let dir = crate::notebook::testing::TempDir::new("runtime-channel");
        let app = crate::runtime::testing::app(dir.path());
        app.gateway
            .add_defined(
                crate::channel::DEFINITION,
                [("user".to_owned(), "U1".to_owned())].into(),
                None,
            )
            .unwrap();
        app.lookout
            .add_source(
                "Slack",
                "slack",
                crate::lookout::ARRIVED,
                &serde_json::json!({}),
                crate::lookout::Feed::Chat,
                Some(60),
            )
            .unwrap();

        let kept = take_from_channel(
            &app,
            &slack_arrival(serde_json::json!({ "type": "message", "channel_type": "im", "channel": "D1", "user": "U1", "ts": "1.1", "text": "明日の予定は？" })),
        )
        .unwrap()
        .input;
        let again = take_from_channel(
            &app,
            &slack_arrival(serde_json::json!({ "type": "message", "channel_type": "im", "channel": "D1", "user": "U1", "ts": "1.1", "text": "明日の予定は？" })),
        )
        .unwrap()
        .input;
        take_from_channel(
            &app,
            &slack_arrival(serde_json::json!({ "type": "message", "channel_type": "channel", "channel": "C1", "user": "U2", "ts": "1.2", "text": "障害が出ています" })),
        )
        .unwrap();

        let input = kept.unwrap();
        assert!(matches!(&input.source, Source::Slack { workspace, .. } if workspace == "slack"));
        assert_eq!(app.store.next_pending().unwrap().unwrap().input, input);
        assert!(again.is_none(), "Slack sending it again keeps it once");
        let heard = app.lookout.heard_on("slack").unwrap();
        assert_eq!(heard.len(), 1);
        assert_eq!(heard[0].title, "障害が出ています");
    }

    #[test]
    fn a_message_slack_delivers_for_both_the_bot_and_the_owner_is_heard_once() {
        let dir = crate::notebook::testing::TempDir::new("runtime-channel-twice");
        let app = crate::runtime::testing::app(dir.path());
        app.gateway
            .add_defined(
                crate::channel::DEFINITION,
                [("user".to_owned(), "U1".to_owned())].into(),
                None,
            )
            .unwrap();
        app.lookout
            .add_source(
                "Slack",
                "slack",
                crate::lookout::ARRIVED,
                &serde_json::json!({}),
                crate::lookout::Feed::Chat,
                Some(60),
            )
            .unwrap();
        let event = serde_json::json!({ "type": "message", "channel_type": "channel", "channel": "C1", "user": "U2", "ts": "7.1", "text": "<@U1> 至急" });
        let for_owner = Arrival {
            about: String::new(),
            over: false,
            payload: serde_json::json!({
                "envelope_id": "e2",
                "type": "events_api",
                "payload": { "event": event, "authorizations": [{ "user_id": "U1", "is_bot": false }] },
            }),
            ..slack_arrival(event.clone())
        };

        take_from_channel(&app, &slack_arrival(event.clone())).unwrap();
        take_from_channel(&app, &for_owner).unwrap();

        assert_eq!(app.lookout.heard_on("slack").unwrap().len(), 1);
    }

    #[test]
    fn a_message_not_taken_the_first_time_can_still_be_taken_when_the_bot_sees_it() {
        let dir = crate::notebook::testing::TempDir::new("runtime-channel-later");
        let app = crate::runtime::testing::app(dir.path());
        app.gateway
            .add_defined(
                crate::channel::DEFINITION,
                [("user".to_owned(), "U1".to_owned())].into(),
                None,
            )
            .unwrap();
        app.lookout
            .add_source(
                "Slack",
                "slack",
                crate::lookout::ARRIVED,
                &serde_json::json!({}),
                crate::lookout::Feed::Chat,
                Some(60),
            )
            .unwrap();
        let event = serde_json::json!({ "type": "message", "channel_type": "channel", "channel": "C1", "user": "U2", "ts": "7.2", "text": "障害です" });
        let for_owner = Arrival {
            about: String::new(),
            over: false,
            payload: serde_json::json!({
                "type": "events_api",
                "payload": { "event": event, "authorizations": [{ "user_id": "U1", "is_bot": false }] },
            }),
            ..slack_arrival(event.clone())
        };

        take_from_channel(&app, &for_owner).unwrap();
        take_from_channel(&app, &slack_arrival(event)).unwrap();

        assert_eq!(app.lookout.heard_on("slack").unwrap().len(), 1);
    }

    #[test]
    fn the_owners_message_to_the_bot_delivered_for_the_owner_is_an_input_once_its_dm_is_known() {
        let dir = crate::notebook::testing::TempDir::new("runtime-channel-dm");
        let app = crate::runtime::testing::app(dir.path());
        let slack = app
            .gateway
            .add_defined(
                crate::channel::DEFINITION,
                [("user".to_owned(), "U1".to_owned())].into(),
                None,
            )
            .unwrap();
        remember_bot_dm(&app, &slack.id, "D1").unwrap();
        let event = serde_json::json!({ "type": "message", "channel_type": "im", "channel": "D1", "user": "U1", "ts": "7.3", "text": "明日は？" });
        let arrival = Arrival {
            about: String::new(),
            over: false,
            connection: slack.id.clone(),
            payload: serde_json::json!({
                "type": "events_api",
                "payload": { "event": event, "authorizations": [{ "user_id": "U1", "is_bot": false }] },
            }),
            ..slack_arrival(event.clone())
        };

        let kept = take_from_channel(&app, &arrival).unwrap().input;

        assert!(kept.is_some());
        assert_eq!(bot_dm(&app, &slack.id).unwrap().as_deref(), Some("D1"));
    }

    fn channel_app(dir: &std::path::Path) -> App {
        let app = crate::runtime::testing::app(dir);
        app.gateway
            .add_defined(
                crate::channel::DEFINITION,
                [
                    ("user".to_owned(), "U1".to_owned()),
                    ("bot_user".to_owned(), "B1".to_owned()),
                ]
                .into(),
                None,
            )
            .unwrap();
        app.lookout
            .add_source(
                "Slack",
                "slack",
                crate::lookout::ARRIVED,
                &serde_json::json!({}),
                crate::lookout::Feed::Chat,
                Some(60),
            )
            .unwrap();
        app
    }

    fn said_in(channel: &str, user: &str, ts: &str, text: &str) -> Arrival {
        slack_arrival(
            serde_json::json!({ "type": "message", "channel_type": "channel", "channel": channel, "user": user, "ts": ts, "text": text }),
        )
    }

    type Judged = std::future::Ready<anyhow::Result<Option<bool>>>;

    fn judging(answer: Option<bool>) -> impl FnOnce(Item, Vec<String>) -> Judged {
        move |_, _| std::future::ready(Ok(answer))
    }

    #[test]
    fn talk_in_an_unattended_channel_waits_for_the_lookouts_batch() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-none");
        let app = channel_app(dir.path());

        let owner = take_from_channel(&app, &said_in("C1", "U1", "8.1", "それで")).unwrap();
        let other = take_from_channel(&app, &said_in("C1", "U2", "8.2", "障害です")).unwrap();

        assert!(owner.input.is_none() && owner.attended.is_none());
        assert!(other.input.is_none() && other.attended.is_none());
        assert!(app.store.next_pending().unwrap().is_none());
        assert_eq!(app.lookout.heard_on("slack").unwrap().len(), 2);
    }

    #[test]
    fn a_mention_starts_attending_its_channel() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-mention");
        let app = channel_app(dir.path());
        let mention = slack_arrival(
            serde_json::json!({ "type": "app_mention", "channel": "C1", "user": "U1", "ts": "8.3", "text": "<@B1> 予定は？" }),
        );

        assert!(take_from_channel(&app, &mention).unwrap().input.is_some());
        let owner = take_from_channel(&app, &said_in("C1", "U1", "8.4", "それで")).unwrap();
        let other =
            take_from_channel(&app, &said_in("C1", "U2", "8.5", "横からすみません")).unwrap();
        let elsewhere = take_from_channel(&app, &said_in("C2", "U1", "8.6", "別の話")).unwrap();

        assert!(owner.input.is_none(), "not an input until weighed");
        assert!(owner.attended.unwrap().by_owner);
        assert!(!other.attended.unwrap().by_owner);
        assert!(elsewhere.attended.is_none());
        assert!(
            attention::attending(&app.store, "slack", "C1", Utc::now()).unwrap(),
            "a mention attends its channel"
        );
    }

    #[test]
    fn a_mention_in_an_attended_channel_is_left_to_its_mention_copy() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-mention-again");
        let app = channel_app(dir.path());
        attention::attend(&app.store, "slack", "C1", Utc::now()).unwrap();
        let mention = slack_arrival(
            serde_json::json!({ "type": "app_mention", "channel": "C1", "user": "U1", "ts": "8.8", "text": "<@B1> もう一つ" }),
        );

        let plain = take_from_channel(&app, &said_in("C1", "U1", "8.8", "<@B1> もう一つ")).unwrap();
        let taken = take_from_channel(&app, &mention).unwrap();

        assert!(plain.attended.is_none(), "a mention is not weighed");
        assert!(taken.input.is_some(), "the mention copy is still an input");
    }

    #[test]
    fn the_owners_words_delivered_twice_are_weighed_once() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-twice");
        let app = channel_app(dir.path());
        attention::attend(&app.store, "slack", "C1", Utc::now()).unwrap();
        let event = serde_json::json!({ "type": "message", "channel_type": "channel", "channel": "C1", "user": "U1", "ts": "8.7", "text": "それで" });
        let for_owner = Arrival {
            about: String::new(),
            over: false,
            payload: serde_json::json!({
                "type": "events_api",
                "payload": { "event": event, "authorizations": [{ "user_id": "U1", "is_bot": false }] },
            }),
            ..slack_arrival(event.clone())
        };

        assert!(
            take_from_channel(&app, &slack_arrival(event))
                .unwrap()
                .attended
                .is_some()
        );
        assert!(
            take_from_channel(&app, &for_owner)
                .unwrap()
                .attended
                .is_none()
        );
    }

    fn said(user: &str, ts: &str, text: &str) -> Said {
        Said {
            channel: "C1".into(),
            ts: ts.into(),
            thread_ts: Some("8.0".into()),
            user: user.into(),
            text: text.into(),
            by_owner: user == "U1",
            files: Vec::new(),
        }
    }

    #[tokio::test]
    async fn the_owners_words_said_to_the_secretary_become_an_input_with_what_came_before() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-yes");
        let app = channel_app(dir.path());
        let now = Utc::now();
        attention::attend(&app.store, "slack", "C1", now).unwrap();

        let aside = attend_to(
            &app,
            "slack",
            said("U2", "9.1", "今日は在宅です"),
            judging(Some(false)),
            now,
        )
        .await
        .unwrap();
        let input = attend_to(
            &app,
            "slack",
            said("U1", "9.2", "じゃあ15時で"),
            judging(Some(true)),
            now,
        )
        .await
        .unwrap()
        .unwrap();

        assert!(aside.is_none());
        assert_eq!(input.sender, Sender::Owner);
        assert!(matches!(
            &input.source,
            Source::Slack { channel, thread_ts: Some(t), unmentioned: true, .. } if channel == "C1" && t == "8.0"
        ));
        assert!(input.text.contains("<@U2>: 今日は在宅です"));
        assert!(input.text.ends_with("じゃあ15時で"));
        assert_eq!(app.store.next_pending().unwrap().unwrap().input, input);
        assert!(
            attention::context(&app.store, "slack", "C1")
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_file_alone_is_weighed_by_its_name_and_goes_along() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-file");
        let app = channel_app(dir.path());
        attention::attend(&app.store, "slack", "C1", Utc::now()).unwrap();
        let file = attachments::Attached::OnSlack {
            name: "画面.png".into(),
            mime: "image/png".into(),
            size: 12,
            path: "x".into(),
        };
        let mut said = said("U1", "9.7", "");
        said.files = vec![file.clone()];
        let mut weighed = String::new();

        let input = attend_to(
            &app,
            "slack",
            said,
            |item, _| {
                weighed = item.body;
                std::future::ready(Ok(Some(true)))
            },
            Utc::now(),
        )
        .await
        .unwrap()
        .unwrap();

        assert!(weighed.contains("画面.png"));
        assert_eq!(input.attached(), [file]);
    }

    #[tokio::test]
    async fn someone_elses_words_said_to_the_secretary_reach_it_as_theirs() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-other");
        let app = channel_app(dir.path());
        attention::attend(&app.store, "slack", "C1", Utc::now()).unwrap();

        let input = attend_to(
            &app,
            "slack",
            said("U2", "9.3", "秘書さん、資料どこ？"),
            judging(Some(true)),
            Utc::now(),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(input.sender, Sender::Other);
        assert!(matches!(&input.source, Source::Slack { user, .. } if user == "U2"));
    }

    #[tokio::test]
    async fn words_not_said_to_the_secretary_or_over_the_cap_reach_no_one() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-no");
        let app = channel_app(dir.path());
        let start = Utc::now() - chrono::Duration::minutes(2);
        attention::attend(&app.store, "slack", "C1", start).unwrap();

        let no = attend_to(
            &app,
            "slack",
            said("U1", "9.4", "了解です"),
            judging(Some(false)),
            Utc::now(),
        )
        .await
        .unwrap();
        let over = attend_to(
            &app,
            "slack",
            said("U1", "9.5", "それで"),
            judging(None),
            Utc::now(),
        )
        .await
        .unwrap();

        assert!(no.is_none() && over.is_none());
        assert!(app.store.next_pending().unwrap().is_none());
        assert!(
            !attention::attending(
                &app.store,
                "slack",
                "C1",
                start + chrono::Duration::minutes(i64::from(attention::DEFAULT_MINUTES))
            )
            .unwrap(),
            "words not said to the secretary do not keep the channel attended"
        );
    }

    #[tokio::test]
    async fn words_said_to_the_secretary_keep_the_channel_attended() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-extend");
        let app = channel_app(dir.path());
        let start = Utc::now() - chrono::Duration::minutes(2);
        attention::attend(&app.store, "slack", "C1", start).unwrap();

        attend_to(
            &app,
            "slack",
            said("U1", "9.6", "あと一つ"),
            judging(Some(true)),
            Utc::now(),
        )
        .await
        .unwrap();

        assert!(
            attention::attending(
                &app.store,
                "slack",
                "C1",
                start + chrono::Duration::minutes(i64::from(attention::DEFAULT_MINUTES) + 1)
            )
            .unwrap()
        );
    }

    fn leaked(slack: FakeSlack) -> &'static FakeSlack {
        Box::leak(Box::new(slack))
    }

    /// What Slack was told to show, once as many statuses as `count` came.
    async fn statuses(slack: &FakeSlack, count: usize) -> Vec<(String, String, String)> {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while slack.statuses().len() < count {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the statuses are shown");
        slack.statuses()
    }

    fn status(text: &str) -> (String, String, String) {
        ("C1".into(), "8.0".into(), text.into())
    }

    async fn weigh(
        app: &App,
        slack: &'static FakeSlack,
        said: Said,
        answer: Option<bool>,
    ) -> Option<Input> {
        let judge = judging(answer);
        let asked = answer.is_some();
        let received = std::time::Instant::now();
        take_attended(app, Some(slack), "slack", said, asked, judge, received)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_owners_unmentioned_words_are_shown_as_worked_on_until_the_turn_ends() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-status");
        let app = channel_app(dir.path());
        let slack = leaked(FakeSlack::default());

        let input = weigh(&app, slack, said("U1", "9.1", "それで"), Some(true))
            .await
            .unwrap();

        let id = app.store.id_of(&input.dedup_key()).unwrap().unwrap();
        assert_eq!(
            statuses(slack, 2).await,
            [status(presence::WORKING), status(presence::WORKING)],
            "while it is weighed, and once it is an input"
        );
        app.store.start_turn(id, "all").unwrap();
        app.store.end_turn().unwrap();
        assert_eq!(statuses(slack, 3).await[2], status(""));
    }

    #[tokio::test]
    async fn the_owners_words_are_shown_while_they_are_weighed() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-status-weighed");
        let app = channel_app(dir.path());
        let slack = leaked(FakeSlack::default());
        let judge = |_, _| async move {
            assert_eq!(statuses(slack, 1).await, [status(presence::WORKING)]);
            Ok(Some(false))
        };

        let received = std::time::Instant::now();
        let words = said("U1", "9.1", "了解です");
        take_attended(&app, Some(slack), "slack", words, true, judge, received)
            .await
            .unwrap();

        assert_eq!(
            slack.statuses(),
            [status(presence::WORKING), status("")],
            "words not said to the secretary are shown no longer"
        );
    }

    #[tokio::test]
    async fn the_status_goes_away_when_weighing_fails() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-status-failed");
        let app = channel_app(dir.path());
        let slack = leaked(FakeSlack::default());
        let judge = |_, _| std::future::ready(Err(anyhow::anyhow!("no answer")));

        let received = std::time::Instant::now();
        let words = said("U1", "9.1", "それで");
        let _ = take_attended(&app, Some(slack), "slack", words, true, judge, received).await;

        assert_eq!(slack.statuses(), [status(presence::WORKING), status("")]);
    }

    #[tokio::test]
    async fn nothing_is_shown_of_words_triage_is_not_asked_about() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-status-cap");
        let app = channel_app(dir.path());
        let slack = leaked(FakeSlack::default());

        weigh(&app, slack, said("U1", "9.1", "それで"), None).await;

        assert!(slack.statuses().is_empty());
    }

    #[tokio::test]
    async fn nothing_is_shown_of_what_others_say() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-status-other");
        let app = channel_app(dir.path());
        let slack = leaked(FakeSlack::default());

        for (ts, answer) in [("9.1", true), ("9.2", false)] {
            weigh(&app, slack, said("U2", ts, "資料どこ？"), Some(answer)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        assert!(slack.statuses().is_empty());
    }

    #[tokio::test]
    async fn words_not_said_to_the_secretary_leave_the_status_of_an_input_in_the_same_thread() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-status-busy");
        let app = channel_app(dir.path());
        let slack = leaked(FakeSlack::default());
        weigh(&app, slack, said("U1", "9.1", "調べて"), Some(true)).await;
        let before = statuses(slack, 2).await;

        weigh(&app, slack, said("U1", "9.2", "了解です"), Some(false)).await;

        assert_eq!(slack.statuses(), before);
    }

    #[tokio::test]
    async fn unmentioned_words_get_no_reaction_where_slack_shows_no_status() {
        let dir = crate::notebook::testing::TempDir::new("runtime-attend-status-none");
        let app = channel_app(dir.path());
        let slack = leaked(FakeSlack {
            no_status: true,
            ..Default::default()
        });

        let input = weigh(&app, slack, said("U1", "9.1", "それで"), Some(true)).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        assert!(input.is_some());
        assert!(slack.reactions.lock().unwrap().is_empty());
    }
}
