//! What each agent did, and the gateway's log, a page at a time.

use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::gateway::log::LogQuery;
use famulen_core::permissions::split_tool;

use crate::paging::{page, page_limit};
use crate::pb::{self, *};
use crate::{Api, OrFail};

/// The most journal entries read at first to fill a page of the log; more are
/// read when the filter leaves too few.
const LOG_WINDOW: usize = 600;

impl Api {
    /// The service and the name of a tool, as the log shows them.
    fn tool_label(&self, full: &str) -> (String, String) {
        match split_tool(full) {
            Some((famulen_core::hub::SERVER_NAME, tool)) => ("Famulen".to_owned(), tool.to_owned()),
            Some((server, tool)) => (
                self.app
                    .connections
                    .get(server)
                    .ok()
                    .flatten()
                    .map(|c| c.name)
                    .unwrap_or_else(|| server.to_owned()),
                tool.to_owned(),
            ),
            None => ("Claude Code".to_owned(), full.to_owned()),
        }
    }

    /// The log's items, newest first, each with the journal id it begins at,
    /// from journal entries given newest first. The entries of a turn whose
    /// input is not among them are left for the page that holds the input.
    fn log_items(
        &self,
        entries: &[famulen_core::journal::Entry],
    ) -> Vec<(i64, pb::__buffa::oneof::log_item::Item)> {
        use pb::__buffa::oneof::log_item::Item;
        let messages: std::collections::HashMap<i64, String> = self
            .app
            .console
            .recent(1000)
            .unwrap_or_default()
            .into_iter()
            .map(|m| (m.id, m.text))
            .collect();
        let mut items: Vec<(i64, Item)> = Vec::new();
        for e in entries.iter().rev() {
            let open = matches!(items.last(), Some((_, Item::Turn(t))) if t.reply.is_empty());
            match e.kind.as_str() {
                "input" => {
                    let origin = e
                        .detail
                        .strip_prefix('[')
                        .and_then(|r| r.split_once(']'))
                        .map(|(o, _)| match o {
                            "console" => "コンソール".to_owned(),
                            o if o.starts_with("slack") => "Slack".to_owned(),
                            o => o.to_owned(),
                        })
                        .unwrap_or_default();
                    let asked = e
                        .detail
                        .split_once("message: #")
                        .and_then(|(_, r)| r.split_whitespace().next()?.parse::<i64>().ok())
                        .and_then(|id| messages.get(&id).cloned())
                        .unwrap_or_default();
                    items.push((
                        e.id,
                        Item::Turn(Box::new(LogTurn {
                            at: e.created_at,
                            origin,
                            asked,
                            ..Default::default()
                        })),
                    ));
                }
                "tool" if open => {
                    if let Some((_, Item::Turn(t))) = items.last_mut() {
                        let (name, args) = e.detail.split_once(' ').unwrap_or((&e.detail, ""));
                        let (service, tool) = self.tool_label(name);
                        t.steps.push(LogStep {
                            service,
                            tool,
                            args: args.to_owned(),
                            ..Default::default()
                        });
                    }
                }
                "permission" if open => {
                    if let Some((_, Item::Turn(t))) = items.last_mut() {
                        let (name, verdict) = e.detail.rsplit_once(": ").unwrap_or((&e.detail, ""));
                        let (_, tool) = self.tool_label(name);
                        let outcome = match verdict {
                            "allowed" => "",
                            v if v.starts_with("held") => "確認待ちにした",
                            _ => "させなかった",
                        };
                        if let Some(step) = t
                            .steps
                            .iter_mut()
                            .rev()
                            .find(|s| s.tool == tool && s.outcome.is_empty())
                        {
                            step.outcome = outcome.into();
                        } else if !outcome.is_empty() {
                            t.steps.push(LogStep {
                                tool,
                                outcome: outcome.into(),
                                ..Default::default()
                            });
                        }
                    }
                }
                "text" if open => {
                    if let Some((_, Item::Turn(t))) = items.last_mut() {
                        t.reply = e.detail.clone();
                    }
                }
                "turn" | "session" | "permission" | "tool" | "text" => {}
                _ => items.push((
                    e.id,
                    Item::Event(Box::new(LogEvent {
                        at: e.created_at,
                        kind: e.kind.clone(),
                        detail: e.detail.clone(),
                        ..Default::default()
                    })),
                )),
            }
        }
        items.reverse();
        items
    }
}

impl LogService for Api {
    async fn list_log(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListLogRequest>,
    ) -> ServiceResult<ListLogResponse> {
        use pb::__buffa::oneof::log_item::Item;
        let r = request.to_owned_message();
        let limit = page_limit(r.limit, 120);
        let shown = |item: &Item| match (r.show.as_str(), item) {
            ("talk", Item::Turn(_)) | ("other", Item::Event(_)) => true,
            ("talk" | "other", _) => false,
            _ => true,
        };
        let mut window = LOG_WINDOW;
        let items = loop {
            let entries = self.app.journal.before(r.before, window).or_fail()?;
            let items: Vec<_> = self
                .log_items(&entries)
                .into_iter()
                .filter(|(_, item)| shown(item))
                .collect();
            if items.len() > limit || entries.len() < window {
                break items;
            }
            window *= 2;
        };
        let (items, more) = page(items, limit);
        let items = items
            .into_iter()
            .map(|(id, item)| LogItem {
                id,
                item: Some(item),
                ..Default::default()
            })
            .collect();
        Response::ok(ListLogResponse {
            items,
            more,
            ..Default::default()
        })
    }

    async fn list_triage_log(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, LogPageRequest>,
    ) -> ServiceResult<ListTriageLogResponse> {
        let r = request.to_owned_message();
        let limit = page_limit(r.limit, 50);
        let rows = self.app.triage.before(r.before, limit + 1).or_fail()?;
        let (rows, more) = page(rows, limit);
        let decisions = rows
            .into_iter()
            .map(|d| TriageDecision {
                id: d.id,
                at: d.at,
                source: d.source,
                title: d.title,
                addressed: d.addressed,
                reason: d.reason,
                ..Default::default()
            })
            .collect();
        Response::ok(ListTriageLogResponse {
            decisions,
            more,
            ..Default::default()
        })
    }

    async fn list_dream_log(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, LogPageRequest>,
    ) -> ServiceResult<ListDreamLogResponse> {
        let r = request.to_owned_message();
        let limit = page_limit(r.limit, 20);
        let rows = famulen_core::dreaming::dreams_before(&self.app.store, r.before, limit + 1)
            .or_fail()?;
        let (rows, more) = page(rows, limit);
        Response::ok(ListDreamLogResponse {
            dreams: rows.into_iter().map(crate::memory::dream).collect(),
            more,
            ..Default::default()
        })
    }

    async fn list_lookout_log(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, LogPageRequest>,
    ) -> ServiceResult<ListLookoutLogResponse> {
        let r = request.to_owned_message();
        let limit = page_limit(r.limit, 50);
        let rows = self.app.lookout.before(r.before, limit + 1).or_fail()?;
        let (rows, more) = page(rows, limit);
        Response::ok(ListLookoutLogResponse {
            decisions: rows.into_iter().map(crate::lookout::decision).collect(),
            more,
            ..Default::default()
        })
    }
}

impl GatewayLogService for Api {
    async fn list_gateway_log(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListGatewayLogRequest>,
    ) -> ServiceResult<ListGatewayLogResponse> {
        let r = request.to_owned_message();
        let limit = page_limit(r.limit, 50);
        let rows = self
            .app
            .gateway
            .log_page(&LogQuery {
                actor: r.actor,
                connection: r.connection,
                decision: r.decision,
                before: r.before,
                limit: limit + 1,
            })
            .or_fail()?;
        let (rows, more) = page(rows, limit);
        let entries = rows
            .into_iter()
            .map(|(id, e)| GatewayLogEntry {
                id,
                at: e.at,
                actor: e.actor,
                connection: e.connection,
                tool: e.tool,
                decision: e.decision,
                reason: e.reason,
                secret: e.secret,
                outcome: e.outcome,
                digest: e.digest,
                ms: e.ms,
                ..Default::default()
            })
            .collect();
        Response::ok(ListGatewayLogResponse {
            entries,
            more,
            ..Default::default()
        })
    }
}
