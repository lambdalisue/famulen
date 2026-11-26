//! What a worker's shell reaches on this very machine: the API and the
//! console that run here, Adroit's socket, the SSH agent, the gateway, the
//! vault's key, the notebook and its clone. A real Claude Code with the
//! owner's login runs one command in the fence a worker is given.
//!
//! Nothing is opened here: what is probed is what this machine has. Run it
//! alone, from the checkout Famulen runs from, with Famulen running:
//!
//! ```sh
//! FAMULEN_PROBE_CHECKOUT=<checkout> cargo test -p famulen-core live_ -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use super::{fence, hidden_env};
use crate::agents::{Adapter as _, Answer, Ask, Body, Work, claude::Claude};
use crate::config::{Config, Defaults};

/// One place a worker must not reach, and whether it is there to be reached.
struct Target {
    name: String,
    /// The probe's Python that reaches it.
    reach: String,
    there: bool,
}

fn file(name: &str, path: &Path, reach: &str) -> Target {
    Target {
        name: name.to_owned(),
        reach: format!("{reach}({:?})", path.display().to_string()),
        there: path.exists(),
    }
}

fn tcp(name: &str, port: u16) -> Target {
    Target {
        name: name.to_owned(),
        reach: format!("socket.create_connection((\"127.0.0.1\", {port}), 3)"),
        there: std::net::TcpStream::connect(("127.0.0.1", port)).is_ok(),
    }
}

#[cfg(unix)]
fn unix(name: &str, path: &Path) -> Target {
    Target {
        name: name.to_owned(),
        reach: format!("unix({:?})", path.display().to_string()),
        there: std::os::unix::net::UnixStream::connect(path).is_ok(),
    }
}

fn targets(config: &Config, checkout: &Path) -> Vec<Target> {
    let home = crate::sealed::home().unwrap();
    let mut targets = vec![
        tcp("api", config.api.listen.port()),
        Target {
            name: "api-http".into(),
            reach: format!(
                "urllib.request.urlopen(\"http://localhost:{}/health\", timeout=3)",
                config.api.listen.port()
            ),
            there: true,
        },
        tcp("console", 3000),
        file("gateway", &config.gateway_dir(), "os.listdir"),
        file("vault-key", &checkout.join(".env"), "read"),
        file("notebook", &config.notebook_dir(), "os.listdir"),
        file("database", &config.database_path(), "read"),
        file(
            "executors",
            &config.data_dir.join("executors"),
            "os.listdir",
        ),
    ];
    for clone in &config.workers_sealed {
        targets.push(file("clone", clone, "os.listdir"));
    }
    #[cfg(unix)]
    {
        // Every socket Adroit keeps, wherever below its state it is.
        let mut dirs = vec![home.join(".local/state/adroit")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                use std::os::unix::fs::FileTypeExt as _;
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    dirs.push(entry.path());
                } else if kind.is_socket() {
                    let name = entry
                        .path()
                        .strip_prefix(&home)
                        .unwrap()
                        .display()
                        .to_string();
                    targets.push(unix(&name, &entry.path()));
                }
            }
        }
        if let Some(agent) = std::env::var_os("SSH_AUTH_SOCK") {
            targets.push(unix("ssh-agent", &PathBuf::from(agent)));
        }
    }
    targets
}

fn probe(targets: &[Target], own: &Path) -> String {
    let mut probe = String::from(
        r#"import os, socket, urllib.request
def told(name, do):
    try: print(name, (do(), "reached")[1])
    except Exception as e: print(name, "kept", type(e).__name__)
def read(path): return open(path, "rb").read(16)
def write(path):
    os.makedirs(os.path.dirname(path), exist_ok=True); open(path, "a").write("")
def unix(path):
    s = socket.socket(socket.AF_UNIX); s.connect(path)
"#,
    );
    probe.push_str(&format!(
        "told(\"own\", lambda: read({:?}))\n",
        own.display().to_string()
    ));
    for target in targets {
        probe.push_str(&format!(
            "told({:?}, lambda: {})\n",
            target.name, target.reach
        ));
    }
    probe
}

/// Run `work` in a real Claude Code, allowing it nothing but Bash, and gather
/// what its commands put out.
async fn run(work: &Work) -> String {
    let launch = Claude.launch(work, None).unwrap();
    let mut command = tokio::process::Command::new(&launch.program);
    for name in hidden_env() {
        command.env_remove(name);
    }
    // A development shell names an SDK the system's own python3 is not in.
    command.env_remove("DEVELOPER_DIR").env_remove("SDKROOT");
    let mut child = command
        .args(&launch.args)
        .envs(&launch.env)
        .current_dir(&work.workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in &launch.input {
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut codec = Claude.codec();
    let mut told = String::new();
    let run = async {
        while let Some(line) = lines.next_line().await.unwrap() {
            for body in codec.read(&line) {
                match body {
                    Body::Asked { question } => {
                        let answer = match &question.ask {
                            Ask::Tool { tool, .. } if tool == "Bash" => Answer::Allow,
                            Ask::Tool { .. } => Answer::Refuse { why: "no".into() },
                            Ask::Server { .. } | Ask::Unknown { .. } => continue,
                        };
                        let line = Claude.answer(&question, &answer).unwrap();
                        stdin
                            .write_all(format!("{line}\n").as_bytes())
                            .await
                            .unwrap();
                    }
                    Body::ToolAnswered { output, .. } => told.push_str(&output),
                    Body::TurnEnded { .. } => return,
                    _ => {}
                }
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(300), run)
        .await
        .expect("an end in time");
    told
}

#[tokio::test]
#[ignore = "runs a real Claude Code with the owner's login, against what runs here"]
async fn live_a_worker_reaches_nothing_famulen_keeps_on_this_machine() {
    let checkout = PathBuf::from(
        std::env::var("FAMULEN_PROBE_CHECKOUT").expect("the checkout Famulen runs from"),
    );
    let env = |name: &str| std::env::var(name).ok();
    let config = Config::load(None, env, Defaults::from_system(env).unwrap()).unwrap();
    let kept = crate::runtime::kept_from_workers_started_in(&config, &checkout).unwrap();
    let home = crate::sealed::home().unwrap();
    let workdir = home.join(format!(".famulen-live-{}", std::process::id()));
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(workdir.join("a.md"), "own").unwrap();
    let targets = targets(&config, &checkout);
    std::fs::write(
        workdir.join("probe.py"),
        probe(&targets, &workdir.join("a.md")),
    )
    .unwrap();
    let work = Work {
        instructions: "This is the owner's own check of a sandbox. Run the one command you are given with the Bash tool, once, exactly as written, and reply with nothing but its output.".into(),
        message: "/usr/bin/python3 probe.py".into(),
        tools: vec!["Bash".into()],
        workdir: workdir.clone(),
        fence: fence(&kept, &config.data_dir.join("workers"), &workdir),
        ..Work::default()
    };
    report(&targets, &run(&work).await, &workdir);
}

/// Every target that is there is kept, and the worker's own file is not.
fn report(targets: &[Target], told: &str, workdir: &Path) {
    std::fs::remove_dir_all(workdir).ok();
    println!("{told}");
    assert!(told.contains("own reached"), "{told}");
    let mut reached = Vec::new();
    for target in targets {
        let line = told
            .lines()
            .find(|l| l.split_whitespace().next() == Some(target.name.as_str()))
            .unwrap_or_default();
        println!(
            "{:<10} {} {line}",
            target.name,
            if target.there { "there" } else { "absent" }
        );
        if target.there && !line.contains(" kept") {
            reached.push(target.name.as_str());
        }
    }
    assert!(reached.is_empty(), "reached: {reached:?}\n{told}");
}

fn write(name: &str, path: &Path) -> Target {
    Target {
        name: name.to_owned(),
        reach: format!("write({:?})", path.display().to_string()),
        there: true,
    }
}

/// Work run directly writes its workspace and what jj and Git keep for it,
/// and nothing that would run as the owner, nor anything a worker may not
/// reach. A real jj makes the workspace of a repository made for it.
#[tokio::test]
#[ignore = "runs a real Claude Code with the owner's login, against what runs here"]
async fn live_work_run_directly_writes_only_its_workspace_and_reaches_nothing_famulen_keeps() {
    let checkout = PathBuf::from(
        std::env::var("FAMULEN_PROBE_CHECKOUT").expect("the checkout Famulen runs from"),
    );
    let env = |name: &str| std::env::var(name).ok();
    let config = Config::load(None, env, Defaults::from_system(env).unwrap()).unwrap();
    let kept = crate::runtime::kept_from_workers_started_in(&config, &checkout).unwrap();
    let home = crate::sealed::home().unwrap();
    let root = home.join(format!(".famulen-live-fence-{}", std::process::id()));
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("a.md"), "own").unwrap();
    let jj = |args: &[&str]| {
        let done = std::process::Command::new("jj")
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            done.status.success(),
            "{}",
            String::from_utf8_lossy(&done.stderr)
        );
    };
    jj(&["git", "init", "--colocate"]);
    jj(&["describe", "-m", "a"]);
    let repo = repo.canonicalize().unwrap();
    let name = "task-probe-run1".to_owned();
    let workspace = repo.join(".claude/worktrees").join(&name);
    let place = crate::workers::direct::Place {
        checkout: repo.clone(),
        workspace: workspace.clone(),
        name,
        directory: workspace.clone(),
    };
    let direct = crate::workers::Direct::new(repo.clone());
    crate::workers::direct::prepare(&direct, &place, "@")
        .await
        .unwrap();

    let (jjd, git) = (repo.join(".jj"), repo.join(".git"));
    let mut targets = targets(&config, &checkout);
    targets.extend([
        write("repo-config", &jjd.join("repo/config.toml")),
        write("repo-git-target", &jjd.join("repo/store/git_target")),
        write("repo-workspace-config", &jjd.join("workspace-config.toml")),
        write(
            "workspace-config",
            &workspace.join(".jj/workspace-config.toml"),
        ),
        write("git-config", &git.join("config")),
        write("git-hook", &git.join("hooks/post-checkout")),
        write("git-info", &git.join("info/attributes")),
        write("git-modules", &git.join("modules/x")),
        write("git-commondir", &git.join("commondir")),
        write("checkout-file", &repo.join("a.md")),
    ]);
    let wanted = [
        write("workspace-file", &workspace.join("b.md")),
        write("jj-op-heads", &jjd.join("repo/op_heads/famulen-probe")),
        write("git-objects", &git.join("objects/famulen-probe")),
    ];
    let mut script = probe(&targets, &workspace.join("a.md"));
    for target in &wanted {
        script.push_str(&format!(
            "told({:?}, lambda: {})\n",
            target.name, target.reach
        ));
    }
    std::fs::write(workspace.join("probe.py"), script).unwrap();
    let work = Work {
        instructions: "This is the owner's own check of a sandbox. Run the one command you are given with the Bash tool, once, exactly as written, and reply with nothing but its output.".into(),
        message: "/usr/bin/python3 probe.py".into(),
        tools: vec!["Bash".into()],
        workdir: workspace.clone(),
        fence: crate::workers::direct::fence(&kept, &config.data_dir.join("workers"), &place),
        ..Work::default()
    };
    let told = run(&work).await;
    for target in &wanted {
        assert!(
            told.contains(&format!("{} reached", target.name)),
            "{}: {told}",
            target.name
        );
    }
    let unchanged = std::fs::read_to_string(repo.join("a.md")).unwrap() == "own";
    report(&targets, &told, &root);
    assert!(unchanged);
}
