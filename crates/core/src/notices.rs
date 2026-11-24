//! Notices to the owner's browsers and phones, through Web Push, of what
//! waits for their answer.

mod words;

use std::sync::Arc;
use std::time::Duration;

use famulen_gateway::{Gateway, Pushed, Subscription, Ticket};
use futures_util::future::{BoxFuture, join_all};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use crate::permissions::Permissions;
use crate::persona::Gender;
use crate::store::{Store, now};

/// Who the gateway's log says sent a notice.
const ACTOR: &str = "famulen";
const DETAIL_KEY: &str = "notices.detail";
/// The longest a device is called, in characters.
const NAME_CHARS: usize = 60;

/// How long to wait before each try after the first.
const WAITS: &[Duration] = &[Duration::from_secs(5), Duration::from_secs(60)];

/// How much of a request a notice tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Detail {
    /// That one waits.
    Number,
    /// What it would do, and where.
    #[default]
    Deed,
    /// Also the values of the call.
    Values,
}

impl Detail {
    fn as_str(self) -> &'static str {
        match self {
            Detail::Number => "number",
            Detail::Deed => "deed",
            Detail::Values => "values",
        }
    }

    /// What the owner chose; the default until then.
    pub fn load(store: &Store) -> anyhow::Result<Self> {
        Ok(match store.state(DETAIL_KEY)?.as_deref() {
            Some("number") => Detail::Number,
            Some("values") => Detail::Values,
            _ => Detail::Deed,
        })
    }

    pub fn save(self, store: &Store) -> anyhow::Result<()> {
        store.set_state(DETAIL_KEY, self.as_str())
    }
}

/// A browser notices are sent to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: i64,
    pub name: String,
    pub endpoint: String,
    pub added_at: i64,
    /// Why the last notice did not go; empty when it did.
    pub failed: String,
}

/// What hands a message to a push service.
trait Deliver: Send + Sync {
    fn deliver(
        &self,
        to: Subscription,
        message: Vec<u8>,
        ttl: Duration,
    ) -> BoxFuture<'static, anyhow::Result<Pushed>>;
}

impl Deliver for Gateway {
    fn deliver(
        &self,
        to: Subscription,
        message: Vec<u8>,
        ttl: Duration,
    ) -> BoxFuture<'static, anyhow::Result<Pushed>> {
        let gateway = self.clone();
        Box::pin(async move { gateway.push(ACTOR, &to, &message, ttl).await })
    }
}

#[derive(Clone)]
pub struct Notices {
    store: Store,
    permissions: Permissions,
    through: Arc<dyn Deliver>,
    waits: &'static [Duration],
}

impl Notices {
    pub fn new(store: Store, permissions: Permissions) -> Self {
        let through = Arc::new(permissions.gateway().clone());
        Self {
            store,
            permissions,
            through,
            waits: WAITS,
        }
    }

    /// The key browsers subscribe with.
    pub fn key(&self) -> anyhow::Result<String> {
        self.permissions.gateway().push_key()
    }

    /// Send notices to a browser too. One already known is renewed.
    pub fn add(&self, to: &Subscription, name: impl AsRef<str>) -> anyhow::Result<()> {
        to.check()?;
        let name: String = name.as_ref().trim().chars().take(NAME_CHARS).collect();
        let name = if name.is_empty() {
            "ブラウザ"
        } else {
            &name
        };
        self.store.conn().execute(
            "INSERT INTO notice_devices (endpoint, p256dh, auth, name, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (endpoint) DO UPDATE SET p256dh = excluded.p256dh,
             auth = excluded.auth, name = excluded.name, failed = ''",
            params![to.endpoint, to.p256dh, to.auth, name, now()],
        )?;
        Ok(())
    }

    pub fn remove(&self, id: i64) -> anyhow::Result<()> {
        self.store
            .conn()
            .execute("DELETE FROM notice_devices WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The oldest first.
    pub fn devices(&self) -> anyhow::Result<Vec<Device>> {
        let conn = self.store.conn();
        let mut statement = conn.prepare(
            "SELECT id, name, endpoint, added_at, failed FROM notice_devices ORDER BY id",
        )?;
        let devices = statement
            .query_map([], |row| {
                Ok(Device {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    endpoint: row.get(2)?,
                    added_at: row.get(3)?,
                    failed: row.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(devices)
    }

    /// Tell every device of a request that waits for the owner's answer.
    pub async fn tell(&self, ticket: &Ticket) -> anyhow::Result<()> {
        let message = self.message(ticket)?;
        let devices = self.devices()?;
        join_all(devices.iter().map(|d| async {
            if let Err(error) = self.send_again(ticket.id, d.id, &message).await {
                tracing::warn!(%error, device = %d.name, "could not send a notice");
            }
        }))
        .await;
        Ok(())
    }

    /// Send to a device, and try again while the request still waits and the
    /// device is still one to tell.
    async fn send_again(&self, request: i64, id: i64, message: &Value) -> anyhow::Result<()> {
        for wait in self.waits {
            match self.send(id, message).await {
                Ok(()) => return Ok(()),
                Err(error) => tracing::debug!(%error, "a notice did not go; trying again"),
            }
            tokio::time::sleep(*wait).await;
            let left = self.devices()?.iter().any(|d| d.id == id);
            if !left || crate::approvals::answered(&self.store, request)? {
                return Ok(());
            }
        }
        self.send(id, message).await
    }

    /// Send a device a notice of nothing, so that the owner sees it arrive.
    pub async fn try_device(&self, id: i64) -> anyhow::Result<()> {
        let message = self.notice(
            "Famulen",
            "知らせはこの端末に届きます",
            json!({ "tag": "try" }),
        )?;
        self.send(id, &message).await
    }

    fn message(&self, ticket: &Ticket) -> anyhow::Result<Value> {
        let body = match Detail::load(&self.store)? {
            Detail::Number => "答えを待っている確認があります".to_owned(),
            Detail::Deed => self.deed(ticket)?,
            // A phone shows notices while it is locked.
            // What a command holds cannot be told from its words.
            Detail::Values
                if ticket.connection.is_empty() || crate::approvals::may_show_secret(ticket) =>
            {
                self.deed(ticket)?
            }
            Detail::Values => {
                let deed = self.deed(ticket)?;
                match words::values(&ticket.shown).as_str() {
                    "" => deed,
                    values => format!("{deed}\n{values}"),
                }
            }
        };
        self.notice(
            &format!("確認 #{}", ticket.id),
            &body,
            json!({ "tag": format!("confirm-{}", ticket.id), "confirm": ticket.id }),
        )
    }

    fn notice(&self, title: &str, body: &str, rest: Value) -> anyhow::Result<Value> {
        let mut message = rest;
        message["title"] = title.into();
        message["body"] = body.into();
        let gender = Gender::load(&self.store)?.as_str();
        message["icon"] = format!("/icons/{gender}/app-icon-192.png").into();
        Ok(message)
    }

    fn deed(&self, ticket: &Ticket) -> anyhow::Result<String> {
        let connection = self.permissions.connection(&ticket.connection);
        let deed = words::deed(&ticket.connection, &ticket.tool, connection.as_ref());
        let asked = crate::approvals::asks_questions(&self.permissions, ticket);
        Ok(if asked {
            "あなたの答えを求めています".into()
        } else {
            format!("{deed}許可を求めています")
        })
    }

    /// Send to a device, and keep whether it went. One that takes no more is
    /// forgotten.
    async fn send(&self, id: i64, message: &Value) -> anyhow::Result<()> {
        let to = self
            .store
            .conn()
            .query_row(
                "SELECT endpoint, p256dh, auth FROM notice_devices WHERE id = ?1",
                [id],
                |row| {
                    Ok(Subscription {
                        endpoint: row.get(0)?,
                        p256dh: row.get(1)?,
                        auth: row.get(2)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("no such device"))?;
        // A notice outlives what it tells of no longer than its ticket.
        let ttl = Duration::from_secs(famulen_gateway::TICKET_LIFETIME_SECS as u64);
        let sent = self
            .through
            .deliver(to, message.to_string().into_bytes(), ttl)
            .await;
        match &sent {
            Ok(Pushed::Gone) => {
                self.remove(id)?;
                anyhow::bail!("the device takes no more notices, so it was taken off the list");
            }
            Ok(Pushed::Taken) => self.keep_failure(id, "")?,
            Err(error) => self.keep_failure(id, &format!("{error:#}"))?,
        }
        sent.map(drop)
    }

    fn keep_failure(&self, id: i64, failed: &str) -> anyhow::Result<()> {
        self.store.conn().execute(
            "UPDATE notice_devices SET failed = ?1 WHERE id = ?2",
            params![failed, id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Mutex;

    use super::*;

    /// Keeps what it is handed; an endpoint that says `gone` takes nothing,
    /// one that says `fail` fails and `later` fails once.
    #[derive(Default)]
    pub(crate) struct Kept(pub Mutex<Vec<(String, Value)>>, Mutex<bool>);

    impl Deliver for Arc<Kept> {
        fn deliver(
            &self,
            to: Subscription,
            message: Vec<u8>,
            _ttl: Duration,
        ) -> BoxFuture<'static, anyhow::Result<Pushed>> {
            let kept = self.clone();
            Box::pin(async move {
                if to.endpoint.ends_with("fail") {
                    anyhow::bail!("refused");
                }
                if to.endpoint.ends_with("later")
                    && !std::mem::replace(&mut *kept.1.lock().unwrap(), true)
                {
                    anyhow::bail!("busy");
                }
                if to.endpoint.ends_with("gone") {
                    return Ok(Pushed::Gone);
                }
                let message = serde_json::from_slice(&message)?;
                kept.0.lock().unwrap().push((to.endpoint, message));
                Ok(Pushed::Taken)
            })
        }
    }

    pub(crate) fn notices(store: Store, permissions: Permissions) -> (Notices, Arc<Kept>) {
        let kept = Arc::new(Kept::default());
        let notices = Notices {
            store,
            permissions,
            through: Arc::new(kept.clone()),
            waits: &[Duration::ZERO],
        };
        (notices, kept)
    }

    pub(crate) fn device(name: &str) -> Subscription {
        Subscription {
            endpoint: format!("https://push.example.com/{name}"),
            p256dh: "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4".into(),
            auth: "BTBZMqHH6r4Tts7J_aSIgg".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{device, notices};
    use super::*;

    fn setup() -> (Notices, Arc<testing::Kept>) {
        let store = Store::open_in_memory().unwrap();
        let gateway = crate::permissions::test_gateway(
            famulen_gateway::store::Store::open_in_memory().unwrap(),
        );
        let permissions = Permissions::new(store.clone(), gateway);
        notices(store, permissions)
    }

    fn ticket(tool: &str, shown: Value) -> Ticket {
        Ticket {
            id: 7,
            actor: "secretary".into(),
            connection: String::new(),
            tool: tool.into(),
            shown,
            reason: String::new(),
            origin: None,
            status: "held".into(),
            created_at: 0,
        }
    }

    fn body(kept: &testing::Kept) -> String {
        let kept = kept.0.lock().unwrap();
        kept.last().unwrap().1["body"].as_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn every_device_is_told_of_a_request_and_which_it_is() {
        let (notices, kept) = setup();
        notices.add(&device("phone"), "iPhone").unwrap();
        notices.add(&device("mac"), "Mac").unwrap();
        notices
            .tell(&ticket("Bash", json!({ "command": "ls" })))
            .await
            .unwrap();
        let kept = kept.0.lock().unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].1["title"], "確認 #7");
        assert_eq!(kept[0].1["confirm"], 7);
        assert_eq!(kept[0].1["icon"], "/icons/male/app-icon-192.png");
    }

    #[tokio::test]
    async fn the_owner_chooses_how_much_a_notice_tells() {
        let (notices, kept) = setup();
        notices.add(&device("phone"), "iPhone").unwrap();
        let mut call = ticket("send_message", json!({ "to": "a@example.com" }));
        call.connection = "gmail".into();
        notices.tell(&call).await.unwrap();
        assert_eq!(body(&kept), "つなぎ先でメッセージを送る許可を求めています");

        Detail::Number.save(&notices.store).unwrap();
        notices.tell(&call).await.unwrap();
        assert_eq!(body(&kept), "答えを待っている確認があります");

        Detail::Values.save(&notices.store).unwrap();
        notices.tell(&call).await.unwrap();
        assert_eq!(
            body(&kept),
            "つなぎ先でメッセージを送る許可を求めています\na@example.com"
        );
    }

    #[tokio::test]
    async fn the_values_of_a_call_that_may_show_a_secret_are_not_told() {
        let (notices, kept) = setup();
        notices.add(&device("phone"), "iPhone").unwrap();
        Detail::Values.save(&notices.store).unwrap();
        let mut call = ticket("send_message", json!({ "text": "password: hunter2" }));
        call.connection = "gmail".into();
        notices.tell(&call).await.unwrap();
        assert!(!body(&kept).contains("hunter2"));
        let call = ticket("Bash", json!({ "command": "curl -u me:hunter2 x" }));
        notices.tell(&call).await.unwrap();
        assert_eq!(body(&kept), "コマンドを実行する許可を求めています");
    }

    #[tokio::test]
    async fn a_device_that_takes_no_more_is_forgotten_and_a_failure_is_kept() {
        let (notices, kept) = setup();
        for name in ["gone", "fail", "phone"] {
            notices.add(&device(name), name).unwrap();
        }
        notices.tell(&ticket("Bash", json!({}))).await.unwrap();
        assert_eq!(kept.0.lock().unwrap().len(), 1);
        let left: Vec<_> = notices
            .devices()
            .unwrap()
            .into_iter()
            .map(|d| (d.name, d.failed))
            .collect();
        assert_eq!(
            left,
            [
                ("fail".to_owned(), "refused".to_owned()),
                ("phone".to_owned(), String::new())
            ]
        );
    }

    #[tokio::test]
    async fn a_notice_that_did_not_go_is_sent_again_while_the_request_waits() {
        let (notices, kept) = setup();
        notices.add(&device("later"), "iPhone").unwrap();
        notices.tell(&ticket("Bash", json!({}))).await.unwrap();
        assert_eq!(kept.0.lock().unwrap().len(), 1);
        assert_eq!(notices.devices().unwrap()[0].failed, "");
    }

    #[test]
    fn a_device_added_again_is_renewed_and_one_removed_is_told_no_more() {
        let (notices, _) = setup();
        notices.add(&device("phone"), "a").unwrap();
        notices.add(&device("phone"), "b").unwrap();
        let devices = notices.devices().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "b");
        notices.remove(devices[0].id).unwrap();
        assert!(notices.devices().unwrap().is_empty());
    }

    #[test]
    fn what_is_no_push_service_is_refused() {
        let (notices, _) = setup();
        let mut to = device("x");
        to.endpoint = "https://127.0.0.1/x".into();
        assert!(notices.add(&to, "x").is_err());
        assert!(notices.devices().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_device_can_be_tried_alone() {
        let (notices, kept) = setup();
        notices.add(&device("phone"), "iPhone").unwrap();
        notices.add(&device("mac"), "Mac").unwrap();
        let id = notices.devices().unwrap()[1].id;
        notices.try_device(id).await.unwrap();
        let kept = kept.0.lock().unwrap();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].0.ends_with("mac"));
    }
}
