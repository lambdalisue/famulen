//! The notebook as a whole: searching it, and keeping it to its rules.

use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::workgroups::ALL;
use famulen_notebook::check;
use famulen_notebook::search::Query;

use crate::pb::*;
use crate::{Api, OrFail, changed, given, writer};

impl Api {
    fn findings(&self) -> anyhow::Result<Vec<Finding>> {
        Ok(check::check(&self.app.notebook)?
            .into_iter()
            .map(|f| Finding {
                path: f.path,
                line: f.line as i32,
                rule: f.rule.to_owned(),
                error: f.severity == check::Severity::Error,
                why: f.why,
                ..Default::default()
            })
            .collect())
    }
}

impl NotebookService for Api {
    async fn search_notebook(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SearchNotebookRequest>,
    ) -> ServiceResult<SearchNotebookResponse> {
        let r = request.to_owned_message();
        let mut fields: Vec<(String, String)> = r.fields.into_iter().collect();
        fields.sort();
        let query = Query {
            text: r.text,
            scopes: r.scopes,
            workgroup: given(&r.workgroup).filter(|w| *w != ALL).map(str::to_owned),
            fields,
            from: given(&r.from).map(str::to_owned),
            to: given(&r.to).map(str::to_owned),
            forgotten: r.forgotten,
            limit: r.limit.max(0) as usize,
        };
        let hits = self
            .app
            .index
            .search(&query)
            .or_fail()?
            .into_iter()
            .map(|h| NotebookHit {
                path: h.path,
                scope: h.scope,
                id: h.id,
                title: h.title,
                snippet: h.snippet,
                score: h.score,
                ..Default::default()
            })
            .collect();
        Response::ok(SearchNotebookResponse {
            hits,
            ..Default::default()
        })
    }

    async fn check_notebook(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<CheckNotebookResponse> {
        Response::ok(CheckNotebookResponse {
            findings: self.findings().or_fail()?,
            ..Default::default()
        })
    }

    async fn fix_notebook(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<FixNotebookResponse> {
        let by = writer(&ctx);
        by.may_write().or_fail()?;
        let _hold = self.app.notebook.hold().or_fail()?;
        let changed_files = check::fix(&self.app.notebook, &[]).or_fail()?;
        if !changed_files.is_empty() {
            self.app
                .notebook
                .commit(&by.signature(), "手帳の書き方を直した", &changed_files)
                .or_fail()?;
            changed(self);
        }
        Response::ok(FixNotebookResponse {
            changed: changed_files,
            findings: self.findings().or_fail()?,
            ..Default::default()
        })
    }
}
