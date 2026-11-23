//! The core of famulen.

pub mod approvals;
pub mod asking;
pub mod calendars;
pub mod config;
pub mod console_log;
pub mod hub;
pub mod input;
pub mod journal;
pub mod lookout;
pub mod notebook;
pub mod permissions;
pub mod records;
pub mod sealed;
pub mod settings;
pub mod slack;
pub mod store;
pub mod triage;
pub mod workgroups;

pub use famulen_gateway as gateway;
pub use famulen_gateway::{catalog, connections, mcp};
