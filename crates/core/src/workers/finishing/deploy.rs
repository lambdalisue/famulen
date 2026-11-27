//! Putting what was merged to use. The command runs the merged code with
//! nothing keeping it in, and goes on whatever becomes of Famulen: it may
//! well be what restarts it.

use std::fs::{File, TryLockError};
use std::path::Path;
use std::process::Stdio;

use anyhow::Context as _;
use tokio::process::Command;

use super::check::tail;
use super::repo::{Repo, output};

/// Runs the command, and keeps what it wrote and how it ended. The input of
/// this shell is the lock, held for as long as it lives and kept from the
/// command, which may leave a daemon behind.
const WRAPPER: &str = r#"/bin/sh -c "$1" </dev/null >"$2/log" 2>&1; echo $? >"$2/status.tmp" && mv "$2/status.tmp" "$2/status""#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    Yes,
    /// Another deployment is under way, and nothing was touched.
    Busy,
}

/// Move the checkout onto what was merged and start `command` there, one
/// deployment at a time: `lock` is held from before the checkout is touched
/// until the command ends, whatever becomes of Famulen. What it comes to is
/// kept in `kept`. A checkout on the target that holds the work already, as
/// after a later merge was deployed, is not moved back.
/// Where deploying runs, and what it takes up.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Source<'a> {
    pub jj: &'a str,
    pub checkout: &'a Path,
    /// The remote and the branch pinned when the work began.
    pub remote: &'a str,
    pub target: &'a str,
    /// The commit the work was merged as.
    pub merged: &'a str,
}

pub(crate) async fn start_deploy(
    source: &Source<'_>,
    command: &str,
    kept: &Path,
    lock: &Path,
) -> anyhow::Result<Started> {
    let Source {
        jj,
        checkout,
        remote,
        target,
        merged,
    } = *source;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock)
        .context("could not open the lock of deploying")?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(Started::Busy),
        Err(TryLockError::Error(error)) => return Err(error.into()),
    }
    let ran = |args: &[&str]| {
        let mut jj = Command::new(jj);
        jj.arg("-R").arg(checkout).args(args).current_dir(checkout);
        output(jj)
    };
    anyhow::ensure!(
        ran(&["log", "--no-graph", "-r", "@", "-T", "empty"])
            .await?
            .trim()
            == "true",
        "the checkout at {} has work in it, so it is left as it is",
        checkout.display()
    );
    std::fs::create_dir_all(kept)?;
    match std::fs::remove_file(kept.join("status")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    let at = ran(&[
        "log",
        "--no-graph",
        "-r",
        "@-",
        "-T",
        r#"commit_id ++ "\n""#,
    ])
    .await?;
    let repo = Repo {
        jj: jj.to_owned(),
        checkout: checkout.to_path_buf(),
        workspace: checkout.to_path_buf(),
    };
    if !repo.holds(remote, target, merged, at.trim()).await? {
        ran(&["new", merged]).await?;
    }
    let mut run = std::process::Command::new("/bin/sh");
    run.args(["-c", WRAPPER, "sh", command])
        .arg(kept)
        .current_dir(checkout)
        .stdin(lock)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for name in crate::workers::agent::hidden_env() {
        run.env_remove(name);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        run.process_group(0);
    }
    let mut child = run.spawn().context("could not start deploying")?;
    std::thread::spawn(move || child.wait());
    Ok(Started::Yes)
}

/// How the command started by [`start_deploy`] ended, once it has.
pub(crate) fn deployed(kept: &Path) -> anyhow::Result<Option<Result<(), String>>> {
    let status = match std::fs::read_to_string(kept.join("status")) {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(match status.trim() {
        "0" => Ok(()),
        _ => Err(tail(&kept.join("log"))?),
    }))
}

#[cfg(all(test, unix))]
mod tests;
