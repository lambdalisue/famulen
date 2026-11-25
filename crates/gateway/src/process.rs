//! What programs the gateway (and the program using it) starts may see.

/// The variables of the environment no started program may see: every
/// `FAMULEN_*` one, which carries the vault key and the apps' secrets, and
/// an API key that would bill a model CLI instead of its login.
fn hidden_env(names: impl IntoIterator<Item = String>) -> Vec<String> {
    names
        .into_iter()
        .filter(|name| name.starts_with("FAMULEN_") || name == "ANTHROPIC_API_KEY")
        .collect()
}

/// The names of them that are set.
pub fn hidden_vars() -> Vec<String> {
    hidden_env(std::env::vars_os().filter_map(|(name, _)| name.into_string().ok()))
}

/// Start `command` without [`hidden_vars`].
pub fn hide_secrets(command: &mut tokio::process::Command) {
    for name in hidden_vars() {
        command.env_remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn famulen_own_variables_and_the_api_key_are_kept_from_other_programs() {
        let hidden = hidden_env(
            [
                "FAMULEN_VAULT_KEY",
                "PATH",
                "FAMULEN_SLACK_CLIENT_SECRET",
                "ANTHROPIC_API_KEY",
                "HOME",
            ]
            .map(String::from),
        );
        assert_eq!(
            hidden,
            [
                "FAMULEN_VAULT_KEY",
                "FAMULEN_SLACK_CLIENT_SECRET",
                "ANTHROPIC_API_KEY"
            ]
        );
    }
}
