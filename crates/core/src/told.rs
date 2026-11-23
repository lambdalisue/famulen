//! What a connected service tells the secretary itself, without the lookout
//! reading it first.

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::connections::Transport;
use crate::gateway::Arrival;
use crate::input::{Input, Sender, Source};
use crate::slack::web::{Outgoing, Slack};

/// The destination of receivers whose arrivals go straight to the secretary.
pub const SECRETARY: &str = "secretary";

/// What the secretary was told about one thing, and what they said of it,
/// kept until it is over.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Told {
    connection: String,
    id: String,
    input: i64,
    said: Vec<Said>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Said {
    workspace: String,
    channel: String,
    ts: String,
    text: String,
}

fn key(connection: &str, receiver: &str, about: &str) -> String {
    format!("told.{connection}.{receiver}.{about}")
}

/// Give what arrived to the secretary: what its definition says of it, then
/// the service's own words. The same thing told again is not given twice.
/// What was told before and holds no more is taken back.
pub fn tell(app: &App, arrival: &Arrival) -> anyhow::Result<bool> {
    let key = key(&arrival.connection, &arrival.receiver, &arrival.about);
    let id = told_id(arrival);
    let before: Option<Told> = app
        .store
        .state(&key)?
        .and_then(|kept| serde_json::from_str(&kept).ok());
    if let Some(before) = before.filter(|b| arrival.over || b.id != id) {
        app.store.clear_state(&key)?;
        take_back(app, before)?;
    }
    if arrival.over {
        return Ok(false);
    }
    let connection = app.connections.get(&arrival.connection)?;
    let note = connection
        .and_then(|c| match c.transport {
            Transport::Defined { definition, .. } => Some(definition),
            _ => None,
        })
        .and_then(|id| app.gateway.definitions().get(&id).cloned())
        .and_then(|def| def.receive.get(&arrival.receiver)?.note.clone())
        .unwrap_or_default();
    let input = Input {
        sender: Sender::System,
        source: Source::Arrival {
            connection: arrival.connection.clone(),
            receiver: arrival.receiver.clone(),
            id: id.clone(),
            about: arrival.about.clone(),
        },
        text: format!("{}\n---\n{}", note.trim(), arrival.payload),
    };
    let Some(input) = app.store.enqueue(&input)? else {
        return Ok(false);
    };
    let told = Told {
        connection: arrival.connection.clone(),
        id,
        input,
        said: Vec::new(),
    };
    app.store.set_state(&key, serde_json::to_string(&told)?)?;
    app.inputs.notify_one();
    Ok(true)
}

/// Take back what was said of a thing that is over: a turn not begun does not
/// begin, what the owner was asked is asked no more, and what they were told
/// says so.
fn take_back(app: &App, told: Told) -> anyhow::Result<()> {
    app.store.drop_pending(told.input)?;
    let withdrawn = app.gateway.withdraw_decided(&told.connection, &told.id)?;
    let answered = app.gateway.answered_decided(&told.connection, &told.id)?;
    let (store, console) = (app.store.clone(), app.console.clone());
    let slack = app.slack.read().unwrap_or_else(|p| p.into_inner()).clone();
    tokio::spawn(async move {
        let main = slack.as_ref().and_then(|s| s.get(None).ok());
        for id in withdrawn {
            if let Err(error) = crate::approvals::withdraw(&store, main, id).await {
                tracing::warn!(%error, id, "could not take back a request");
            }
        }
        console.notify(crate::console_log::Event::Approvals);
        for said in told.said {
            let Some(slack) = slack
                .as_ref()
                .and_then(|s| s.get(Some(&said.workspace)).ok())
            else {
                continue;
            };
            let outcome = if answered {
                "✓ 答えを届けました"
            } else {
                "✓ もう答えは要りません（ほかで済みました）"
            };
            let message = Outgoing {
                text: format!("{}\n\n{outcome}", said.text),
                ..Default::default()
            };
            if let Err(error) = slack.update(&said.channel, &said.ts, message).await {
                tracing::warn!(%error, "could not say that it is over");
            }
        }
    });
    Ok(())
}

/// What the running turn was told, when it is of an arrival: its key, and
/// whether that still holds.
fn running(app: &App) -> anyhow::Result<Option<(String, Option<Told>)>> {
    let Some(row) = app.store.running_input()? else {
        return Ok(None);
    };
    let Source::Arrival {
        connection,
        receiver,
        id,
        about,
    } = row.input.source
    else {
        return Ok(None);
    };
    let key = key(&connection, &receiver, &about);
    let told = app
        .store
        .state(&key)?
        .and_then(|kept| serde_json::from_str::<Told>(&kept).ok())
        .filter(|told| told.id == id);
    Ok(Some((key, told)))
}

/// Whether the turn is of an arrival that is over, so nothing is to be said.
pub fn is_over(app: &App) -> anyhow::Result<bool> {
    Ok(running(app)?.is_some_and(|(_, told)| told.is_none()))
}

/// Remember what the secretary said on Slack of what they were told, to
/// write on it once that is over.
pub fn said(app: &App, workspace: &str, channel: &str, ts: &str, text: &str) -> anyhow::Result<()> {
    let Some((key, Some(mut told))) = running(app)? else {
        return Ok(());
    };
    told.said.push(Said {
        workspace: workspace.to_owned(),
        channel: channel.to_owned(),
        ts: ts.to_owned(),
        text: text.to_owned(),
    });
    app.store.set_state(&key, serde_json::to_string(&told)?)
}

/// What tells this arrival from others: its question's id, else its number,
/// else all it says.
fn told_id(arrival: &Arrival) -> String {
    let told = ["/question/id", "/seq"]
        .iter()
        .find_map(|at| arrival.payload.pointer(at))
        .unwrap_or(&arrival.payload);
    told.as_str()
        .map_or_else(|| told.to_string(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn what_a_service_tells_reaches_the_secretary_once_with_what_to_do_about_it() {
        let dir = crate::notebook::testing::TempDir::new("told");
        let app = crate::runtime::testing::app(dir.path());
        app.gateway
            .add_defined("adroit", Default::default(), None)
            .unwrap();
        let arrival = |seq: i64, question: &str| Arrival {
            connection: "adroit".into(),
            receiver: "asks".into(),
            received_at: 0,
            about: "s1".into(),
            over: false,
            headers: Default::default(),
            payload: json!({ "seq": seq, "name": "fix", "question": { "id": question } }),
        };

        assert!(tell(&app, &arrival(1, "q1")).unwrap());
        assert!(!tell(&app, &arrival(2, "q1")).unwrap(), "asked again");
        assert!(tell(&app, &arrival(3, "q2")).unwrap());

        let input = app.store.next_pending().unwrap().unwrap().input;
        let text = input.render();
        assert!(text.starts_with("[arrival] connection: adroit receiver: asks\n"));
        let (said, told) = text.split_once("\n---\n").unwrap();
        assert!(said.contains("read_session"), "{said}");
        assert!(told.contains("\"fix\""), "{told}");
    }

    #[tokio::test]
    async fn what_is_over_before_the_secretary_reads_it_is_not_read() {
        let dir = crate::notebook::testing::TempDir::new("told-over");
        let app = crate::runtime::testing::app(dir.path());
        app.gateway
            .add_defined("adroit", Default::default(), None)
            .unwrap();
        let arrival = |about: &str, question: &str, over: bool| Arrival {
            connection: "adroit".into(),
            receiver: "asks".into(),
            received_at: 0,
            about: about.into(),
            over,
            headers: Default::default(),
            payload: json!({ "seq": 1, "question": { "id": question } }),
        };

        tell(&app, &arrival("s1", "q1", false)).unwrap();
        tell(&app, &arrival("s2", "q2", false)).unwrap();
        tell(&app, &arrival("s1", "", true)).unwrap();
        let left = app.store.next_pending().unwrap().unwrap();
        assert!(left.input.text.contains("q2"), "{}", left.input.text);

        tell(&app, &arrival("s2", "q3", false)).unwrap();
        let left = app.store.next_pending().unwrap().unwrap();
        assert!(left.input.text.contains("q3"), "another is asked now");

        assert!(app.store.start_turn(left.id, "all").unwrap());
        assert!(!is_over(&app).unwrap());
        said(&app, "slack", "D1", "1.0", "聞いています").unwrap();
        tell(&app, &arrival("s2", "", true)).unwrap();
        assert!(is_over(&app).unwrap(), "nothing is to be said of it");
    }
}
