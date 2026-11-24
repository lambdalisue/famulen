//! The secretary's name, and which of the two in the brand the owner has.

use crate::store::Store;

const GENDER_KEY: &str = "persona.gender";

/// What the secretary is called until the owner names it: a nickname of
/// Famulen.
const DEFAULT_NAME: &str = "ファム";

/// The secretary's name, which the owner chooses; ファム until then.
pub fn name(store: &Store) -> String {
    store
        .state("persona.name")
        .ok()
        .flatten()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_NAME.to_owned())
}

/// Decides the secretary's icons everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Gender {
    #[default]
    Male,
    Female,
}

impl Gender {
    /// Also the name of the brand's directory with its icons.
    pub fn as_str(self) -> &'static str {
        match self {
            Gender::Male => "male",
            Gender::Female => "female",
        }
    }

    /// What the owner chose; the default until then.
    pub fn load(store: &Store) -> anyhow::Result<Self> {
        Ok(match store.state(GENDER_KEY)?.as_deref() {
            Some("female") => Gender::Female,
            _ => Gender::Male,
        })
    }

    pub fn save(self, store: &Store) -> anyhow::Result<()> {
        store.set_state(GENDER_KEY, self.as_str())
    }
}
