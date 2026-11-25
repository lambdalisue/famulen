//! The jobs that run whatever the engine does.

use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Timelike, Utc};

use tokio::sync::broadcast::error::RecvError;

use super::dream::{Dreamt, dream_and_record};
use super::keep;
use super::receive::sync_receivers;
use super::watch::poll_lookout;
use crate::app::App;
use crate::console_log::Event;
use crate::dream_schedule;
use crate::dreaming;
use crate::models::Role;
use crate::people;
use crate::store::Store;

const CHORE_TICK: Duration = Duration::from_secs(60);
const FIND_MODELS_EVERY: Duration = Duration::from_secs(60 * 60);
const CALENDAR_EVERY: i64 = 15 * 60;
const DIGEST_AT: u32 = 8;

/// Build the engine again between turns once a change to a service definition
/// goes in, so the session's model CLI reads the connections' new tools.
pub(super) async fn rebuild_when_redefined(app: App) {
    let mut redefined = app.gateway.subscribe_redefined();
    loop {
        match redefined.recv().await {
            Ok(id) => tracing::info!(definition = %id, "a service definition changed"),
            Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => break,
        }
        app.changed.notify_one();
    }
}

pub(super) async fn keep_finding_models(app: App) {
    let mut tick = tokio::time::interval(FIND_MODELS_EVERY);
    loop {
        tick.tick().await;
        if let Err(error) = app.find_models().await {
            tracing::warn!(%error, "could not register the models on this machine");
        }
        app.console.notify(Event::Records);
    }
}

/// Apart from the other chores, since a model may take long over each mail.
pub(super) async fn read_purchases(app: App) {
    let mut tick = tokio::time::interval(CHORE_TICK);
    loop {
        tick.tick().await;
        if let Err(error) = crate::purchases::past::go_on(&app).await {
            tracing::warn!(%error, "could not go back over past mail");
        }
        if let Err(error) = crate::purchases::screen_left(&app).await {
            tracing::warn!(%error, "could not screen the mail left for what was bought");
        }
        if let Err(error) = crate::purchases::read_waiting(&app).await {
            tracing::warn!(%error, "could not read the mail of what was bought");
        }
    }
}

pub(super) async fn chores(app: App) {
    let mut tick = tokio::time::interval(CHORE_TICK);
    let mut last_calendar = 0_i64;
    loop {
        tick.tick().await;
        let tz = app.tz();
        let now = Utc::now();
        let local = now.with_timezone(&tz);
        if now.timestamp() - last_calendar >= CALENDAR_EVERY {
            last_calendar = now.timestamp();
            app.refresh_calendars().await;
            app.console.notify(Event::Records);
        }
        if let Err(error) = app.attachments.forget_old(now.timestamp()) {
            tracing::warn!(%error, "could not throw away old attachments");
        }
        app.read_every_mail().await;
        app.renew_google_watches(app.receive_url.as_deref()).await;
        if let Err(error) = app.gateway.refresh_due().await {
            tracing::warn!(%error, "could not refresh logins");
        }
        if let Err(error) = sync_receivers(&app) {
            tracing::warn!(%error, "could not open the receiving connections");
        }
        if let Err(error) = remind(&app, local.date_naive()) {
            tracing::warn!(%error, "could not check due dates");
        }
        if let Err(error) = poll_lookout(&app).await {
            tracing::warn!(%error, "the lookout failed");
        }
        match daily_due(&app.store, "lookout.digest_on", DIGEST_AT, local) {
            Ok(true) => match app.lookout.digest() {
                Ok(Some(input)) => {
                    let _ = keep(&app.store, &app.inputs, &input);
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "could not make the digest"),
            },
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, "could not check the digest"),
        }
        dream_when_due(&app, now, local.naive_local()).await;
    }
}

/// Dream when the owner's schedule says so, enough notes wait, or a dream
/// that failed is to be tried again. Without a model, or past its daily cap,
/// there is nothing to try.
async fn dream_when_due(app: &App, now: DateTime<Utc>, local: NaiveDateTime) {
    if app.models.assignment(Role::Dream).ok().flatten().is_none()
        || !app
            .models
            .within_cap(Role::Dream, app.tz(), now)
            .unwrap_or(false)
    {
        return;
    }
    let waiting = dreaming::waiting_notes(&app.memory_dir)
        .unwrap_or_default()
        .iter()
        // Others talk all day; a dream for each few notes of it would cost more
        // than it sorts.
        .filter(|(_, note)| !people::is_noted(note))
        .count();
    let due = match dream_schedule::due(&app.store, now, local, waiting) {
        Ok(Some(due)) => due,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, "could not tell whether to dream");
            return;
        }
    };
    let succeeded = match dream_and_record(app).await {
        Dreamt::Applied => true,
        Dreamt::Failed => false,
        Dreamt::Busy => return,
    };
    if let Err(error) = dream_schedule::ended(&app.store, due, succeeded, Utc::now()) {
        tracing::warn!(%error, "could not note how the dream went");
    }
}

fn remind(app: &App, today: chrono::NaiveDate) -> anyhow::Result<()> {
    let made = app.records.remind_deadlines(today)?;
    if !made.is_empty() {
        app.console.notify(Event::Records);
    }
    Ok(())
}

/// Whether the daily job at `hour` is due: past the hour today and not yet
/// run today. Marks it run.
fn daily_due(
    store: &Store,
    key: &str,
    hour: u32,
    now: chrono::DateTime<chrono_tz::Tz>,
) -> anyhow::Result<bool> {
    let today = now.date_naive().to_string();
    if now.hour() < hour || store.state(key)?.as_deref() == Some(today.as_str()) {
        return Ok(false);
    }
    store.set_state(key, &today)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn a_daily_job_runs_once_after_its_hour() {
        let store = Store::open_in_memory().unwrap();
        let tz = chrono_tz::Asia::Tokyo;
        let early = tz.with_ymd_and_hms(2026, 11, 21, 3, 0, 0).unwrap();
        let late = tz.with_ymd_and_hms(2026, 11, 21, 5, 0, 0).unwrap();
        assert!(!daily_due(&store, "k", 4, early).unwrap());
        assert!(daily_due(&store, "k", 4, late).unwrap());
        assert!(!daily_due(&store, "k", 4, late).unwrap());
        assert!(
            daily_due(
                &store,
                "k",
                4,
                tz.with_ymd_and_hms(2026, 11, 22, 4, 0, 0).unwrap()
            )
            .unwrap()
        );
    }
}
