//! Where a run run directly passes between its agent and its finishing.

use std::time::Duration;

use super::*;
use crate::workers::{Finishing, OwnHands, pin};

/// Fix how `run` is finished as it begins, where the owner allowed its
/// repository to be.
pub(super) async fn pin_finishing(app: &App, run: &Run, place: &Place) -> anyhow::Result<()> {
    if Finishing::load(&app.store, run.id)?.is_some() {
        return Ok(());
    }
    let branch = format!(
        "famulen/{}",
        crate::workers::adroit::token(&run.order.code, run.id)
    );
    let jj = &app
        .workers
        .direct
        .as_ref()
        .context("no work runs directly now")?
        .jj;
    if let Some(pinned) = pin(&app.workers.finishing, jj, &place.checkout, branch).await? {
        Finishing::new(pinned).save(&app.store, run.id)?;
    }
    Ok(())
}

/// End the agent of a run whose turn is over, and take back what it was let
/// call: nothing of the finishing is done while it could still act.
pub(in crate::workers) async fn end_turn(app: &App, run: i64) -> anyhow::Result<()> {
    direct::forget_asked(app, run)?;
    let executor = executor(app, run);
    executor.stop(STOP_GRACE).await.ok();
    revoke(app, run);
    executor.forget().await
}

pub(in crate::workers) fn own_hands(app: &App, run: &Run) -> anyhow::Result<OwnHands> {
    Ok(OwnHands {
        app: app.clone(),
        run: run.id,
        place: direct::kept_place(&app.store, run.id).context("it has no workspace")?,
        direct: app
            .workers
            .direct
            .clone()
            .context("no work runs directly now")?,
        task: run.order.code.clone(),
        title: run.order.title.clone(),
        request: run.order.request.clone(),
    })
}

impl Job {
    /// Give a run the next turn of its conversation, with what its finishing
    /// has to tell it.
    pub(in crate::workers) async fn turn(
        app: &App,
        run: &Run,
        message: &str,
    ) -> anyhow::Result<Self> {
        let job = Self::again(app, run);
        let mut finishing =
            Finishing::load(&app.store, run.id)?.context("it has nothing to finish")?;
        // Started by a Famulen that went away before it knew.
        if !matches!(
            job.executor.standing().await?,
            Standing::Working | Standing::Foreign | Standing::Over
        ) {
            anyhow::ensure!(job.place.is_some(), "it does not run directly");
            anyhow::ensure!(app.workers.direct.is_some(), "no work runs directly now");
            super::super::fit(app, &run.order)?;
            let conversation = app
                .store
                .state(direct::conversation_key(run.id))?
                .context("its conversation was not kept")?;
            job.executor.forget().await?;
            app.store.clear_state(direct::resumed_key(run.id))?;
            app.store.clear_state(direct::went_on_key(run.id))?;
            direct::forget_asked(app, run.id)?;
            revoke(app, run.id);
            let until = chrono::Utc::now().timestamp() + DIRECT_TIME_LIMIT.as_secs() as i64;
            app.permissions.grant_job(run.id, &run.order.tools, until)?;
            let started = job
                .start(message, None, Some(&conversation), false, DIRECT_TIME_LIMIT)
                .await;
            if let Err(error) = started {
                revoke(app, run.id);
                return Err(error);
            }
            app.workers
                .sights
                .asked(run.id, chrono::Utc::now().timestamp_millis(), message);
        }
        finishing.given(chrono::Utc::now().timestamp());
        finishing.save(&app.store, run.id)?;
        Ok(job)
    }
}

/// How long the turn under way may still take, one handed back by the
/// finishing counting from when it was given.
pub(super) fn left_of(app: &App, run: &Run) -> anyhow::Result<Duration> {
    let began = Finishing::load(&app.store, run.id)?
        .and_then(|f| f.turn_began)
        .or(run.started_at)
        .unwrap_or(run.created_at);
    let used = Duration::from_secs((chrono::Utc::now().timestamp() - began).max(0) as u64);
    DIRECT_TIME_LIMIT
        .checked_sub(used)
        .context("it took as long as a run may")
}
