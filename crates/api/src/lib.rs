//! Famulen's API: everything it does, served over Connect to the console and
//! given to the secretary as its own tools.

// The generated service traits return `impl Encodable<T>` so an implementation
// may stream a borrowed body; every implementation here returns the owned
// message itself, the form connectrpc documents for that case.
#![allow(refining_impl_trait)]

mod calendar;
mod channels;
mod connections;
mod console;
mod definitions;
mod logs;
mod lookout;
mod memory;
mod models;
mod notebook;
mod paging;
mod permissions;
mod records;
mod routines;
mod settings;
pub mod tools;
mod vault;
mod workgroups;

use std::sync::Arc;

use connectrpc::{ConnectError, RequestContext};
use famulen_core::app::App;
use famulen_core::gateway::{Outcome, held_message};
use famulen_core::notebook::Writer;

pub mod proto {
    connectrpc::include_generated!();
}

pub use proto::famulen::v1 as pb;

/// The API's descriptor set, comments included.
pub const DESCRIPTOR_SET: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/famulen.binpb"));

/// The header a call carries to say who makes it: the secretary through its
/// tools, or the lookout or a worker. The console sends none.
pub const CALLER_HEADER: &str = "famulen-caller";
pub const SECRETARY: &str = "secretary";
pub const LOOKOUT: &str = "lookout";
pub const WORKER: &str = "worker";

/// The owner, whom no caller header names.
pub const OWNER: &str = "owner";

/// Every service of the API, over one [`App`].
#[derive(Clone)]
pub struct Api {
    pub(crate) app: App,
}

impl Api {
    pub fn new(app: App) -> Self {
        Self { app }
    }

    /// The Famulen this API serves.
    pub fn app(&self) -> &App {
        &self.app
    }

    /// The Connect router with every service.
    pub fn router(self) -> connectrpc::Router {
        use pb::*;
        let api = Arc::new(self);
        let router = connectrpc::Router::new();
        let router = RecordServiceExt::register(api.clone(), router);
        let router = ConsoleServiceExt::register(api.clone(), router);
        let router = CalendarServiceExt::register(api.clone(), router);
        let router = WorkgroupServiceExt::register(api.clone(), router);
        let router = SettingsServiceExt::register(api.clone(), router);
        let router = ModelServiceExt::register(api.clone(), router);
        let router = RoutineServiceExt::register(api.clone(), router);
        let router = LogServiceExt::register(api.clone(), router);
        let router = ConnectionServiceExt::register(api.clone(), router);
        let router = ChannelServiceExt::register(api.clone(), router);
        let router = LookoutServiceExt::register(api.clone(), router);
        let router = NotebookServiceExt::register(api.clone(), router);
        let router = VaultServiceExt::register(api.clone(), router);
        let router = GatewayLogServiceExt::register(api.clone(), router);
        MemoryServiceExt::register(api.clone(), router)
    }

    /// The router as an axum app, with a plain liveness probe.
    pub fn axum(self) -> axum::Router {
        axum::Router::new()
            .route("/health", axum::routing::get(|| async { "OK" }))
            .fallback_service(self.router().into_axum_service())
    }
}

/// Who makes a call: whoever names themselves in the caller header, and the
/// owner on the console, which names no one.
pub(crate) fn actor(ctx: &RequestContext) -> &'static str {
    match ctx.header(CALLER_HEADER).and_then(|v| v.to_str().ok()) {
        Some(SECRETARY) => SECRETARY,
        Some(LOOKOUT) => LOOKOUT,
        Some(WORKER) => WORKER,
        _ => OWNER,
    }
}

/// Whether the secretary made this call through its tools.
pub(crate) fn by_secretary(ctx: &RequestContext) -> bool {
    actor(ctx) == SECRETARY
}

/// Who a call writes the notebook as.
pub(crate) fn writer(ctx: &RequestContext) -> Writer {
    match actor(ctx) {
        SECRETARY => Writer::Secretary,
        LOOKOUT => Writer::Lookout,
        WORKER => Writer::Worker,
        _ => Writer::Owner,
    }
}

/// The workgroups a new record goes in: the ones given, or, when the secretary
/// gives none, the workgroup its running turn is in.
pub(crate) fn workgroups_for(api: &Api, ctx: &RequestContext, given: Vec<String>) -> Vec<String> {
    if !given.is_empty() || !by_secretary(ctx) {
        return given;
    }
    match api.app.workgroups.working() {
        Ok(working) if working != famulen_core::workgroups::ALL => vec![working],
        _ => Vec::new(),
    }
}

/// What a caller gave for a field; nothing when it is empty.
pub(crate) fn given(text: &str) -> Option<&str> {
    Some(text.trim()).filter(|t| !t.is_empty())
}

/// A call the gateway may hold for the owner's answer. A held one comes back
/// as the message its caller reads; the call itself waits in the gateway.
pub(crate) fn unless_held<T>(outcome: Outcome<T>) -> Result<T, ConnectError> {
    match outcome {
        Outcome::Done(done) => Ok(done),
        Outcome::Held(ticket) => Err(fail(held_message(ticket.id))),
    }
}

/// An error from Famulen, as the caller reads it.
pub(crate) fn fail(error: impl std::fmt::Display) -> ConnectError {
    ConnectError::invalid_argument(error.to_string())
}

pub(crate) trait OrFail<T> {
    fn or_fail(self) -> Result<T, ConnectError>;
}

impl<T> OrFail<T> for anyhow::Result<T> {
    fn or_fail(self) -> Result<T, ConnectError> {
        self.map_err(|e| fail(format!("{e:#}")))
    }
}

/// Records changed: screens showing them may read them again.
pub(crate) fn changed(api: &Api) {
    api.app
        .console
        .notify(famulen_core::console_log::Event::Records);
}
