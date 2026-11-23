//! A Slack that records what famulen does with it, for tests.

use std::sync::Mutex;

use super::web::{Outgoing, Slack};

#[derive(Default)]
pub struct FakeSlack {
    pub posts: Mutex<Vec<(String, Outgoing)>>,
    pub updates: Mutex<Vec<(String, String, Outgoing)>>,
    pub reactions: Mutex<Vec<(String, String, String)>>,
    pub statuses: Mutex<Vec<(String, String, String)>>,
    pub fail: bool,
    /// Slack shows no status here.
    pub no_status: bool,
}

impl FakeSlack {
    pub fn posts(&self) -> Vec<(String, Outgoing)> {
        self.posts.lock().unwrap().clone()
    }

    pub fn statuses(&self) -> Vec<(String, String, String)> {
        self.statuses.lock().unwrap().clone()
    }

    pub fn updates(&self) -> Vec<(String, String, Outgoing)> {
        self.updates.lock().unwrap().clone()
    }
}

impl Slack for &FakeSlack {
    async fn post(&self, channel: &str, message: Outgoing) -> anyhow::Result<String> {
        if self.fail {
            anyhow::bail!("channel_not_found");
        }
        let mut posts = self.posts.lock().unwrap();
        posts.push((channel.into(), message));
        Ok(format!("9.{}", posts.len()))
    }

    async fn update(&self, channel: &str, ts: &str, message: Outgoing) -> anyhow::Result<()> {
        self.updates
            .lock()
            .unwrap()
            .push((channel.into(), ts.into(), message));
        Ok(())
    }

    async fn open_dm(&self, user: &str) -> anyhow::Result<String> {
        Ok(format!("DM-{user}"))
    }

    async fn set_status(&self, channel: &str, thread: &str, status: &str) -> anyhow::Result<()> {
        anyhow::ensure!(!self.no_status, "method_not_supported_for_channel_type");
        self.statuses
            .lock()
            .unwrap()
            .push((channel.into(), thread.into(), status.into()));
        Ok(())
    }

    async fn react(&self, channel: &str, ts: &str, name: &str) -> anyhow::Result<()> {
        self.reactions
            .lock()
            .unwrap()
            .push((channel.into(), ts.into(), name.into()));
        Ok(())
    }
}
