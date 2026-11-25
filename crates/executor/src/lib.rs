//! The executor: the parent of one agent's process. It is a process of its
//! own, one for each piece of work, so that Famulen may stop and start again
//! while the agent works on. It knows nothing of agents: it starts a program,
//! keeps every line the program writes and is written, in order and numbered,
//! and passes them on to whoever holds its key.
//!
//! It outlives the Famulen that started it, so a Famulen of another version
//! may be the one that asks. Both say which protocols they speak. Whatever
//! they share, work can always be stopped, and what was kept can be read once
//! it is over.

mod client;
mod journal;
mod process;
mod proof;
mod server;
mod socket;
mod wire;

pub use client::*;
pub use server::*;
pub use wire::*;
