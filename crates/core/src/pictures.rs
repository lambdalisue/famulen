//! The pictures people show of themselves, kept here rather than in the
//! notebook: Git would keep each one that was replaced, and share them all.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use famulen_gateway::{Gateway, Wanted};
use futures_util::future::BoxFuture;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use sha2::Digest;
use url::Url;

use crate::app::App;
use crate::attachments::kind_of;
use crate::channel::Workspace;
use crate::console_log::Event;
use crate::records::{PARTIES, PartyAccount, Record, accounts, is_person};
use crate::slack::names::is_unknown;
use crate::store::Store;

/// Where Slack keeps what members put up; a picture elsewhere is one Slack
/// made up for a member who put up none.
const HOST: &str = "avatars.slack-edge.com";
const PICTURE: Wanted = Wanted {
    types: &["image/png", "image/jpeg", "image/gif", "image/webp"],
    most: 1024 * 1024,
    cut: false,
    hosts: &[HOST],
};
/// The size Slack itself shrank it to.
const SIZE: &str = "image_192";
/// How long a picture stands before Slack is asked again.
const EVERY: i64 = 24 * 60 * 60;
/// How long before Slack is asked again after it could not be.
const AGAIN: i64 = 60 * 60;
/// How long one workspace's answer is waited for.
const EACH: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// What an account shows.
#[derive(Debug, PartialEq, Eq)]
enum Shown {
    At(Url),
    Nothing,
}

/// What a `users.info` answer shows; nothing is told of one that does not say.
fn shown(answer: &Value) -> Option<Shown> {
    let url = answer["user"]["profile"][SIZE].as_str()?;
    let url = Url::parse(url).ok()?;
    Some(
        match url.scheme() == "https" && url.host_str() == Some(HOST) {
            true => Shown::At(url),
            false => Shown::Nothing,
        },
    )
}

/// The accounts of a person whose pictures are theirs.
fn pictured(party: &Record) -> Vec<PartyAccount> {
    if party.kind != PARTIES || !is_person(party) {
        return Vec::new();
    }
    let mut all = accounts(party);
    all.retain(|a| a.slack_member().is_some());
    all
}

/// How pictures are read, so that a test can read its own.
type Read = Arc<dyn Fn(Url) -> BoxFuture<'static, anyhow::Result<Vec<u8>>> + Send + Sync>;

#[derive(Clone)]
pub struct Pictures {
    store: Store,
    read: Read,
}

impl Pictures {
    pub fn new(store: Store, gateway: Gateway) -> Self {
        let read: Read = Arc::new(move |url| {
            let gateway = gateway.clone();
            Box::pin(async move { Ok(gateway.read_public("famulen", url, PICTURE).await?.body) })
        });
        Self { store, read }
    }

    /// What the picture of a party is asked for by.
    pub fn of(&self, party: &Record) -> Option<String> {
        pictured(party).iter().find_map(|a| {
            let (_, id) = self.kept(&a.id).ok()??;
            Some(id).filter(|id| !id.is_empty())
        })
    }

    pub fn get(&self, id: impl AsRef<str>) -> anyhow::Result<Option<Picture>> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT mime, bytes FROM account_pictures WHERE id = ?1 AND id <> ''",
                [id.as_ref()],
                |r| {
                    Ok(Picture {
                        mime: r.get(0)?,
                        bytes: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Where the picture kept for an account was, and its id.
    fn kept(&self, account: &str) -> anyhow::Result<Option<(String, String)>> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT url, id FROM account_pictures WHERE account = ?1",
                [account],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    fn is_due(&self, account: &str, now: i64) -> anyhow::Result<bool> {
        let due: Option<i64> = self
            .store
            .conn()
            .query_row(
                "SELECT due FROM account_pictures WHERE account = ?1",
                [account],
                |r| r.get(0),
            )
            .optional()?;
        Ok(due.is_none_or(|due| due <= now))
    }

    fn ask_again(&self, account: &str, due: i64) -> anyhow::Result<()> {
        self.store.conn().execute(
            "INSERT INTO account_pictures (account, due) VALUES (?1, ?2)
             ON CONFLICT (account) DO UPDATE SET due = excluded.due",
            params![account, due],
        )?;
        Ok(())
    }

    /// Throw away what is kept for accounts other than these. Whether a
    /// picture went with it.
    fn forget_others(&self, accounts: &BTreeSet<String>) -> anyhow::Result<bool> {
        let kept: Vec<(String, String)> = {
            let conn = self.store.conn();
            let mut statement = conn.prepare("SELECT account, id FROM account_pictures")?;
            statement
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?
        };
        let mut went = false;
        for (account, id) in kept.iter().filter(|(a, _)| !accounts.contains(a)) {
            self.store
                .conn()
                .execute("DELETE FROM account_pictures WHERE account = ?1", [account])?;
            went |= !id.is_empty();
        }
        Ok(went)
    }

    /// Keep what an account shows now. Whether that is another picture.
    async fn keep(&self, account: &str, shown: Shown) -> anyhow::Result<bool> {
        let (was_url, was) = self.kept(account)?.unwrap_or_default();
        let (url, id, mime, bytes) = match shown {
            Shown::At(url) if url.as_str() == was_url => return Ok(false),
            Shown::At(url) => {
                let bytes = (self.read)(url.clone()).await?;
                let mime = kind_of(&bytes)
                    .map(|(mime, _)| mime)
                    .filter(|m| m.starts_with("image/"))
                    .ok_or_else(|| anyhow::anyhow!("not a picture"))?;
                let hash = sha2::Sha256::digest(&bytes);
                let id: String = hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
                (url.to_string(), id, mime, bytes)
            }
            Shown::Nothing => Default::default(),
        };
        self.store.conn().execute(
            "UPDATE account_pictures SET url = ?2, id = ?3, mime = ?4, bytes = ?5 WHERE account = ?1",
            params![account, url, id, mime, bytes],
        )?;
        Ok(id != was)
    }
}

impl App {
    /// Read the pictures of the people whose Slack accounts are confirmed,
    /// each once a day.
    pub async fn refresh_pictures(&self, now: i64) -> anyhow::Result<()> {
        let accounts: Vec<PartyAccount> = self
            .records
            .list(PARTIES)?
            .iter()
            .flat_map(pictured)
            .collect();
        let ids = accounts.iter().map(|a| a.id.clone()).collect();
        if self.pictures.forget_others(&ids)? {
            self.console.notify(Event::Records);
        }
        let workspaces = self.workspaces()?;
        if workspaces.is_empty() {
            return Ok(());
        }
        for a in &accounts {
            let Some(member) = a.slack_member() else {
                continue;
            };
            if !self.pictures.is_due(&a.id, now)? {
                continue;
            }
            self.pictures.ask_again(&a.id, now + AGAIN)?;
            let refreshed = async {
                match self.slack_picture(&workspaces, member).await? {
                    Some(shown) => self.pictures.keep(&a.id, shown).await,
                    None => Ok(false),
                }
            };
            match refreshed.await {
                Ok(other) => {
                    self.pictures.ask_again(&a.id, now + EVERY)?;
                    if other {
                        self.console.notify(Event::Records);
                    }
                }
                Err(error) => {
                    tracing::warn!(%member, %error, "could not read a member's picture");
                }
            }
        }
        Ok(())
    }

    /// What a member shows in the first workspace they put up a picture in;
    /// nothing when no workspace knows them. That there is none is said only
    /// when every workspace could be asked.
    async fn slack_picture(
        &self,
        workspaces: &[Workspace],
        member: &str,
    ) -> anyhow::Result<Option<Shown>> {
        let mut found = None;
        let mut failed = None;
        for w in workspaces {
            let asked = self.tool_json(&w.id, "user_info", json!({ "user": member }));
            let answer = tokio::time::timeout(EACH, asked)
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("timed out")));
            match answer.map(|answer| shown(&answer)) {
                Ok(Some(Shown::At(url))) => return Ok(Some(Shown::At(url))),
                Ok(Some(Shown::Nothing)) => found = Some(Shown::Nothing),
                Ok(None) => failed = Some(anyhow::anyhow!("the answer names no picture")),
                Err(error) if is_unknown(&error) => {}
                Err(error) => failed = Some(error),
            }
        }
        match failed {
            Some(error) => Err(error),
            None => Ok(found),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::notebook::Writer;
    use crate::notebook::testing::TempDir;
    use crate::records::Patch;
    use crate::slack::names::tests::with_slack_answering;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n1";

    /// Famulen with a stand-in for Slack, what it answers as each member's
    /// picture, and what is read from where its pictures are.
    struct Stand {
        app: App,
        asked: Arc<Mutex<Vec<String>>>,
        url: Arc<Mutex<Option<String>>>,
        bytes: Arc<Mutex<Option<Vec<u8>>>>,
        read: Arc<Mutex<Vec<String>>>,
    }

    impl Stand {
        async fn new(dir: &TempDir) -> Self {
            let url = Arc::new(Mutex::new(Some(at("1"))));
            let answers = url.clone();
            let (mut app, _, asked) =
                with_slack_answering(dir.path(), move |_| match &*answers.lock().unwrap() {
                    Some(url) => json!({ "ok": true, "user": { "profile": { SIZE: url } } }),
                    None => json!({ "ok": false, "error": "user_not_found" }),
                })
                .await;
            let bytes = Arc::new(Mutex::new(Some(PNG.to_vec())));
            let read: Arc<Mutex<Vec<String>>> = Arc::default();
            let (giving, seen) = (bytes.clone(), read.clone());
            app.pictures.read = Arc::new(move |url| {
                seen.lock().unwrap().push(url.into());
                let given = giving.lock().unwrap().clone();
                Box::pin(async move { given.ok_or_else(|| anyhow::anyhow!("down")) })
            });
            Self {
                app,
                asked,
                url,
                bytes,
                read,
            }
        }

        fn party(&self, name: &str, kind: &str) -> String {
            let patch = Patch::titled(name).with("Kind", kind);
            let added = self.app.records.add(Writer::Owner, PARTIES, patch);
            added.unwrap().id
        }

        fn link(&self, party: &str, service: &str, handle: &str, confirmed: bool) {
            self.app
                .records
                .link_account(Writer::Owner, party, service, handle, confirmed, "")
                .unwrap();
        }

        fn of(&self, party: &str) -> Option<String> {
            let party = self.app.records.need(PARTIES, party).unwrap();
            self.app.pictures.of(&party)
        }

        fn bytes_of(&self, party: &str) -> Option<Vec<u8>> {
            let picture = self.app.pictures.get(self.of(party)?).unwrap()?;
            assert_eq!(picture.mime, "image/png");
            Some(picture.bytes)
        }
    }

    fn at(name: &str) -> String {
        format!("https://{HOST}/{name}_192.png")
    }

    #[tokio::test]
    async fn the_picture_of_a_person_with_a_confirmed_slack_account_is_kept() {
        let dir = TempDir::new("pictures-kept");
        let s = Stand::new(&dir).await;
        let person = s.party("田中", "person");
        s.link(&person, "slack", "T1/U1", true);

        s.app.refresh_pictures(0).await.unwrap();
        assert_eq!(s.bytes_of(&person).as_deref(), Some(PNG));
        assert_eq!(*s.asked.lock().unwrap(), ["U1"]);
        assert_eq!(*s.read.lock().unwrap(), [at("1")]);
    }

    #[tokio::test]
    async fn nothing_is_asked_of_guessed_accounts_other_services_and_organizations() {
        let dir = TempDir::new("pictures-others");
        let s = Stand::new(&dir).await;
        let person = s.party("田中", "person");
        s.link(&person, "slack", "U1", false);
        s.link(&person, "gmail", "a@example.com", true);
        let org = s.party("会社", "org");
        s.link(&org, "slack", "U2", true);

        s.app.refresh_pictures(0).await.unwrap();
        assert!(s.asked.lock().unwrap().is_empty());

        s.app
            .records
            .answer_account(Writer::Owner, "slack:U1", true)
            .unwrap();
        s.app.refresh_pictures(60).await.unwrap();
        assert!(s.of(&person).is_some());
    }

    #[tokio::test]
    async fn slack_is_asked_once_a_day_and_a_picture_is_read_only_when_it_changed() {
        let dir = TempDir::new("pictures-daily");
        let s = Stand::new(&dir).await;
        let person = s.party("田中", "person");
        s.link(&person, "slack", "U1", true);
        s.app.refresh_pictures(0).await.unwrap();
        s.app.refresh_pictures(EVERY - 1).await.unwrap();
        assert_eq!(s.asked.lock().unwrap().len(), 1);
        s.app.refresh_pictures(EVERY).await.unwrap();
        assert_eq!(s.asked.lock().unwrap().len(), 2);
        assert_eq!(s.read.lock().unwrap().len(), 1);

        let first = s.of(&person);
        let new = [PNG, b"2"].concat();
        *s.url.lock().unwrap() = Some(at("2"));
        *s.bytes.lock().unwrap() = Some(new.clone());
        s.app.refresh_pictures(EVERY * 2).await.unwrap();
        assert_ne!(s.of(&person), first);
        assert_eq!(s.bytes_of(&person), Some(new));
        assert_eq!(s.app.pictures.get(first.unwrap()).unwrap(), None);
    }

    #[tokio::test]
    async fn a_picture_that_could_not_be_read_leaves_the_one_before_and_is_tried_again() {
        let dir = TempDir::new("pictures-failed");
        let s = Stand::new(&dir).await;
        let person = s.party("田中", "person");
        s.link(&person, "slack", "U1", true);
        s.app.refresh_pictures(0).await.unwrap();

        let mut now = EVERY;
        for (url, bytes) in [
            (Some("not an address".into()), None),
            (Some(at("2")), None),
            (Some(at("2")), Some(b"<svg/>".to_vec())),
        ] {
            *s.url.lock().unwrap() = url;
            *s.bytes.lock().unwrap() = bytes;
            s.app.refresh_pictures(now).await.unwrap();
            assert_eq!(s.bytes_of(&person).as_deref(), Some(PNG));
            now += AGAIN;
        }
        assert_eq!(s.asked.lock().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn a_member_no_workspace_knows_keeps_the_picture_and_is_asked_for_a_day_later() {
        let dir = TempDir::new("pictures-unknown");
        let s = Stand::new(&dir).await;
        let person = s.party("田中", "person");
        s.link(&person, "slack", "U1", true);
        s.app.refresh_pictures(0).await.unwrap();

        *s.url.lock().unwrap() = None;
        s.app.refresh_pictures(EVERY).await.unwrap();
        s.app.refresh_pictures(EVERY + AGAIN).await.unwrap();
        assert_eq!(s.bytes_of(&person).as_deref(), Some(PNG));
        assert_eq!(s.asked.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_workspace_that_could_not_be_asked_is_not_taken_for_one_without_a_picture() {
        let dir = TempDir::new("pictures-two");
        let s = Stand::new(&dir).await;
        *s.url.lock().unwrap() = Some("https://secure.gravatar.com/avatar/x".into());
        let here = s.app.workspaces().unwrap().remove(0);
        let down = Workspace {
            id: "gone".into(),
            ..here.clone()
        };
        let alone = s.app.slack_picture(std::slice::from_ref(&here), "U1").await;
        assert_eq!(alone.unwrap(), Some(Shown::Nothing));
        let both = [down, here];
        assert!(s.app.slack_picture(&both, "U1").await.is_err());
        *s.url.lock().unwrap() = Some(at("1"));
        let found = s.app.slack_picture(&both, "U1").await.unwrap();
        assert!(matches!(found, Some(Shown::At(_))));
    }

    #[tokio::test]
    async fn a_picture_slack_made_up_is_not_read_and_takes_the_one_before_away() {
        let dir = TempDir::new("pictures-none");
        let s = Stand::new(&dir).await;
        let person = s.party("田中", "person");
        s.link(&person, "slack", "U1", true);
        s.app.refresh_pictures(0).await.unwrap();

        *s.url.lock().unwrap() = Some("https://secure.gravatar.com/avatar/x".into());
        s.app.refresh_pictures(EVERY).await.unwrap();
        assert_eq!(s.of(&person), None);
        assert_eq!(s.read.lock().unwrap().len(), 1);
        assert_eq!(s.app.pictures.get("").unwrap(), None);
    }

    #[tokio::test]
    async fn the_picture_of_an_account_that_is_no_ones_any_more_is_thrown_away() {
        let dir = TempDir::new("pictures-gone");
        let s = Stand::new(&dir).await;
        let person = s.party("田中", "person");
        s.link(&person, "slack", "U1", true);
        s.app.refresh_pictures(0).await.unwrap();
        let id = s.of(&person).unwrap();

        s.app
            .records
            .answer_account(Writer::Owner, "slack:U1", false)
            .unwrap();
        assert_eq!(s.of(&person), None);
        s.app.refresh_pictures(1).await.unwrap();
        assert_eq!(s.app.pictures.get(id).unwrap(), None);
    }
}
