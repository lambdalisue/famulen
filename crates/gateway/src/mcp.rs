//! A client for MCP servers: stdio and streamable HTTP, with OAuth.

mod client;
mod config;
mod error;
#[cfg(test)]
pub(crate) mod fake;
mod headers;
mod http;
mod jsonrpc;
mod oauth;
mod sse;
mod stdio;
#[cfg(test)]
mod tests;

pub use client::*;
pub use config::*;
pub use error::*;
pub use oauth::*;
