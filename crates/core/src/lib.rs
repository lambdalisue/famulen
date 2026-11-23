//! The core of famulen.

pub mod asking;
pub mod console_log;
pub mod hub;
pub mod input;
pub mod journal;
pub mod notebook;
pub mod records;
pub mod store;
pub mod workgroups;

pub use famulen_gateway as gateway;
pub use famulen_gateway::{catalog, connections, mcp};
