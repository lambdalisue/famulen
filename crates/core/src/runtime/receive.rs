//! Where what the gateway's receivers take in goes.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::app::App;
use crate::channel::CHANNEL;
use crate::gateway::Arrival;
use crate::lookout::Item;
use crate::told::SECRETARY;

const LOOKOUT: &str = "lookout";
const CALENDAR: &str = "calendar";

/// The destination a receiver goes to unless the owner chose another: the
/// one its definition suggests when Famulen has it, else the lookout.
fn default_destination(suggested: Option<&str>) -> &'static str {
    [CHANNEL, CALENDAR, LOOKOUT, SECRETARY]
        .into_iter()
        .find(|d| Some(*d) == suggested)
        .unwrap_or(LOOKOUT)
}

/// Send every receiver that has no destination yet to its default one, and
/// have the gateway open and close its connections to match. Runs at start
/// and again whenever connections may have changed, so a connection added
/// while Famulen runs starts receiving without a restart.
pub fn sync_receivers(app: &App) -> anyhow::Result<()> {
    for c in app.connections.list()? {
        for (receiver, to) in app.gateway.receivers_of(&c.id)? {
            if to.is_none() && !app.gateway.destination_chosen(&c.id, &receiver)? {
                let suggested = app.gateway.suggested_destination(&c.id, &receiver)?;
                app.gateway.set_destination(
                    crate::gateway::OWNER,
                    &c.id,
                    &receiver,
                    Some(default_destination(suggested.as_deref())),
                )?;
            }
        }
    }
    app.gateway.sync_connections()
}

/// Take what the gateway's receivers hand over: register Famulen's
/// destinations, send each receiver to one, and open the doors. What comes
/// for the channel waits in `inbox` for the engine.
pub(super) fn receive(
    app: &App,
    listen: Option<SocketAddr>,
    inbox: mpsc::UnboundedSender<Arrival>,
) -> anyhow::Result<()> {
    app.gateway.register_destination(
        CHANNEL,
        Arc::new(move |a: Arrival| {
            if inbox.send(a).is_err() {
                tracing::warn!("the channel is gone; what Slack sent is dropped");
            }
        }),
    );
    let told_app = app.clone();
    app.gateway.register_destination(
        SECRETARY,
        Arc::new(move |a: Arrival| {
            if let Err(error) = of_a_delegated_task(&told_app, &a) {
                tracing::warn!(connection = %a.connection, %error, "could not tell the worker pool what arrived");
            }
            if let Err(error) = crate::told::tell(&told_app, &a) {
                tracing::warn!(connection = %a.connection, %error, "could not tell the secretary what arrived");
            }
        }),
    );
    let lookout_app = app.clone();
    app.gateway.register_destination(
        LOOKOUT,
        Arc::new(move |a: Arrival| {
            if let Err(error) = watched(&lookout_app, &a) {
                tracing::warn!(connection = %a.connection, %error, "could not read what arrived for the lookout");
            }
        }),
    );
    let calendar_app = app.clone();
    app.gateway.register_destination(
        CALENDAR,
        Arc::new(move |_| calendar_app.refresh_calendars_later()),
    );
    if let Some(listen) = listen {
        let serving = app.gateway.serve_receivers(listen);
        tokio::spawn(async move {
            if let Err(error) = serving.await {
                tracing::error!(%error, "the receivers stopped");
            }
        });
        tracing::info!(%listen, "receiving");
    }
    sync_receivers(app)
}

/// Whether what arrived is of a session that runs a delegated task. The pool
/// is told, and tells the secretary itself how the task ended.
fn of_a_delegated_task(app: &App, arrival: &Arrival) -> anyhow::Result<bool> {
    let adroit = app
        .connections
        .get(&arrival.connection)?
        .is_some_and(|c| crate::workers::adroit::is_adroit(&c));
    match arrival.payload["id"].as_str() {
        Some(session) if adroit => app.workers.heard_of(session),
        _ => Ok(false),
    }
}

/// Hand what arrived to the lookout, as heard from its connection.
fn watched(app: &App, arrival: &Arrival) -> anyhow::Result<()> {
    if of_a_delegated_task(app, arrival)? {
        return Ok(());
    }
    let text = match &arrival.payload {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let item = Item {
        title: text.chars().take(120).collect(),
        body: text,
    };
    app.lookout.hear(&arrival.connection, &item)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn each_receiver_is_sent_to_its_default_destination() {
        let dir = crate::notebook::testing::TempDir::new("runtime-receivers");
        let app = crate::channel::tests::app_with_glasses(dir.path());
        let glasses = app
            .gateway
            .add_defined("glasses", Default::default(), None)
            .unwrap();
        let google = app
            .gateway
            .add_defined(crate::arrivals::GOOGLE, Default::default(), None)
            .unwrap();
        let slack = app
            .gateway
            .add_defined(crate::channel::DEFINITION, Default::default(), None)
            .unwrap();

        sync_receivers(&app).unwrap();

        let to = |c: &str, r: &str| app.gateway.destination_of(c, r).unwrap();
        assert_eq!(to(&google.id, "calendar").as_deref(), Some(CALENDAR));
        assert_eq!(to(&slack.id, "events").as_deref(), Some(CHANNEL));
        assert_eq!(
            to(&glasses.id, "said").as_deref(),
            Some(CHANNEL),
            "an added definition's suggestion is followed too"
        );
        assert_eq!(to(&glasses.id, "seen").as_deref(), Some(LOOKOUT));

        app.gateway
            .set_destination(crate::gateway::OWNER, &google.id, "calendar", None)
            .unwrap();
        sync_receivers(&app).unwrap();
        assert_eq!(
            to(&google.id, "calendar"),
            None,
            "a receiver the owner closed is not sent back to its default"
        );
        app.gateway.close_connections();
    }
}
