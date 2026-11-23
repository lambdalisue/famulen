//! The CLI as a person or an AI runs it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "famulen-notebook-cli-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_famulen-notebook"))
        .arg("--notebook")
        .arg(dir)
        .args(args)
        .env_remove("FAMULEN_NOTEBOOK")
        .env("FAMULEN_NOTEBOOK_AUTHOR", "Alice <alice@example.com>")
        .output()
        .unwrap()
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = run(dir, args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn last_commit(dir: &Path) -> String {
    let out = Command::new("git")
        .args(["log", "-1", "--format=%an <%ae>%n%B"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn a_record_is_added_listed_changed_and_removed_with_who_wrote_it_in_each_commit() {
    let dir = temp("records");
    ok(&dir, &["init"]);

    let id = ok(
        &dir,
        &[
            "--agent",
            "Claude Code 2.1.0",
            "--model",
            "claude-opus-5-5",
            "--source",
            "slack:C1/1.0",
            "add",
            "tasks",
            "--title",
            "見積もりの返事をまとめる",
            "--field",
            "Status=pending",
            "--field",
            "workgroup=[work]",
        ],
    );
    let id = id.trim();
    assert_eq!(id.len(), 26, "{id}");
    let commit = last_commit(&dir);
    assert!(
        commit.starts_with("Alice <alice@example.com>\n"),
        "{commit}"
    );
    assert!(
        commit.contains("Notebook-Tool: famulen-notebook-cli "),
        "{commit}"
    );
    assert!(
        commit.contains("Notebook-Agent: Claude Code 2.1.0"),
        "{commit}"
    );
    assert!(
        commit.contains("Notebook-Model: claude-opus-5-5"),
        "{commit}"
    );
    assert!(commit.contains("Notebook-Source: slack:C1/1.0"), "{commit}");

    let listed = ok(&dir, &["list", "tasks"]);
    let code = format!("task-{}", &id[20..]);
    assert_eq!(listed, format!("{code}\t見積もりの返事をまとめる\n"));
    ok(
        &dir,
        &[
            "set",
            "tasks",
            &code,
            "--field",
            "Status=done",
            "--field",
            "Details/番号=A-1",
        ],
    );
    ok(&dir, &["log", "tasks", &code, "送った"]);
    let file = ok(&dir, &["get", &format!("records/tasks/{id}.md")]);
    assert!(file.contains("- Status: done\n"), "{file}");
    assert!(file.contains("## Details\n\n- 番号: A-1\n"), "{file}");
    assert!(file.contains(" owner: 送った\n"), "{file}");
    assert!(file.contains("workgroup: [work]"), "{file}");
    assert!(
        !run(&dir, &["set", "tasks", &code, "--field", "Due=tomorrow"])
            .status
            .success()
    );
    ok(&dir, &["rm", "tasks", &code]);
    assert_eq!(ok(&dir, &["list", "tasks"]), "");
    assert!(!run(&dir, &["rm", "tasks", &code]).status.success());
}

#[test]
fn memories_are_written_down_kept_and_found() {
    let dir = temp("memory");
    ok(&dir, &["init"]);

    let core = ok(
        &dir,
        &[
            "remember",
            "core",
            "返事は短く、要点を先に",
            "--attr",
            "src=said",
        ],
    );
    let long = ok(
        &dir,
        &[
            "keep",
            "semantic",
            "tanaka-sweets",
            "--summary",
            "田中さんは甘いものが好き",
            "--text",
            "[[records/parties/p1|田中さん]] は和菓子が好き。",
            "--attr",
            "src=inferred",
        ],
    );
    assert_ne!(core.trim(), long.trim());
    let found = ok(&dir, &["search", "和菓子", "--json"]);
    let hits: serde_json::Value = serde_json::from_str(&found).unwrap();
    assert_eq!(hits[0]["path"], "memory/long/semantic/tanaka-sweets.md");
    assert_eq!(hits[0]["title"], "田中さんは甘いものが好き");
    let short = ok(&dir, &["search", "要点"]);
    assert!(short.contains("memory/core.md#"), "{short}");
}

#[test]
fn check_says_what_strays_and_fails_until_it_is_fixed() {
    let dir = temp("check");
    ok(&dir, &["init"]);
    assert_eq!(ok(&dir, &["check"]), "");

    std::fs::write(dir.join("notes.md"), "# 置き場所のないメモ\n").unwrap();
    let warned = ok(&dir, &["check"]);
    assert!(warned.contains("notes.md:1: warning[stray]"), "{warned}");

    let id = ok(&dir, &["add", "tasks", "--title", "x"]);
    let path = dir.join(format!("records/tasks/{}.md", id.trim()));
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replace(
            "# x\n",
            "# x\n\n- due: 2026-11-28T17:00\n- Status: someday\n",
        ),
    )
    .unwrap();
    let out = run(&dir, &["check", "--fix", "--json"]);

    assert_eq!(out.status.code(), Some(1));
    let found: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rules: Vec<&str> = found
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["rule"].as_str().unwrap())
        .collect();
    assert_eq!(rules, ["stray", "value"]);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("- Status: someday\n- Due: 2026-11-28 17:00\n")
    );
    assert!(last_commit(&dir).contains("手帳の書き方を直した"));
}

#[test]
fn an_edit_made_by_hand_is_committed_in_the_signers_name() {
    let dir = temp("commit");
    ok(&dir, &["init"]);
    std::fs::write(
        dir.join("memory/core.md"),
        "# 心得\n\n- 手で書いた {id: m1}\n",
    )
    .unwrap();

    ok(
        &dir,
        &[
            "--author",
            "Bob <bob@example.com>",
            "commit",
            "-m",
            "心得を直した",
        ],
    );

    let commit = last_commit(&dir);
    assert!(
        commit.starts_with("Bob <bob@example.com>\n心得を直した\n"),
        "{commit}"
    );
}

#[test]
fn a_file_outside_the_notebook_cannot_be_read() {
    let dir = temp("get");
    ok(&dir, &["init"]);

    assert!(!run(&dir, &["get", "../etc/passwd"]).status.success());
    assert!(!run(&dir, &["get", "/etc/passwd"]).status.success());
    assert!(!run(&dir, &["get", ".git/config"]).status.success());
}

#[test]
fn a_memory_whose_writer_is_unknown_is_not_written() {
    let dir = temp("unsigned");
    ok(&dir, &["init"]);
    let files = || {
        ["memory/core.md", "memory/long/semantic/tea.md"]
            .map(|rel| std::fs::read_to_string(dir.join(rel)).ok())
    };
    let before = files();

    for args in [
        &["--author", "nobody", "remember", "core", "x"][..],
        &[
            "--author",
            "nobody",
            "keep",
            "semantic",
            "tea",
            "--summary",
            "x",
        ],
    ] {
        assert!(!run(&dir, args).status.success());
    }

    assert_eq!(files(), before);
}
