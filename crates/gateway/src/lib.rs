//! The gateway: the one door between a person and the outside services.
//! What it promises is written in `SPEC.md` next to this crate.

pub mod catalog;
pub mod connections;
pub mod definitions;
mod lock;
pub mod log;
pub mod mcp;
pub mod openapi;
pub mod permissions;
pub mod presence;
pub mod process;
pub mod store;
pub mod template;


/// One thing read from a service, as the reader that asked for it wants it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Item {
    pub title: String,
    pub body: String,
}
