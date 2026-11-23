//! Receivers: the gateway opens the doors its connections' definitions name,
//! checks what comes in, and hands it untouched to the destination chosen
//! for the connection's receiver. It never reads what arrived.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::Gateway;
use crate::store::now;

mod http;
mod lines;
mod opened;
pub(super) use opened::{Endpoints, Running};

/// Something that arrived, as it arrived.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrival {
    /// What it is about, for a receiver that tells (`per`).
    pub about: String,
    /// What was handed over about it before holds no more; `payload` is how
    /// it stands now.
    pub over: bool,
    pub connection: String,
    /// The name of the receiver it came to, as the definition names it.
    pub receiver: String,
    pub received_at: i64,
    /// The request's headers, for what came to the HTTP receiver.
    pub headers: BTreeMap<String, String>,
    /// The body or the event, as JSON when it reads as JSON.
    pub payload: Value,
}

/// One thing a receiver took in, as the log keeps it: never its content.
#[derive(Debug, Clone, PartialEq)]
pub struct Receipt {
    pub at: i64,
    pub connection: String,
    pub receiver: String,
    pub verified: bool,
    /// The SHA-256 of the payload, in hex.
    pub digest: String,
    pub size: i64,
}

impl Arrival {
    /// What arrived now at `receiver` of `connection`, about nothing told.
    fn new(connection: &str, receiver: &str, payload: Value) -> Self {
        Self {
            about: String::new(),
            over: false,
            connection: connection.to_owned(),
            receiver: receiver.to_owned(),
            received_at: now(),
            headers: BTreeMap::new(),
            payload,
        }
    }
}

/// A destination: who takes what arrives for the receivers sent to it.
pub type Destination = Arc<dyn Fn(Arrival) + Send + Sync>;

#[derive(Clone, Default)]
pub(super) struct Destinations {
    by_name: Arc<RwLock<HashMap<String, Destination>>>,
}

/// The destination a receiver closed by choice is kept under.
const NOWHERE: &str = "";

impl Gateway {
    /// Let `name` take what arrives for the receivers sent to it.
    pub fn register_destination(&self, name: impl Into<String>, to: Destination) {
        crate::lock::write(&self.destinations.by_name).insert(name.into(), to);
    }

    /// The destinations a receiver can be sent to, by name.
    pub fn destination_names(&self) -> Vec<String> {
        let mut names: Vec<String> = crate::lock::read(&self.destinations.by_name)
            .keys()
            .cloned()
            .collect();
        names.sort();
        names
    }

    /// Send what arrives at `receiver` of `connection` to `name`, or nowhere,
    /// as `actor` chose; the change is logged.
    pub fn set_destination(
        &self,
        actor: &str,
        connection: &str,
        receiver: &str,
        name: Option<&str>,
    ) -> anyhow::Result<()> {
        if self.connections.get(connection)?.is_some() {
            anyhow::ensure!(
                self.receivers_of(connection)?
                    .iter()
                    .any(|(r, _)| r == receiver),
                "{connection} has no receiver {receiver}"
            );
        }
        self.log_change_by(
            actor,
            connection,
            "destination.set",
            &format!("{receiver} → {}", name.unwrap_or("nowhere")),
        )?;
        self.store.conn().execute(
            "INSERT INTO destinations (connection, receiver, name) VALUES (?1, ?2, ?3)
             ON CONFLICT (connection, receiver) DO UPDATE SET name = ?3",
            params![connection, receiver, name.unwrap_or(NOWHERE)],
        )?;
        Ok(())
    }

    /// Where what arrives at `receiver` of `connection` goes; `None` when it
    /// goes nowhere, so the receiver is not opened.
    pub fn destination_of(
        &self,
        connection: &str,
        receiver: &str,
    ) -> anyhow::Result<Option<String>> {
        Ok(self
            .chosen_destination(connection, receiver)?
            .filter(|n| n != NOWHERE))
    }

    /// Whether a destination was chosen for the receiver, nowhere included.
    pub fn destination_chosen(&self, connection: &str, receiver: &str) -> anyhow::Result<bool> {
        Ok(self.chosen_destination(connection, receiver)?.is_some())
    }

    fn chosen_destination(
        &self,
        connection: &str,
        receiver: &str,
    ) -> anyhow::Result<Option<String>> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT name FROM destinations WHERE connection = ?1 AND receiver = ?2",
                params![connection, receiver],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The receivers a connection's definition names, each with where it is
    /// sent.
    pub fn receivers_of(&self, connection: &str) -> anyhow::Result<Vec<(String, Option<String>)>> {
        let Some(c) = self.connections.get(connection)? else {
            return Ok(Vec::new());
        };
        let Some(Ok(def)) = self.definition_of(&c) else {
            return Ok(Vec::new());
        };
        def.receive
            .keys()
            .map(|r| Ok((r.clone(), self.destination_of(connection, r)?)))
            .collect()
    }

    /// The destination a connection's definition suggests for one of its
    /// receivers: a name for the program using the gateway to choose, never
    /// chosen by the gateway itself.
    pub fn suggested_destination(
        &self,
        connection: &str,
        receiver: &str,
    ) -> anyhow::Result<Option<String>> {
        let Some(c) = self.connections.get(connection)? else {
            return Ok(None);
        };
        let Some(Ok(def)) = self.definition_of(&c) else {
            return Ok(None);
        };
        Ok(def
            .receive
            .get(receiver)
            .and_then(|r| r.destination.clone()))
    }

    /// Hand what arrived to its receiver's destination, and log it.
    fn deliver(&self, arrival: Arrival) -> anyhow::Result<bool> {
        let name = self.destination_of(&arrival.connection, &arrival.receiver)?;
        let to = name.as_ref().and_then(|n| {
            crate::lock::read(&self.destinations.by_name)
                .get(n)
                .cloned()
        });
        if !arrival.over {
            self.log_receipt(
                &arrival.connection,
                &arrival.receiver,
                &arrival.payload,
                true,
            )?;
        }
        match to {
            Some(to) => {
                to(arrival);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn log_receipt(
        &self,
        connection: &str,
        receiver: &str,
        payload: &Value,
        verified: bool,
    ) -> anyhow::Result<()> {
        let text = payload.to_string();
        let digest = Sha256::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        self.store.conn().execute(
            "INSERT INTO receipts (at, connection, receiver, verified, digest, size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                now(),
                connection,
                receiver,
                verified,
                digest,
                text.len() as i64
            ],
        )?;
        Ok(())
    }

    /// The receipts kept, newest first.
    pub fn receipts(&self, limit: usize) -> anyhow::Result<Vec<Receipt>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT at, connection, receiver, verified, digest, size
             FROM receipts ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(Receipt {
                at: r.get(0)?,
                connection: r.get(1)?,
                receiver: r.get(2)?,
                verified: r.get(3)?,
                digest: r.get(4)?,
                size: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests;
