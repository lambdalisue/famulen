//! Changing service definitions from inside: a caller proposes a definition's
//! whole text, the change is held for the owner, and only the owner's answer
//! writes it where the gateway reads definitions from. Every change is kept
//! with the file it replaced, so the owner can take the last one back.

use std::path::PathBuf;

use rusqlite::params;
use serde_json::{Value, json};

use crate::connections::Transport;
use crate::definitions::{Definition, Definitions, loosened, parse};
use crate::log::LogEntry;
use crate::store::now;

use super::Gateway;
use super::judge::{Call, OWNER, Outcome, Verdict};

/// The name changes to the definitions go by when they are judged and logged.
pub const DEFINITIONS: &str = "definitions";
/// A change to one definition, as the tool the owner is asked about.
pub const CHANGE_DEFINITION: &str = "change_definition";

/// Where the gateway reads its definitions from, besides the bundled ones.
#[derive(Debug, Default)]
pub(super) struct Sources {
    /// Where changes are written; none when the gateway was given no place.
    pub(super) dir: Option<PathBuf>,
    pub(super) user: Vec<(String, String)>,
}

/// One change that went in, or one taken back.
#[derive(Debug, Clone, PartialEq)]
pub struct DefinitionChange {
    pub id: i64,
    pub definition: String,
    /// `change`, or `revert` for one taken back.
    pub kind: String,
    /// The definition in use before and after; `None` when there was none.
    pub before: Option<String>,
    pub after: Option<String>,
    pub actor: String,
    pub why: String,
    pub at: i64,
}

impl Gateway {
    /// Read the definitions again from where they were first read.
    pub(super) fn read_definitions(&self) {
        let from = &self.definitions_from;
        *crate::lock::write(&self.definitions) = Definitions::load(from.dir.as_deref(), &from.user);
    }

    /// The text of the definition `id` in use now, if there is one.
    fn text_in_use(&self, id: &str) -> Option<String> {
        self.definitions().get(id).map(|d| d.text.clone())
    }

    fn place_of(&self, id: &str) -> anyhow::Result<PathBuf> {
        let dir = self.definitions_from.dir.as_ref().ok_or_else(|| {
            anyhow::anyhow!("this gateway has no place to keep service definitions")
        })?;
        Ok(dir.join(format!("{id}.toml")))
    }

    /// Check a proposed definition as reading it would, and against the one
    /// in use: one that does not read, or that would let a tool run more
    /// freely than now, is refused with why.
    fn check_proposed(&self, id: &str, text: &str) -> anyhow::Result<Definition> {
        let def = parse(text).map_err(|e| anyhow::anyhow!("the definition does not read: {e}"))?;
        anyhow::ensure!(def.id == id, "the definition's id is {}, not {id}", def.id);
        let current = self.definitions().get(id).cloned();
        let reasons = loosened(current.as_ref(), &def);
        anyhow::ensure!(
            reasons.is_empty(),
            "the change would loosen what the owner is asked: {}",
            reasons.join("; ")
        );
        Ok(def)
    }

    /// Propose `text` as the whole of definition `id` for `actor`, saying
    /// `why`. The owner's change goes in at once; anyone else's is held for
    /// the owner, and a definition that is refused is not held at all.
    pub async fn propose_definition(
        &self,
        actor: impl AsRef<str>,
        id: impl AsRef<str>,
        text: impl AsRef<str>,
        why: impl AsRef<str>,
    ) -> anyhow::Result<Outcome<()>> {
        let (actor, id, text, why) = (
            actor.as_ref(),
            id.as_ref().trim(),
            text.as_ref(),
            why.as_ref().trim(),
        );
        self.place_of(id)?;
        self.check_proposed(id, text)?;
        let input = json!({
            "definition": id,
            "toml": text,
            "why": why,
            "before": self.text_in_use(id),
        });
        let call = Call::new(actor, DEFINITIONS, CHANGE_DEFINITION, input.clone());
        match self.judge(&call)? {
            Verdict::Allow { .. } => {
                self.apply_definition(actor, &input).await?;
                Ok(Outcome::Done(()))
            }
            Verdict::Hold(ticket) => Ok(Outcome::Held(ticket)),
            Verdict::Deny { reason } => anyhow::bail!("{reason}"),
        }
    }

    /// Make a held change the owner approved, as its caller.
    pub(super) async fn run_held_definition(
        &self,
        ticket: &super::Ticket,
        input: &Value,
    ) -> anyhow::Result<()> {
        let result = self.apply_definition(&ticket.actor, input).await;
        if let Err(error) = &result {
            self.log().write(&LogEntry {
                actor: ticket.actor.clone(),
                connection: field(input, "definition").to_owned(),
                tool: "definition.change".into(),
                decision: "deny".into(),
                reason: format!("#{}: {error:#}", ticket.id),
                ..LogEntry::default()
            })?;
        }
        result
    }

    async fn apply_definition(&self, actor: &str, input: &Value) -> anyhow::Result<()> {
        let (id, text, why) = (
            field(input, "definition"),
            field(input, "toml"),
            field(input, "why"),
        );
        let before = self.text_in_use(id);
        anyhow::ensure!(
            before.as_deref() == input.get("before").and_then(Value::as_str),
            "{id} changed after this was proposed; propose the change again against the definition in use now"
        );
        self.check_proposed(id, text)?;
        self.write_definition(id, Some(text))
            .and_then(|previous| self.record_change(id, "change", before, previous, actor, why))?;
        self.after_change(id).await;
        Ok(())
    }

    /// Write the file of definition `id`, or remove it for `None`; the file
    /// it replaced.
    fn write_definition(&self, id: &str, text: Option<&str>) -> anyhow::Result<Option<String>> {
        let place = self.place_of(id)?;
        let previous = std::fs::read_to_string(&place).ok();
        match text {
            Some(text) => {
                if let Some(dir) = place.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&place, text)?;
            }
            None if previous.is_some() => std::fs::remove_file(&place)?,
            None => {}
        }
        self.read_definitions();
        Ok(previous)
    }

    fn record_change(
        &self,
        id: &str,
        kind: &str,
        before: Option<String>,
        previous: Option<String>,
        actor: &str,
        why: &str,
    ) -> anyhow::Result<()> {
        let after = self.text_in_use(id);
        self.store.conn().execute(
            "INSERT INTO definition_changes (definition, kind, before, after, previous, actor, why, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, kind, before, after, previous, actor, why, now()],
        )?;
        self.log_change_by(actor, id, &format!("definition.{kind}"), why)
    }

    /// Let the connections made from definition `id` use it: their clients
    /// are opened again and their tools read again.
    async fn after_change(&self, id: &str) {
        let made_from: Vec<String> = self
            .connections
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|c| matches!(&c.transport, Transport::Defined { definition, .. } if definition == id))
            .map(|c| c.id)
            .collect();
        for connection in made_from {
            self.clients.lock().await.remove(&connection);
            if let Err(error) = self.generate_account_values(&connection) {
                tracing::warn!(%connection, %error, "could not make the generated account values");
            }
            if let Err(error) = self.tools_of(&connection).await {
                tracing::debug!(%connection, %error, "the tools were not read again");
            }
        }
        let _ = self.redefined.send(id.to_owned());
    }

    /// Take back the last change to definition `id`: the file it replaced is
    /// put back, or removed when there was none. Only the owner does this,
    /// and taking back is itself kept as a change.
    pub async fn revert_definition(
        &self,
        actor: impl AsRef<str>,
        id: impl AsRef<str>,
    ) -> anyhow::Result<()> {
        let (actor, id) = (actor.as_ref(), id.as_ref());
        anyhow::ensure!(actor == OWNER, "only the owner takes a definition back");
        let previous: Option<String> = self
            .store
            .conn()
            .query_row(
                "SELECT previous FROM definition_changes WHERE definition = ?1 ORDER BY id DESC LIMIT 1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("{id} has no change to take back"))?;
        let before = self.text_in_use(id);
        let replaced = self.write_definition(id, previous.as_deref())?;
        self.record_change(id, "revert", before, replaced, actor, "")?;
        self.after_change(id).await;
        Ok(())
    }

    /// The changes to definition `id`, or to every definition for `None`,
    /// the newest first.
    pub fn definition_changes(
        &self,
        id: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<DefinitionChange>> {
        let conn = self.store.conn();
        let mut statement = conn.prepare(
            "SELECT id, definition, kind, before, after, actor, why, at FROM definition_changes
             WHERE ?1 IS NULL OR definition = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let changes = statement
            .query_map(params![id, limit as i64], |row| {
                Ok(DefinitionChange {
                    id: row.get(0)?,
                    definition: row.get(1)?,
                    kind: row.get(2)?,
                    before: row.get(3)?,
                    after: row.get(4)?,
                    actor: row.get(5)?,
                    why: row.get(6)?,
                    at: row.get(7)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(changes)
    }

    /// The ids of definitions from now on, as changes to them go in.
    pub fn subscribe_redefined(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.redefined.subscribe()
    }
}

fn field<'a>(input: &'a Value, name: &str) -> &'a str {
    input.get(name).and_then(Value::as_str).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::connections::Connections;
    use crate::permissions::{AutoCourse, Permission, auto_course};
    use crate::store::Store;
    use crate::{ANY, Answer};

    const BILLING: &str = r#"format = 1
id = "billing"
name = "Billing"

[transport]
http = { base = "http://127.0.0.1:9" }

[[tools]]
name = "list_invoices"
description = "List invoices."
method = "GET"
path = "/invoices"
"#;

    const DELETE: &str = r#"
[[tools]]
name = "delete_invoice"
description = "Delete one invoice."
method = "DELETE"
path = "/invoices/{/id}"
"#;

    struct Scratch {
        gateway: Gateway,
        dir: PathBuf,
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// A gateway that reads `billing` from the owner's place, with the
    /// secretary granted everything and one connection made from it.
    fn scratch(name: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("gateway-redefine-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("billing.toml"), BILLING).unwrap();
        let store = Store::open_in_memory().unwrap();
        let gateway = Gateway::new(
            store.clone(),
            Connections::new(store),
            reqwest::Client::new(),
            None,
        )
        .with_definitions(Some(&dir), &[]);
        gateway.grant("secretary", ANY, ANY).unwrap();
        gateway
            .add_defined("billing", BTreeMap::new(), None)
            .unwrap();
        Scratch { gateway, dir }
    }

    fn on_disk(s: &Scratch) -> String {
        std::fs::read_to_string(s.dir.join("billing.toml")).unwrap()
    }

    async fn tools(g: &Gateway) -> Vec<String> {
        g.tools_of("billing")
            .await
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn held(g: &Gateway, text: &str) -> i64 {
        match g
            .propose_definition("secretary", "billing", text, "to delete invoices")
            .await
            .unwrap()
        {
            Outcome::Held(ticket) => ticket.id,
            Outcome::Done(()) => panic!("the secretary's change is held"),
        }
    }

    #[tokio::test]
    async fn a_proposed_change_is_held_and_the_definition_stays_until_the_owner_answers() {
        let s = scratch("held");
        let text = format!("{BILLING}{DELETE}");

        let id = held(&s.gateway, &text).await;

        assert_eq!(on_disk(&s), BILLING);
        assert_eq!(tools(&s.gateway).await, ["list_invoices"]);
        let ticket = s.gateway.ticket(id).unwrap().unwrap();
        assert_eq!(
            (ticket.connection.as_str(), ticket.tool.as_str()),
            (DEFINITIONS, CHANGE_DEFINITION)
        );
        assert_eq!(ticket.shown["before"], BILLING);
        assert_eq!(ticket.shown["toml"], text.as_str());
        assert_eq!(ticket.shown["why"], "to delete invoices");
    }

    #[tokio::test]
    async fn an_approved_change_is_written_read_again_and_its_writing_tool_asks() {
        let s = scratch("approved");
        let text = format!("{BILLING}{DELETE}");
        let id = held(&s.gateway, &text).await;

        let answered = s
            .gateway
            .answer_ticket(id, Answer::Approve)
            .await
            .unwrap()
            .unwrap();

        assert!(answered.ran.unwrap().is_ok());
        assert_eq!(on_disk(&s), text);
        assert_eq!(tools(&s.gateway).await, ["list_invoices", "delete_invoice"]);
        let permissions = s.gateway.permissions();
        assert_eq!(
            permissions.default_of("billing", "delete_invoice").unwrap(),
            Permission::Auto
        );
        assert_eq!(
            auto_course(
                permissions.marks_of("billing", "delete_invoice").unwrap(),
                "delete_invoice"
            ),
            AutoCourse::Ask
        );
        let call = Call::new(
            "secretary",
            "billing",
            "delete_invoice",
            json!({ "id": "1" }),
        );
        assert!(matches!(s.gateway.judge(&call).unwrap(), Verdict::Hold(_)));
        let change = &s.gateway.definition_changes(Some("billing"), 10).unwrap()[0];
        assert_eq!(
            (
                change.kind.as_str(),
                change.actor.as_str(),
                change.why.as_str()
            ),
            ("change", "secretary", "to delete invoices")
        );
        assert_eq!(change.before.as_deref(), Some(BILLING));
        assert_eq!(change.after.as_deref(), Some(text.as_str()));
        assert!(
            s.gateway
                .log_entries(5)
                .unwrap()
                .iter()
                .any(|e| e.tool == "definition.change" && e.connection == "billing")
        );
    }

    #[tokio::test]
    async fn a_rejected_change_changes_nothing() {
        let s = scratch("rejected");
        let id = held(&s.gateway, &format!("{BILLING}{DELETE}")).await;

        let answered = s
            .gateway
            .answer_ticket(id, Answer::Reject)
            .await
            .unwrap()
            .unwrap();

        assert!(answered.ran.is_none());
        assert_eq!(on_disk(&s), BILLING);
        assert_eq!(tools(&s.gateway).await, ["list_invoices"]);
        assert!(s.gateway.definition_changes(None, 10).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_definition_that_does_not_read_is_not_even_held() {
        let s = scratch("broken");
        let broken = BILLING.replace("method = \"GET\"", "method = \"FETCH\"");
        let other_id = BILLING.replace("id = \"billing\"", "id = \"invoices\"");

        for text in [broken, other_id] {
            let error = s
                .gateway
                .propose_definition("secretary", "billing", &text, "")
                .await
                .unwrap_err();
            assert!(!error.to_string().is_empty());
        }

        assert!(s.gateway.held_tickets().unwrap().is_empty());
        assert_eq!(on_disk(&s), BILLING);
    }

    #[tokio::test]
    async fn a_change_that_would_let_a_writing_tool_run_unasked_is_refused() {
        let s = scratch("loosen");
        let text = format!("{BILLING}{DELETE}\n[permissions]\ndelete_invoice = \"allow\"\n");

        let error = s
            .gateway
            .propose_definition("secretary", "billing", &text, "")
            .await
            .unwrap_err();

        assert!(error.to_string().contains("delete_invoice"), "{error}");
        assert!(s.gateway.held_tickets().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_approved_change_does_not_go_in_once_it_stops_reading() {
        let s = scratch("stale-broken");
        let id = held(&s.gateway, &format!("{BILLING}{DELETE}")).await;
        // Someone breaks the proposal's text before the owner answers, as
        // a hand-edited row would.
        s.gateway
            .store
            .conn()
            .execute(
                "UPDATE tickets SET input = json_set(input, '$.toml', 'format = 1') WHERE id = ?1",
                params![id],
            )
            .unwrap();

        let answered = s
            .gateway
            .answer_ticket(id, Answer::Approve)
            .await
            .unwrap()
            .unwrap();

        assert!(answered.ran.unwrap().is_err());
        assert_eq!(on_disk(&s), BILLING);
        assert!(s.gateway.definition_changes(None, 10).unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_approved_change_does_not_go_in_when_the_definition_moved_since() {
        let s = scratch("stale");
        let id = held(&s.gateway, &format!("{BILLING}{DELETE}")).await;
        let moved = BILLING.replace("List invoices.", "List every invoice.");
        std::fs::write(s.dir.join("billing.toml"), &moved).unwrap();
        s.gateway.read_definitions();

        let answered = s
            .gateway
            .answer_ticket(id, Answer::Approve)
            .await
            .unwrap()
            .unwrap();

        let error = answered.ran.unwrap().unwrap_err();
        assert!(error.to_string().contains("changed after"), "{error}");
        assert_eq!(on_disk(&s), moved);
    }

    #[tokio::test]
    async fn always_allow_lets_one_change_in_and_the_next_is_still_asked() {
        let s = scratch("always");
        let id = held(&s.gateway, &format!("{BILLING}{DELETE}")).await;

        s.gateway
            .answer_ticket(id, Answer::AlwaysAllow)
            .await
            .unwrap();

        let next = format!("{BILLING}{DELETE}").replace("Delete one invoice.", "Delete it.");
        held(&s.gateway, &next).await;
        s.gateway
            .permissions()
            .set_scoped(
                "conn:definitions",
                CHANGE_DEFINITION,
                Some(Permission::Allow),
            )
            .unwrap();
        held(&s.gateway, &next).await;
    }

    #[tokio::test]
    async fn the_owner_can_forbid_changes_to_definitions() {
        let s = scratch("forbid");
        s.gateway
            .set_permission(
                OWNER,
                "conn:definitions",
                CHANGE_DEFINITION,
                Some(Permission::Forbid),
            )
            .unwrap();

        let error = s
            .gateway
            .propose_definition("secretary", "billing", format!("{BILLING}{DELETE}"), "")
            .await
            .unwrap_err();

        assert!(error.to_string().contains("forbade"), "{error}");
    }

    #[tokio::test]
    async fn the_owner_takes_the_last_change_back_and_the_tools_follow() {
        let s = scratch("revert");
        let text = format!("{BILLING}{DELETE}");
        let id = held(&s.gateway, &text).await;
        s.gateway.answer_ticket(id, Answer::Approve).await.unwrap();

        assert!(
            s.gateway
                .revert_definition("secretary", "billing")
                .await
                .is_err()
        );
        s.gateway.revert_definition(OWNER, "billing").await.unwrap();

        assert_eq!(on_disk(&s), BILLING);
        assert_eq!(tools(&s.gateway).await, ["list_invoices"]);
        let changes = s.gateway.definition_changes(Some("billing"), 10).unwrap();
        assert_eq!(
            changes.iter().map(|c| c.kind.as_str()).collect::<Vec<_>>(),
            ["revert", "change"]
        );
        assert_eq!(changes[0].before.as_deref(), Some(text.as_str()));
        assert_eq!(changes[0].after.as_deref(), Some(BILLING));
    }

    #[tokio::test]
    async fn taking_back_a_new_definition_removes_its_file() {
        let s = scratch("revert-new");
        let notes = BILLING
            .replace("\"billing\"", "\"notes\"")
            .replace("Billing", "Notes");
        let Outcome::Held(ticket) = s
            .gateway
            .propose_definition("secretary", "notes", &notes, "a new service")
            .await
            .unwrap()
        else {
            panic!("held");
        };
        assert_eq!(ticket.shown["before"], Value::Null);
        s.gateway
            .answer_ticket(ticket.id, Answer::Approve)
            .await
            .unwrap();
        assert!(s.gateway.definitions().get("notes").is_some());

        s.gateway.revert_definition(OWNER, "notes").await.unwrap();

        assert!(!s.dir.join("notes.toml").exists());
        assert!(s.gateway.definitions().get("notes").is_none());
        assert!(
            s.gateway
                .revert_definition(OWNER, "billing")
                .await
                .unwrap_err()
                .to_string()
                .contains("no change")
        );
    }

    #[tokio::test]
    async fn a_change_that_went_in_is_told_to_subscribers() {
        let s = scratch("told");
        let mut told = s.gateway.subscribe_redefined();
        let id = held(&s.gateway, &format!("{BILLING}{DELETE}")).await;
        assert!(told.try_recv().is_err());

        s.gateway.answer_ticket(id, Answer::Approve).await.unwrap();

        assert_eq!(told.try_recv().unwrap(), "billing");
    }

    #[tokio::test]
    async fn without_a_place_for_definitions_a_change_is_refused() {
        let store = Store::open_in_memory().unwrap();
        let g = Gateway::new(
            store.clone(),
            Connections::new(store),
            reqwest::Client::new(),
            None,
        );
        g.grant("secretary", ANY, ANY).unwrap();

        let error = g
            .propose_definition("secretary", "billing", BILLING, "")
            .await
            .unwrap_err();

        assert!(error.to_string().contains("no place"), "{error}");
    }
}
