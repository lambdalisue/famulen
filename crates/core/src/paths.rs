//! Telling whether two paths are the same place, the way the system judges
//! it. On Windows one place has many spellings: either separator, any case,
//! `\\?\`, the drive as Git Bash writes it.

use std::path::{Path, PathBuf};

/// `path` in the one spelling its place is compared by where missing a match
/// would do harm: a drive's share on any host counts as the drive here, since
/// the host may be this machine by another name. Not for showing or opening.
pub(crate) fn key(path: &Path) -> PathBuf {
    if cfg!(windows) {
        windows::key(&path.to_string_lossy()).into()
    } else {
        path.to_path_buf()
    }
}

/// Whether `path` is `dir` or under it, as both are written.
pub(crate) fn under(path: &Path, dir: &Path) -> bool {
    if cfg!(windows) {
        let place = |p: &Path| PathBuf::from(windows::place(&p.to_string_lossy()));
        place(path).starts_with(place(dir))
    } else {
        path.starts_with(dir)
    }
}

/// Whether `path` could be `dir` or under it, as [`key`] tells.
pub(crate) fn may_be_under(path: &Path, dir: &Path) -> bool {
    key(path).starts_with(key(dir))
}

/// Whether `path` is `dir` or under it, name by name and whatever its case.
/// Where a directory tells case apart, only the disk can say.
pub(crate) fn may_be_within(path: &Path, dir: &Path) -> bool {
    if cfg!(windows) {
        let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_ascii_lowercase());
        lower(path).starts_with(lower(dir))
    } else {
        path.starts_with(dir)
    }
}

/// `path` as other programs and the owner write it: Windows puts `\\?\` in
/// front of a path it resolved.
pub(crate) fn plain(path: PathBuf) -> PathBuf {
    if cfg!(windows)
        && let Some(plain) = path.to_str().and_then(windows::plain)
    {
        return plain.into();
    }
    path
}

/// Windows' reading of paths, on their text alone: kept apart from
/// [`std::path`] so that the rules are the same, and tested, on every system.
/// Off Windows only the tests call it.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) mod windows {
    /// Beyond this, a path needs its `\\?\` to be opened.
    const LONGEST_PLAIN: usize = 260;

    /// What a path by its device starts with.
    const DEVICES: [&str; 3] = ["//?/", "//./", "/??/"];

    /// `path` without its `\\?\`, when that names the same place.
    pub(crate) fn plain(path: &str) -> Option<String> {
        let rest = path.strip_prefix(r"\\?\")?;
        let plain = match rest.strip_prefix(r"UNC\") {
            Some(share) => format!(r"\\{share}"),
            None if drive(rest).is_some_and(|(_, rest)| rest.starts_with('\\')) => rest.to_owned(),
            None => return None,
        };
        // Such a name is another one without the prefix.
        let kept = plain
            .split(['\\', '/'])
            .any(|name| name.ends_with(['.', ' ']) && name != "." && name != "..");
        (plain.len() < LONGEST_PLAIN && !kept).then_some(plain)
    }

    /// `path` in the one spelling of the place it surely is: a `\\?\` path
    /// keeps the dots and spaces that tell its names apart.
    pub(crate) fn place(path: &str) -> String {
        let slashed = slashed(path);
        let kept = path.starts_with(r"\\?\").then(|| as_written(&slashed));
        kept.flatten()
            .unwrap_or_else(|| worked_out(&slashed))
            .to_lowercase()
    }

    pub(crate) fn key(path: &str) -> String {
        worked_out(&without_share(slashed(path), |_| true)).to_lowercase()
    }

    /// Every place `path` may mean when a tool reads it from `base`, with
    /// `.` and `..` worked out: names as Windows opens them, with forward
    /// slashes, and where that drops dots or spaces, names as written, as the
    /// `\\?\` path that opens them so. `None` when it hangs on a drive's own
    /// current directory, which only the tool's process knows, or on a device
    /// that is no drive's letter or share, such as a volume by its id.
    pub(crate) fn readings(base: &str, path: &str) -> Option<Vec<String>> {
        let base = slashed(base);
        let device = DEVICES
            .iter()
            .any(|d| path.replace('\\', "/").starts_with(d));
        let path = slashed(path);
        if device && !path.starts_with("//") && drive(&path).is_none() {
            return None;
        }
        let mut whole = Vec::new();
        if path.starts_with("//") {
            whole.push(path);
        } else if let Some((letter, rest)) = drive(&path) {
            let here = drive(&base).is_some_and(|(b, _)| b.eq_ignore_ascii_case(&letter));
            if rest.starts_with('/') {
                whole.push(path);
            } else if here {
                whole.push(format!("{base}/{rest}"));
            } else {
                return None;
            }
        } else if path.starts_with('/') {
            let (root, _) = split_root(&base);
            whole.push(format!("{root}{path}"));
            whole.extend(unix_drive(&path));
        } else {
            whole.push(format!("{base}/{path}"));
        }
        Some(
            whole
                .iter()
                .flat_map(|path| {
                    let verbatim = as_written(path).map(|path| verbatim(&path));
                    [Some(worked_out(path)), verbatim]
                })
                .flatten()
                .collect(),
        )
    }

    /// A shell command's text with every separator and case made one, for
    /// [`spellings`] to be looked for in.
    pub(crate) fn command(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            let c = if c == '\\' { '/' } else { c };
            if c != '/' || !out.ends_with('/') {
                out.extend(c.to_lowercase());
            }
        }
        out
    }

    /// The ways a shell may spell the place whose [`key`] is `key`.
    pub(crate) fn spellings(key: &str) -> Vec<String> {
        let mut spellings = vec![command(key)];
        if let Some((letter, rest)) = drive(key) {
            spellings.push(rest.to_owned());
            for mount in ["", "/cygdrive", "/mnt"] {
                spellings.push(format!("{mount}/{letter}{rest}"));
            }
        }
        spellings
    }

    /// Forward slashes only, without the prefixes that change nothing about
    /// the place, and a drive's administrative share on this machine as the
    /// drive itself.
    fn slashed(path: &str) -> String {
        let mut path = path.replace('\\', "/");
        loop {
            let lower = path.to_ascii_lowercase();
            path = if lower.starts_with("//?/unc/") || lower.starts_with("//./unc/") {
                format!("//{}", &path["//?/unc/".len()..])
            } else if DEVICES.iter().any(|d| lower.starts_with(d)) {
                path["//?/".len()..].to_owned()
            } else {
                break;
            };
        }
        without_share(path, |host| {
            ["localhost", "127.0.0.1", "--1.ipv6-literal.net"]
                .iter()
                .any(|local| host.eq_ignore_ascii_case(local))
        })
    }

    /// `//host/C$/rest` as `C:/rest`, where `host` is `taken` for this
    /// machine.
    fn without_share(path: String, taken: impl Fn(&str) -> bool) -> String {
        if let Some((host, share)) = path.strip_prefix("//").and_then(|p| p.split_once('/'))
            && taken(host)
            && let Some((letter, rest)) = share.split_once('$')
            && letter.len() == 1
            && letter.starts_with(|c: char| c.is_ascii_alphabetic())
            && (rest.is_empty() || rest.starts_with('/'))
        {
            return format!("{letter}:/{}", rest.trim_start_matches('/'));
        }
        path
    }

    /// `C:rest` as its letter and the rest.
    fn drive(path: &str) -> Option<(char, &str)> {
        let letter = path.chars().next().filter(char::is_ascii_alphabetic)?;
        Some((letter, path[1..].strip_prefix(':')?))
    }

    /// `/c/rest` and the like, as Git Bash, Cygwin and WSL write a drive.
    fn unix_drive(path: &str) -> Option<String> {
        let lower = path.to_ascii_lowercase();
        let mount = ["/cygdrive/", "/mnt/", "/"]
            .into_iter()
            .find(|m| lower.starts_with(m))?;
        let (letter, rest) = path[mount.len()..]
            .split_once('/')
            .unwrap_or((&path[mount.len()..], ""));
        (letter.len() == 1 && letter.starts_with(|c: char| c.is_ascii_alphabetic()))
            .then(|| format!("{letter}:/{rest}"))
    }

    /// `.` and `..` worked out, and each name as Windows opens it: without
    /// dots and spaces at its end, or a stream after `:`.
    fn worked_out(path: &str) -> String {
        let (root, rest) = split_root(path);
        let mut names: Vec<&str> = Vec::new();
        for name in rest.split('/') {
            let name = name.split(':').next().unwrap_or_default();
            if name.trim_end_matches(' ') == ".." {
                names.pop();
                continue;
            }
            let name = name.trim_end_matches(['.', ' ']);
            if !name.is_empty() {
                names.push(name);
            }
        }
        if names.is_empty() {
            return format!("{root}/");
        }
        names.iter().fold(root, |path, name| path + "/" + name)
    }

    /// `path` with the dots and spaces at the end of its names kept, where it
    /// has any: as it is opened through `\\?\`, which Node puts before every
    /// path.
    fn as_written(path: &str) -> Option<String> {
        let (root, rest) = split_root(path);
        let mut names: Vec<&str> = Vec::new();
        for name in rest.split('/') {
            match name {
                "" | "." => {}
                ".." => {
                    names.pop();
                }
                name => names.push(name),
            }
        }
        if !names.iter().any(|name| name.ends_with(['.', ' '])) {
            return None;
        }
        Some(format!("{root}/{}", names.join("/")))
    }

    fn verbatim(path: &str) -> String {
        let path = match path.strip_prefix("//") {
            Some(share) => format!("UNC/{share}"),
            None => path.to_owned(),
        };
        format!(r"\\?\{}", path.replace('/', r"\"))
    }

    /// The drive or share `path` starts with, and the rest.
    fn split_root(path: &str) -> (String, &str) {
        match drive(path) {
            Some((letter, rest)) => (format!("{letter}:"), rest),
            None => match path.strip_prefix("//") {
                Some(unc) => {
                    let (host, rest) = unc.split_once('/').unwrap_or((unc, ""));
                    let (share, rest) = rest.split_once('/').unwrap_or((rest, ""));
                    (format!("//{host}/{share}"), rest)
                }
                None => (String::new(), path),
            },
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn one_place_has_one_key_however_it_is_written() {
            for written in [
                r"C:\Data\Gateway",
                "c:/data/gateway",
                r"C:/Data\gateway\",
                r"\\?\C:\Data\Gateway",
                r"\\.\c:\data\gateway",
                r"\??\c:\data\gateway",
                r"\\localhost\C$\data\gateway",
                r"\\?\UNC\127.0.0.1\c$\data\gateway",
                r"C:\data\.\x\..\gateway",
                r"C:\data\gateway.",
                r"C:\data \gateway. .",
                r"C:\data\gateway::$INDEX_ALLOCATION",
                r"C:\..\data\gateway",
            ] {
                assert_eq!(key(written), "c:/data/gateway", "{written}");
            }
            assert_eq!(key(r"C:\"), "c:/");
            assert_eq!(key(r"\\Host\Share\X"), "//host/share/x");
            assert_eq!(key(r"D:\data\gateway"), "d:/data/gateway");
            assert_eq!(key(r"C:\data\gateway-notes"), "c:/data/gateway-notes");
        }

        #[test]
        fn a_path_is_read_from_its_base_as_windows_and_git_bash_would() {
            let read = |path| readings(r"C:\data\workspace", path).unwrap();
            for path in [
                r"C:\data\gateway",
                r"..\gateway",
                "../gateway",
                r"C:..\gateway",
                r"\data\gateway",
            ] {
                assert_eq!(read(path), ["C:/data/gateway"], "{path}");
            }
            for path in [
                "/c/data/gateway",
                "/cygdrive/c/data/gateway",
                "/mnt/c/data/gateway",
            ] {
                assert!(
                    read(path).contains(&"c:/data/gateway".to_owned()),
                    "{path}: {:?}",
                    read(path)
                );
            }
            assert_eq!(read("."), ["C:/data/workspace"]);
            assert_eq!(read(r"\\host\share\x"), ["//host/share/x"]);
            assert_eq!(
                readings(r"\\host\share\data\workspace", r"\data\gateway").unwrap(),
                ["//host/share/data/gateway"]
            );
        }

        #[test]
        fn a_path_from_another_drives_current_directory_cannot_be_told() {
            assert_eq!(readings(r"C:\data\workspace", "D:gateway"), None);
            assert_eq!(readings(r"\\host\share", "C:gateway"), None);
        }

        #[test]
        fn a_path_by_a_device_that_is_no_drive_or_share_cannot_be_told() {
            for path in [
                r"\\?\Volume{4c1b02c1-d990-11dc-99ae-806e6f6e6963}\data",
                "//?/Volume{4c1b02c1-d990-11dc-99ae-806e6f6e6963}/data",
                r"\\.\GLOBALROOT\Device\HarddiskVolume1\data",
            ] {
                assert_eq!(readings(r"C:\data\workspace", path), None, "{path}");
            }
        }

        #[test]
        fn a_name_that_ends_in_a_dot_is_also_read_as_written() {
            for (path, written) in [
                (r"\\?\C:\w\link.\x", r"\\?\C:\w\link.\x"),
                (r"c:/w/link./x", r"\\?\c:\w\link.\x"),
                (r"link \x", r"\\?\C:\data\workspace\link \x"),
                (r"\\h\s\a\..\link.", r"\\?\UNC\h\s\link."),
            ] {
                let read = readings(r"C:\data\workspace", path).unwrap();
                assert!(read.contains(&written.to_owned()), "{path}: {read:?}");
            }
        }

        #[test]
        fn a_drives_share_is_the_drive_only_on_this_machine_unless_it_must_not_be_missed() {
            assert_eq!(place(r"\\localhost\C$\data"), "c:/data");
            assert_eq!(place(r"\\server\C$\data"), "//server/c$/data");
            assert_eq!(key(r"\\server\C$\data"), "c:/data");
        }

        #[test]
        fn a_long_path_is_another_place_by_the_dot_its_name_ends_in() {
            assert_eq!(place(r"\\?\C:\out."), "c:/out.");
            assert_eq!(place(r"\\?\C:\out"), "c:/out");
            assert_eq!(place(r"C:\out."), "c:/out");
        }

        #[test]
        fn a_command_names_a_place_in_any_shells_spelling() {
            let named = |text: &str| {
                let text = command(text);
                spellings("c:/data/gateway")
                    .iter()
                    .any(|s| text.contains(s))
            };
            for text in [
                r"type C:\Data\Gateway\x",
                r"cat C:\\data\\gateway/x",
                "cat c:/data//gateway",
                "cat /c/data/gateway/x",
                "cat /cygdrive/c/data/gateway/x",
                "cat /mnt/c/data/gateway/x",
                r"Get-Content \data\gateway\x",
            ] {
                assert!(named(text), "{text}");
            }
            assert!(!named(r"type C:\Data\Other"));
        }

        #[test]
        fn the_long_form_is_dropped_only_where_the_plain_one_is_the_same_place() {
            assert_eq!(plain(r"\\?\C:\data").as_deref(), Some(r"C:\data"));
            assert_eq!(plain(r"\\?\UNC\h\s\x").as_deref(), Some(r"\\h\s\x"));
            assert_eq!(plain(r"\\?\Volume{1}\x"), None);
            assert_eq!(plain(&format!(r"\\?\C:\{}", "x".repeat(300))), None);
            for name in ["data.", "data ", r"data.\x"] {
                assert_eq!(plain(&format!(r"\\?\C:\{name}")), None, "{name}");
            }
            assert_eq!(plain(r"C:\data"), None);
        }
    }
}
