//! Slack: the channel between the owner and Famulen. The gateway receives
//! Slack's events and hands them over untouched; reading them and posting
//! through the Slack connection's tools lives here.

pub mod attention;
pub mod clients;
pub mod events;
#[cfg(test)]
pub mod fake;
pub mod icons;
pub mod mrkdwn;
pub mod presence;
pub mod reply;
pub mod web;
