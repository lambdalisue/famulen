//! Stands in for `claude` and `codex` in tests, which compile it with plain
//! `rustc`: a program of its own runs on every OS, where a script does not.
//! It keeps what it was called with beside itself, and answers with what it
//! finds there.

use std::io::{Read, Write};
use std::path::Path;

fn main() {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap();
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::fs::write(dir.join("args"), args.join("\0")).unwrap();
    let thinking = std::env::var("MAX_THINKING_TOKENS").unwrap_or_default();
    std::fs::write(dir.join("thinking"), thinking).unwrap();
    if !dir.join("unread").exists() {
        let mut stdin = Vec::new();
        std::io::stdin().read_to_end(&mut stdin).unwrap();
        std::fs::write(dir.join("stdin"), stdin).unwrap();
    }
    std::io::stdout().write_all(&told(dir, "stdout")).unwrap();
    std::io::stderr().write_all(&told(dir, "stderr")).unwrap();
    std::process::exit(if dir.join("fails").exists() { 1 } else { 0 });
}

fn told(dir: &Path, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap_or_default()
}
