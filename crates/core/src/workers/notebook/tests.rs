use super::*;
use crate::notebook::Writer;
use crate::notebook::testing::TempDir;
use crate::records::{MEMOS, Patch, TASKS};

fn memo(app: &App, title: &str, workgroup: Option<&str>) -> String {
    let patch = Patch {
        notes: Some(format!("zebra {title}")),
        workgroups: workgroup.map(|w| vec![w.to_owned()]),
        ..Patch::titled(title)
    };
    app.records
        .add(Writer::Secretary, MEMOS, patch)
        .unwrap()
        .rel()
}

fn put(app: &App, path: &str, text: &str) {
    let path = app.notebook.root().join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn granted(app: &App, order: Order) -> Notes {
    let until = chrono::Utc::now().timestamp() + 600;
    app.permissions.grant_job(7, &[], until).unwrap();
    Notes::new(app.clone(), 7, order)
}

fn order(notebook: Reading, files: &[&str], search: &[&str]) -> Order {
    Order {
        notebook,
        workgroups: vec!["work".into()],
        files: files.iter().map(|f| (*f).to_owned()).collect(),
        search: search.iter().map(|f| (*f).to_owned()).collect(),
        ..crate::workers::runs::tests::order(crate::workers::Runner::Agent)
    }
}

fn read(notes: &Notes, path: &str) -> anyhow::Result<String> {
    notes.answer(READ, json!({ "path": path }))
}

#[test]
fn each_level_gives_its_tools_and_none_writes() {
    for (given, want) in [
        (order(Reading::None, &[], &[]), vec![]),
        (order(Reading::Excerpt, &[], &[]), vec![]),
        (order(Reading::Named, &[], &[]), vec![]),
        (order(Reading::Named, &["a.md"], &[]), vec![READ]),
        (order(Reading::Search, &[], &["memos"]), vec![READ, SEARCH]),
        (order(Reading::Propose, &[], &[]), vec![PROPOSE]),
    ] {
        assert_eq!(names(&given), want, "{:?}", given.notebook);
    }
}

#[test]
fn only_the_named_files_are_read_whatever_the_path_is_written_as() {
    let tmp = TempDir::new("notes-named");
    let app = crate::runtime::testing::app(tmp.path());
    put(&app, "memory/core.md", "the core");
    put(&app, "memory/other.md", "not given");
    put(&app, ".git/config", "history");
    std::fs::write(tmp.path().join("outside.md"), "outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        app.notebook.root().join("memory/other.md"),
        app.notebook.root().join("memory/link.md"),
    )
    .unwrap();
    let notes = granted(&app, order(Reading::Named, &["memory/core.md"], &[]));

    assert_eq!(read(&notes, "memory/core.md").unwrap(), "the core");
    assert_eq!(read(&notes, "[[memory/core]]").unwrap(), "the core");
    let outside = tmp.path().join("outside.md").display().to_string();
    for path in [
        "memory/other.md",
        "memory/link.md",
        "memory/../memory/other.md",
        "../outside.md",
        outside.as_str(),
        ".git/config",
        "memory",
        "memory/missing.md",
    ] {
        assert!(read(&notes, path).is_err(), "{path}");
    }
    assert!(notes.answer(SEARCH, json!({ "text": "" })).is_err());
    for name in ["notebook_write", PROPOSE] {
        assert!(notes.answer(name, json!({})).is_err(), "{name}");
    }
}

#[test]
fn search_keeps_to_its_kinds_and_the_tasks_workgroups() {
    let tmp = TempDir::new("notes-search");
    let app = crate::runtime::testing::app(tmp.path());
    let ours = memo(&app, "ours", Some("work"));
    let common = memo(&app, "common", None);
    let theirs = memo(&app, "theirs", Some("personal"));
    let task = app
        .records
        .add(Writer::Secretary, TASKS, Patch::titled("zebra task"))
        .unwrap()
        .rel();
    put(&app, "memory/core.md", "zebra memory");
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        app.notebook.root().join("memory/core.md"),
        app.notebook.root().join("records/memos/linked.md"),
    )
    .unwrap();
    let notes = granted(&app, order(Reading::Search, &[], &["memos"]));

    let found = notes.answer(SEARCH, json!({ "text": "zebra" })).unwrap();
    assert!(found.contains(&ours) && found.contains(&common), "{found}");
    for out in [theirs.as_str(), task.as_str(), "memory/", "linked"] {
        assert!(!found.contains(out), "{out}: {found}");
    }
    assert!(read(&notes, &ours).unwrap().contains("zebra ours"));
    for path in [&theirs, &task, "records/memos/linked.md", "memory/core.md"] {
        assert!(read(&notes, path).is_err(), "{path}");
    }
}

#[test]
fn nothing_is_read_once_the_grant_is_over() {
    let tmp = TempDir::new("notes-over");
    let app = crate::runtime::testing::app(tmp.path());
    put(&app, "memory/core.md", "the core");
    let notes = granted(&app, order(Reading::Named, &["memory/core.md"], &[]));

    app.permissions.grant_job(7, &[], 1).unwrap();
    assert!(read(&notes, "memory/core.md").is_err(), "past its deadline");
    let notes = granted(&app, order(Reading::Named, &["memory/core.md"], &[]));
    app.permissions.revoke_job(7).unwrap();
    assert!(read(&notes, "memory/core.md").is_err(), "stopped");
}

#[test]
fn a_proposal_is_kept_apart_and_the_notebook_is_not_written() {
    let tmp = TempDir::new("notes-propose");
    let app = crate::runtime::testing::app(tmp.path());
    let before = app.records.list(MEMOS).unwrap().len();
    let notes = granted(&app, order(Reading::Propose, &[], &[]));

    for n in 0..MOST_PROPOSALS {
        let args = json!({ "title": format!("t{n}"), "content": "c" });
        notes.answer(PROPOSE, args).unwrap();
    }
    assert!(notes.answer(PROPOSE, json!({ "title": "more" })).is_err());
    let long = "x".repeat(MOST_PROPOSED + 1);
    assert!(notes.answer(PROPOSE, json!({ "content": long })).is_err());
    assert_eq!(proposals(&app.store, 7).len(), MOST_PROPOSALS);
    assert_eq!(app.records.list(MEMOS).unwrap().len(), before);
    forget_proposals(&app.store, 7).unwrap();
    assert!(proposals(&app.store, 7).is_empty());
}

fn listed(text: &str) -> Vec<String> {
    text.split(", ")
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_owner_is_asked_before_work_starts_that_mixes_or_searches_or_proposes() {
    let order = |notebook, files: &str, search: &str, tools: &str| Order {
        notebook,
        files: listed(files),
        search: listed(search),
        tools: listed(tools),
        ..Order::default()
    };
    use Reading::*;
    for (what, ordered, asked) in [
        ("named files alone", order(Named, "a.md", "", ""), false),
        (
            "others' words alone",
            order(Named, "", "", "WebSearch, mcp__mail__read"),
            false,
        ),
        (
            "no reading",
            order(Reading::None, "a.md", "", "WebFetch"),
            false,
        ),
        (
            "files and tools of this machine",
            order(Named, "a.md", "", "Read, Bash"),
            false,
        ),
        (
            "files and a connection",
            order(Named, "a.md", "", "mcp__mail__read"),
            true,
        ),
        (
            "files and the web",
            order(Named, "a.md", "", "WebFetch"),
            true,
        ),
        ("a search", order(Search, "", "memos", ""), true),
        ("a proposal", order(Propose, "", "", ""), true),
        (
            "an excerpt and the web",
            order(Excerpt, "", "", "WebFetch"),
            true,
        ),
    ] {
        assert_eq!(!start_reasons(&ordered).is_empty(), asked, "{what}");
    }
}

#[test]
fn work_given_the_notebook_has_no_web_unless_it_is_named() {
    let order = |files: &str, tools: &str| Order {
        files: listed(files),
        tools: listed(tools),
        ..Order::default()
    };
    let web = |tools: Vec<String>| tools.iter().any(|t| t == "WebFetch" || t == "WebSearch");
    assert!(web(handed_tools(&order("", ""), true)));
    assert!(!web(handed_tools(&order("a.md", ""), true)));
    assert!(!web(handed_tools(&order("a.md", "mcp__mail__read"), true)));
    assert!(web(handed_tools(&order("a.md", "Read, WebFetch"), true)));
}
