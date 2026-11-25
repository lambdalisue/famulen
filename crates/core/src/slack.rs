//! Slack: the channel between the owner and Famulen. The gateway receives
//! Slack's events and hands them over untouched; reading them and posting
//! through the Slack connection's tools lives here.

pub mod app_icon;
pub mod attention;
pub mod clients;
pub mod events;
#[cfg(test)]
pub mod fake;
pub mod looks;
pub mod mrkdwn;
pub mod names;
pub mod presence;
pub mod reply;
pub mod web;
