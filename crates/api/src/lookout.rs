//! The lookout: what it reads and what it decided.

use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::connections::Transport;
use famulen_core::lookout::{ARRIVED, Feed, Outcome, Route, arrived_in};
use famulen_core::mail_line::{Line, past};
use famulen_core::models::Role as CoreRole;
use famulen_core::workgroups::MemberKind;

use crate::connections::is_slack;
use crate::pb::*;
use crate::{Api, OrFail, changed, fail, given};

/// Lookouts that can be added with one press on a connection that has the
/// tool: the tool, its name suffix, what it reads, and its arguments.
const TOOL_PRESETS: &[(&str, &str, &str, &str)] = &[
    (
        "clickup_search",
        "で動いた自分のタスク",
        "あなたの担当で最近動いたもの",
        r#"{"filters":{"assignees":["me"]},"sort":[{"field":"updated_at","direction":"desc"}],"count":20}"#,
    ),
    (
        "list_notifications",
        "の通知",
        "あなたへのメンション、レビューの依頼、担当の Issue など",
        "{}",
    ),
    (
        "list-recently-edited-notes",
        "で直されたページ",
        "最近直されたページ",
        "{}",
    ),
    (
        "list_recent_files",
        "で動いたファイル",
        "最近変わったファイル",
        "{}",
    ),
];

fn outcome_kind(outcome: Outcome) -> LookoutOutcome {
    match outcome {
        Outcome::Unrelated => LookoutOutcome::LOOKOUT_OUTCOME_UNRELATED,
        Outcome::Logged => LookoutOutcome::LOOKOUT_OUTCOME_LOGGED,
        Outcome::Digest => LookoutOutcome::LOOKOUT_OUTCOME_DIGEST,
        Outcome::Notify => LookoutOutcome::LOOKOUT_OUTCOME_NOTIFY,
        Outcome::Act => LookoutOutcome::LOOKOUT_OUTCOME_ACT,
    }
}

pub(crate) fn decision(d: famulen_core::lookout::Decision) -> LookoutDecision {
    LookoutDecision {
        id: d.id,
        source: d.source,
        at: d.at,
        title: d.title,
        passed_on: matches!(d.outcome, Outcome::Notify | Outcome::Act),
        outcome: d.outcome.label().into(),
        outcome_kind: outcome_kind(d.outcome).into(),
        reason: d.reason,
        feedback: match d.feedback.as_deref() {
            Some("wanted") => 1,
            Some(_) => -1,
            None => 0,
        },
        ..Default::default()
    }
}

fn feed_label(feed: Feed) -> &'static str {
    match feed {
        Feed::Mail => "メール",
        Feed::Chat => "チャット",
        Feed::Other => "ほか",
    }
}

fn lookout_default((feed, every): (Feed, i64)) -> LookoutDefault {
    LookoutDefault {
        feed: feed.as_str().into(),
        label: feed_label(feed).into(),
        interval_minutes: (every / 60) as i32,
        ..Default::default()
    }
}

fn place(p: famulen_core::lookout::Place) -> LookoutPlace {
    LookoutPlace {
        id: p.id,
        name: p.name,
        ..Default::default()
    }
}

fn places_from(places: Vec<LookoutPlace>) -> Vec<famulen_core::lookout::Place> {
    places
        .into_iter()
        .map(|p| famulen_core::lookout::Place {
            id: p.id,
            name: p.name,
        })
        .collect()
}

fn route_of(route: Option<Route>) -> LookoutRoute {
    match route {
        None => LookoutRoute::LOOKOUT_ROUTE_UNSPECIFIED,
        Some(Route::Secretary) => LookoutRoute::LOOKOUT_ROUTE_SECRETARY,
        Some(Route::Digest) => LookoutRoute::LOOKOUT_ROUTE_DIGEST,
        Some(Route::Log) => LookoutRoute::LOOKOUT_ROUTE_LOG,
    }
}

fn route_from(route: LookoutRoute) -> Option<Route> {
    match route {
        LookoutRoute::LOOKOUT_ROUTE_UNSPECIFIED => None,
        LookoutRoute::LOOKOUT_ROUTE_SECRETARY => Some(Route::Secretary),
        LookoutRoute::LOOKOUT_ROUTE_DIGEST => Some(Route::Digest),
        LookoutRoute::LOOKOUT_ROUTE_LOG => Some(Route::Log),
    }
}

fn source(s: famulen_core::lookout::LookoutSource, workgroups: Vec<String>) -> LookoutSource {
    LookoutSource {
        places: s.places().into_iter().map(place).collect(),
        route: route_of(s.route).into(),
        workgroups,
        id: s.id,
        name: s.name,
        connection: s.connection,
        tool: s.tool,
        args: s.args.to_string(),
        interval_minutes: (s.interval / 60) as i32,
        enabled: s.enabled,
        own_interval: s.own_interval.is_some(),
        feed: s.feed.as_str().into(),
        ..Default::default()
    }
}

fn args_from(text: &str) -> Result<serde_json::Value, ConnectError> {
    match given(text) {
        None => Ok(serde_json::json!({})),
        Some(text) => serde_json::from_str(text).map_err(fail),
    }
}

impl Api {
    fn ensure_reads(&self, connection: &str, tool: &str) -> Result<(), ConnectError> {
        if self
            .app
            .permissions
            .only_reads(connection, tool)
            .or_fail()?
        {
            return Ok(());
        }
        Err(fail(format!(
            "the lookout calls only tools that say they only read; {tool} does not"
        )))
    }

    /// Whether anything arrives on a connection.
    fn receives(&self, c: &famulen_core::connections::Connection) -> bool {
        match &c.transport {
            Transport::Defined { definition, .. } => self
                .app
                .gateway
                .definitions()
                .get(definition)
                .is_some_and(|d| !d.receive.is_empty()),
            _ => false,
        }
    }

    async fn lookout_choices(&self, id: &str) -> anyhow::Result<LookoutChoices> {
        let c = self
            .app
            .connections
            .get(id)?
            .ok_or_else(|| anyhow::anyhow!("no connection {id}"))?;
        let (places, tools) =
            tokio::join!(self.app.lookout_places(id), self.app.gateway.tools_of(id));
        let problem = [places.as_ref().err(), tools.as_ref().err()]
            .into_iter()
            .flatten()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        let schemas = tools.unwrap_or_default();
        let mut readers = Vec::new();
        for t in self.app.connections.tools(id)? {
            if !self.app.permissions.only_reads(id, &t.name)? {
                continue;
            }
            let input_schema = schemas
                .iter()
                .find(|s| s["name"] == t.name.as_str())
                .map(|s| s["inputSchema"].to_string())
                .unwrap_or_default();
            readers.push(LookoutTool {
                name: t.name,
                description: t.description,
                input_schema,
                ..Default::default()
            });
        }
        Ok(LookoutChoices {
            receives: self.receives(&c),
            place_label: match self.app.feed_of(id)? {
                Feed::Mail => "ラベル",
                Feed::Chat => "チャンネル",
                Feed::Other => "",
            }
            .into(),
            places: places.unwrap_or_default().into_iter().map(place).collect(),
            tools: readers,
            problem,
            ..Default::default()
        })
    }

    fn need_source(&self, id: i64) -> anyhow::Result<famulen_core::lookout::LookoutSource> {
        self.app
            .lookout
            .sources()?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| anyhow::anyhow!("no lookout {id}"))
    }

    /// A source after a change to it, which screens showing it may read again.
    fn changed_source(&self, id: i64) -> anyhow::Result<LookoutSource> {
        changed(self);
        self.lookout_source(self.need_source(id)?)
    }

    fn lookout_source(
        &self,
        s: famulen_core::lookout::LookoutSource,
    ) -> anyhow::Result<LookoutSource> {
        let workgroups = self
            .app
            .workgroups
            .workgroups_of(MemberKind::Lookout, s.id.to_string())?;
        Ok(source(s, workgroups))
    }

    fn presets(
        &self,
        sources: &[famulen_core::lookout::LookoutSource],
    ) -> anyhow::Result<Vec<LookoutPreset>> {
        let mut presets = Vec::new();
        for w in self.app.workspaces()? {
            presets.push(LookoutPreset {
                name: format!("Slack · {}", w.name),
                what: "bot がいるチャンネルの発言、あなたへのメンション、ほかの人からの DM".into(),
                connection: w.id,
                tool: ARRIVED.into(),
                args: "{}".into(),
                ..Default::default()
            });
        }
        for c in self.app.connections.list()? {
            if famulen_core::arrivals::is_google(&c) {
                presets.push(LookoutPreset {
                    name: format!("Gmail · {}", c.account),
                    what: "受信箱の新着".into(),
                    connection: c.id.clone(),
                    tool: ARRIVED.into(),
                    args: "{}".into(),
                    ..Default::default()
                });
                continue;
            }
            if self.receives(&c) && !is_slack(&c) {
                presets.push(LookoutPreset {
                    name: format!("{} · 届いたもの", c.name),
                    what: "受け口に届いたもの".into(),
                    connection: c.id.clone(),
                    tool: ARRIVED.into(),
                    args: "{}".into(),
                    ..Default::default()
                });
            }
        }
        for c in self.app.connections.list()? {
            let tools = self.app.connections.tools(&c.id)?;
            for (tool, suffix, what, args) in TOOL_PRESETS {
                if tools.iter().any(|t| t.name == *tool) {
                    presets.push(LookoutPreset {
                        name: format!("{}{suffix}", c.name),
                        what: (*what).into(),
                        connection: c.id.clone(),
                        tool: (*tool).into(),
                        args: (*args).into(),
                        ..Default::default()
                    });
                }
            }
        }
        presets.retain(|p| {
            !sources
                .iter()
                .any(|s| s.connection == p.connection && s.tool == p.tool)
        });
        Ok(presets)
    }

    fn past_mail(
        &self,
        line: Line,
        c: &famulen_core::connections::Connection,
    ) -> anyhow::Result<PastMail> {
        let name = if c.account.is_empty() {
            c.name.clone()
        } else {
            format!("{} · {}", c.name, c.account)
        };
        let Some(p) = past::of(&self.app.store, line, &c.id)? else {
            return Ok(PastMail {
                connection: c.id.clone(),
                name,
                ..Default::default()
            });
        };
        Ok(PastMail {
            connection: c.id.clone(),
            name,
            state: match p.standing {
                past::Standing::Going => PastMailState::PAST_MAIL_STATE_GOING,
                past::Standing::Stopped => PastMailState::PAST_MAIL_STATE_STOPPED,
                past::Standing::Done => PastMailState::PAST_MAIL_STATE_DONE,
            }
            .into(),
            since: p.since.to_string(),
            month: p.month.to_string(),
            found: p.found,
            left: p.left,
            added: p.added,
            problem: p.problem,
            searched: p.searched,
            ..Default::default()
        })
    }

    fn past_mailboxes(&self, line: Line) -> anyhow::Result<PastMailList> {
        let list = self.app.connections.list()?;
        let mailboxes = list
            .iter()
            .filter(|c| famulen_core::arrivals::is_google(c))
            .map(|c| self.past_mail(line, c))
            .collect::<anyhow::Result<_>>()?;
        Ok(PastMailList {
            mailboxes,
            ..Default::default()
        })
    }

    fn start_past(&self, line: Line, connection: &str, since: &str) -> anyhow::Result<PastMail> {
        let mailbox = self.need_mailbox(connection)?;
        let today = chrono::Utc::now()
            .with_timezone(&self.app.tz())
            .date_naive();
        past::start(&self.app.store, line, &mailbox.id, since.parse()?, today)?;
        changed(self);
        self.past_mail(line, &mailbox)
    }

    fn stop_past(&self, line: Line, connection: &str) -> anyhow::Result<PastMail> {
        let mailbox = self.need_mailbox(connection)?;
        past::stop(&self.app.store, line, &mailbox.id)?;
        changed(self);
        self.past_mail(line, &mailbox)
    }

    fn need_mailbox(&self, id: &str) -> anyhow::Result<famulen_core::connections::Connection> {
        self.app
            .connections
            .get(id)?
            .filter(famulen_core::arrivals::is_google)
            .ok_or_else(|| anyhow::anyhow!("no mailbox {id}"))
    }

    fn lookout_overview(&self) -> anyhow::Result<LookoutOverview> {
        let sources = self.app.lookout.sources()?;
        let presets = self.presets(&sources)?;
        let since = chrono::Utc::now().timestamp() - 86_400;
        let funnel = self.app.lookout.funnel(since)?;
        let tz = self.app.tz();
        let now = chrono::Utc::now();
        let model_calls_today = self
            .app
            .models
            .used_today(CoreRole::Lookout, tz, now)
            .map(|(_, c)| c)
            .unwrap_or(0);
        Ok(LookoutOverview {
            sources: sources
                .into_iter()
                .map(|s| self.lookout_source(s))
                .collect::<anyhow::Result<_>>()?,
            presets,
            seen: funnel[0],
            passed: funnel[2],
            model_calls_today,
            profile: famulen_core::runtime::relevance_profile(&self.app),
            defaults: self
                .app
                .lookout
                .defaults()?
                .into_iter()
                .map(lookout_default)
                .collect(),
            ..Default::default()
        })
    }
}

impl LookoutService for Api {
    async fn get_lookout(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<LookoutOverview> {
        Response::ok(self.lookout_overview().or_fail()?)
    }

    async fn add_lookout(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, AddLookoutRequest>,
    ) -> ServiceResult<LookoutSource> {
        let r = request.to_owned_message();
        let tool = given(&r.tool).unwrap_or(ARRIVED);
        let args = if tool == ARRIVED {
            arrived_in(&places_from(r.places))
        } else {
            self.ensure_reads(&r.connection, tool)?;
            args_from(&r.args)?
        };
        let every = (r.interval_minutes > 0).then(|| i64::from(r.interval_minutes) * 60);
        let feed = self.app.feed_of(&r.connection).or_fail()?;
        let id = self
            .app
            .lookout
            .add_source(&r.name, &r.connection, tool, &args, feed, every)
            .or_fail()?;
        Response::ok(self.changed_source(id).or_fail()?)
    }

    async fn get_lookout_choices(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<LookoutChoices> {
        let id = request.id.to_owned();
        Response::ok(self.lookout_choices(&id).await.or_fail()?)
    }

    async fn set_lookout_reading(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetLookoutReadingRequest>,
    ) -> ServiceResult<LookoutSource> {
        let r = request.to_owned_message();
        let args = args_from(&r.args)?;
        self.ensure_reads(&self.need_source(r.id).or_fail()?.connection, &r.tool)?;
        self.app
            .lookout
            .set_reading(r.id, &r.tool, &args)
            .or_fail()?;
        Response::ok(self.changed_source(r.id).or_fail()?)
    }

    async fn set_lookout_places(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetLookoutPlacesRequest>,
    ) -> ServiceResult<LookoutSource> {
        let r = request.to_owned_message();
        self.app
            .lookout
            .set_places(r.id, &places_from(r.places))
            .or_fail()?;
        Response::ok(self.changed_source(r.id).or_fail()?)
    }

    async fn update_lookout(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, UpdateLookoutRequest>,
    ) -> ServiceResult<LookoutSource> {
        let r = request.to_owned_message();
        self.need_source(r.id).or_fail()?;
        if let Some(name) = &r.name {
            self.app.lookout.rename(r.id, name).or_fail()?;
        }
        if let Some(route) = r.route {
            let route = route.as_known().ok_or_else(|| fail("no such route"))?;
            self.app
                .lookout
                .set_route(r.id, route_from(route))
                .or_fail()?;
        }
        if let Some(workgroup) = &r.workgroup {
            let id = r.id.to_string();
            let workgroups = &self.app.workgroups;
            if !workgroup.is_empty() {
                workgroups
                    .set_member(workgroup, MemberKind::Lookout, &id, true)
                    .or_fail()?;
            }
            for other in workgroups
                .workgroups_of(MemberKind::Lookout, &id)
                .or_fail()?
            {
                if other != *workgroup {
                    workgroups
                        .set_member(&other, MemberKind::Lookout, &id, false)
                        .or_fail()?;
                }
            }
        }
        Response::ok(self.changed_source(r.id).or_fail()?)
    }

    async fn set_lookout_enabled(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetLookoutEnabledRequest>,
    ) -> ServiceResult<LookoutSource> {
        let (id, on) = (request.id, request.enabled);
        self.app.lookout.set_enabled(id, on).or_fail()?;
        Response::ok(self.changed_source(id).or_fail()?)
    }

    async fn set_lookout_default(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetLookoutDefaultRequest>,
    ) -> ServiceResult<LookoutDefault> {
        let feed = Feed::ALL
            .into_iter()
            .find(|f| f.as_str() == request.feed)
            .ok_or_else(|| fail(format!("no feed {}", request.feed)))?;
        let every = i64::from(request.interval_minutes) * 60;
        self.app.lookout.set_default(feed, every).or_fail()?;
        changed(self);
        Response::ok(lookout_default((feed, every)))
    }

    async fn set_lookout_interval(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetLookoutIntervalRequest>,
    ) -> ServiceResult<LookoutSource> {
        let (id, minutes) = (request.id, request.interval_minutes);
        self.app
            .lookout
            .set_interval(id, (minutes > 0).then(|| i64::from(minutes) * 60))
            .or_fail()?;
        Response::ok(self.changed_source(id).or_fail()?)
    }

    async fn remove_lookout(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ById>,
    ) -> ServiceResult<Empty> {
        self.app.lookout.remove_source(request.id).or_fail()?;
        changed(self);
        Response::ok(Empty::default())
    }

    async fn list_past_purchases(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<PastMailList> {
        Response::ok(self.past_mailboxes(Line::Purchases).or_fail()?)
    }

    async fn start_past_purchases(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, StartPastMailRequest>,
    ) -> ServiceResult<PastMail> {
        let started = self.start_past(Line::Purchases, request.connection, request.since);
        Response::ok(started.or_fail()?)
    }

    async fn stop_past_purchases(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<PastMail> {
        Response::ok(self.stop_past(Line::Purchases, request.id).or_fail()?)
    }

    async fn list_past_contracts(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<PastMailList> {
        Response::ok(self.past_mailboxes(Line::Contracts).or_fail()?)
    }

    async fn start_past_contracts(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, StartPastMailRequest>,
    ) -> ServiceResult<PastMail> {
        let started = self.start_past(Line::Contracts, request.connection, request.since);
        Response::ok(started.or_fail()?)
    }

    async fn stop_past_contracts(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<PastMail> {
        Response::ok(self.stop_past(Line::Contracts, request.id).or_fail()?)
    }

    async fn give_feedback(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GiveFeedbackRequest>,
    ) -> ServiceResult<Empty> {
        let (id, wanted) = (request.id, request.wanted);
        let _hold = self.app.notebook.hold().or_fail()?;
        // The owner's answer is something for the dream to learn from.
        let note = |decision: &famulen_core::lookout::Decision| {
            let note = format!(
                "見張りの判断「{}」（{}）はあなたにとって{}",
                decision.title,
                decision.outcome.label(),
                if wanted {
                    "拾ってほしかった"
                } else {
                    "要らなかった"
                }
            );
            famulen_core::memory::append_episode(
                &self.app.memory_dir,
                chrono::Utc::now()
                    .with_timezone(&self.app.tz())
                    .naive_local(),
                &note,
                famulen_core::memory::EpisodeState::Waiting,
            )
            .map(drop)
        };
        self.app
            .lookout
            .feedback(id, wanted, note)
            .or_fail()?
            .ok_or_else(|| fail(format!("no decision {id}")))?;
        Response::ok(Empty::default())
    }
}
