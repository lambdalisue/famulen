//! Dreaming, and recording how it went.

use chrono::Utc;

use crate::app::App;
use crate::console_log::Event;
use crate::dreaming;
use crate::models::Role;
use crate::notebook_place;

/// Set while a dream runs, so the console can say so and a second one waits.
pub const DREAMING_KEY: &str = "dream.running";

/// Dream once and record how it went, unless a dream is already running.
pub async fn dream_and_record(app: &App) {
    if app.store.state(DREAMING_KEY).ok().flatten().is_some()
        || notebook_place::moving(&app.store).is_ok_and(|to| to.is_some())
    {
        return;
    }
    let _ = app
        .store
        .set_state(DREAMING_KEY, Utc::now().timestamp().to_string());
    app.console.notify(Event::Records);
    match dream(app).await {
        Ok(status) => app.journal.record("dream", status),
        Err(error) => {
            tracing::warn!(%error, "the dream failed");
            app.journal
                .record("dream", format!("見られなかった（{error}）"));
        }
    }
    let _ = app.store.clear_state(DREAMING_KEY);
    app.console.notify(Event::Records);
}

async fn dream(app: &App) -> anyhow::Result<String> {
    let tz = app.tz();
    let today = Utc::now().with_timezone(&tz).date_naive();
    let models = app.models.clone();
    let outcome = dreaming::dream(
        &app.store,
        &app.records,
        &app.memory_dir,
        today,
        |system, prompt| async move {
            let schema = dreaming::plan_schema();
            let completion = models
                .complete(Role::Dream, tz, &system, &prompt, Some(&schema))
                .await?;
            completion.json.ok_or_else(|| {
                anyhow::anyhow!("the dream answered without JSON: {}", completion.text)
            })
        },
    )
    .await?;
    Ok(outcome.status)
}
