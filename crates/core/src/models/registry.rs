//! Registered models, their assignment to roles with daily caps, and what
//! each call used.

mod usage;

pub use usage::*;

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::config::EFFORTS;
use crate::gateway::Gateway;
use crate::models::{
    self, ModelKind, ModelSpec, Role, Source, Usage, check_assignment, estimate_cost_yen, found_id,
};
use crate::store::{Store, now};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Registered {
    pub spec: ModelSpec,
    /// What a subscription costs per month, for the usage page.
    pub monthly_yen: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapUnit {
    Yen,
    Calls,
}

impl CapUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yen => "yen",
            Self::Calls => "calls",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub role: Role,
    pub model: String,
    pub cap: Option<f64>,
    pub cap_unit: CapUnit,
    /// How much the model thinks, as `claude --effort` takes it; `None` is
    /// the model's default. Only an agent CLI keeps one.
    pub effort: Option<String>,
}

#[derive(Clone)]
pub struct Models {
    store: Store,
    /// What sends a model's call with its key; without it, calls go out
    /// without one.
    gateway: Option<Gateway>,
}

pub fn role_id(role: Role) -> &'static str {
    match role {
        Role::Secretary => "secretary",
        Role::Dream => "dream",
        Role::Lookout => "lookout",
        Role::Triage => "triage",
        Role::Assess => "assess",
        Role::Voice => "voice",
    }
}

pub fn role_from_id(id: &str) -> Option<Role> {
    Role::ALL.iter().copied().find(|r| role_id(*r) == id)
}

/// Who changes the models: the owner, from the console.
const OWNER: &str = "owner";

/// Set for a role once its default model was put in place.
fn seeded_key(role: Role) -> String {
    format!("models.defaults.{}", role_id(role))
}

impl Models {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            gateway: None,
        }
    }

    /// Calls models through `gateway`, which keeps their keys.
    pub fn with_gateway(self, gateway: Gateway) -> Self {
        Self {
            gateway: Some(gateway),
            ..self
        }
    }

    /// The gateway that keeps the models' keys.
    pub fn gateway(&self) -> Option<&Gateway> {
        self.gateway.as_ref()
    }

    pub fn register(&self, spec: ModelSpec, monthly_yen: Option<f64>) -> anyhow::Result<()> {
        anyhow::ensure!(!spec.id.trim().is_empty(), "a model needs an id");
        anyhow::ensure!(!spec.name.trim().is_empty(), "a model needs a name");
        self.store.conn().execute(
            "INSERT INTO models (id, name, spec, monthly_yen) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (id) DO UPDATE SET name = ?2, spec = ?3, monthly_yen = ?4",
            params![
                spec.id,
                spec.name,
                serde_json::to_string(&spec)?,
                monthly_yen
            ],
        )?;
        if let Some(id) = found_as(&spec) {
            self.store
                .conn()
                .execute("UPDATE found_models SET removed = 0 WHERE id = ?1", [id])?;
        }
        Ok(())
    }

    /// Registers what `source` offers now. A model the owner removed stays
    /// removed, and one registered before keeps its fee and its roles. A model
    /// `source` offers no more goes unless a role uses it.
    pub fn take_in(&self, source: Source, offered: Vec<ModelSpec>) -> anyhow::Result<()> {
        let registered = self.registered()?;
        for spec in &offered {
            let removed: Option<bool> = self
                .store
                .conn()
                .query_row(
                    "SELECT removed FROM found_models WHERE id = ?1",
                    [&spec.id],
                    |r| r.get(0),
                )
                .optional()?;
            self.store.conn().execute(
                "INSERT INTO found_models (id, source, spec) VALUES (?1, ?2, ?3)
                 ON CONFLICT (id) DO UPDATE SET spec = ?3, present = 1",
                params![spec.id, source.as_str(), serde_json::to_string(spec)?],
            )?;
            let same = registered
                .iter()
                .find(|m| found_as(&m.spec).as_deref() == Some(&spec.id));
            match same {
                Some(m) if m.spec.id != spec.id => {}
                Some(m) => self.register(spec.clone(), m.monthly_yen)?,
                None if removed == Some(true) => {}
                None => self.register(spec.clone(), None)?,
            }
        }
        let known: Vec<String> = {
            let conn = self.store.conn();
            let mut stmt = conn.prepare("SELECT id FROM found_models WHERE source = ?1")?;
            let rows = stmt.query_map([source.as_str()], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        for id in known {
            if offered.iter().any(|spec| spec.id == id) {
                continue;
            }
            self.store
                .conn()
                .execute("UPDATE found_models SET present = 0 WHERE id = ?1", [&id])?;
            if self.role_using(&id)?.is_none() {
                self.forget(&id)?;
            }
        }
        Ok(())
    }

    /// Whether the registered model `id` is one found on this machine.
    pub fn is_found(&self, id: impl AsRef<str>) -> anyhow::Result<bool> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT 1 FROM found_models WHERE id = ?1",
                [id.as_ref()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The models on this machine that the owner removed.
    pub fn left_out(&self) -> anyhow::Result<Vec<ModelSpec>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT spec FROM found_models WHERE present = 1 AND removed = 1 ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }

    /// Registers a model of [`Models::left_out`] again.
    pub fn bring_back(&self, id: impl AsRef<str>) -> anyhow::Result<()> {
        let spec = self
            .left_out()?
            .into_iter()
            .find(|spec| spec.id == id.as_ref())
            .ok_or_else(|| anyhow::anyhow!("no model {} was removed", id.as_ref()))?;
        self.register(spec, None)
    }

    /// The models each role starts with, once per role: Claude through the
    /// agent CLI, Sonnet for the secretary, Opus for the dream, Haiku for the
    /// lookout, for triage and for assessing, each thinking as deep as its
    /// work needs. A role added later gets its default when it first appears;
    /// a role already assigned keeps its model, and what the owner removes
    /// later is not brought back.
    pub fn seed_defaults(&self) -> anyhow::Result<()> {
        let claude = |id: &str, name: &str, model: &str| ModelSpec {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: models::ModelKind::AgentCli,
            endpoint: None,
            model: model.to_owned(),
            cli: Default::default(),
            price: Default::default(),
        };
        let defaults = [
            (
                claude("claude-sonnet", "Claude (Sonnet)", "sonnet"),
                vec![Role::Secretary],
            ),
            (
                claude("claude-opus", "Claude (Opus)", "opus"),
                vec![Role::Dream],
            ),
            (
                claude("claude-haiku", "Claude (Haiku)", "haiku"),
                vec![Role::Lookout, Role::Triage, Role::Assess],
            ),
        ];
        for (spec, roles) in defaults {
            let mut open = Vec::new();
            for role in roles {
                if self.store.state(seeded_key(role))?.is_some() {
                    continue;
                }
                self.store.set_state(seeded_key(role), "1")?;
                if self.assignment(role)?.is_none() {
                    open.push(role);
                }
            }
            // A model no role needs is not registered, so the list holds only
            // what is used.
            if open.is_empty() {
                continue;
            }
            let id = spec.id.clone();
            if self.model(&id)?.is_none() {
                self.register(spec, None)?;
            }
            for role in open {
                self.assign(role, &id, None, role.default_effort())?;
            }
        }
        Ok(())
    }

    pub fn unregister(&self, id: impl AsRef<str>) -> anyhow::Result<()> {
        let id = id.as_ref();
        anyhow::ensure!(
            self.role_using(id)?.is_none(),
            "model {id} is assigned to a role"
        );
        let spec = self.model(id)?.map(|m| m.spec);
        if let Some(spec) = spec
            && let Some((source, found)) = Source::of(&spec).zip(found_as(&spec))
        {
            // Kept even before it was found, so that finding it later does not
            // bring it back.
            self.store.conn().execute(
                "INSERT INTO found_models (id, source, spec, present, removed)
                 VALUES (?1, ?2, ?3, 0, 1)
                 ON CONFLICT (id) DO UPDATE SET removed = 1",
                params![found, source.as_str(), serde_json::to_string(&spec)?],
            )?;
        }
        self.forget(id)
    }

    fn role_using(&self, id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .store
            .conn()
            .query_row("SELECT role FROM roles WHERE model = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?)
    }

    fn forget(&self, id: &str) -> anyhow::Result<()> {
        self.store
            .conn()
            .execute("DELETE FROM models WHERE id = ?1", [id])?;
        if let Some(gateway) = &self.gateway {
            gateway.remove_model_key(OWNER, id)?;
        }
        Ok(())
    }

    pub fn registered(&self) -> anyhow::Result<Vec<Registered>> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare("SELECT spec, monthly_yen FROM models ORDER BY name")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (spec, monthly_yen) = row?;
            out.push(Registered {
                spec: serde_json::from_str(&spec)?,
                monthly_yen,
            });
        }
        Ok(out)
    }

    pub fn model(&self, id: impl AsRef<str>) -> anyhow::Result<Option<Registered>> {
        Ok(self
            .registered()?
            .into_iter()
            .find(|m| m.spec.id == id.as_ref()))
    }

    pub fn assign(
        &self,
        role: Role,
        model: impl AsRef<str>,
        cap: Option<f64>,
        effort: Option<&str>,
    ) -> anyhow::Result<()> {
        let effort = effort.map(str::trim).filter(|e| !e.is_empty());
        if let Some(effort) = effort {
            anyhow::ensure!(
                EFFORTS.contains(&effort),
                "effort must be one of {}: {effort}",
                EFFORTS.join(", ")
            );
        }
        let registered = self
            .model(model.as_ref())?
            .ok_or_else(|| anyhow::anyhow!("no model {}", model.as_ref()))?;
        check_assignment(role, &registered.spec)?;
        let effort = effort.filter(|_| registered.spec.kind == ModelKind::AgentCli);
        let unit = if registered.spec.price.is_metered() {
            CapUnit::Yen
        } else {
            CapUnit::Calls
        };
        self.store.conn().execute(
            "INSERT INTO roles (role, model, cap, cap_unit, effort) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (role) DO UPDATE SET model = ?2, cap = ?3, cap_unit = ?4, effort = ?5",
            params![
                role_id(role),
                registered.spec.id,
                cap,
                unit.as_str(),
                effort
            ],
        )?;
        Ok(())
    }

    pub fn unassign(&self, role: Role) -> anyhow::Result<()> {
        self.store
            .conn()
            .execute("DELETE FROM roles WHERE role = ?1", [role_id(role)])?;
        Ok(())
    }

    pub fn assignment(&self, role: Role) -> anyhow::Result<Option<Assignment>> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT model, cap, cap_unit, effort FROM roles WHERE role = ?1",
                [role_id(role)],
                |r| {
                    Ok(Assignment {
                        role,
                        model: r.get(0)?,
                        cap: r.get(1)?,
                        cap_unit: if r.get::<_, String>(2)? == "yen" {
                            CapUnit::Yen
                        } else {
                            CapUnit::Calls
                        },
                        effort: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn record(&self, role: Role, model: &ModelSpec, usage: &Usage) -> anyhow::Result<f64> {
        let cost = estimate_cost_yen(&model.price, usage);
        self.store.conn().execute(
            "INSERT INTO usage (at, role, model, input_tokens, output_tokens, calls, cost_yen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                now(),
                role_id(role),
                model.id,
                usage.input_tokens.map(|t| t as i64),
                usage.output_tokens.map(|t| t as i64),
                usage.calls as i64,
                cost
            ],
        )?;
        Ok(cost)
    }

    /// What a role used today (owner's day) in its cap's unit.
    pub fn used_today(
        &self,
        role: Role,
        tz: chrono_tz::Tz,
        at: DateTime<Utc>,
    ) -> anyhow::Result<(f64, i64)> {
        let start = day_start(at, tz);
        let (cost, calls): (f64, i64) = self.store.conn().query_row(
            "SELECT COALESCE(SUM(cost_yen), 0), COALESCE(SUM(calls), 0) FROM usage WHERE role = ?1 AND at >= ?2",
            params![role_id(role), start],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((cost, calls))
    }

    /// Whether a role may make another call today.
    pub fn within_cap(
        &self,
        role: Role,
        tz: chrono_tz::Tz,
        at: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        let Some(assignment) = self.assignment(role)? else {
            return Ok(false);
        };
        let Some(cap) = assignment.cap else {
            return Ok(true);
        };
        let (cost, calls) = self.used_today(role, tz, at)?;
        Ok(match assignment.cap_unit {
            CapUnit::Yen => cost < cap,
            CapUnit::Calls => (calls as f64) < cap,
        })
    }

    /// Run `complete` for a role within its cap and record the usage.
    pub async fn complete(
        &self,
        role: Role,
        tz: chrono_tz::Tz,
        system: &str,
        prompt: &str,
        schema: Option<&serde_json::Value>,
    ) -> anyhow::Result<models::Completion> {
        let (spec, effort) = self.spec_for(role, tz)?;
        let completion = models::complete(
            &spec,
            effort.as_deref(),
            system,
            prompt,
            schema,
            self.via(role),
        )
        .await?;
        self.record(role, &spec, &completion.usage)?;
        Ok(completion)
    }

    /// [`Models::complete`] for a short judgement, through
    /// [`models::complete_brief`].
    pub async fn complete_brief(
        &self,
        role: Role,
        tz: chrono_tz::Tz,
        system: &str,
        prompt: &str,
        schema: Option<&serde_json::Value>,
    ) -> anyhow::Result<models::Completion> {
        let (spec, effort) = self.spec_for(role, tz)?;
        let completion = models::complete_brief(
            &spec,
            effort.as_deref(),
            system,
            prompt,
            schema,
            self.via(role),
        )
        .await?;
        self.record(role, &spec, &completion.usage)?;
        Ok(completion)
    }

    /// Run `classify` for a role within its cap and record the usage.
    pub async fn classify(
        &self,
        role: Role,
        tz: chrono_tz::Tz,
        labels: &[&str],
        instruction: &str,
        item: &str,
    ) -> anyhow::Result<String> {
        let (spec, effort) = self.spec_for(role, tz)?;
        let (label, usage) = models::classify(
            &spec,
            effort.as_deref(),
            labels,
            instruction,
            item,
            self.via(role),
        )
        .await?;
        self.record(role, &spec, &usage)?;
        Ok(label)
    }

    /// The gateway a role's model calls go through, named for the role.
    fn via(&self, role: Role) -> Option<models::Via<'_>> {
        self.gateway.as_ref().map(|gateway| models::Via {
            gateway,
            actor: role_id(role),
        })
    }

    /// The model a role calls now, with the effort chosen for it.
    fn spec_for(
        &self,
        role: Role,
        tz: chrono_tz::Tz,
    ) -> anyhow::Result<(ModelSpec, Option<String>)> {
        let assignment = self
            .assignment(role)?
            .ok_or_else(|| anyhow::anyhow!("no model is assigned to {}", role.label()))?;
        anyhow::ensure!(
            self.within_cap(role, tz, Utc::now())?,
            "{} reached today's cap",
            role.label()
        );
        let spec = self
            .model(&assignment.model)?
            .ok_or_else(|| anyhow::anyhow!("model {} is gone", assignment.model))?
            .spec;
        Ok((spec, assignment.effort))
    }
}

/// The id `spec` has when it is found on this machine.
fn found_as(spec: &ModelSpec) -> Option<String> {
    Source::of(spec).map(|source| found_id(source, &spec.model))
}

fn day_start(at: DateTime<Utc>, tz: chrono_tz::Tz) -> i64 {
    let local = at.with_timezone(&tz).date_naive();
    tz.from_local_datetime(&local.and_hms_opt(0, 0, 0).unwrap_or_default())
        .earliest()
        .map(|d| d.timestamp())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ModelKind, Price};

    fn spec(id: &str, kind: ModelKind, price: Price) -> ModelSpec {
        ModelSpec {
            id: id.into(),
            name: id.into(),
            kind,
            endpoint: None,
            model: "m".into(),
            cli: Default::default(),
            price,
        }
    }

    fn models() -> Models {
        let b = Models::new(Store::open_in_memory().unwrap());
        b.register(
            spec("claude", ModelKind::AgentCli, Price::default()),
            Some(15000.0),
        )
        .unwrap();
        b.register(
            spec(
                "api",
                ModelKind::Api,
                Price {
                    per_call_yen: Some(10.0),
                    ..Default::default()
                },
            ),
            None,
        )
        .unwrap();
        b.register(spec("local", ModelKind::Local, Price::default()), None)
            .unwrap();
        b
    }

    fn offered(model: &str, name: &str) -> ModelSpec {
        ModelSpec {
            id: format!("claude-{model}"),
            name: name.into(),
            model: model.into(),
            ..spec("", ModelKind::AgentCli, Price::default())
        }
    }

    fn empty() -> Models {
        Models::new(Store::open_in_memory().unwrap())
    }

    fn ids(b: &Models) -> Vec<String> {
        b.registered()
            .unwrap()
            .into_iter()
            .map(|m| m.spec.id)
            .collect()
    }

    #[test]
    fn what_this_machine_offers_is_registered_and_marked_as_found() {
        let b = empty();
        b.register(spec("mine", ModelKind::Api, Price::default()), None)
            .unwrap();
        b.take_in(Source::Claude, vec![offered("opus", "Claude Opus 5.5")])
            .unwrap();
        assert_eq!(ids(&b), ["claude-opus", "mine"]);
        assert!(b.is_found("claude-opus").unwrap());
        assert!(!b.is_found("mine").unwrap());
    }

    #[test]
    fn a_model_the_owner_removed_is_not_brought_back_until_they_ask() {
        let b = empty();
        b.take_in(Source::Claude, vec![offered("opus", "Opus")])
            .unwrap();
        b.unregister("claude-opus").unwrap();
        b.take_in(Source::Claude, vec![offered("opus", "Opus")])
            .unwrap();
        assert!(ids(&b).is_empty());
        let left: Vec<_> = b.left_out().unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(left, ["claude-opus"]);

        b.bring_back("claude-opus").unwrap();
        b.take_in(Source::Claude, vec![offered("opus", "Opus")])
            .unwrap();
        assert_eq!(ids(&b), ["claude-opus"]);
        assert!(b.left_out().unwrap().is_empty());
    }

    #[test]
    fn a_model_removed_before_it_was_found_stays_removed() {
        let b = empty();
        b.seed_defaults().unwrap();
        b.unassign(Role::Dream).unwrap();
        b.unregister("claude-opus").unwrap();
        b.take_in(Source::Claude, vec![offered("opus", "Opus")])
            .unwrap();
        assert!(b.model("claude-opus").unwrap().is_none());
    }

    #[test]
    fn finding_again_renews_the_name_and_moves_no_role() {
        let b = empty();
        b.seed_defaults().unwrap();
        let kept = b.model("claude-opus").unwrap().unwrap().spec;
        b.register(kept, Some(3000.0)).unwrap();
        let before: Vec<_> = Role::ALL.map(|r| b.assignment(r).unwrap()).into();
        b.take_in(
            Source::Claude,
            vec![offered("opus", "Claude Opus 5.5"), offered("new", "New")],
        )
        .unwrap();
        let opus = b.model("claude-opus").unwrap().unwrap();
        assert_eq!(opus.spec.name, "Claude Opus 5.5");
        assert_eq!(opus.monthly_yen, Some(3000.0));
        let after: Vec<_> = Role::ALL.map(|r| b.assignment(r).unwrap()).into();
        assert_eq!(after, before);
    }

    #[test]
    fn a_model_offered_no_more_goes_unless_a_role_uses_it() {
        let b = empty();
        b.take_in(
            Source::Claude,
            vec![offered("opus", "Opus"), offered("old", "Old")],
        )
        .unwrap();
        b.assign(Role::Dream, "claude-old", None, None).unwrap();
        b.take_in(Source::Claude, vec![offered("opus", "Opus")])
            .unwrap();
        assert_eq!(ids(&b), ["claude-old", "claude-opus"]);

        b.assign(Role::Dream, "claude-opus", None, None).unwrap();
        b.take_in(Source::Claude, vec![offered("opus", "Opus")])
            .unwrap();
        assert_eq!(ids(&b), ["claude-opus"]);
        assert!(
            b.left_out().unwrap().is_empty(),
            "the owner did not remove it"
        );

        b.take_in(
            Source::Claude,
            vec![offered("opus", "Opus"), offered("old", "Old")],
        )
        .unwrap();
        assert!(b.model("claude-old").unwrap().is_some());
    }

    #[test]
    fn one_source_leaves_the_models_of_another_alone() {
        let b = empty();
        b.take_in(Source::Claude, vec![offered("opus", "Opus")])
            .unwrap();
        b.take_in(Source::Codex, Vec::new()).unwrap();
        b.take_in(Source::Ollama, Vec::new()).unwrap();
        assert_eq!(ids(&b), ["claude-opus"]);
    }

    #[test]
    fn a_model_the_owner_registered_themselves_is_not_doubled_or_renamed() {
        let b = empty();
        let mine = ModelSpec {
            id: "mine".into(),
            ..offered("opus", "Mine")
        };
        b.register(mine, None).unwrap();
        b.take_in(Source::Claude, vec![offered("opus", "Opus")])
            .unwrap();
        assert_eq!(ids(&b), ["mine"]);
        assert_eq!(b.model("mine").unwrap().unwrap().spec.name, "Mine");
    }

    #[test]
    fn the_secretary_takes_only_an_agent_cli() {
        let b = models();
        assert!(b.assign(Role::Secretary, "local", None, None).is_err());
        b.assign(Role::Secretary, "claude", None, None).unwrap();
    }

    #[test]
    fn the_effort_is_kept_with_an_agent_cli_assignment_and_dropped_for_others() {
        let b = models();
        b.assign(Role::Dream, "claude", None, Some("max")).unwrap();
        b.assign(Role::Triage, "local", None, Some("max")).unwrap();
        b.assign(Role::Secretary, "claude", None, None).unwrap();
        let effort_of = |role| b.assignment(role).unwrap().unwrap().effort;
        assert_eq!(effort_of(Role::Dream).as_deref(), Some("max"));
        assert_eq!(effort_of(Role::Triage), None);
        assert_eq!(effort_of(Role::Secretary), None);
    }

    #[test]
    fn an_unknown_effort_is_refused() {
        let b = models();
        assert!(b.assign(Role::Dream, "claude", None, Some("fast")).is_err());
        assert!(b.assignment(Role::Dream).unwrap().is_none());
    }

    #[test]
    fn a_metered_model_is_capped_in_yen_and_others_by_calls() {
        let b = models();
        b.assign(Role::Lookout, "api", Some(25.0), None).unwrap();
        b.assign(Role::Dream, "local", Some(2.0), None).unwrap();
        assert_eq!(
            b.assignment(Role::Lookout).unwrap().unwrap().cap_unit,
            CapUnit::Yen
        );
        assert_eq!(
            b.assignment(Role::Dream).unwrap().unwrap().cap_unit,
            CapUnit::Calls
        );
    }

    #[test]
    fn a_role_at_its_cap_is_not_called_again() {
        let b = models();
        let tz = chrono_tz::Asia::Tokyo;
        b.assign(Role::Lookout, "api", Some(25.0), None).unwrap();
        let api = b.model("api").unwrap().unwrap().spec;
        let call = Usage {
            calls: 1,
            ..Default::default()
        };
        b.record(Role::Lookout, &api, &call).unwrap();
        b.record(Role::Lookout, &api, &call).unwrap();
        assert!(b.within_cap(Role::Lookout, tz, Utc::now()).unwrap());
        b.record(Role::Lookout, &api, &call).unwrap();
        assert!(!b.within_cap(Role::Lookout, tz, Utc::now()).unwrap());
    }

    #[test]
    fn a_new_installation_starts_with_claude_and_keeps_the_owners_choices() {
        let b = Models::new(Store::open_in_memory().unwrap());
        b.seed_defaults().unwrap();
        let model_of = |role| b.assignment(role).unwrap().unwrap().model;
        assert_eq!(model_of(Role::Secretary), "claude-sonnet");
        assert_eq!(model_of(Role::Dream), "claude-opus");
        assert_eq!(model_of(Role::Lookout), "claude-haiku");
        assert_eq!(model_of(Role::Triage), "claude-haiku");
        assert_eq!(b.model("claude-opus").unwrap().unwrap().spec.model, "opus");
        let effort_of = |role| b.assignment(role).unwrap().unwrap().effort;
        assert_eq!(effort_of(Role::Secretary).as_deref(), Some("medium"));
        assert_eq!(effort_of(Role::Dream).as_deref(), Some("xhigh"));
        assert_eq!(effort_of(Role::Lookout).as_deref(), Some("medium"));
        assert_eq!(effort_of(Role::Triage).as_deref(), Some("low"));
        assert_eq!(model_of(Role::Assess), "claude-haiku");
        assert_eq!(effort_of(Role::Assess).as_deref(), Some("low"));
        b.assign(Role::Dream, "claude-haiku", None, None).unwrap();
        b.seed_defaults().unwrap();
        assert_eq!(model_of(Role::Dream), "claude-haiku", "seeded once");
    }

    #[test]
    fn a_role_added_later_gets_its_default_once_and_the_others_stay_as_left() {
        let b = Models::new(Store::open_in_memory().unwrap());
        b.seed_defaults().unwrap();
        b.unassign(Role::Dream).unwrap();
        b.unassign(Role::Assess).unwrap();
        b.store.clear_state(seeded_key(Role::Assess)).unwrap();

        b.seed_defaults().unwrap();

        assert!(b.assignment(Role::Dream).unwrap().is_none());
        assert_eq!(
            b.assignment(Role::Assess).unwrap().unwrap().model,
            "claude-haiku"
        );
    }

    #[test]
    fn defaults_fill_only_the_roles_left_open() {
        let b = Models::new(Store::open_in_memory().unwrap());
        b.register(
            ModelSpec {
                id: "mine".into(),
                name: "Mine".into(),
                kind: models::ModelKind::AgentCli,
                endpoint: None,
                model: "x".into(),
                cli: Default::default(),
                price: Default::default(),
            },
            None,
        )
        .unwrap();
        b.assign(Role::Dream, "mine", None, None).unwrap();
        b.seed_defaults().unwrap();
        assert_eq!(b.assignment(Role::Dream).unwrap().unwrap().model, "mine");
        assert_eq!(
            b.assignment(Role::Secretary).unwrap().unwrap().model,
            "claude-sonnet"
        );
        assert!(
            b.model("claude-opus").unwrap().is_none(),
            "a default no role needs is not added"
        );
    }

    #[test]
    fn an_assigned_model_cannot_be_unregistered() {
        let b = models();
        b.assign(Role::Dream, "local", None, None).unwrap();
        assert!(b.unregister("local").is_err());
        b.unregister("api").unwrap();
    }
}
