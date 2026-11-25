//! A Slack that records what famulen does with it, for tests.

use std::sync::Mutex;

use super::web::{Outgoing, Refused, Room, Slack};
use crate::gateway::Fetched;

#[derive(Default)]
pub struct FakeSlack {
    pub posts: Mutex<Vec<(String, Outgoing)>>,
    /// By channel and the one who sees it.
    pub ephemerals: Mutex<Vec<(String, String, Outgoing)>>,
    pub updates: Mutex<Vec<(String, String, Outgoing)>>,
    pub reactions: Mutex<Vec<(String, String, String)>>,
    pub statuses: Mutex<Vec<(String, String, String)>>,
    /// What each file's path holds; any other cannot be fetched.
    pub files: Mutex<Vec<(String, Fetched)>>,
    /// What is told of each conversation; nothing is told of any other.
    pub rooms: Mutex<Vec<(String, Room)>>,
    pub fail: bool,
    /// Slack refuses messages for one person.
    pub no_ephemeral: bool,
    /// Slack shows no status here.
    pub no_status: bool,
}

impl FakeSlack {
    pub fn posts(&self) -> Vec<(String, Outgoing)> {
        self.posts.lock().unwrap().clone()
    }

    pub fn ephemerals(&self) -> Vec<(String, String, Outgoing)> {
        self.ephemerals.lock().unwrap().clone()
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

    async fn post_ephemeral(
        &self,
        channel: &str,
        user: &str,
        message: Outgoing,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!self.no_ephemeral, "user_not_in_channel");
        self.ephemerals
            .lock()
            .unwrap()
            .push((channel.into(), user.into(), message));
        Ok(())
    }

    async fn update(&self, channel: &str, ts: &str, message: Outgoing) -> anyhow::Result<()> {
        self.updates
            .lock()
            .unwrap()
            .push((channel.into(), ts.into(), message));
        Ok(())
    }

    async fn download(&self, path: &str) -> anyhow::Result<Fetched> {
        let files = self.files.lock().unwrap();
        let found = files.iter().find(|(p, _)| p == path);
        found.map(|(_, file)| file.clone()).ok_or_else(|| {
            Refused {
                tool: "download_file".into(),
                why: match path {
                    "away" => crate::gateway::SENT_ELSEWHERE.into(),
                    _ => "answered 404 Not Found".into(),
                },
            }
            .into()
        })
    }

    async fn open_dm(&self, user: &str) -> anyhow::Result<String> {
        Ok(format!("DM-{user}"))
    }

    async fn room(&self, channel: &str) -> anyhow::Result<Room> {
        let rooms = self.rooms.lock().unwrap();
        let found = rooms.iter().find(|(c, _)| c == channel);
        found
            .map(|(_, room)| room.clone())
            .ok_or_else(|| anyhow::anyhow!("channel_not_found"))
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
