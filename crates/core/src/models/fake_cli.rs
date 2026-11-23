//! A program standing in for the agent CLIs in tests.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;

/// A stand-in for one test: what it answers with, and what it was last
/// called with.
pub(super) struct FakeCli {
    dir: PathBuf,
    program: PathBuf,
}

impl FakeCli {
    /// One that answers with nothing and ends well.
    pub fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("famulen-models-cli-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join(format!("claude{}", std::env::consts::EXE_SUFFIX));
        // Writing a program from this process and running it at once fails
        // now and then with ETXTBSY: a child forked by another test thread
        // inherits the open write handle until it execs. A link writes nothing.
        std::fs::hard_link(compiled(), &program).unwrap();
        Self { dir, program }
    }

    /// Answer with each of `lines` as JSON.
    pub fn prints(self, lines: &[Value]) -> Self {
        let lines: String = lines.iter().map(|line| format!("{line}\n")).collect();
        self.told("stdout", &lines)
    }

    /// End in failure, saying `stderr`.
    pub fn fails(self, stderr: &str) -> Self {
        self.told("stderr", stderr).told("fails", "")
    }

    /// End without reading the input.
    pub fn without_reading(self) -> Self {
        self.told("unread", "")
    }

    fn told(self, name: &str, text: &str) -> Self {
        std::fs::write(self.dir.join(name), text).unwrap();
        self
    }

    pub fn program(&self) -> &str {
        self.program.to_str().unwrap()
    }

    pub fn args(&self) -> Vec<String> {
        self.kept("args").split('\0').map(String::from).collect()
    }

    /// The argument after `flag`.
    pub fn value_after(&self, flag: &str) -> Option<String> {
        let args = self.args();
        let at = args.iter().position(|arg| arg == flag)?;
        args.get(at + 1).cloned()
    }

    pub fn stdin(&self) -> String {
        self.kept("stdin")
    }

    /// The thinking budget it was given in its environment.
    pub fn thinking(&self) -> String {
        self.kept("thinking")
    }

    fn kept(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap()
    }
}

/// The program, compiled once for this run of the tests.
fn compiled() -> &'static Path {
    static PROGRAM: OnceLock<PathBuf> = OnceLock::new();
    PROGRAM.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("famulen-fake-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join(format!("fake_cli{}", std::env::consts::EXE_SUFFIX));
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fake_cli.rs");
        let output = std::process::Command::new("rustc")
            .args(["--edition", "2024", "-o"])
            .arg(&program)
            .arg(source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        program
    })
}
