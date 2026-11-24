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

#[test]
fn records_are_linked_by_code_or_title_and_the_links_are_listed_both_ways() {
    let dir = temp("links");
    ok(&dir, &["init"]);
    let task = ok(&dir, &["add", "tasks", "--title", "見積もり"]);
    let task = task.trim();
    let code = format!("task-{}", &task[20..]);
    let memo = ok(
        &dir,
        &[
            "add",
            "memos",
            "--title",
            "覚え",
            "--text",
            "[[見積もり]] の続き",
            "--field",
            &format!("Related=[{code}]"),
        ],
    );
    let memo = memo.trim();
    ok(&dir, &["remember", "core", &format!("[[{code}]] を先に")]);

    let file = ok(&dir, &["get", &format!("records/memos/{memo}.md")]);
    let link = format!("[[records/tasks/{task}|見積もり]]");
    assert!(
        file.contains(&format!("- Related: {link}\n\n{link} の続き\n")),
        "{file}"
    );
    let shown = ok(&dir, &["links", "tasks", &code, "--json"]);
    let shown: serde_json::Value = serde_json::from_str(&shown).unwrap();
    let back = shown["backlinks"].as_array().unwrap();
    assert_eq!(back.len(), 3, "{shown}");
    assert_eq!(back[2]["source"], "memory/core");
    assert_eq!(back[0]["from"], "Related");
    let out = ok(&dir, &["links", "memos", memo]);
    assert!(
        out.contains(&format!("->\trecords/tasks/{task}\t見積もり\tRelated")),
        "{out}"
    );

    ok(
        &dir,
        &[
            "keep",
            "semantic",
            "estimate",
            "--summary",
            &format!("[[{code}]] の覚え"),
            "--text",
            "詳細",
        ],
    );
    let kept = ok(&dir, &["get", "memory/long/semantic/estimate.md"]);
    assert!(kept.contains(&format!("{link} の覚え")), "{kept}");

    ok(&dir, &["add", "memos", "--title", "見積もり"]);
    let refused = run(
        &dir,
        &["add", "memos", "--title", "x", "--text", "[[見積もり]]"],
    );
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains(&code));
    assert_eq!(ok(&dir, &["check"]), "");
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A shared notebook and two copies of it.
fn copies(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = temp(name);
    let made = dir.join("made");
    ok(&made, &["init"]);
    git(&dir, &["clone", "-q", "--bare", "made", "shared.git"]);
    let shared = dir.join("shared.git");
    let copy = |name: &str| {
        let to = dir.join(name);
        ok(
            &dir,
            &["clone", shared.to_str().unwrap(), to.to_str().unwrap()],
        );
        to
    };
    let (one, two) = (copy("one"), copy("two"));
    (shared, one, two)
}

#[test]
fn what_is_written_in_a_copy_is_sent_and_found_in_another() {
    let (_, one, two) = copies("sync");
    let id = ok(&one, &["add", "tasks", "--title", "見積もり"]);

    let found = ok(&two, &["search", "見積もり"]);
    assert!(found.contains(id.trim()), "{found}");

    ok(&one, &["remember", "episodic", "一"]);
    ok(&two, &["remember", "episodic", "二"]);
    ok(&one, &["sync"]);
    let found = ok(&one, &["search", "--in", "memory/episodic"]);
    assert!(found.contains("一") && found.contains("二"), "{found}");
}

#[test]
fn a_copy_is_read_and_written_while_the_shared_notebook_is_away() {
    let (shared, one, two) = copies("sync-away");
    let gone = shared.with_extension("gone");
    std::fs::rename(&shared, &gone).unwrap();
    let out = run(&one, &["add", "tasks", "--title", "見積もり"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("warning"));
    ok(&one, &["search", "見積もり"]);
    std::fs::rename(&gone, &shared).unwrap();

    ok(&one, &["add", "tasks", "--title", "請求"]);
    let found = ok(&two, &["list", "tasks"]);
    assert!(
        found.contains("見積もり") && found.contains("請求"),
        "{found}"
    );
}

#[test]
fn what_cannot_be_put_together_stays_in_the_copy_and_fails() {
    let (_, one, two) = copies("sync-conflict");
    let id = ok(&one, &["add", "tasks", "--title", "見積もり"]);
    let id = id.trim();
    ok(&two, &["sync"]);
    ok(&one, &["set", "tasks", id, "--title", "見積もり一"]);
    // Written by hand, so that it is not brought in first.
    let path = two.join(format!("records/tasks/{id}.md"));
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("# 見積もり", "# 見積もり二")).unwrap();

    let out = run(&two, &["commit"]);

    assert!(!out.status.success());
    let told = String::from_utf8_lossy(&out.stderr);
    assert!(
        told.contains("committed here") && told.contains(id),
        "{told}"
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("見積もり二")
    );
    assert!(last_commit(&two).contains("手で書き換えた"));
}

#[test]
fn a_notebook_not_cloned_is_not_synced() {
    let dir = temp("sync-none");
    ok(&dir, &["init"]);
    git(&dir, &["remote", "add", "origin", "https://127.0.0.1:1/x"]);
    ok(&dir, &["add", "tasks", "--title", "x"]);
    assert!(!run(&dir, &["sync"]).status.success());
}

#[cfg(unix)]
#[test]
fn a_write_the_shared_notebook_goes_away_from_is_no_failure() {
    use std::os::unix::fs::PermissionsExt as _;
    let (shared, one, _) = copies("sync-gone");
    let hook = shared.join("hooks/update");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    let gone = shared.with_extension("gone");
    let script = format!(
        "#!/bin/sh\nmv '{}' '{}'\nexit 1\n",
        shared.display(),
        gone.display()
    );
    std::fs::write(&hook, script).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    git(&shared, &["config", "core.hooksPath", "hooks"]);

    let out = run(&one, &["add", "tasks", "--title", "見積もり"]);

    let told = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{told}");
    assert!(
        told.contains("warning") && told.contains("not sent"),
        "{told}"
    );
}
