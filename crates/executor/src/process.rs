//! Processes by their number.

/// End a process that leads a group, every process in the group, and what
/// descends from it though it left the group. `left` keeps those from one
/// time to the next: once their parent has ended, nothing tells they were
/// its own. One whose parent ended before any look is not found.
pub(crate) fn end_group(pid: u32, at_once: bool, left: &mut Vec<(u32, String)>) {
    #[cfg(unix)]
    if let Some(group) = rustix::process::Pid::from_raw(pid as i32) {
        use rustix::process::{Pid, Signal};
        let signal = if at_once { Signal::KILL } else { Signal::TERM };
        // Looked for before any signal, which could take their parents away.
        for found in descendants(pid) {
            if left.iter().all(|(seen, _)| *seen != found)
                && let Some(born) = born(found)
            {
                left.push((found, born));
            }
        }
        rustix::process::kill_process_group(group, signal).ok();
        for (one, when) in left.iter() {
            // The number may have gone to another since.
            if born(*one).as_ref() == Some(when)
                && let Some(one) = Pid::from_raw(*one as i32)
            {
                rustix::process::kill_process(one, signal).ok();
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, at_once, left);
}

/// The processes that descend from one that is there.
#[cfg(unix)]
fn descendants(pid: u32) -> Vec<u32> {
    let listing = told("/bin/ps", &["-A", "-o", "pid=,ppid="]).unwrap_or_default();
    let pairs: Vec<(u32, u32)> = listing
        .lines()
        .filter_map(|line| {
            let mut numbers = line.split_whitespace().map(str::parse);
            Some((numbers.next()?.ok()?, numbers.next()?.ok()?))
        })
        .collect();
    let mut found = vec![pid];
    let mut at = 0;
    while let Some(&parent) = found.get(at) {
        for &(child, of) in &pairs {
            if of == parent && !found.contains(&child) {
                found.push(child);
            }
        }
        at += 1;
    }
    found.split_off(1)
}

/// Ask a process to end.
pub(crate) fn ask_to_end(pid: u32) {
    #[cfg(unix)]
    if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
        rustix::process::kill_process(pid, rustix::process::Signal::TERM).ok();
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Which time of being up the system is in.
pub(crate) fn boot() -> Option<String> {
    if cfg!(target_os = "linux") {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
        Some(boot.trim().to_owned())
    } else if cfg!(unix) {
        told("/usr/sbin/sysctl", &["-n", "kern.bootsessionuuid"])
    } else {
        None
    }
}

/// When a process that is there now began, as the system tells it.
pub(crate) fn born(pid: u32) -> Option<String> {
    if cfg!(target_os = "linux") {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The program's name may hold anything, a bracket too.
        let (_, rest) = stat.rsplit_once(')')?;
        Some(rest.split_whitespace().nth(19)?.to_owned())
    } else if cfg!(unix) {
        told("/bin/ps", &["-o", "lstart=", "-p", &pid.to_string()])
    } else {
        None
    }
}

fn told(program: &str, args: &[&str]) -> Option<String> {
    let told = std::process::Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    let text = String::from_utf8(told.stdout).ok()?.trim().to_owned();
    (told.status.success() && !text.is_empty()).then_some(text)
}
