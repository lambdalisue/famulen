use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// How a registered model is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    AgentCli,
    Local,
    Api,
    Judge,
    Transcribe,
}

impl ModelKind {
    pub const ALL: [ModelKind; 5] = [
        ModelKind::AgentCli,
        ModelKind::Local,
        ModelKind::Api,
        ModelKind::Judge,
        ModelKind::Transcribe,
    ];

    /// The name the owner reads in the console.
    pub fn label(self) -> &'static str {
        match self {
            ModelKind::AgentCli => "エージェント CLI",
            ModelKind::Local => "ローカル",
            ModelKind::Api => "API",
            ModelKind::Judge => "判定 API",
            ModelKind::Transcribe => "文字起こし",
        }
    }
}

/// The program an agent CLI runs, which decides how it is called.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cli {
    #[default]
    Claude,
    Codex,
}

impl Cli {
    /// The executable run when the model names none.
    pub fn executable(self) -> &'static str {
        match self {
            Cli::Claude => "claude",
            Cli::Codex => "codex",
        }
    }

    /// The name the owner reads in the console.
    pub fn label(self) -> &'static str {
        match self {
            Cli::Claude => "Claude Code",
            Cli::Codex => "Codex",
        }
    }
}

/// What a call costs. All `None` means a subscription or a free model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub input_per_mtok_yen: Option<f64>,
    pub output_per_mtok_yen: Option<f64>,
    pub per_call_yen: Option<f64>,
}

impl Price {
    /// Whether calls are paid per use, so a daily cap is set in yen.
    pub fn is_metered(&self) -> bool {
        self.input_per_mtok_yen.is_some()
            || self.output_per_mtok_yen.is_some()
            || self.per_call_yen.is_some()
    }
}

/// A model the owner registered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSpec {
    pub id: String,
    pub name: String,
    pub kind: ModelKind,
    /// The base URL of an OpenAI-compatible API, or the executable of an
    /// agent CLI.
    pub endpoint: Option<String>,
    pub model: String,
    /// Read only for an agent CLI.
    pub cli: Cli,
    pub price: Price,
}

impl ModelSpec {
    /// The executable of an agent CLI.
    pub fn executable(&self) -> &str {
        self.endpoint
            .as_deref()
            .unwrap_or_else(|| self.cli.executable())
    }
}

/// A job a registered model is assigned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Secretary,
    Dream,
    Lookout,
    Triage,
    /// Weighs the calls the auto permission leaves to a model.
    Assess,
    /// Does one delegated task.
    Worker,
    Voice,
}

impl Role {
    pub const ALL: [Role; 7] = [
        Role::Secretary,
        Role::Dream,
        Role::Lookout,
        Role::Triage,
        Role::Assess,
        Role::Worker,
        Role::Voice,
    ];

    /// How deep the role thinks until the owner picks otherwise, as
    /// `claude --effort` takes it; `None` leaves it to the model.
    pub fn default_effort(self) -> Option<&'static str> {
        match self {
            Role::Secretary | Role::Lookout => Some("medium"),
            Role::Dream => Some("xhigh"),
            Role::Triage | Role::Assess => Some("low"),
            Role::Worker | Role::Voice => None,
        }
    }

    /// The name the owner reads in the console.
    pub fn label(self) -> &'static str {
        match self {
            Role::Secretary => "秘書",
            Role::Dream => "夢",
            Role::Lookout => "見張り",
            Role::Triage => "トリアージ",
            Role::Assess => "見極め",
            Role::Worker => "手代",
            Role::Voice => "声の聞き取り",
        }
    }

    /// The kinds of model this role can be assigned, in the order the
    /// console offers them.
    pub fn allowed_kinds(self) -> &'static [ModelKind] {
        use ModelKind::*;
        match self {
            Role::Secretary | Role::Worker => &[AgentCli],
            Role::Dream => &[AgentCli, Local, Api],
            Role::Lookout => &[AgentCli, Local, Api],
            Role::Triage => &[Judge, AgentCli, Local, Api],
            Role::Assess => &[AgentCli, Local, Api],
            Role::Voice => &[Transcribe],
        }
    }

    /// Whether `spec` can be assigned to this role.
    pub fn accepts(self, spec: &ModelSpec) -> bool {
        check_assignment(self, spec).is_ok()
    }
}

/// Refuses to assign `spec` to `role` when its kind does not fit the role.
pub fn check_assignment(role: Role, spec: &ModelSpec) -> Result<()> {
    let allowed = role.allowed_kinds();
    if !allowed.contains(&spec.kind) {
        let allowed = allowed
            .iter()
            .map(|kind| kind.label())
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "{} is a {} model; {} accepts only: {allowed}",
            spec.name,
            spec.kind.label(),
            role.label(),
        );
    }
    // Their tool calls are judged over Claude Code's own protocol.
    if matches!(role, Role::Secretary | Role::Worker) && spec.cli != Cli::Claude {
        bail!(
            "{} runs on {}; {} works only through {}",
            spec.name,
            spec.cli.label(),
            role.label(),
            Cli::Claude.label(),
        );
    }
    Ok(())
}

/// What one or more calls used. Tokens are `None` when the model did not
/// report them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub calls: u64,
}

/// Estimates what `usage` cost at `price`. Unreported tokens count as none.
pub fn estimate_cost_yen(price: &Price, usage: &Usage) -> f64 {
    let per_token = |tokens: Option<u64>, per_mtok: Option<f64>| {
        tokens.unwrap_or(0) as f64 * per_mtok.unwrap_or(0.0) / 1_000_000.0
    };
    per_token(usage.input_tokens, price.input_per_mtok_yen)
        + per_token(usage.output_tokens, price.output_per_mtok_yen)
        + usage.calls as f64 * price.per_call_yen.unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: ModelKind) -> ModelSpec {
        ModelSpec {
            id: "m1".into(),
            name: "test model".into(),
            kind,
            endpoint: None,
            model: "m".into(),
            cli: Default::default(),
            price: Price::default(),
        }
    }

    #[test]
    fn each_role_accepts_exactly_the_kinds_in_the_spec_table() {
        use ModelKind::*;
        let table: [(Role, &[ModelKind]); 7] = [
            (Role::Secretary, &[AgentCli]),
            (Role::Dream, &[AgentCli, Local, Api]),
            (Role::Lookout, &[AgentCli, Local, Api]),
            (Role::Triage, &[Judge, AgentCli, Local, Api]),
            (Role::Assess, &[AgentCli, Local, Api]),
            (Role::Worker, &[AgentCli]),
            (Role::Voice, &[Transcribe]),
        ];
        for (role, allowed) in table {
            for kind in ModelKind::ALL {
                assert_eq!(
                    check_assignment(role, &spec(kind)).is_ok(),
                    allowed.contains(&kind),
                    "{role:?} / {kind:?}",
                );
            }
        }
    }

    #[test]
    fn secretary_accepts_only_an_agent_cli() {
        assert!(check_assignment(Role::Secretary, &spec(ModelKind::AgentCli)).is_ok());
        assert!(check_assignment(Role::Secretary, &spec(ModelKind::Api)).is_err());
    }

    #[test]
    fn codex_serves_every_role_an_agent_cli_can_but_the_secretary_and_the_worker() {
        let codex = ModelSpec {
            cli: Cli::Codex,
            ..spec(ModelKind::AgentCli)
        };
        for role in Role::ALL {
            assert_eq!(
                role.accepts(&codex),
                !matches!(role, Role::Secretary | Role::Worker)
                    && role.allowed_kinds().contains(&ModelKind::AgentCli),
                "{role:?}"
            );
        }
    }

    #[test]
    fn refusal_names_the_model_the_role_and_the_accepted_kinds() {
        let error = check_assignment(Role::Voice, &spec(ModelKind::Local))
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "test model is a ローカル model; 声の聞き取り accepts only: 文字起こし"
        );
    }

    #[test]
    fn labels_are_the_names_the_owner_reads() {
        let kinds: Vec<_> = ModelKind::ALL.iter().map(|k| k.label()).collect();
        assert_eq!(
            kinds,
            [
                "エージェント CLI",
                "ローカル",
                "API",
                "判定 API",
                "文字起こし"
            ]
        );
        let roles: Vec<_> = Role::ALL.iter().map(|r| r.label()).collect();
        assert_eq!(
            roles,
            [
                "秘書",
                "夢",
                "見張り",
                "トリアージ",
                "見極め",
                "手代",
                "声の聞き取り"
            ]
        );
    }

    #[test]
    fn cost_is_estimated_from_tokens_and_calls() {
        let per_token = Price {
            input_per_mtok_yen: Some(100.0),
            output_per_mtok_yen: Some(400.0),
            per_call_yen: None,
        };
        let per_call = Price {
            per_call_yen: Some(0.5),
            ..Price::default()
        };
        let usage = |input, output, calls| Usage {
            input_tokens: input,
            output_tokens: output,
            calls,
        };
        let table = [
            (per_token, usage(Some(1_000_000), Some(500_000), 1), 300.0),
            (per_token, usage(Some(2_000), None, 1), 0.2),
            (per_token, usage(None, None, 3), 0.0),
            (per_call, usage(Some(1_000_000), Some(1_000_000), 4), 2.0),
            (
                Price::default(),
                usage(Some(1_000_000), Some(1_000_000), 9),
                0.0,
            ),
        ];
        for (price, usage, expected) in table {
            let cost = estimate_cost_yen(&price, &usage);
            assert!(
                (cost - expected).abs() < 1e-9,
                "{price:?} {usage:?}: {cost}"
            );
        }
    }

    #[test]
    fn only_a_priced_model_is_metered() {
        assert!(!Price::default().is_metered());
        assert!(
            Price {
                per_call_yen: Some(1.0),
                ..Price::default()
            }
            .is_metered()
        );
    }

    #[test]
    fn spec_round_trips_through_json_with_snake_case_kind() {
        let spec = spec(ModelKind::AgentCli);
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json["kind"], "agent_cli");
        assert_eq!(serde_json::from_value::<ModelSpec>(json).unwrap(), spec);
    }
}
