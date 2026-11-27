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

fn deployable(remote: &str) -> bool {
    repository_name(remote).as_deref() == Some(OWN_REPOSITORY)
}

#[cfg(test)]
mod tests;
