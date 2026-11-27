//! Running the review of a run's work: an agent beside the one that did it,
//! kept as that one is, with nothing to write by and nobody to ask.

use super::*;
use crate::workers::finishing::{REVIEW_PROMPT, REVIEW_TOOLS};

/// How long one review may take.
const LIMIT: Duration = Duration::from_secs(30 * 60);

fn review_executor(app: &App, run: i64) -> Executor {
    app.workers.executors.of(format!("{run}-review"))
}

/// End the review of a run, if one is going.
pub(in crate::workers) async fn forget_review(app: &App, run: i64) -> anyhow::Result<()> {
    review_executor(app, run).forget().await
}

/// `fence` with nothing left to write to.
fn reading_only(mut fence: Fence, workspace: &Path) -> Fence {
    fence.unwritable.push(workspace.to_owned());
    fence.writable.clear();
    fence
}

/// How the review is started: nothing of the repository is to be written.
fn review_work(app: &App, place: &Place, message: String) -> anyhow::Result<(Work, ModelSpec)> {
    let (bin, model, effort, spec) = model(app)?;
    let fence = reading_only(
        direct::fence(&app.workers.kept, &app.workers.dir, place),
        &place.workspace,
    );
    let work = Work {
        program: Some(bin),
        model,
        effort,
        instructions: REVIEW_PROMPT.to_owned(),
        message,
        tools: REVIEW_TOOLS.iter().map(|t| (*t).to_owned()).collect(),
        withheld: LAUNCHERS.iter().map(|t| (*t).to_owned()).collect(),
        servers: Vec::new(),
        workdir: place.directory.clone(),
        fence,
        needs: needs(),
        parts: false,
        conversation: None,
    };
    Ok((work, spec))
}

/// Review the work of `run`: the last message of the reviewer. One under way
/// from before is ended and the review made anew.
pub(in crate::workers) async fn review(
    app: &App,
    run: i64,
    place: &Place,
    message: String,
) -> anyhow::Result<String> {
    let (work, spec) = review_work(app, place, message)?;
    let launch = AGENT.adapter().launch(&work, None)?;
    forget_review(app, run).await?;
    let start = Start {
        program: launch.program,
        args: launch.args,
        env: launch.env,
        hidden_env: hidden_env(),
        cwd: place.directory.clone(),
        input: launch.input,
        limit: LIMIT,
        limit_bytes: BYTES_LIMIT,
        meta: Value::Null,
    };
    let executor = app
        .workers
        .executors
        .start(format!("{run}-review"), start)
        .await?;
    let tools: Vec<String> = work.tools;
    let bounds = Bounds::new(
        &tools,
        &place.directory,
        Some(&place.workspace),
        &app.workers.kept,
    );
    let ended = read(app, run, &spec, &executor, &bounds).await;
    forget_review(app, run).await.ok();
    ended
}

async fn read(
    app: &App,
    run: i64,
    spec: &ModelSpec,
    executor: &Executor,
    bounds: &Bounds,
) -> anyhow::Result<String> {
    let adapter = AGENT.adapter();
    let mut codec = adapter.codec();
    let mut records = executor.attach(0);
    let mut used = Usage {
        calls: 1,
        ..Usage::default()
    };
    let ended = loop {
        let record = match records.next().await? {
            Next::Record(record) => record,
            Next::Gone | Next::Over => break None,
        };
        let What::Out { line } = record.what else {
            continue;
        };
        let mut ended = None;
        for body in codec.read(&line) {
            match body {
                Body::Asked { question } => {
                    let answer = match &question.ask {
                        Ask::Tool { tool, input, .. } => match bounds.refusal(tool, input) {
                            None => Answer::Allow,
                            Some(why) => Answer::Refuse { why },
                        },
                        _ => Answer::Unsupported,
                    };
                    let line = adapter.answer(&question, &answer)?;
                    executor
                        .write(format!("{ANSWER}{}", record.seq), line)
                        .await?;
                }
                Body::Used {
                    of: Counted::Turn,
                    input,
                    output,
                    ..
                } => {
                    used.input_tokens = Some(input);
                    used.output_tokens = Some(output);
                }
                Body::TurnEnded { ok, text } => ended = Some((ok, text)),
                _ => {}
            }
        }
        if ended.is_some() {
            break ended;
        }
    };
    if let Err(error) = app.models.record_run(run, spec, &used) {
        tracing::warn!(%error, "could not count what a review used");
    }
    match ended {
        Some((true, text)) => Ok(text),
        Some((false, text)) => anyhow::bail!("the review went wrong: {text}"),
        None => anyhow::bail!("the review ended without a result"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_review_is_kept_as_the_work_is_with_nothing_to_write() {
        let work = Fence {
            sealed: vec!["/keys".into()],
            writable: vec!["/work".into()],
            ..Fence::default()
        };
        let kept = reading_only(work, Path::new("/work"));
        assert_eq!(kept.sealed, [PathBuf::from("/keys")]);
        assert_eq!(kept.unwritable, [PathBuf::from("/work")]);
        assert!(kept.writable.is_empty());
    }
}
