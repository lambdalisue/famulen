//! Dreaming, and recording how it went.

use chrono::Utc;
use serde_json::Value;

use crate::app::App;
use crate::console_log::Event;
use crate::dreaming;
use crate::models::Role;
use crate::notebook_place;
use crate::tidying;

/// Set while a dream runs, so the console can say so and a second one waits.
pub const DREAMING_KEY: &str = "dream.running";

/// How a dream that was asked for went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dreamt {
    /// The model's answer was applied, changing something or not.
    Applied,
    Failed,
    /// Another dream is running, or the notebook is moving: none was seen.
    Busy,
}

/// Dream once and record how it went, unless a dream is already running.
pub async fn dream_and_record(app: &App) -> Dreamt {
    if app.store.state(DREAMING_KEY).ok().flatten().is_some()
        || notebook_place::moving(&app.store).is_ok_and(|to| to.is_some())
    {
        return Dreamt::Busy;
    }
    let _ = app
        .store
        .set_state(DREAMING_KEY, Utc::now().timestamp().to_string());
    app.console.notify(Event::Records);
    let dreamt = match dream(app).await {
        Ok((status, applied)) => {
            app.journal.record("dream", status);
            if applied {
                Dreamt::Applied
            } else {
                Dreamt::Failed
            }
        }
        Err(error) => {
            tracing::warn!(%error, "the dream failed");
            app.journal
                .record("dream", format!("見られなかった（{error}）"));
            Dreamt::Failed
        }
    };
    let _ = app.store.clear_state(DREAMING_KEY);
    app.console.notify(Event::Records);
    dreamt
}

async fn dream(app: &App) -> anyhow::Result<(String, bool)> {
    let tz = app.tz();
    let today = Utc::now().with_timezone(&tz).date_naive();
    let outcome = dreaming::dream(
        &app.store,
        &app.records,
        &app.memory_dir,
        today,
        |system, prompt| ask(app, system, prompt, dreaming::plan_schema()),
    )
    .await?;
    if !outcome.applied {
        return Ok((outcome.status, false));
    }
    // Memory stands by now, whatever becomes of the records.
    let tidied = tidying::tidy(
        &app.store,
        &app.records,
        &app.memory_dir,
        today,
        |system, prompt| ask(app, system, prompt, tidying::schema()),
    )
    .await;
    let (changes, status) = match tidied {
        Ok(None) => return Ok((outcome.status, true)),
        Ok(Some(tidied)) => (tidied.changes, tidied.status),
        Err(error) => {
            tracing::warn!(%error, "the records were not tidied");
            (Vec::new(), format!("記録は整えられなかった（{error}）"))
        }
    };
    if let Err(error) = dreaming::add_to_night(&app.store, outcome.night, changes, &status) {
        tracing::warn!(%error, "what tidying did was not added to its night");
    }
    Ok((format!("{}。{status}", outcome.status), true))
}

async fn ask(app: &App, system: String, prompt: String, schema: Value) -> anyhow::Result<Value> {
    let completion = app
        .models
        .complete(Role::Dream, app.tz(), &system, &prompt, Some(&schema))
        .await?;
    completion
        .json
        .ok_or_else(|| anyhow::anyhow!("the dream answered without JSON: {}", completion.text))
}
