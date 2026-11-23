//! Settings the owner changes on the console while famulen runs. A value
//! chosen there wins over the environment and the config file, which only
//! give the starting value.

use chrono_tz::Tz;

use crate::config::{Claude, EFFORTS};
use crate::store::Store;

/// A setting the console can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Model,
    Effort,
    ClaudeBin,
    Timezone,
}

impl Key {
    pub const ALL: [Key; 4] = [Key::Model, Key::Effort, Key::ClaudeBin, Key::Timezone];

    /// The form field and the database key.
    pub fn name(self) -> &'static str {
        match self {
            Key::Model => "model",
            Key::Effort => "effort",
            Key::ClaudeBin => "claude_bin",
            Key::Timezone => "timezone",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }

    fn storage_key(self) -> String {
        format!("setting.{}", self.name())
    }

    /// Check a value before it is saved. Returns it as stored.
    pub fn check(self, value: &str) -> anyhow::Result<String> {
        let value = value.trim();
        let ok = match self {
            Key::Model => value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '[' | ']')),
            Key::Effort => EFFORTS.contains(&value),
            Key::ClaudeBin => !value.chars().any(char::is_control),
            Key::Timezone => value.parse::<Tz>().is_ok(),
        };
        anyhow::ensure!(ok && !value.is_empty(), "{}", self.hint());
        Ok(value.to_owned())
    }

    fn hint(self) -> String {
        match self {
            Key::Model => "not a model name".into(),
            Key::Effort => format!("effort must be one of {}", EFFORTS.join(", ")),
            Key::ClaudeBin => "not a command".into(),
            Key::Timezone => "a time zone looks like Asia/Tokyo".into(),
        }
    }
}

/// What applies before anything is chosen on the console: the environment, the
/// config file, then the defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct Base {
    pub claude: Claude,
    pub timezone: Tz,
}

impl Base {
    fn value(&self, key: Key) -> Option<String> {
        match key {
            Key::Model => self.claude.model.clone(),
            Key::Effort => self.claude.effort.clone(),
            Key::ClaudeBin => Some(self.claude.bin.clone()),
            Key::Timezone => Some(self.timezone.name().to_owned()),
        }
    }
}

/// Every setting as it applies now.
#[derive(Debug, Clone, PartialEq)]
pub struct Effective {
    pub claude: Claude,
    pub timezone: Tz,
}

/// One setting as the console shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// Chosen on the console.
    pub from_console: bool,
    /// What applies now.
    pub value: Option<String>,
}

#[derive(Clone)]
pub struct Settings {
    store: Store,
    base: Base,
}

impl Settings {
    pub fn new(store: Store, base: Base) -> Self {
        Self { store, base }
    }

    fn chosen(&self, key: Key) -> anyhow::Result<Option<String>> {
        self.store.state(key.storage_key())
    }

    fn get(&self, key: Key) -> anyhow::Result<Option<String>> {
        Ok(self.chosen(key)?.or_else(|| self.base.value(key)))
    }

    pub fn effective(&self) -> anyhow::Result<Effective> {
        let claude = Claude {
            bin: self
                .get(Key::ClaudeBin)?
                .unwrap_or_else(|| self.base.claude.bin.clone()),
            model: self.get(Key::Model)?,
            effort: self.get(Key::Effort)?,
        };
        let timezone = self
            .get(Key::Timezone)?
            .and_then(|tz| tz.parse().ok())
            .unwrap_or(self.base.timezone);
        Ok(Effective { claude, timezone })
    }

    /// The owner's time zone; UTC when the settings cannot be read.
    pub fn timezone(&self) -> Tz {
        self.effective().map(|e| e.timezone).unwrap_or(Tz::UTC)
    }

    pub fn shown(&self, key: Key) -> anyhow::Result<Shown> {
        let chosen = self.chosen(key)?;
        let from_console = chosen.is_some();
        let value = chosen.or_else(|| self.base.value(key));
        Ok(Shown {
            from_console,
            value,
        })
    }

    /// Apply changes from one form: `Some(value)` sets, `None` goes back to
    /// the environment or file's value. Nothing is saved unless every value
    /// is valid.
    pub fn apply(&self, changes: &[(Key, Option<&str>)]) -> anyhow::Result<()> {
        let checked: Vec<(Key, Option<String>)> = changes
            .iter()
            .map(|(key, value)| Ok((*key, value.map(|v| key.check(v)).transpose()?)))
            .collect::<anyhow::Result<_>>()?;
        for (key, value) in checked {
            match value {
                Some(value) => self.store.set_state(key.storage_key(), value)?,
                None => self.store.clear_state(key.storage_key())?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Base {
        Base {
            claude: Claude {
                bin: "claude".into(),
                model: Some("sonnet".into()),
                effort: None,
            },
            timezone: chrono_tz::Asia::Tokyo,
        }
    }

    fn settings() -> Settings {
        Settings::new(Store::open_in_memory().unwrap(), base())
    }

    #[test]
    fn without_a_choice_the_environment_or_files_value_applies() {
        let s = settings();
        assert_eq!(
            s.effective().unwrap().claude.model.as_deref(),
            Some("sonnet")
        );
        assert_eq!(
            s.shown(Key::Model).unwrap(),
            Shown {
                from_console: false,
                value: Some("sonnet".into())
            }
        );
    }

    #[test]
    fn claude_and_the_time_zone_follow_the_site() {
        let s = settings();
        s.apply(&[
            (Key::Model, Some("haiku")),
            (Key::Effort, Some("low")),
            (Key::ClaudeBin, Some("/opt/claude")),
            (Key::Timezone, Some("Europe/Paris")),
        ])
        .unwrap();
        let e = s.effective().unwrap();
        assert_eq!(
            e.claude,
            Claude {
                bin: "/opt/claude".into(),
                model: Some("haiku".into()),
                effort: Some("low".into())
            }
        );
        assert_eq!(e.timezone, chrono_tz::Europe::Paris);
        assert!(s.shown(Key::Model).unwrap().from_console);
    }

    #[test]
    fn clearing_a_choice_goes_back_to_the_environment_or_files_value() {
        let s = settings();
        s.apply(&[(Key::Model, Some("haiku"))]).unwrap();
        s.apply(&[(Key::Model, None)]).unwrap();
        assert_eq!(
            s.effective().unwrap().claude.model.as_deref(),
            Some("sonnet")
        );
    }

    #[test]
    fn nothing_is_saved_when_any_value_is_invalid() {
        let s = settings();
        assert!(
            s.apply(&[(Key::Model, Some("haiku")), (Key::Effort, Some("fast"))])
                .is_err()
        );
        assert_eq!(
            s.effective().unwrap().claude.model.as_deref(),
            Some("sonnet")
        );
    }

    #[test]
    fn values_are_checked_for_their_shape() {
        assert!(
            Key::Model
                .check("opus --dangerously-skip-permissions")
                .is_err()
        );
        assert!(Key::Model.check("claude-opus-5-5[1m]").is_ok());
        assert!(Key::Effort.check("fast").is_err());
        assert!(Key::Timezone.check("Mars/Base").is_err());
        assert!(Key::ClaudeBin.check("claude\nrm").is_err());
    }
}
