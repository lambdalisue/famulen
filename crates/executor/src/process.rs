//! Processes by their number.

/// End a process that leads a group, and every process in the group.
pub(crate) fn end_group(pid: u32, at_once: bool) {
    #[cfg(unix)]
    if let Some(group) = rustix::process::Pid::from_raw(pid as i32) {
        use rustix::process::Signal;
        let signal = if at_once { Signal::KILL } else { Signal::TERM };
        rustix::process::kill_process_group(group, signal).ok();
    }
    #[cfg(not(unix))]
    let _ = (pid, at_once);
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
