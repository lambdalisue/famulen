//! Which of the two secretaries in the brand the owner has.

use crate::store::Store;

const GENDER_KEY: &str = "persona.gender";

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
