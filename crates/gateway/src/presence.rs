//! Whether the owner is at this machine.

use std::process::Command;
use std::time::Duration;

/// What tells whether the owner is at this machine, and what they look at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Presence {
    /// How long since they last touched it, as [`idle`] tells.
    pub idle: Option<Duration>,
    /// The app in front, by its bundle id.
    pub front: Option<String>,
}

/// How it stands now.
pub fn now() -> Presence {
    Presence {
        idle: idle(),
        front: front(),
    }
}

fn front() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let ask = |args: &[&str]| {
        let output = Command::new("/usr/bin/lsappinfo")
            .args(args)
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let front = ask(&["front"])?;
    let told = ask(&["info", "-only", "bundleid", &front])?;
    told.rsplit_once('=')
        .map(|(_, id)| id.trim_matches('"').to_owned())
        .filter(|id| !id.is_empty())
}

/// How long since the owner last touched this machine; longer than anything
/// while its screen is locked. `None` when the machine does not say, as one
/// with no desktop does not.
pub fn idle() -> Option<Duration> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let ask = |args: &[&str]| {
        let output = Command::new("/usr/sbin/ioreg").args(args).output().ok()?;
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    if value(&ask(&["-n", "Root", "-d1"])?, "IOConsoleLocked").as_deref() == Some("Yes") {
        return Some(Duration::MAX);
    }
    let hid = ask(&["-r", "-c", "IOHIDSystem", "-d", "1", "-k", "HIDIdleTime"])?;
    value(&hid, "HIDIdleTime")?
        .parse()
        .ok()
        .map(Duration::from_nanos)
}

/// The value printed for `key`.
fn value(text: &str, key: &str) -> Option<String> {
    let (_, after) = text.split_once(&format!("\"{key}\" = "))?;
    let end = after
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(after.len());
    Some(after[..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_read_out_of_what_the_machine_prints() {
        let text = "  |   \"HIDIdleTime\" = 1234567\n  |   \"IOConsoleLocked\" = Yes\n";
        assert_eq!(value(text, "HIDIdleTime").as_deref(), Some("1234567"));
        assert_eq!(value(text, "IOConsoleLocked").as_deref(), Some("Yes"));
        assert_eq!(value(text, "Other"), None);
    }
}
