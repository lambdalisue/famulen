//! Fixing, as work begins, what its finishing may do.

use std::path::Path;

use super::{Pinned, Repo, Rule};
use crate::workers::finish::repository_name;

/// The only repository whose merged work Famulen puts to use.
const OWN_REPOSITORY: &str = "github.com/lambdalisue/famulen";

/// What the finishing of work on `checkout` is held to, its pull request
/// coming from `branch`; none where the owner allowed none.
pub(crate) async fn pin(
    rules: &[Rule],
    jj: &str,
    checkout: &Path,
    branch: String,
) -> anyhow::Result<Option<Pinned>> {
    let Some(rule) = rules.iter().find(|r| {
        r.repository
            .canonicalize()
            .is_ok_and(|repository| repository == checkout)
    }) else {
        return Ok(None);
    };
    if let Some(key) = Repo::credential_setting(jj, checkout).await? {
        anyhow::bail!(
            "Git's setting {key} holds a key the agent could read: \
             move it to a credential helper that keeps it in the keychain"
        );
    }
    let (remote, target) = Repo::remote(jj, checkout).await?;
    let mut rule = rule.clone();
    rule.deploy = rule.deploy.filter(|_| deployable(&remote));
    Ok(Some(Pinned {
        rule,
        remote,
        target,
        branch,
    }))
}

/// The first setting in a `git config --list -z` that holds a key itself,
/// named without anything of its value.
pub(super) fn credential_key(listing: &str) -> Option<String> {
    listing.split('\0').find_map(|entry| {
        let (key, value) = entry.split_once('\n')?;
        let lower = key.to_ascii_lowercase();
        let last = lower.rsplit('.').next().unwrap_or_default();
        let holds = !value.is_empty()
            && (has_userinfo(value)
                || has_userinfo(key)
                || matches!(last, "extraheader" | "cookiefile" | "smtppass")
                || last.ends_with("token")
                || last.ends_with("password")
                || (last == "helper" && value.contains("password=")));
        holds.then(|| unaddressed(key))
    })
}

/// Whether an http address in `text` says who is asking.
fn has_userinfo(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["http://", "https://"].iter().any(|scheme| {
        lower.match_indices(scheme).any(|(at, _)| {
            lower[at + scheme.len()..]
                .split(['/', '?', '#', ' '])
                .next()
                .is_some_and(|authority| authority.contains('@'))
        })
    })
}

/// The key with the address in its middle left out: it may be the secret.
fn unaddressed(key: &str) -> String {
    match (key.split_once('.'), key.rsplit_once('.')) {
        (Some((first, rest)), Some((_, last))) if rest.contains("://") => {
            format!("{first}.<url>.{last}")
        }
        _ => key.to_owned(),
    }
}

fn deployable(remote: &str) -> bool {
    repository_name(remote).as_deref() == Some(OWN_REPOSITORY)
}

#[cfg(test)]
mod tests;
