//! Judging every call: who may call what, under which permission, and whether
//! it runs, waits for the owner or is refused.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use rusqlite::{OptionalExtension, params};
use serde_json::{Map, Value};

use crate::log::{Log, LogEntry};
use crate::permissions::{AutoCourse, Marks, Permission, Permissions, auto_course};
use crate::store::now;

use super::Gateway;

mod tickets;
pub(super) use tickets::is_held_entry;
pub use tickets::{Answer, Answered, TICKET_LIFETIME_SECS, Ticket, held_message};

/// The caller who is the owner: confirmations are not asked of them.
pub const OWNER: &str = "owner";

/// Any connection or any tool, in an actor's grants.
pub const ANY: &str = "*";

/// Who weighs the calls the auto permission leaves to a model, in the log.
pub const ASSESSOR: &str = "assessor";

/// One call to be judged.
#[derive(Debug, Clone, Default)]
pub struct Call {
    pub actor: String,
    pub connection: String,
    pub tool: String,
    pub input: Value,
    /// What the caller makes this call for, in its own words; kept on the
    /// ticket and never judged by.
    pub origin: Option<String>,
    /// What the tool says of itself, for a tool the gateway has not listed,
    /// such as the caller's built-in tools.
    pub marks: Option<Marks>,
    /// Why this call is asked about even when its permission allows it.
    pub ask: Option<String>,
    /// Let this call through a confirm permission without asking; a forbid permission
    /// still stops it.
    pub waive: bool,
    /// Input fields that carry a secret.
    pub secret_fields: Vec<String>,
    /// Why the caller found this call must not run whatever the permissions say,
    /// such as a file tool reaching where no tool may look.
    pub refuse: Option<String>,
    /// Refuse this call where the owner would be asked: its caller cannot
    /// wait for their answer.
    pub never_ask: bool,
    /// What the assessor made of this call, once it was weighed.
    pub assessment: Option<Assessment>,
    /// What the call decides, read from where it is, for the owner to see.
    pub decides: Option<Value>,
    /// Keep all that a command prints, not only its head: the caller reads it
    /// as data, and no model reads it.
    pub whole: bool,
}

/// How the refusal of a call that would have been asked about begins.
pub const NEEDS_OWNER: &str = "this needs the owner's approval, which its caller cannot ask for";

/// What the assessor made of a call the auto permission left to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assessment {
    /// As asked, undoable and kept from other people: it runs.
    Allow { reason: String },
    /// The owner is asked.
    Ask { reason: String },
}

/// Weighs the calls the auto permission leaves to a model. The program using
/// the gateway gives one; without it, those calls are asked about.
pub trait Assessor: Send + Sync {
    fn assess<'a>(&'a self, call: &'a Call) -> BoxFuture<'a, anyhow::Result<Assessment>>;
}

impl Call {
    pub fn new(
        actor: impl Into<String>,
        connection: impl Into<String>,
        tool: impl Into<String>,
        input: Value,
    ) -> Self {
        Self {
            actor: actor.into(),
            connection: connection.into(),
            tool: tool.into(),
            input,
            ..Self::default()
        }
    }

    /// The input with `…` in place of the fields that carry a secret, and
    /// those fields.
    fn without_secrets(&self) -> (Value, Map<String, Value>) {
        let mut shown = self.input.clone();
        let mut secrets = Map::new();
        if let Value::Object(fields) = &mut shown {
            for field in &self.secret_fields {
                if let Some(value) = fields.get_mut(field) {
                    secrets.insert(field.clone(), std::mem::replace(value, "…".into()));
                }
            }
        }
        (shown, secrets)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Allow { reason: String },
    Hold(Ticket),
    Deny { reason: String },
}

/// What became of a call that was not refused.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome<T> {
    Done(T),
    Held(Box<Ticket>),
}

fn unasked(why: &str) -> Verdict {
    Verdict::Deny {
        reason: format!("{NEEDS_OWNER} ({why})"),
    }
}

impl Gateway {
    pub(super) fn permissions(&self) -> Permissions {
        Permissions::new(self.store.clone())
    }

    /// Log a change `actor` made to `connection`.
    pub(super) fn log_change_by(
        &self,
        actor: &str,
        connection: &str,
        change: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        self.log().write(&LogEntry {
            actor: actor.to_owned(),
            connection: connection.to_owned(),
            tool: change.to_owned(),
            decision: "allow".into(),
            reason: reason.to_owned(),
            outcome: "ok".into(),
            ..LogEntry::default()
        })
    }

    /// Choose a permission for a tool under `conn:<id>`, `kind:<kind>` or
    /// [`crate::permissions::BUILTINS`] for `actor`, logged; `None` goes back to the default.
    pub fn set_permission(
        &self,
        actor: impl AsRef<str>,
        scope: impl AsRef<str>,
        tool: impl AsRef<str>,
        permission: Option<Permission>,
    ) -> anyhow::Result<()> {
        let (scope, tool) = (scope.as_ref(), tool.as_ref());
        self.permissions().set_scoped(scope, tool, permission)?;
        let chosen = permission.map_or("default", Permission::as_str);
        self.log_change_by(
            actor.as_ref(),
            scope,
            "permission.set",
            &format!("{tool}: {chosen}"),
        )
    }

    pub(super) fn log(&self) -> Log {
        Log::new(self.store.clone())
    }

    /// Let `actor` call `tool` of `connection`; either may be [`ANY`].
    pub fn grant(
        &self,
        actor: impl AsRef<str>,
        connection: impl AsRef<str>,
        tool: impl AsRef<str>,
    ) -> anyhow::Result<()> {
        self.store.conn().execute(
            "INSERT OR IGNORE INTO actor_grants (actor, connection, tool) VALUES (?1, ?2, ?3)",
            params![actor.as_ref(), connection.as_ref(), tool.as_ref()],
        )?;
        Ok(())
    }

    /// Let what was granted to `actor` hold only before `until`, in seconds
    /// since the epoch.
    pub fn expire(&self, actor: impl AsRef<str>, until: i64) -> anyhow::Result<()> {
        self.store.conn().execute(
            "INSERT INTO actor_deadlines (actor, until) VALUES (?1, ?2)
             ON CONFLICT (actor) DO UPDATE SET until = excluded.until",
            params![actor.as_ref(), until],
        )?;
        Ok(())
    }

    /// `actor` is done with: take back all that was granted to it, what it
    /// still waits for the owner on, and the approvals it has not used. The
    /// ids of the tickets that waited.
    pub fn revoke(&self, actor: impl AsRef<str>) -> anyhow::Result<Vec<i64>> {
        let actor = actor.as_ref();
        self.store
            .conn()
            .execute("DELETE FROM actor_grants WHERE actor = ?1", params![actor])?;
        self.store.conn().execute(
            "DELETE FROM actor_deadlines WHERE actor = ?1",
            params![actor],
        )?;
        let mut withdrawn = Vec::new();
        for ticket in self.tickets_of(actor)? {
            self.drop_pass(ticket.id)?;
            if self.withdraw_ticket(ticket.id)? {
                withdrawn.push(ticket.id);
            }
        }
        Ok(withdrawn)
    }

    /// Whether anything is granted to `actor` now.
    pub fn holds_any(&self, actor: impl AsRef<str>) -> anyhow::Result<bool> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT 1 FROM actor_grants WHERE actor = ?1
                   AND NOT EXISTS (SELECT 1 FROM actor_deadlines
                                   WHERE actor = ?1 AND until <= ?2)",
                params![actor.as_ref(), now()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    fn granted(&self, call: &Call) -> anyhow::Result<bool> {
        if call.actor == OWNER {
            return Ok(true);
        }
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT 1 FROM actor_grants WHERE actor = ?1
                   AND connection IN (?2, '*') AND tool IN (?3, '*')
                   AND NOT EXISTS (SELECT 1 FROM actor_deadlines
                                   WHERE actor = ?1 AND until <= ?4)",
                params![call.actor, call.connection, call.tool, now()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The permission a call falls under and why.
    fn permission_of(&self, call: &Call) -> anyhow::Result<(Permission, String)> {
        let permissions = self.permissions();
        if call.connection == super::DEFINITIONS {
            if let Some((Permission::Forbid, _)) =
                permissions.chosen(&call.connection, &call.tool)?
            {
                return Ok((
                    Permission::Forbid,
                    "permission forbid set on the connection".into(),
                ));
            }
            return Ok((
                Permission::Confirm,
                "a change to a service definition always goes in by the owner's answer".into(),
            ));
        }
        if let Some((permission, source)) = permissions.chosen(&call.connection, &call.tool)? {
            let source = match source {
                crate::permissions::PermissionSource::Connection => "the connection",
                crate::permissions::PermissionSource::Kind => "the kind",
                crate::permissions::PermissionSource::Default => "the default",
            };
            return Ok((
                permission,
                format!("permission {} set on {source}", permission.as_str()),
            ));
        }
        let permission = permissions.default_of(&call.connection, &call.tool)?;
        Ok((
            permission,
            format!("permission {} by default", permission.as_str()),
        ))
    }

    /// What the auto permission does with a call before any model is asked.
    /// A change to the vault is always asked about.
    fn auto_course_of(&self, call: &Call) -> anyhow::Result<AutoCourse> {
        if call.connection == super::VAULT {
            return Ok(AutoCourse::Ask);
        }
        let marks = match call.marks {
            Some(marks) => marks,
            None => self.permissions().marks_of(&call.connection, &call.tool)?,
        };
        Ok(auto_course(marks, &call.tool))
    }

    /// What the auto permission makes of a call: the reason it runs, or the
    /// reason the owner is asked.
    fn auto(&self, call: &Call, why: &str) -> anyhow::Result<Result<String, String>> {
        Ok(match self.auto_course_of(call)? {
            AutoCourse::Run => Ok(format!("{why}: it only reads")),
            AutoCourse::Ask => Err(format!(
                "{why}: it cannot be undone or it reaches other people"
            )),
            AutoCourse::Assess => match &call.assessment {
                Some(Assessment::Allow { reason }) => {
                    Ok(format!("{why}: the assessor let it run: {reason}"))
                }
                Some(Assessment::Ask { reason }) => {
                    Err(format!("{why}: the assessor asks the owner: {reason}"))
                }
                None => Err(format!("{why}: nothing weighed it")),
            },
        })
    }

    /// Whether a call is one the assessor must weigh before it is judged.
    fn needs_assessment(&self, call: &Call) -> anyhow::Result<bool> {
        if call.assessment.is_some()
            || call.actor == OWNER
            || call.waive
            || call.refuse.is_some()
            || !self.granted(call)?
        {
            return Ok(false);
        }
        Ok(self.permission_of(call)?.0 == Permission::Auto
            && self.auto_course_of(call)? == AutoCourse::Assess)
    }

    /// Let a call be weighed by the assessor from now on.
    pub fn set_assessor(&self, assessor: Arc<dyn Assessor>) {
        *crate::lock::write(&self.assessor) = Some(assessor);
    }

    /// What the assessor makes of a call, logged. Secret fields are not shown
    /// to it; without an assessor, or when it fails, the owner is asked.
    async fn assess(&self, call: &Call) -> anyhow::Result<Assessment> {
        let assessor = crate::lock::read(&self.assessor).clone();
        let assessment = match assessor {
            None => Assessment::Ask {
                reason: "no assessor is set".into(),
            },
            Some(assessor) => {
                let mut shown = call.clone();
                shown.input = call.without_secrets().0;
                assessor
                    .assess(&shown)
                    .await
                    .unwrap_or_else(|error| Assessment::Ask {
                        reason: format!("the assessor failed: {error:#}"),
                    })
            }
        };
        let (decision, reason) = match &assessment {
            Assessment::Allow { reason } => ("allow", reason),
            Assessment::Ask { reason } => ("confirm", reason),
        };
        self.log().write(&LogEntry {
            actor: ASSESSOR.into(),
            connection: call.connection.clone(),
            tool: call.tool.clone(),
            decision: decision.into(),
            reason: format!("for {}: {reason}", call.actor),
            outcome: "assessed".into(),
            ..LogEntry::default()
        })?;
        Ok(assessment)
    }

    /// Judge one call as [`Gateway::judge`] does, having the assessor weigh it
    /// first when the auto permission leaves it to the assessor.
    pub async fn judge_assessed(&self, call: &Call) -> anyhow::Result<Verdict> {
        if call.actor != OWNER && call.refuse.is_none() && self.granted(call)? {
            match self
                .decided_by(&call.connection, &call.tool, &call.input)
                .await
            {
                Ok(None) => {}
                Ok(decides) => {
                    return self.judge(&Call {
                        decides,
                        ..call.clone()
                    });
                }
                Err(error) => {
                    return self.judge(&Call {
                        refuse: Some(format!("{error:#}")),
                        ..call.clone()
                    });
                }
            }
        }
        if !self.needs_assessment(call)? {
            return self.judge(call);
        }
        let assessment = self.assess(call).await?;
        self.judge(&Call {
            assessment: Some(assessment),
            ..call.clone()
        })
    }

    /// Judge one call; a refused or held one is logged here. An allowed call
    /// is logged by whoever makes it, with how it went. A call the auto
    /// permission leaves to the assessor is asked about unless it carries the
    /// assessor's answer; [`Gateway::judge_assessed`] gets one.
    pub fn judge(&self, call: &Call) -> anyhow::Result<Verdict> {
        self.lapse_tickets()?;
        let verdict = self.decide(call)?;
        let (decision, reason) = match &verdict {
            Verdict::Allow { reason } => ("allow", reason.clone()),
            Verdict::Hold(ticket) => (
                "confirm",
                format!("held as #{}: {}", ticket.id, ticket.reason),
            ),
            Verdict::Deny { reason } => ("deny", reason.clone()),
        };
        if decision != "allow" {
            self.log().write(&LogEntry {
                actor: call.actor.clone(),
                connection: call.connection.clone(),
                tool: call.tool.clone(),
                decision: decision.into(),
                reason,
                ..LogEntry::default()
            })?;
        }
        if let Verdict::Hold(ticket) = &verdict {
            let _ = self.held.send(ticket.clone());
        }
        Ok(verdict)
    }

    fn decide(&self, call: &Call) -> anyhow::Result<Verdict> {
        if let Some(reason) = &call.refuse {
            return Ok(Verdict::Deny {
                reason: reason.clone(),
            });
        }
        if !self.granted(call)? {
            return Ok(Verdict::Deny {
                reason: format!(
                    "{} is not granted {} of {}",
                    call.actor, call.tool, call.connection
                ),
            });
        }
        let (permission, why) = self.permission_of(call)?;
        if call.actor != OWNER
            && permission != Permission::Forbid
            && self.asks_each_time(&call.connection, &call.tool)?
        {
            if call.never_ask {
                return Ok(unasked("the owner is asked of every call"));
            }
            return Ok(match &call.decides {
                Some(decides) => Verdict::Hold(match self.held_on(call, decides)? {
                    Some(ticket) => ticket,
                    None => self.hold(call, "the owner is asked of every call")?,
                }),
                None => Verdict::Deny {
                    reason: "what it decides was not read".into(),
                },
            });
        }
        let runs = match permission {
            Permission::Forbid => {
                return Ok(Verdict::Deny {
                    reason: format!("{why}: the owner forbade it. Do not try to get around it."),
                });
            }
            Permission::Allow => Ok(why),
            Permission::Confirm => Err(why),
            Permission::Auto => self.auto(call, &why)?,
        };
        let asked = match runs {
            Ok(reason) => match &call.ask {
                Some(ask) if call.actor != OWNER => ask.clone(),
                _ => return Ok(Verdict::Allow { reason }),
            },
            Err(reason) => match &call.ask {
                Some(ask) => format!("{ask}\n{reason}"),
                None => reason,
            },
        };
        if call.actor == OWNER || call.waive {
            return Ok(Verdict::Allow {
                reason: format!("{asked}; not asked of {}", call.actor),
            });
        }
        if call.never_ask {
            return Ok(unasked(&asked));
        }
        if self.take_pass(call)? {
            return Ok(Verdict::Allow {
                reason: format!("{asked}; approved once"),
            });
        }
        Ok(Verdict::Hold(self.hold(call, &asked)?))
    }
}

#[cfg(test)]
mod tests;
