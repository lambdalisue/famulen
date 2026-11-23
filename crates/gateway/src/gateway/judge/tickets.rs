//! The tickets held calls wait on: shown, answered once, or lapsed. The
//! fields of a held call that carry a secret are sealed in the vault until
//! the ticket is answered, so a ticket keeps none in plain form.

use rusqlite::params;
use serde_json::{Map, Value};

use super::super::{DEFINITIONS, Gateway, VAULT};
use super::{Call, OWNER};
use crate::log::LogEntry;
use crate::permissions::{Permission, scope_of};
use crate::store::now;

/// How long a ticket waits for its answer before it lapses.
pub const TICKET_LIFETIME_SECS: i64 = 24 * 60 * 60;

/// The vault entry a held call's secret fields are sealed in.
fn sealed_name(ticket: i64) -> String {
    format!("held:{ticket}")
}

/// Whether a vault entry belongs to a held call rather than to anyone.
pub(in crate::gateway) fn is_held_entry(name: &str) -> bool {
    name.starts_with("held:")
}

/// What a caller reads when its call is held.
pub fn held_message(id: i64) -> String {
    format!(
        "Held for the owner's approval as request #{id}. Do not call it again now: finish what you can without it and end your turn. When the owner answers, you will receive an [approval] input."
    )
}

/// A call held for the owner's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Ticket {
    pub id: i64,
    pub actor: String,
    pub connection: String,
    pub tool: String,
    /// The input as it may be shown: secret fields read `…`.
    pub shown: Value,
    pub reason: String,
    /// `held`, `approved`, `rejected`, `always_allowed` or `lapsed`.
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Approve,
    Reject,
    AlwaysAllow,
}

impl Answer {
    fn status(self) -> &'static str {
        match self {
            Answer::Approve => "approved",
            Answer::Reject => "rejected",
            Answer::AlwaysAllow => "always_allowed",
        }
    }
}

/// A ticket just answered, with what the gateway did on the caller's behalf
/// when the held call cannot be made again by its caller.
#[derive(Debug)]
pub struct Answered {
    pub ticket: Ticket,
    pub ran: Option<anyhow::Result<()>>,
}

impl Gateway {
    pub(super) fn hold(&self, call: &Call, reason: &str) -> anyhow::Result<Ticket> {
        let (mut shown, secrets) = call.without_secrets();
        let sealing = match (secrets.is_empty(), &self.vault) {
            (true, _) => None,
            (false, Some(vault)) => Some(vault),
            (false, None) => {
                anyhow::bail!("a call carrying a secret cannot be held without a vault")
            }
        };
        let input = shown.to_string();
        if let Some(decides) = &call.decides {
            shown = serde_json::json!({ "decides": decides, "call": shown });
        }
        let created_at = now();
        let id = {
            let conn = self.store.conn();
            conn.execute(
                "INSERT INTO tickets (actor, connection, tool, input, shown, reason, created_at)
                 VALUES (?1, ?2, ?3, ?7, ?4, ?5, ?6)",
                params![
                    call.actor,
                    call.connection,
                    call.tool,
                    shown.to_string(),
                    reason,
                    created_at,
                    input
                ],
            )?;
            conn.last_insert_rowid()
        };
        if let Some(vault) = sealing {
            let name = sealed_name(id);
            vault.put(
                &name,
                Value::Object(secrets).to_string(),
                "a held call",
                &call.actor,
            )?;
            self.store.conn().execute(
                "UPDATE tickets SET sealed = ?1 WHERE id = ?2",
                params![name, id],
            )?;
        }
        Ok(Ticket {
            id,
            actor: call.actor.clone(),
            connection: call.connection.clone(),
            tool: call.tool.clone(),
            shown,
            reason: reason.to_owned(),
            status: "held".into(),
            created_at,
        })
    }

    pub(super) fn take_pass(&self, call: &Call) -> anyhow::Result<bool> {
        let conn = self.store.conn();
        let candidates: Vec<(i64, String)> = {
            let mut statement = conn.prepare(
                "SELECT ticket, input FROM passes
                 WHERE actor = ?1 AND connection = ?2 AND tool = ?3 AND used = 0 ORDER BY ticket",
            )?;
            statement
                .query_map(params![call.actor, call.connection, call.tool], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<Result<_, _>>()?
        };
        let Some((ticket, _)) = candidates.into_iter().find(|(_, granted)| {
            serde_json::from_str::<Value>(granted).is_ok_and(|g| g == call.input)
        }) else {
            return Ok(false);
        };
        conn.execute(
            "UPDATE passes SET used = 1 WHERE ticket = ?1",
            params![ticket],
        )?;
        Ok(true)
    }

    /// Tickets new from now on, as they are held.
    pub fn subscribe_held(&self) -> tokio::sync::broadcast::Receiver<Ticket> {
        self.held.subscribe()
    }

    /// The tickets still waiting for an answer, oldest first.
    pub fn held_tickets(&self) -> anyhow::Result<Vec<Ticket>> {
        self.lapse_tickets()?;
        self.tickets_where("status = 'held'")
    }

    pub fn ticket(&self, id: i64) -> anyhow::Result<Option<Ticket>> {
        self.lapse_tickets()?;
        Ok(self.tickets_where(&format!("id = {id}"))?.pop())
    }

    fn tickets_where(&self, filter: &str) -> anyhow::Result<Vec<Ticket>> {
        let conn = self.store.conn();
        let mut statement = conn.prepare(&format!(
            "SELECT id, actor, connection, tool, shown, reason, status, created_at
             FROM tickets WHERE {filter} ORDER BY id"
        ))?;
        let rows = statement.query_map([], |row| {
            Ok((
                Ticket {
                    id: row.get(0)?,
                    actor: row.get(1)?,
                    connection: row.get(2)?,
                    tool: row.get(3)?,
                    shown: Value::Null,
                    reason: row.get(5)?,
                    status: row.get(6)?,
                    created_at: row.get(7)?,
                },
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (mut ticket, shown) = row?;
            ticket.shown = serde_json::from_str(&shown)?;
            Ok(ticket)
        })
        .collect()
    }

    /// The tickets about the question `id` of a connection that the owner is
    /// asked of each time, with how each stands.
    fn decided(&self, connection: &str, id: &str) -> anyhow::Result<Vec<Ticket>> {
        let all = self.tickets_where("shown LIKE '%\"decides\"%'")?;
        Ok(all
            .into_iter()
            .filter(|t| t.connection == connection && t.shown["decides"]["id"] == id)
            .collect())
    }

    /// Take back the held tickets about a question that is asked no more;
    /// their ids.
    pub fn withdraw_decided(&self, connection: &str, id: &str) -> anyhow::Result<Vec<i64>> {
        let mut withdrawn = Vec::new();
        for ticket in self.decided(connection, id)? {
            let changed = self.store.conn().execute(
                "UPDATE tickets SET status = 'lapsed', answered_at = ?1 WHERE id = ?2 AND status = 'held'",
                params![now(), ticket.id],
            )?;
            if changed > 0 {
                self.drop_sealed(ticket.id)?;
                withdrawn.push(ticket.id);
            }
        }
        Ok(withdrawn)
    }

    /// Take back a held ticket whose call is wanted no more. Whether it was
    /// still held.
    pub fn withdraw_ticket(&self, id: i64) -> anyhow::Result<bool> {
        let changed = self.store.conn().execute(
            "UPDATE tickets SET status = 'lapsed', answered_at = ?1 WHERE id = ?2 AND status = 'held'",
            params![now(), id],
        )?;
        if changed > 0 {
            self.drop_sealed(id)?;
        }
        Ok(changed > 0)
    }

    /// The newest ticket of the same call by the same caller, however it
    /// stands.
    pub fn ticket_for(&self, call: &Call) -> anyhow::Result<Option<Ticket>> {
        Ok(self.tickets_where("1")?.into_iter().rev().find(|t| {
            t.actor == call.actor
                && t.connection == call.connection
                && t.tool == call.tool
                && t.shown == call.input
        }))
    }

    /// Whether the owner answered a question through a ticket.
    pub fn answered_decided(&self, connection: &str, id: &str) -> anyhow::Result<bool> {
        Ok(self
            .decided(connection, id)?
            .iter()
            .any(|t| !matches!(t.status.as_str(), "held" | "lapsed")))
    }

    /// Answer the questions a held ticket puts to the owner, once, and make
    /// its call with the answers. `None` when it was answered or lapsed.
    pub async fn fill_ticket(
        &self,
        id: i64,
        replies: Vec<String>,
    ) -> anyhow::Result<Option<Answered>> {
        let Some(ticket) = self.ticket(id)?.filter(|t| t.status == "held") else {
            return Ok(None);
        };
        let questions = self.questions_of(&ticket.connection, &ticket.tool, &ticket.shown)?;
        anyhow::ensure!(
            !questions.is_empty() && questions.len() == replies.len(),
            "there are {} questions to answer",
            questions.len()
        );
        anyhow::ensure!(
            replies.iter().all(|r| !r.trim().is_empty()),
            "a question is not answered"
        );
        let fills = self
            .each_time(&ticket.connection, &ticket.tool)?
            .and_then(|(_, _, each)| each.fills)
            .ok_or_else(|| anyhow::anyhow!("it takes no answers"))?;
        let mut input = self.unsealed_input(id)?;
        input[fills.as_str()] = replies.into();
        self.store.conn().execute(
            "UPDATE tickets SET input = ?1 WHERE id = ?2 AND status = 'held'",
            params![input.to_string(), id],
        )?;
        self.answer_ticket(id, Answer::Approve).await
    }

    /// Answer a held ticket, once. `None` when it was answered or lapsed.
    pub async fn answer_ticket(&self, id: i64, answer: Answer) -> anyhow::Result<Option<Answered>> {
        self.lapse_tickets()?;
        let changed = self.store.conn().execute(
            "UPDATE tickets SET status = ?1, answered_at = ?2 WHERE id = ?3 AND status = 'held'",
            params![answer.status(), now(), id],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        let ticket = self.ticket(id)?.expect("just answered");
        let input = self.unsealed_input(id)?;
        self.drop_sealed(id)?;
        self.log().write(&LogEntry {
            actor: OWNER.into(),
            connection: ticket.connection.clone(),
            tool: ticket.tool.clone(),
            decision: if answer == Answer::Reject {
                "deny"
            } else {
                "allow"
            }
            .into(),
            reason: format!("#{id} {} for {}", answer.status(), ticket.actor),
            outcome: "answered".into(),
            ..LogEntry::default()
        })?;
        let ran = self.carry_out(&ticket, input, answer).await?;
        Ok(Some(Answered { ticket, ran }))
    }

    /// Do what an answer asks for: make the change or the call only the
    /// gateway can make, or let the caller make theirs once. `None` when
    /// there was nothing for the gateway to run.
    async fn carry_out(
        &self,
        ticket: &Ticket,
        input: Value,
        answer: Answer,
    ) -> anyhow::Result<Option<anyhow::Result<()>>> {
        if let Some((_, _, each)) = self.each_time(&ticket.connection, &ticket.tool)? {
            let rejected = answer == Answer::Reject;
            return self
                .carry_out_decided(ticket, input, rejected, each.fills.as_deref())
                .await
                .map(Some);
        }
        if answer == Answer::Reject {
            return Ok(None);
        }
        if ticket.connection == DEFINITIONS {
            return Ok(Some(self.run_held_definition(ticket, &input).await));
        }
        if answer == Answer::AlwaysAllow {
            self.permissions().set_scoped(
                &scope_of(&ticket.connection),
                &ticket.tool,
                Some(Permission::Allow),
            )?;
        }
        if ticket.connection == VAULT {
            return Ok(Some(self.run_held_vault(ticket, &input)));
        }
        self.store.conn().execute(
            "INSERT INTO passes (ticket, actor, connection, tool, input) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                ticket.id,
                ticket.actor,
                ticket.connection,
                ticket.tool,
                input.to_string()
            ],
        )?;
        Ok(None)
    }

    /// Make the call the owner was asked of each time, as they answered, and
    /// log how it went.
    async fn carry_out_decided(
        &self,
        ticket: &Ticket,
        input: Value,
        rejected: bool,
        fills: Option<&str>,
    ) -> anyhow::Result<anyhow::Result<()>> {
        let ran = if !rejected && fills.is_some_and(|fills| input[fills].is_null()) {
            Err(anyhow::anyhow!("its questions were not answered"))
        } else {
            self.run_decided(&ticket.connection, &ticket.tool, input, rejected)
                .await
        };
        self.log().write(&LogEntry {
            actor: ticket.actor.clone(),
            connection: ticket.connection.clone(),
            tool: ticket.tool.clone(),
            decision: if rejected { "deny" } else { "allow" }.into(),
            reason: format!("#{} carried out as the owner answered", ticket.id),
            outcome: if ran.is_ok() { "ok" } else { "error" }.into(),
            ..LogEntry::default()
        })?;
        Ok(ran)
    }

    /// The held input with its secret fields put back.
    fn unsealed_input(&self, id: i64) -> anyhow::Result<Value> {
        let (input, sealed): (String, String) = self.store.conn().query_row(
            "SELECT input, sealed FROM tickets WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let mut input: Value = serde_json::from_str(&input)?;
        if sealed.is_empty() {
            return Ok(input);
        }
        let vault = self
            .vault
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("the held secret cannot be opened without the vault"))?;
        let secrets: Map<String, Value> = match vault.reveal(&sealed)? {
            Some(text) => serde_json::from_str(&text)?,
            None => Map::new(),
        };
        if let Value::Object(fields) = &mut input {
            fields.extend(secrets);
        }
        Ok(input)
    }

    fn drop_sealed(&self, id: i64) -> anyhow::Result<()> {
        if let Some(vault) = &self.vault {
            vault.delete(sealed_name(id))?;
        }
        Ok(())
    }

    /// Let tickets that waited too long lapse, and drop what they sealed.
    pub(super) fn lapse_tickets(&self) -> anyhow::Result<()> {
        let ids: Vec<i64> = {
            let conn = self.store.conn();
            let mut statement =
                conn.prepare("SELECT id FROM tickets WHERE status = 'held' AND created_at < ?1")?;
            statement
                .query_map(params![now() - TICKET_LIFETIME_SECS], |row| row.get(0))?
                .collect::<Result<_, _>>()?
        };
        for id in ids {
            self.store.conn().execute(
                "UPDATE tickets SET status = 'lapsed', answered_at = ?1 WHERE id = ?2",
                params![now(), id],
            )?;
            self.drop_sealed(id)?;
        }
        Ok(())
    }
}
