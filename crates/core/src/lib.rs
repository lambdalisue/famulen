//! The core of famulen.

pub mod app;
pub mod approvals;
pub mod arrivals;
pub mod asking;
pub mod assessing;
pub mod calendars;
pub mod channel;
pub mod claude;
pub mod config;
pub mod console_log;
pub mod dream_settings;
pub mod dreaming;
pub mod hub;
pub mod input;
pub mod journal;
pub mod lookout;
pub mod memory;
pub mod models;
pub mod notebook;
pub mod notebook_place;
pub mod permissions;
pub mod records;
pub mod runtime;
pub mod schedule;
pub mod sealed;
pub mod settings;
pub mod slack;
pub mod store;
pub mod told;
pub mod triage;
pub mod workgroups;
pub mod writable;

pub use famulen_gateway as gateway;
pub use famulen_gateway::{catalog, connections, mcp};
