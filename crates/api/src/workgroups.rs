//! Workgroups: what each shows and uses.

use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::workgroups::{self as core, MemberKind as Kind, PURPOSES};

use crate::calendar::{mode, mode_from};
use crate::pb::{self, *};
use crate::{Api, OrFail, changed, fail, given, writer};

fn workgroup(c: core::Workgroup) -> Workgroup {
    Workgroup {
        id: c.id,
        name: c.name,
        color: c.color,
        ..Default::default()
    }
}

fn kind_from(k: pb::MemberKind) -> Option<Kind> {
    match k {
        pb::MemberKind::MEMBER_KIND_CHANNEL => Some(Kind::Channel),
        pb::MemberKind::MEMBER_KIND_CONNECTION => Some(Kind::Connection),
        pb::MemberKind::MEMBER_KIND_LOOKOUT => Some(Kind::Lookout),
        _ => None,
    }
}

impl Api {
    /// Every connection that can be chosen or put in a workgroup, as (id, label).
    fn connection_choices(&self) -> anyhow::Result<Vec<(String, String)>> {
        Ok(self
            .app
            .connections
            .list()?
            .into_iter()
            .map(|c| {
                let label = if c.account.is_empty() {
                    c.name
                } else {
                    format!("{} · {}", c.name, c.account)
                };
                (c.id, label)
            })
            .collect())
    }

    fn workgroup_detail(&self, id: &str) -> anyhow::Result<WorkgroupDetail> {
        let c = self
            .app
            .workgroups
            .get(id)?
            .ok_or_else(|| anyhow::anyhow!("no workgroup {id}"))?;
        let calendars = self
            .app
            .calendars
            .list()?
            .into_iter()
            .map(|cal| {
                Ok(CalendarSetting {
                    calendar: cal.id.clone(),
                    mode: mode(cal.mode(&c.id)).into(),
                    name: cal.name,
                    ..Default::default()
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let choices: Vec<Choice> = self
            .connection_choices()?
            .into_iter()
            .map(|(id, label)| Choice {
                id,
                label,
                ..Default::default()
            })
            .collect();
        let accounts = PURPOSES
            .iter()
            .map(|(purpose, label)| {
                Ok(AccountSetting {
                    purpose: (*purpose).to_owned(),
                    label: (*label).to_owned(),
                    connection: self
                        .app
                        .workgroups
                        .account(&c.id, purpose)?
                        .unwrap_or_default(),
                    choices: choices.clone(),
                    ..Default::default()
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut candidates: Vec<(Kind, String, String)> =
            vec![(Kind::Channel, "console".into(), "このブラウザ".into())];
        for w in self.app.workspaces()? {
            candidates.push((
                Kind::Channel,
                format!("slack:{}", w.id),
                format!("Slack · {}", w.name),
            ));
        }
        for (id, label) in self.connection_choices()? {
            candidates.push((Kind::Connection, id, label));
        }
        for s in self.app.lookout.sources()? {
            candidates.push((
                Kind::Lookout,
                s.id.to_string(),
                format!("見張り · {}", s.name),
            ));
        }
        let members = candidates
            .into_iter()
            .map(|(kind, id, label)| {
                Ok(Member {
                    kind: match kind {
                        Kind::Channel => pb::MemberKind::MEMBER_KIND_CHANNEL,
                        Kind::Connection => pb::MemberKind::MEMBER_KIND_CONNECTION,
                        Kind::Lookout => pb::MemberKind::MEMBER_KIND_LOOKOUT,
                    }
                    .into(),
                    member: self.app.workgroups.has_member(&c.id, kind, &id)?,
                    id,
                    label,
                    ..Default::default()
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(WorkgroupDetail {
            workgroup: workgroup(c).into(),
            calendars,
            accounts,
            members,
            ..Default::default()
        })
    }
}

impl WorkgroupService for Api {
    async fn list_workgroups(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListWorkgroupsResponse> {
        Response::ok(ListWorkgroupsResponse {
            workgroups: self
                .app
                .workgroups
                .list()
                .or_fail()?
                .into_iter()
                .map(workgroup)
                .collect(),
            ..Default::default()
        })
    }

    async fn get_workgroup(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<WorkgroupDetail> {
        Response::ok(self.workgroup_detail(request.id).or_fail()?)
    }

    async fn add_workgroup(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddWorkgroupRequest>,
    ) -> ServiceResult<Workgroup> {
        let color = if request.color.is_empty() {
            core::COLORS[0].1
        } else {
            request.color
        };
        let c = self
            .app
            .workgroups
            .add(writer(&ctx), request.name, color)
            .or_fail()?;
        changed(self);
        Response::ok(workgroup(c))
    }

    async fn update_workgroup(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateWorkgroupRequest>,
    ) -> ServiceResult<Workgroup> {
        let r = request.to_owned_message();
        self.app
            .workgroups
            .update(writer(&ctx), &r.id, r.name.as_deref(), r.color.as_deref())
            .or_fail()?;
        let c = self
            .app
            .workgroups
            .get(&r.id)
            .or_fail()?
            .ok_or_else(|| fail(format!("no workgroup {}", r.id)))?;
        changed(self);
        Response::ok(workgroup(c))
    }

    async fn delete_workgroup(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Empty> {
        let id = request.id.to_owned();
        if id == core::ALL {
            return Err(fail("すべて cannot be removed"));
        }
        let by = writer(&ctx);
        self.app.workgroups.remove(by, &id).or_fail()?;
        self.app.calendars.forget_workgroup(by, &id).or_fail()?;
        self.app.records.forget_workgroup(by, &id).or_fail()?;
        changed(self);
        Response::ok(Empty::default())
    }

    async fn set_member(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetMemberRequest>,
    ) -> ServiceResult<Empty> {
        let r = request.to_owned_message();
        let kind = r
            .kind
            .as_known()
            .and_then(kind_from)
            .ok_or_else(|| fail("say what kind of member"))?;
        self.app
            .workgroups
            .set_member(&r.workgroup, kind, &r.member, r.on)
            .or_fail()?;
        changed(self);
        self.app.changed.notify_one();
        Response::ok(Empty::default())
    }

    async fn set_calendar_mode(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetCalendarModeRequest>,
    ) -> ServiceResult<Empty> {
        let r = request.to_owned_message();
        let m = r
            .mode
            .as_known()
            .and_then(mode_from)
            .ok_or_else(|| fail("say show, faint or hide"))?;
        self.app
            .calendars
            .set_mode(writer(&ctx), &r.calendar, &r.workgroup, m)
            .or_fail()?;
        changed(self);
        Response::ok(Empty::default())
    }

    async fn set_account(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetAccountRequest>,
    ) -> ServiceResult<Empty> {
        let r = request.to_owned_message();
        self.app
            .workgroups
            .set_account(&r.workgroup, &r.purpose, given(&r.connection))
            .or_fail()?;
        changed(self);
        Response::ok(Empty::default())
    }
}
