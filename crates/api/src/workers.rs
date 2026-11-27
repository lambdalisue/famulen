//! Delegated tasks and the pool that runs them.

use chrono::Utc;
use connectrpc::{
    ConnectError, RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream,
};
use famulen_core::conversations::task_key;
use famulen_core::notebook::Writer;
use famulen_core::records::{Field as CoreField, Patch, TASKS, is_over};
use famulen_core::workers::{
    self, Finish, How, Left, Order, Outcome, Placed, Reading, Run, Runner, Settings, Sight,
    Standing, State, WORKER, is_delegated,
};
use famulen_core::workgroups::ALL;

use crate::pb::*;
use crate::records::in_workgroup;
use crate::{Api, OWNER, OrFail, actor, by_secretary, changed, fail, workgroups_for, writer};

/// How many ended runs are listed.
const ENDED: usize = 20;

fn run(r: Run) -> WorkerRun {
    WorkerRun {
        id: r.id,
        task: r.task,
        code: r.order.code,
        title: r.order.title,
        runner: r.order.runner.as_str().into(),
        state: r.state.as_str().into(),
        outcome: r.outcome.map(Outcome::as_str).unwrap_or_default().into(),
        attempt: r.attempt as i32,
        session: r.session,
        approval: r
            .ticket
            .filter(|_| r.state == State::Asking)
            .unwrap_or_default(),
        result: if r.result.is_empty() {
            r.note
        } else {
            r.result
        },
        created_at: r.created_at,
        started_at: r.started_at.unwrap_or_default(),
        ended_at: r.ended_at.unwrap_or_default(),
        ..Default::default()
    }
}

/// A run with what can be seen of it.
fn seen(r: Run, sight: &Sight) -> WorkerRun {
    WorkerRun {
        steps: sight.steps.iter().map(crate::runs::step).collect(),
        plan_from: sight.from.map(|f| f.as_str()).unwrap_or_default().into(),
        percent: (sight.done().unwrap_or_default() * 100.0) as i32,
        doing: sight.doing(),
        ..run(r)
    }
}

fn standing(placed: Placed) -> (&'static str, String) {
    let name = match placed.standing {
        Standing::Next | Standing::Approved if placed.start => "starting",
        Standing::Next => "queued",
        Standing::Paused => "paused",
        Standing::Capped => "capped",
        Standing::Later => "later",
        Standing::Blocked => "blocked",
        Standing::Unfit(why) => return ("unfit", why),
        Standing::Asking => "asking",
        Standing::Approved => "approved",
        Standing::Running => "running",
        Standing::Waiting => "waiting",
        Standing::Idle => "idle",
        Standing::Retrying => "retrying",
        Standing::Unreachable => "unreachable",
        Standing::Still => "still",
    };
    (name, String::new())
}

fn finish(f: Finish) -> WorkerFinish {
    WorkerFinish {
        repository: f.repository,
        steps: f.steps,
        ..Default::default()
    }
}

fn settings(s: Settings) -> WorkerSettings {
    WorkerSettings {
        slots: s.slots as i32,
        paused: s.paused,
        finishes: s.finishes.into_iter().map(finish).collect(),
        bundled_finishes: workers::bundled_finishes()
            .into_iter()
            .map(finish)
            .collect(),
        ..Default::default()
    }
}

fn left(l: Left) -> LeftBehind {
    LeftBehind {
        kind: l.kind.as_str().into(),
        fate: l.fate.as_str().into(),
        why: l.fate.why().into(),
        name: l.name,
        task: l.task,
        code: l.code,
        title: l.title,
        bytes: l
            .bytes
            .and_then(|b| i64::try_from(b).ok())
            .unwrap_or_default(),
        ..Default::default()
    }
}

impl Api {
    async fn tidied(&self, how: How) -> anyhow::Result<LeftBehindList> {
        Ok(LeftBehindList {
            left: workers::tidy(&self.app, how)
                .await?
                .into_iter()
                .map(left)
                .collect(),
            ..Default::default()
        })
    }

    /// A run with what its session asks the owner, for the owner alone.
    fn asking(&self, run: Run, sight: &Sight) -> anyhow::Result<WorkerRun> {
        if run.order.runner == Runner::Direct {
            return Ok(match workers::direct_question(&self.app, &run) {
                Some(q) => self.asking_directly(run, sight, q),
                None => seen(run, sight),
            });
        }
        let question = workers::adroit::question(&self.app, &run, sight)?;
        let mut run = seen(run, sight);
        if let Some(q) = question {
            let approval = self.approval(famulen_core::approvals::Request {
                id: q.ticket,
                tool: q.tool,
                input: q.shown.to_string(),
                status: "pending".into(),
                created_at: 0,
            });
            run.question = Some(RunQuestion {
                id: q.id,
                truncated: q.truncated,
                approval: Some(approval).into(),
                ..Default::default()
            })
            .into();
        }
        Ok(run)
    }

    /// A run run directly, with what it asks the owner: a request of the
    /// gateway as the inbox shows it, or what it asks its user.
    fn asking_directly(&self, run: Run, sight: &Sight, q: workers::DirectQuestion) -> WorkerRun {
        // A request as the gateway keeps it, with its secrets sealed.
        let input = if q.ticket > 0 {
            match self.app.gateway.ticket(q.ticket).ok().flatten() {
                Some(ticket) => ticket.shown.to_string(),
                // What cannot be read is not put to the owner half told.
                None => return seen(run, sight),
            }
        } else {
            q.input.to_string()
        };
        let mut approval = self.approval(famulen_core::approvals::Request {
            id: q.ticket,
            tool: q.tool,
            input,
            status: "pending".into(),
            created_at: 0,
        });
        // What the inbox would show of it, descriptions and all, where it can
        // be read so.
        if q.ticket == 0 && approval.questions.is_empty() {
            approval.questions = q
                .choices
                .into_iter()
                .map(|(question, options, several)| ApprovalQuestion {
                    question,
                    several,
                    choices: options
                        .into_iter()
                        .map(|label| ApprovalChoice {
                            label,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                })
                .collect();
        }
        let mut run = seen(run, sight);
        run.question = Some(RunQuestion {
            id: q.id,
            approval: Some(approval).into(),
            ..Default::default()
        })
        .into();
        run
    }

    /// How a run stands, read anew, without what it did.
    pub(crate) async fn run_state(&self, id: i64) -> anyhow::Result<RunOutput> {
        let (run, sight, why) = workers::watch(&self.app, id).await?;
        let usage = self.app.models.run_usage(run.id)?;
        Ok(RunOutput {
            usage: usage
                .into_iter()
                .filter_map(crate::models::usage_row)
                .collect(),
            changes: crate::runs::changes(&self.app, run.id)?,
            run: Some(self.asking(run, &sight)?).into(),
            why,
            ..Default::default()
        })
    }

    /// Change a delegated task, and have the pool see it.
    fn change_delegated(&self, by: Writer, id: &str, patch: Patch) -> anyhow::Result<Record> {
        let task = self.app.records.need(TASKS, id)?;
        anyhow::ensure!(is_delegated(&task), "{} is not delegated", task.title());
        self.app.records.change(by, TASKS, &task.id, patch)?;
        changed(self);
        self.app.workers.poke();
        self.whole(TASKS, &task.id)
    }
}

impl WorkerService for Api {
    async fn delegate_task(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, DelegateTaskRequest>,
    ) -> ServiceResult<Record> {
        let r = request.to_owned_message();
        if r.title.trim().is_empty() {
            return Err(fail("a record needs a title"));
        }
        let runner = Runner::parse(r.runner.trim())
            .ok_or_else(|| fail("a runner is adroit, agent or direct"))?;
        let trimmed = |text: &str| text.trim().to_owned();
        let order = Order {
            title: trimmed(&r.title),
            runner,
            request: trimmed(&r.request),
            connection: trimmed(&r.connection),
            directory: trimmed(&r.directory),
            worktree: r.worktree,
            base: trimmed(&r.base),
            agent: trimmed(&r.agent),
            model: trimmed(&r.model),
            permissions: trimmed(&r.permissions),
            tools: r.tools,
            files: r.files,
            wait: r.wait,
            notebook: match r.notebook.trim() {
                "" => Reading::default(),
                level => Reading::parse(level)
                    .ok_or_else(|| fail("a notebook is none, excerpt, named, search or propose"))?,
            },
            search: r.search,
            ..Order::default()
        };
        order.check().or_fail()?;
        workers::fit(&self.app, &order).or_fail()?;
        let mut patch = Patch {
            notes: Some(order.request.clone()),
            source: if by_secretary(&ctx) {
                format!("{}が委ねた", self.app.aide_name())
            } else {
                "コンソールで足した".to_owned()
            },
            workgroups: Some(workgroups_for(self, &ctx, r.workgroups)),
            sections: vec![order.section()],
            ..Patch::titled(&order.title)
        }
        .with("Assignee", WORKER);
        for (name, value) in [("Priority", r.priority), ("Start", r.start), ("Due", r.due)] {
            if !value.trim().is_empty() {
                patch = patch.with(name, value.trim());
            }
        }
        if !r.blocked_by.is_empty() {
            let links = r
                .blocked_by
                .iter()
                .map(|id| Ok(self.app.records.need(TASKS, id)?.link()))
                .collect::<anyhow::Result<Vec<_>>>()
                .or_fail()?;
            patch.fields.push(CoreField {
                name: "Blocked by".to_owned(),
                value: String::new(),
                values: links,
            });
        }
        let task = self.app.records.add(writer(&ctx), TASKS, patch).or_fail()?;
        workers::written(&self.app, &task, writer(&ctx)).or_fail()?;
        changed(self);
        self.app.workers.poke();
        Response::ok(self.whole(TASKS, &task.id).or_fail()?)
    }

    async fn list_delegated(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListDelegatedRequest>,
    ) -> ServiceResult<ListDelegatedResponse> {
        let asked = request.to_owned_message();
        let only = match asked.task.as_str() {
            "" => None,
            task => Some(
                self.app
                    .records
                    .get(TASKS, task)
                    .or_fail()?
                    .map_or_else(|| task.to_owned(), |t| t.id),
            ),
        };
        let queue = workers::queue(&self.app, Utc::now())
            .or_fail()?
            .into_iter()
            .filter(|q| only.as_ref().is_none_or(|id| *id == q.task.id))
            .filter(|q| in_workgroup(&q.task.workgroups(), &asked.workgroup))
            .map(|q| {
                let runner = Order::of(&q.task, &q.code)
                    .map(|o| o.runner.as_str().to_owned())
                    .unwrap_or_default();
                let (state, why) = standing(q.placed);
                let run = q
                    .run
                    .map(|r| {
                        let sight = self.app.workers.sights.of(r.id);
                        if actor(&ctx) == OWNER {
                            self.asking(r, &sight)
                        } else {
                            Ok(seen(r, &sight))
                        }
                    })
                    .transpose()?;
                Ok(DelegatedTask {
                    title: q.task.title(),
                    task: q.task.id,
                    code: q.code,
                    runner,
                    state: state.into(),
                    why,
                    run: run.into(),
                    ..Default::default()
                })
            })
            .collect::<anyhow::Result<_>>()
            .or_fail()?;
        let ended = self
            .app
            .workers
            .runs
            .ended_of(
                only.as_deref(),
                Some(asked.workgroup.as_str()).filter(|w| !w.is_empty() && *w != ALL),
                ENDED,
            )
            .or_fail()?
            .into_iter()
            .map(run)
            .collect();
        Response::ok(ListDelegatedResponse {
            queue,
            ended,
            ..Default::default()
        })
    }

    async fn read_run(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ReadRunRequest>,
    ) -> ServiceResult<RunOutput> {
        let mut output = self.run_state(request.id).await.or_fail()?;
        let (app, id) = (self.app.clone(), request.id);
        // Read apart from the runtime: a run may have kept megabytes.
        output.entries = tokio::task::spawn_blocking(move || crate::runs::entries(&app, id, 0))
            .await
            .map_err(anyhow::Error::from)
            .and_then(|read| read)
            .or_fail()?;
        Response::ok(output)
    }

    async fn watch_run(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, WatchRunRequest>,
    ) -> ServiceResult<ServiceStream<RunEvent>> {
        let (id, after) = (request.id, request.after);
        if self.app.workers.runs.get(id).or_fail()?.is_none() {
            return Err(ConnectError::not_found(format!("no run #{id}")));
        }
        // Before what is kept is read, so that nothing falls between.
        let (live, writing) = self.app.workers.sights.ledger().watch(id);
        let first = self.run_state(id).await.or_fail()?;
        Response::stream_ok(crate::runs::watch(
            self.clone(),
            id,
            after,
            first,
            (live, writing),
        ))
    }

    async fn retry_delegated(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Record> {
        let task = self.app.records.need(TASKS, request.id).or_fail()?;
        let open = self
            .app
            .workers
            .runs
            .open()
            .or_fail()?
            .iter()
            .any(|r| r.task == task.id);
        if open {
            return Err(fail(format!("{} is running already", task.title())));
        }
        let code = self.app.records.code(TASKS, &task.id).or_fail()?;
        workers::ordered(&self.app, &task, &code).or_fail()?;
        if by_secretary(&ctx) {
            self.app
                .conversations
                .move_to_running(task_key(&task.id))
                .or_fail()?;
        }
        let patch = Patch::default()
            .with("Status", "pending")
            .with("Waiting for", "");
        Response::ok(
            self.change_delegated(writer(&ctx), &task.id, patch)
                .or_fail()?,
        )
    }

    async fn cancel_delegated(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Record> {
        let task = self.app.records.need(TASKS, request.id).or_fail()?;
        if is_over(&task) {
            return Err(fail(format!("{} is over already", task.title())));
        }
        Response::ok(
            self.change_delegated(
                writer(&ctx),
                &task.id,
                Patch::default().with("Status", "cancelled"),
            )
            .or_fail()?,
        )
    }

    async fn answer_run(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, AnswerRunRequest>,
    ) -> ServiceResult<Empty> {
        let r = request.to_owned_message();
        let answer = crate::console::answer_of(&r.answer)?;
        let run = self.app.workers.runs.get(r.run).or_fail()?;
        if run.is_some_and(|run| run.order.runner == workers::Runner::Direct) {
            workers::answer_direct(&self.app, r.run, &r.question, answer, r.replies)
                .await
                .or_fail()?;
            changed(self);
            return Response::ok(Empty::default());
        }
        workers::adroit::answer(&self.app, r.run, &r.question, answer, r.replies)
            .await
            .or_fail()?;
        changed(self);
        Response::ok(Empty::default())
    }

    async fn ask_run_again(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, AskRunAgainRequest>,
    ) -> ServiceResult<Empty> {
        let run = self.app.workers.runs.get(request.run).or_fail()?;
        if run.is_some_and(|run| run.order.runner == workers::Runner::Direct) {
            return Err(fail(
                "what work run directly asks is put to the owner once; answer it here or in the inbox",
            ));
        }
        workers::adroit::ask_again(&self.app, request.run, request.question)
            .await
            .or_fail()?;
        changed(self);
        Response::ok(Empty::default())
    }

    async fn preview_tidying(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<LeftBehindList> {
        Response::ok(self.tidied(How::Preview).await.or_fail()?)
    }

    async fn tidy_up(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<LeftBehindList> {
        Response::ok(self.tidied(How::Remove).await.or_fail()?)
    }

    async fn get_worker_settings(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<WorkerSettings> {
        Response::ok(settings(Settings::load(&self.app.store).or_fail()?))
    }

    async fn set_worker_settings(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, WorkerSettings>,
    ) -> ServiceResult<WorkerSettings> {
        let chosen = Settings {
            slots: u32::try_from(request.slots).map_err(fail)?,
            paused: request.paused,
            finishes: request
                .to_owned_message()
                .finishes
                .into_iter()
                .map(|f| Finish::new(&f.repository, &f.steps))
                .collect::<anyhow::Result<_>>()
                .or_fail()?,
        };
        chosen.save(&self.app.store).or_fail()?;
        self.app.journal.record(
            "setting",
            format!(
                "workers: {} at once, paused {}, {} finishes",
                chosen.slots,
                chosen.paused,
                chosen.finishes.len()
            ),
        );
        changed(self);
        self.app.workers.poke();
        Response::ok(settings(chosen))
    }
}
