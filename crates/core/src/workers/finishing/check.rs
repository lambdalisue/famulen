//! The commands that check the work. They run what the agent wrote, so they
//! run kept as the agent is and more: no network, no socket, nothing written
//! but the workspace and a temporary directory of their own.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;

use crate::agents::Fence;
use crate::speech::process::Kept;
use crate::workers::direct::real_as_far_as_there;

/// How long one command may take.
const LIMIT: Duration = Duration::from_secs(60 * 60);
/// How much of the end of what a command writes is read back.
const TAIL_BYTES: u64 = 32 * 1024;

const SANDBOX: &str = "/usr/bin/sandbox-exec";

/// What every program needs. Of the system's services, only the one that
/// tells who a user is: the others reach the keychain, the clipboard and
/// other programs.
const BASE: &str = r#"(version 1)
(deny default)
(allow process-exec process-fork)
(allow signal (target same-sandbox))
(allow process-info* (target same-sandbox))
(allow sysctl-read)
(allow ipc-posix-sem ipc-posix-shm)
(allow mach-lookup (global-name "com.apple.system.opendirectoryd.libinfo"))
(allow file-read* file-map-executable)
(allow file-write* file-ioctl (literal "/dev/null") (literal "/dev/zero") (literal "/dev/dtracehelper"))
"#;

/// Run `command` in `workspace`; what it wrote last when it fails. `log`
/// takes what it writes, and must be where the command cannot write.
pub(crate) async fn check(
    command: &str,
    workspace: &Path,
    fence: &Fence,
    log: &Path,
) -> anyhow::Result<Result<(), String>> {
    anyhow::ensure!(
        Path::new(SANDBOX).exists(),
        "checks run only on macOS for now, where they can be kept from the network and the keys"
    );
    let scratch = Scratch::new()?;
    let profile = profile(&real_as_far_as_there(workspace), &scratch.0, fence);
    if let Some(above) = log.parent() {
        std::fs::create_dir_all(above)?;
    }
    let args: [&OsStr; 7] = [
        "-c".as_ref(),
        // The shell Kept starts has no way to change directory.
        r#"cd "$1" && exec "$2" -p "$3" /bin/sh -c "$4""#.as_ref(),
        "sh".as_ref(),
        workspace.as_os_str(),
        SANDBOX.as_ref(),
        profile.as_ref(),
        command.as_ref(),
    ];
    let mut running = Kept::start(
        "/bin/sh",
        &args,
        &[
            ("TMPDIR", scratch.0.as_os_str()),
            ("CARGO_NET_OFFLINE", "true".as_ref()),
        ],
        log,
    )?;
    let passed = match tokio::time::timeout(LIMIT, running.wait()).await {
        Ok(passed) => passed?,
        Err(_) => {
            running.stop().await;
            return Ok(Err(format!(
                "`{command}` took longer than {}s\n{}",
                LIMIT.as_secs(),
                tail(log)?
            )));
        }
    };
    Ok(if passed { Ok(()) } else { Err(tail(log)?) })
}

fn profile(workspace: &Path, scratch: &Path, fence: &Fence) -> String {
    let each = |rule: &str, paths: &mut dyn Iterator<Item = &PathBuf>| -> String {
        paths
            .map(|p| format!("({rule} (subpath {}))\n", quoted(&real_as_far_as_there(p))))
            .collect()
    };
    let (workspace, scratch) = (workspace.to_owned(), scratch.to_owned());
    let kept = [workspace.join(".jj"), workspace.join(".git")];
    format!(
        "{BASE}{}{}{}",
        each("allow file-write*", &mut [&workspace, &scratch].into_iter()),
        each(
            "deny file-write*",
            &mut fence.unwritable.iter().chain(&kept)
        ),
        each(
            "deny file-read* file-write*",
            &mut fence.sealed.iter().chain(&fence.unread)
        ),
    )
}

fn quoted(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(super) fn tail(log: &Path) -> anyhow::Result<String> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut file = std::fs::File::open(log).context("what the check wrote cannot be read")?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(TAIL_BYTES)))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// A temporary directory of one check, gone with it.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> anyhow::Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("famulen-check-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&path)?;
        Ok(Self(path.canonicalize()?))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[cfg(test)]
mod tests;
