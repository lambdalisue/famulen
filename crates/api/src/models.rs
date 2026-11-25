//! The models Famulen runs on, what each does and what was used of them.

use chrono::Utc;
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::models::{
    Cli, ModelKind as Kind, ModelSpec, Price, Role as CoreRole, UsageSpan as CoreSpan,
};

use crate::pb::{self, *};
use crate::{Api, OWNER, OrFail, fail, given};

/// The roles the owner chooses a model for; the voice waits for the glasses.
const ROLES: [(pb::Role, CoreRole, &str); 6] = [
    (pb::Role::ROLE_SECRETARY, CoreRole::Secretary, "秘書"),
    (pb::Role::ROLE_DREAM, CoreRole::Dream, "夢"),
    (pb::Role::ROLE_LOOKOUT, CoreRole::Lookout, "見張り"),
    (pb::Role::ROLE_TRIAGE, CoreRole::Triage, "トリアージ"),
    (pb::Role::ROLE_ASSESS, CoreRole::Assess, "見極め"),
    (pb::Role::ROLE_WORKER, CoreRole::Worker, "手代"),
];

fn core_role(r: pb::Role) -> Option<CoreRole> {
    ROLES.iter().find(|(p, _, _)| *p == r).map(|(_, c, _)| *c)
}

fn kind(k: Kind) -> pb::ModelKind {
    match k {
        Kind::AgentCli => pb::ModelKind::MODEL_KIND_AGENT_CLI,
        Kind::Local => pb::ModelKind::MODEL_KIND_LOCAL,
        Kind::Api => pb::ModelKind::MODEL_KIND_API,
        Kind::Judge => pb::ModelKind::MODEL_KIND_JUDGE,
        Kind::Transcribe => pb::ModelKind::MODEL_KIND_TRANSCRIBE,
    }
}

fn kind_from(k: pb::ModelKind) -> Option<Kind> {
    Some(match k {
        pb::ModelKind::MODEL_KIND_AGENT_CLI => Kind::AgentCli,
        pb::ModelKind::MODEL_KIND_LOCAL => Kind::Local,
        pb::ModelKind::MODEL_KIND_API => Kind::Api,
        pb::ModelKind::MODEL_KIND_JUDGE => Kind::Judge,
        pb::ModelKind::MODEL_KIND_TRANSCRIBE => Kind::Transcribe,
        _ => return None,
    })
}

fn cli(c: Cli) -> pb::AgentCli {
    match c {
        Cli::Claude => pb::AgentCli::AGENT_CLI_CLAUDE_CODE,
        Cli::Codex => pb::AgentCli::AGENT_CLI_CODEX,
    }
}

fn cli_from(c: Option<pb::AgentCli>) -> Cli {
    match c {
        Some(pb::AgentCli::AGENT_CLI_CODEX) => Cli::Codex,
        _ => Cli::Claude,
    }
}

fn model(spec: &ModelSpec, monthly: Option<f64>) -> Model {
    Model {
        id: spec.id.clone(),
        name: spec.name.clone(),
        kind: kind(spec.kind).into(),
        model: spec.model.clone(),
        endpoint: spec.endpoint.clone().unwrap_or_default(),
        input_yen: spec.price.input_per_mtok_yen.unwrap_or(0.0),
        output_yen: spec.price.output_per_mtok_yen.unwrap_or(0.0),
        call_yen: spec.price.per_call_yen.unwrap_or(0.0),
        monthly_yen: monthly.unwrap_or(0.0),
        cli: if spec.kind == Kind::AgentCli {
            cli(spec.cli)
        } else {
            pb::AgentCli::AGENT_CLI_UNSPECIFIED
        }
        .into(),
        ..Default::default()
    }
}

fn slug(name: &str) -> String {
    let s: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_owned();
    if s.is_empty() { "model".into() } else { s }
}

impl Api {
    fn models(&self) -> anyhow::Result<Models> {
        let tz = self.app.tz();
        let now = Utc::now();
        let models = self.app.models.registered()?;
        let candidates = self.app.models.left_out()?;
        let mut roles = Vec::new();
        for (p, role, label) in ROLES {
            let a = self.app.models.assignment(role)?;
            let (yen, calls) = self.app.models.used_today(role, tz, now)?;
            let choices = models
                .iter()
                .map(|m| &m.spec)
                .filter(|s| role.accepts(s))
                .map(|s| s.id.clone())
                .collect();
            roles.push(RoleSetting {
                role: p.into(),
                label: label.into(),
                model: a.as_ref().map(|a| a.model.clone()).unwrap_or_default(),
                cap: a.as_ref().and_then(|a| a.cap).unwrap_or(0.0),
                cap_unit: a
                    .as_ref()
                    .map(|a| {
                        if a.cap_unit == famulen_core::models::CapUnit::Yen {
                            "円"
                        } else {
                            "回"
                        }
                    })
                    .unwrap_or("回")
                    .into(),
                calls_today: calls,
                yen_today: yen,
                choices,
                effort: a
                    .as_ref()
                    .and_then(|a| a.effort.clone())
                    .unwrap_or_default(),
                default_effort: role.default_effort().unwrap_or_default().into(),
                ..Default::default()
            });
        }
        Ok(Models {
            roles,
            models: models
                .iter()
                .map(|m| {
                    Ok(Model {
                        has_key: self.app.gateway.has_model_key(&m.spec.id)?,
                        found: self.app.models.is_found(&m.spec.id)?,
                        ..model(&m.spec, m.monthly_yen)
                    })
                })
                .collect::<anyhow::Result<_>>()?,
            candidates: candidates.iter().map(|s| model(s, None)).collect(),
            ..Default::default()
        })
    }
}

impl ModelService for Api {
    async fn get_models(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Models> {
        Response::ok(self.models().or_fail()?)
    }

    async fn assign_role(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, AssignRoleRequest>,
    ) -> ServiceResult<Models> {
        let r = request.to_owned_message();
        let role = r
            .role
            .as_known()
            .and_then(core_role)
            .ok_or_else(|| fail("say which role"))?;
        let cap = (r.cap > 0.0).then_some(r.cap);
        if r.model.is_empty() {
            return Err(fail("choose a model"));
        }
        self.app
            .models
            .assign(role, &r.model, cap, Some(&r.effort))
            .or_fail()?;
        if role == CoreRole::Secretary {
            self.app.changed.notify_one();
        }
        Response::ok(self.models().or_fail()?)
    }

    async fn register_model(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, RegisterModelRequest>,
    ) -> ServiceResult<Model> {
        let r = request.to_owned_message();
        let k = r
            .kind
            .as_known()
            .and_then(kind_from)
            .ok_or_else(|| fail("say what kind of model"))?;
        if r.name.trim().is_empty() {
            return Err(fail("a model needs a name"));
        }
        let positive = |v: f64| (v > 0.0).then_some(v);
        let spec = ModelSpec {
            id: format!("{}-{}", slug(&r.name), Utc::now().timestamp() % 100_000),
            name: r.name.trim().to_owned(),
            kind: k,
            endpoint: given(&r.endpoint).map(str::to_owned),
            model: r.model.trim().to_owned(),
            cli: cli_from(r.cli.as_known()),
            price: Price {
                input_per_mtok_yen: positive(r.input_yen),
                output_per_mtok_yen: positive(r.output_yen),
                per_call_yen: positive(r.call_yen),
            },
        };
        self.app
            .models
            .register(spec.clone(), positive(r.monthly_yen))
            .or_fail()?;
        if let Some(key) = given(&r.api_key)
            && let Err(error) = self.app.gateway.put_model_key(OWNER, &spec.id, key)
        {
            self.app.models.unregister(&spec.id).or_fail()?;
            return Err(error).or_fail();
        }
        Response::ok(Model {
            has_key: self.app.gateway.has_model_key(&spec.id).or_fail()?,
            ..model(&spec, positive(r.monthly_yen))
        })
    }

    async fn find_models(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Models> {
        self.app.find_models().await.or_fail()?;
        Response::ok(self.models().or_fail()?)
    }

    async fn list_server_models(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListServerModelsRequest>,
    ) -> ServiceResult<ListServerModelsResponse> {
        let models = famulen_core::models::server_models(&self.app.http, request.endpoint)
            .await
            .or_fail()?;
        Response::ok(ListServerModelsResponse {
            models,
            ..Default::default()
        })
    }

    async fn restore_model(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Models> {
        self.app.models.bring_back(request.id).or_fail()?;
        Response::ok(self.models().or_fail()?)
    }

    async fn unregister_model(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Empty> {
        self.app.models.unregister(request.id).or_fail()?;
        Response::ok(Empty::default())
    }

    async fn set_model_key(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetModelKeyRequest>,
    ) -> ServiceResult<Model> {
        let r = request.to_owned_message();
        let registered = self
            .app
            .models
            .model(&r.id)
            .or_fail()?
            .ok_or_else(|| fail("no such model"))?;
        let key = given(&r.api_key).ok_or_else(|| fail("give the new key"))?;
        self.app
            .gateway
            .put_model_key(OWNER, &registered.spec.id, key)
            .or_fail()?;
        Response::ok(Model {
            has_key: self
                .app
                .gateway
                .has_model_key(&registered.spec.id)
                .or_fail()?,
            ..model(&registered.spec, registered.monthly_yen)
        })
    }

    async fn get_conversation_usage(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetUsageRequest>,
    ) -> ServiceResult<ConversationUsage> {
        let tz = self.app.tz();
        let (span, day) = asked_period(&request.to_owned_message(), Utc::now().with_timezone(&tz))?;
        let (from, to) = span
            .period(day)
            .and_then(|period| period.seconds(tz))
            .ok_or_else(|| fail(format!("no usage can be shown around {day}")))?;
        let all = &self.app.conversations;
        let used = all.spent_between(from, to).or_fail()?;
        let total = used.iter().map(|(_, spent)| *spent).sum();
        let conversations = used
            .into_iter()
            .take(NAMED)
            .filter_map(|(id, spent)| {
                let c = all.get(id).transpose()?;
                Some(c.and_then(|c| self.conversation(c, "", spent)))
            })
            .collect::<anyhow::Result<_>>()
            .or_fail()?;
        Response::ok(ConversationUsage {
            conversations,
            total: crate::console::spent(total).into(),
            ..Default::default()
        })
    }

    async fn get_usage(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetUsageRequest>,
    ) -> ServiceResult<Usage> {
        let tz = self.app.tz();
        let now = Utc::now();
        let (span, day) = asked_period(&request.to_owned_message(), now.with_timezone(&tz))?;
        let report = self.app.models.usage(span, day, tz, now).or_fail()?;
        Response::ok(Usage {
            from: report.period.from.to_string(),
            to: report.period.to.to_string(),
            today: report.today.to_string(),
            rows: report.rows.into_iter().filter_map(usage_row).collect(),
            buckets: report
                .buckets
                .into_iter()
                .map(|b| UsageBucket {
                    from: b.period.from.to_string(),
                    to: b.period.to.to_string(),
                    rows: b.rows.into_iter().filter_map(usage_row).collect(),
                    ..Default::default()
                })
                .collect(),
            models: report.models,
            metered_yen: report.metered_yen,
            fixed_monthly_yen: report.fixed_monthly_yen,
            calls_today: report.calls_today,
            ..Default::default()
        })
    }
}

/// How many conversations the usage of a period names.
const NAMED: usize = 50;

/// The span and the day a caller asks the usage of.
fn asked_period(
    r: &GetUsageRequest,
    now: chrono::DateTime<chrono_tz::Tz>,
) -> Result<(CoreSpan, chrono::NaiveDate), connectrpc::ConnectError> {
    let span = match r.span.as_known() {
        Some(pb::UsageSpan::USAGE_SPAN_WEEK) => CoreSpan::Week,
        Some(pb::UsageSpan::USAGE_SPAN_YEAR) => CoreSpan::Year,
        _ => CoreSpan::Month,
    };
    let day = match given(&r.day) {
        None => now.date_naive(),
        Some(day) => chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .map_err(|_| fail(format!("day is YYYY-MM-DD: {day}")))?,
    };
    Ok((span, day))
}

fn usage_row(row: famulen_core::models::UsageRow) -> Option<UsageRow> {
    let (role, _, _) = ROLES.iter().find(|(_, r, _)| *r == row.role)?;
    Some(UsageRow {
        role: (*role).into(),
        model: row.model,
        calls: row.calls,
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        yen: row.cost_yen,
        metered: row.metered,
        ..Default::default()
    })
}
