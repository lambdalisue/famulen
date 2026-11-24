use super::*;
use crate::memory::{EPISODIC, append_episode};
use crate::testing::{TempDir, me, notebook, write};
use crate::{EpisodeState, log};

/// A shared notebook and two copies of it. The shared one is a bare
/// repository, which takes pushes as a served notebook does.
struct Copies {
    tmp: TempDir,
    url: String,
    one: Notebook,
    two: Notebook,
}

impl Copies {
    fn new(name: &str) -> Self {
        let (_made, made) = notebook(name);
        let tmp = TempDir::new(&format!("{name}-copies"));
        let shared = tmp.path().join("shared.git");
        let url = shared.display().to_string();
        let from = made.root().display().to_string();
        let bare = git::command(tmp.path(), &["clone", "-q", "--bare", &from, &url]);
        run(bare, &[]).unwrap();
        let one = clone(&url, tmp.path().join("one")).unwrap();
        let two = clone(&url, tmp.path().join("two")).unwrap();
        Self { tmp, url, one, two }
    }

    fn away<T>(&self, then: impl FnOnce() -> T) -> T {
        let (here, gone) = (Path::new(&self.url), self.tmp.path().join("gone"));
        std::fs::rename(here, &gone).unwrap();
        let out = then();
        std::fs::rename(gone, here).unwrap();
        out
    }
}

fn sync(notebook: &Notebook) -> anyhow::Result<()> {
    Sync::of(notebook).unwrap().sync()
}

fn trouble(result: anyhow::Result<()>) -> Trouble {
    result.unwrap_err().downcast().unwrap()
}

fn put(notebook: &Notebook, rel: &str, text: &str) {
    write(notebook.root(), rel, text);
    notebook.commit(&me(), "x", &[rel]).unwrap();
}

fn read(notebook: &Notebook, rel: &str) -> String {
    std::fs::read_to_string(notebook.root().join(rel)).unwrap()
}

const CORE: &str = "memory/core.md";

#[test]
fn what_one_copy_writes_is_read_in_another() {
    let copies = Copies::new("sync-carry");
    put(&copies.one, CORE, "# 心得\n\n- 一 {id: a}\n");
    sync(&copies.one).unwrap();

    Sync::of(&copies.two).unwrap().pull().unwrap();

    assert_eq!(read(&copies.two, CORE), "# 心得\n\n- 一 {id: a}\n");
    assert_eq!(log(copies.two.root(), 1).unwrap()[0].author, me().author);
}

#[test]
fn episodes_of_one_day_written_apart_are_both_kept() {
    let copies = Copies::new("sync-episodes");
    let at = chrono::NaiveDate::from_ymd_opt(2026, 11, 24)
        .unwrap()
        .and_hms_opt(10, 0, 0)
        .unwrap();
    for (notebook, text) in [(&copies.one, "一"), (&copies.two, "二")] {
        append_episode(&notebook.memory_dir(), at, text, EpisodeState::Waiting).unwrap();
        notebook.commit(&me(), "x", &["memory"]).unwrap();
    }
    sync(&copies.one).unwrap();
    sync(&copies.two).unwrap();
    sync(&copies.one).unwrap();

    let rel = format!("memory/{EPISODIC}/2026-11-24.md");
    let text = read(&copies.one, &rel);
    assert_eq!(text, read(&copies.two, &rel));
    assert!(text.contains("一") && text.contains("二"), "{text}");
    assert_eq!(
        text.matches("{id:").count(),
        2,
        "each item is there once: {text}"
    );
    let merged = log(copies.two.root(), 1).unwrap().remove(0);
    assert!(merged.message.contains("Notebook-Tool:"), "{merged:?}");
}

#[test]
fn different_files_written_apart_are_put_together() {
    let copies = Copies::new("sync-files");
    put(
        &copies.one,
        "records/memos/README.md",
        &format!("{}\n一\n", read(&copies.one, "records/memos/README.md")),
    );
    put(&copies.two, CORE, "# 心得\n\n- 二 {id: b}\n");
    sync(&copies.one).unwrap();
    sync(&copies.two).unwrap();
    assert!(read(&copies.two, "records/memos/README.md").ends_with("一\n"));
    assert!(read(&copies.two, CORE).contains("二"));
}

#[test]
fn an_item_changed_in_two_ways_is_told_and_nothing_here_changes() {
    let copies = Copies::new("sync-conflict");
    put(&copies.one, CORE, "# 心得\n\n- 一 {id: a}\n");
    put(&copies.two, CORE, "# 心得\n\n- 二 {id: a}\n");
    sync(&copies.one).unwrap();
    let before = head(&copies.two).unwrap();

    let told = trouble(sync(&copies.two));

    assert_eq!(told, Trouble::Conflict(vec![CORE.into()]));
    assert_eq!(head(&copies.two).unwrap(), before);
    let again = Sync::of(&copies.two)
        .unwrap()
        .refresh(Duration::from_secs(60));
    assert_eq!(trouble(again), told, "it is told every time");
    assert_eq!(read(&copies.two, CORE), "# 心得\n\n- 二 {id: a}\n");
}

#[test]
fn what_is_written_while_the_shared_notebook_is_away_is_sent_later() {
    let copies = Copies::new("sync-away");
    put(&copies.one, CORE, "# 心得\n\n- 一 {id: a}\n");
    let told = copies.away(|| trouble(sync(&copies.one)));
    assert!(told.passes(), "{told:?}");

    sync(&copies.one).unwrap();
    sync(&copies.two).unwrap();
    assert!(read(&copies.two, CORE).contains("一"));
}

#[test]
fn what_is_not_committed_stays_and_stops_only_what_touches_it() {
    let copies = Copies::new("sync-dirty");
    write(copies.two.root(), CORE, "# 書きかけ\n");
    put(&copies.one, "memory/pending.md", "# 保留\n\n- 一 {id: a}\n");
    sync(&copies.one).unwrap();
    sync(&copies.two).unwrap();
    assert!(read(&copies.two, "memory/pending.md").contains("一"));

    put(&copies.one, CORE, "# 心得\n\n- 一 {id: b}\n");
    sync(&copies.one).unwrap();
    let told = trouble(sync(&copies.two));
    assert!(matches!(told, Trouble::InTheWay(_)), "{told:?}");
    assert_eq!(read(&copies.two, CORE), "# 書きかけ\n");
}

#[cfg(unix)]
#[test]
fn what_is_refused_stays_here_with_the_reason() {
    use std::os::unix::fs::PermissionsExt as _;
    let copies = Copies::new("sync-refused");
    let hook = Path::new(&copies.url).join("hooks/update");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    run(
        git::command(
            Path::new(&copies.url),
            &["config", "core.hooksPath", "hooks"],
        ),
        &[],
    )
    .unwrap();
    put(&copies.one, CORE, "# 心得\n\n- 一 {id: a}\n");

    let told = trouble(sync(&copies.one));

    assert!(
        matches!(&told, Trouble::Refused(why) if why.contains("hook declined")),
        "{told:?}"
    );
    assert!(read(&copies.one, CORE).contains("一"));
}

#[test]
fn a_notebook_not_cloned_is_in_sync_with_nothing() {
    let (_tmp, notebook) = notebook("sync-none");
    run(
        git::command(notebook.root(), &["remote", "add", REMOTE, "x"]),
        &[],
    )
    .unwrap();
    assert!(Sync::of(&notebook).is_none());
}

#[test]
fn a_read_soon_after_another_asks_nothing() {
    let copies = Copies::new("sync-age");
    let sync = Sync::of(&copies.one).unwrap();
    sync.pull().unwrap();
    copies.away(|| {
        sync.refresh(Duration::from_secs(60)).unwrap();
        assert!(sync.refresh(Duration::ZERO).is_err());
    });
}

#[test]
fn what_takes_too_long_is_ended() {
    let mut command = Command::new(if cfg!(windows) { "ping" } else { "sleep" });
    command.args(if cfg!(windows) {
        &["-n", "60", "127.0.0.1"][..]
    } else {
        &["60"]
    });
    let began = Instant::now();
    assert!(!run_within(command, Duration::from_millis(50)).unwrap().ok);
    assert!(began.elapsed() < Duration::from_secs(30));
}

#[test]
fn what_is_no_notebook_is_not_cloned_as_one() {
    let tmp = TempDir::new("sync-other");
    let from = tmp.path().join("from.git");
    let from = from.display().to_string();
    run(
        git::command(tmp.path(), &["init", "-q", "--bare", &from]),
        &[],
    )
    .unwrap();
    assert!(clone(&from, tmp.path().join("to")).is_err());
}

#[test]
fn the_tools_state_is_never_committed_whatever_the_ignore_file_says() {
    let copies = Copies::new("sync-ignore");
    let (_tmp, made) = notebook("sync-ignore-made");
    for notebook in [&copies.one, &made] {
        write(notebook.root(), ".gitignore", "!/.notebook/\n");
        write(notebook.root(), ".notebook/x", "x");
        notebook.commit(&me(), "x", &["."]).unwrap();
        let files = text(plain(notebook, &["ls-files"])).unwrap();
        assert!(!files.contains(".notebook"), "{files}");
    }
}

#[test]
fn one_item_added_in_two_ways_is_told_though_the_lines_go_together() {
    let copies = Copies::new("sync-twice");
    let many: String = (0..20).map(|n| format!("- {n} {{id: k{n}}}\n")).collect();
    put(&copies.one, CORE, &format!("# 心得\n\n{many}"));
    sync(&copies.one).unwrap();
    sync(&copies.two).unwrap();
    put(
        &copies.one,
        CORE,
        &format!("# 心得\n\n- 一 {{id: a}}\n{many}"),
    );
    put(
        &copies.two,
        CORE,
        &format!("# 心得\n\n{many}- 二 {{id: a}}\n"),
    );
    sync(&copies.one).unwrap();
    assert_eq!(
        trouble(sync(&copies.two)),
        Trouble::Conflict(vec![CORE.into()])
    );
}
