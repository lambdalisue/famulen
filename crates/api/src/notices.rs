//! The devices told of what waits for the owner, and how much they are told.

use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::gateway::Subscription;
use famulen_core::notices::Detail;

use crate::pb::*;
use crate::{Api, OrFail, fail};

impl NoticeService for Api {
    async fn get_notices(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetNoticesRequest>,
    ) -> ServiceResult<Notices> {
        let notices = &self.app.notices;
        let key = match self.app.gateway.vault() {
            Some(_) => notices.key().or_fail()?,
            None => String::new(),
        };
        let detail = match Detail::load(&self.app.store).or_fail()? {
            Detail::Number => NoticeDetail::Number,
            Detail::Deed => NoticeDetail::Deed,
            Detail::Values => NoticeDetail::Values,
        };
        let devices = notices
            .devices()
            .or_fail()?
            .into_iter()
            .map(|d| NoticeDevice {
                id: d.id,
                here: d.endpoint == request.endpoint,
                name: d.name,
                added_at: d.added_at,
                failed: d.failed,
                ..Default::default()
            })
            .collect();
        Response::ok(Notices {
            key,
            detail: detail.into(),
            devices,
            ..Default::default()
        })
    }

    async fn add_notice_device(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, AddNoticeDeviceRequest>,
    ) -> ServiceResult<Empty> {
        let r = request.to_owned_message();
        let to = Subscription {
            endpoint: r.endpoint,
            p256dh: r.p256dh,
            auth: r.auth,
        };
        self.app.notices.add(&to, &r.name).or_fail()?;
        self.app
            .journal
            .record("setting", format!("notices go to {} too", r.name));
        Response::ok(Empty::default())
    }

    async fn remove_notice_device(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, NoticeDeviceRequest>,
    ) -> ServiceResult<Empty> {
        self.app.notices.remove(request.id).or_fail()?;
        self.app
            .journal
            .record("setting", "notices go to one device less");
        Response::ok(Empty::default())
    }

    async fn try_notice_device(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, NoticeDeviceRequest>,
    ) -> ServiceResult<Empty> {
        let id = request.id;
        self.app.notices.try_device(id).await.or_fail()?;
        Response::ok(Empty::default())
    }

    async fn set_notice_detail(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetNoticeDetailRequest>,
    ) -> ServiceResult<Empty> {
        let detail = match request.to_owned_message().detail.as_known() {
            Some(NoticeDetail::Number) => Detail::Number,
            Some(NoticeDetail::Deed) => Detail::Deed,
            Some(NoticeDetail::Values) => Detail::Values,
            Some(NoticeDetail::Unspecified) | None => {
                return Err(fail("say number, deed or values"));
            }
        };
        detail.save(&self.app.store).or_fail()?;
        self.app
            .journal
            .record("setting", "how much notices tell changed");
        Response::ok(Empty::default())
    }
}
