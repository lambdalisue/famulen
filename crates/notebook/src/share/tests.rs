use super::*;
use crate::testing::{TempDir, me, notebook, write};
use crate::{Author, log};

const TASK: &str = "records/tasks/01kat3v8q4zj6m2x9r5c7w1hbn.md";

/// A notebook and a copy of it elsewhere.
struct Shared {
    _tmp: TempDir,
    _copy_tmp: TempDir,
    notebook: Notebook,
    copy: PathBuf,
}

impl Shared {
    fn new(name: &str) -> Self {
        let (tmp, notebook) = notebook(name);
        let copy_tmp = TempDir::new(&format!("{name}-copy"));
        let copy = copy_tmp.path().join("copy");
        let from = notebook.root().display().to_string();
        let to = copy.display().to_string();
        copy_git(
            copy_tmp.path(),
            &["clone", "-q", "-c", "core.autocrlf=false", &from, &to],
        );
        Self {
            _tmp: tmp,
            _copy_tmp: copy_tmp,
            notebook,
            copy,
        }
    }

    fn git(&self, args: &[&str]) -> Vec<u8> {
        copy_git(&self.copy, args)
    }

    /// Commit everything changed in the copy.
    fn commit(&self) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", "from the copy"]);
    }

    fn head(&self) -> String {
        head(&self.notebook).unwrap()
    }

    fn copy_head(&self) -> String {
        String::from_utf8(self.git(&["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn push(&self) -> Outcome {
        self.push_to("refs/heads/main", &self.copy_head())
    }

    fn push_to(&self, to: &str, commit: &str) -> Outcome {
        let head = self.head();
        let pushed = push(&self.notebook, &self.request(&head, commit, to), || {
            Ok(true)
        })
        .unwrap();
        let answer = String::from_utf8(pushed.answer).unwrap();
        assert!(answer.contains("unpack ok\n"), "{answer}");
        let sign = match &pushed.outcome {
            Outcome::Taken(_) => format!("ok {to}\n"),
            _ => format!("ng {to} "),
        };
        assert!(answer.contains(&sign), "{answer}");
        pushed.outcome
    }

    fn request(&self, from: &str, commit: &str, to: &str) -> Vec<u8> {
        let mut request = line(&format!("{from} {commit} {to}\0 report-status"));
        request.extend(b"0000");
        if commit.bytes().any(|b| b != b'0') {
            let mut pack = git::command(&self.copy, &["pack-objects", "--revs", "--stdout", "-q"]);
            pack.env_remove("GIT_OBJECT_DIRECTORY");
            request.extend(run(pack, format!("{commit}\n^{from}\n").as_bytes()).unwrap());
        }
        request
    }

    fn refused(&self) -> String {
        let (head, log_before) = (self.head(), self.status());
        match self.push() {
            Outcome::Refused(why) => {
                assert_eq!(self.head(), head);
                assert_eq!(self.status(), log_before);
                why
            }
            other => panic!("{other:?}"),
        }
    }

    /// What is not committed in the notebook.
    fn status(&self) -> String {
        text(plain(&self.notebook, &["status", "--porcelain"])).unwrap()
    }
}

fn copy_git(dir: &Path, args: &[&str]) -> Vec<u8> {
    let mut command = git::command(dir, args);
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    for who in ["AUTHOR", "COMMITTER"] {
        command
            .env(format!("GIT_{who}_NAME"), "Bob")
            .env(format!("GIT_{who}_EMAIL"), "bob@example.com");
    }
    run(command, &[]).unwrap()
}

#[test]
fn a_push_is_in_the_working_tree_and_the_history_with_its_author() {
    let shared = Shared::new("share-push");
    write(
        &shared.copy,
        "memory/core.md",
        "# 心得\n\n- 短く {id: m1}\n",
    );
    std::fs::remove_file(shared.copy.join("memory/pending.md")).unwrap();
    shared.commit();

    let Outcome::Taken(taken) = shared.push() else {
        panic!()
    };

    assert_eq!(taken.files, ["memory/core.md", "memory/pending.md"]);
    assert_eq!(taken.commit, shared.copy_head());
    let root = shared.notebook.root();
    let core = std::fs::read_to_string(root.join("memory/core.md")).unwrap();
    assert_eq!(core, "# 心得\n\n- 短く {id: m1}\n");
    assert!(!root.join("memory/pending.md").exists());
    assert_eq!(shared.status(), "");
    let last = log(root, 1).unwrap().remove(0);
    assert_eq!(last.author, Author::new("Bob", "bob@example.com"));
    let left: Vec<_> = std::fs::read_dir(shared.notebook.state_dir())
        .unwrap()
        .filter(|f| {
            let name = f.as_ref().unwrap().file_name();
            name.to_string_lossy().starts_with("incoming")
        })
        .collect();
    assert!(left.is_empty());
}

#[test]
fn only_what_goes_on_from_the_notebook_is_taken() {
    let shared = Shared::new("share-history");
    write(&shared.copy, "memory/core.md", "# 心得\n\n- 一 {id: m1}\n");
    shared.commit();
    write(shared.notebook.root(), "memory/pending.md", "# 保留\n");
    shared.notebook.commit(&me(), "x", &["."]).unwrap();
    // The copy does not have the notebook's newest commit.
    let old = String::from_utf8(shared.git(&["rev-parse", "HEAD~1"])).unwrap();
    let stale = shared.request(old.trim(), &shared.copy_head(), "refs/heads/main");
    let pushed = push(&shared.notebook, &stale, || Ok(true)).unwrap();
    assert_eq!(pushed.outcome, Outcome::Refused("fetch first".into()));

    let shared = Shared::new("share-rewrite");
    shared.git(&["commit", "-q", "--amend", "-m", "rewritten"]);
    assert!(shared.refused().contains("history"));
    let zero = "0".repeat(40);
    let Outcome::Refused(why) = shared.push_to("refs/heads/main", &zero) else {
        panic!()
    };
    assert!(why.contains("deleted"), "{why}");
    let Outcome::Refused(why) = shared.push_to("refs/heads/other", &shared.copy_head()) else {
        panic!()
    };
    assert!(why.contains("only refs/heads/main"), "{why}");
}

#[test]
fn what_a_notebook_cannot_hold_is_refused() {
    for path in [
        ".gitattributes",
        "records/tasks/.GitAttributes",
        ".gitmodules",
        ".notebook/index.db",
        ".Notebook/x",
    ] {
        let shared = Shared::new("share-path");
        write(&shared.copy, path, "x\n");
        shared.git(&["add", "-f", path]);
        shared.git(&["commit", "-qm", "x"]);
        assert!(shared.refused().contains("cannot be in"), "{path}");
    }
    let shared = Shared::new("share-link");
    let object = String::from_utf8(shared.git(&["rev-parse", "HEAD:README.md"])).unwrap();
    for mode in ["120000", "160000"] {
        let info = format!("{mode},{},x", object.trim());
        shared.git(&["update-index", "--add", "--cacheinfo", &info]);
        shared.git(&["commit", "-qm", "x"]);
        let why = shared.refused();
        assert!(why.contains("not an ordinary file"), "{why}");
        shared.git(&["reset", "-q", "--hard", "HEAD~1"]);
    }
    write(&shared.copy, "x.md", &"x".repeat(FILE_LIMIT as usize + 1));
    shared.commit();
    assert!(shared.refused().contains("too large"));
    std::fs::remove_file(shared.copy.join("x.md")).unwrap();
    shared.commit();
    assert!(
        shared.refused().contains("too large"),
        "it is in the history"
    );
}

#[test]
fn what_would_be_newly_wrong_is_refused_and_what_already_was_is_not() {
    let shared = Shared::new("share-rules");
    write(&shared.copy, TASK, "---\nid: wrong\n---\n\n# x\n");
    shared.commit();
    let why = shared.refused();
    assert!(why.contains(TASK), "{why}");

    let shared = Shared::new("share-rules-before");
    let root = shared.notebook.root();
    write(
        root,
        "memory/core.md",
        "# 心得\n\n- [[どこにもない]] {id: m1}\n",
    );
    shared.notebook.commit(&me(), "x", &["."]).unwrap();
    shared.git(&["pull", "-q"]);
    write(
        &shared.copy,
        "memory/core.md",
        "# 心得\n\n- [[どこにもない]] {id: m1}\n- 足した {id: m2}\n",
    );
    shared.commit();
    assert!(matches!(shared.push(), Outcome::Taken(_)));
}

#[test]
fn a_link_to_what_the_same_push_adds_is_not_wrong() {
    let shared = Shared::new("share-linked");
    let id = "01kat3v8q4zj6m2x9r5c7w1hbn";
    write(
        &shared.copy,
        TASK,
        &format!("---\nid: {id}\n---\n\n# 見積もり\n"),
    );
    write(
        &shared.copy,
        "memory/core.md",
        &format!("# 心得\n\n- [[records/tasks/{id}|見積もり]] {{id: m1}}\n"),
    );
    shared.commit();
    let outcome = shared.push();
    assert!(matches!(outcome, Outcome::Taken(_)), "{outcome:?}");
}

#[test]
fn changes_not_committed_stay_and_a_push_that_touches_them_is_refused() {
    let shared = Shared::new("share-dirty");
    let root = shared.notebook.root();
    write(root, "memory/core.md", "# 心得\n\n- 書きかけ {id: m1}\n");
    write(root, "memory/working/x.md", "# x\n");
    write(
        &shared.copy,
        "memory/pending.md",
        "# 保留\n\n- 一 {id: m2}\n",
    );
    shared.commit();
    assert!(matches!(shared.push(), Outcome::Taken(_)));
    let kept = std::fs::read_to_string(root.join("memory/core.md")).unwrap();
    assert!(kept.contains("書きかけ"));

    write(&shared.copy, "memory/core.md", "# 心得\n\n- 別 {id: m3}\n");
    shared.commit();
    let why = shared.refused();
    assert!(why.contains("not yet committed"), "{why}");
    shared.git(&["reset", "-q", "--hard", "HEAD~1"]);

    // A new file in the way is kept too.
    write(&shared.copy, "memory/working/x.md", "# 別\n");
    shared.commit();
    assert!(shared.refused().contains("not yet committed"));
    assert_eq!(packs(&shared), 1, "what is refused is not kept");
}

/// How many packs the notebook has.
fn packs(shared: &Shared) -> usize {
    std::fs::read_dir(shared.notebook.root().join(".git/objects/pack"))
        .unwrap()
        .filter(|f| {
            let name = f.as_ref().unwrap().file_name();
            name.to_string_lossy().ends_with(".pack")
        })
        .count()
}

#[test]
fn a_file_touched_but_left_as_it_was_does_not_stand_in_the_way() {
    let shared = Shared::new("share-touched");
    let path = shared.notebook.root().join("memory/core.md");
    let text = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, text).unwrap();
    write(&shared.copy, "memory/core.md", "# 心得\n\n- 一 {id: m1}\n");
    shared.commit();
    assert!(matches!(shared.push(), Outcome::Taken(_)));
}

#[cfg(unix)]
#[test]
fn the_notebooks_hooks_do_not_run_for_a_push() {
    use std::os::unix::fs::PermissionsExt as _;
    let shared = Shared::new("share-hooks");
    let ran = shared._tmp.path().join(".notebook/ran");
    let hooks = shared.notebook.root().join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    for hook in [
        "post-index-change",
        "reference-transaction",
        "post-checkout",
    ] {
        let path = hooks.join(hook);
        std::fs::write(&path, format!("#!/bin/sh\ntouch '{}'\n", ran.display())).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    write(&shared.copy, "memory/core.md", "# 心得\n\n- 一 {id: m1}\n");
    shared.commit();
    assert!(matches!(shared.push(), Outcome::Taken(_)));
    assert!(!ran.exists());
}

#[test]
fn a_pack_that_lacks_what_the_commit_needs_is_refused() {
    let shared = Shared::new("share-broken");
    write(&shared.copy, "memory/core.md", "# 心得\n\n- 一 {id: m1}\n");
    shared.commit();
    let mut request = line(&format!(
        "{} {} refs/heads/main\0 report-status",
        shared.head(),
        shared.copy_head()
    ));
    request.extend(b"0000");
    // The commit alone, without its tree and file.
    let pack = git::command(&shared.copy, &["pack-objects", "--stdout", "-q"]);
    request.extend(run(pack, format!("{}\n", shared.copy_head()).as_bytes()).unwrap());
    let pushed = push(&shared.notebook, &request, || Ok(true)).unwrap();
    assert!(
        matches!(pushed.outcome, Outcome::Refused(_)),
        "{:?}",
        pushed.outcome
    );
}

#[test]
fn what_is_no_push_is_an_error_and_an_opening_takes_nothing() {
    let shared = Shared::new("share-junk");
    for junk in [
        &b""[..],
        b"zzzz",
        b"0001",
        b"0009hello",
        b"ffff",
        "0005あ".as_bytes(),
    ] {
        assert!(
            push(&shared.notebook, junk, || Ok(true)).is_err(),
            "{junk:?}"
        );
    }
    let opened = push(&shared.notebook, b"0000", || Ok(true)).unwrap();
    assert_eq!(opened.outcome, Outcome::Nothing);
}

#[test]
fn only_the_notebooks_branch_is_shown() {
    let shared = Shared::new("share-refs");
    let root = shared.notebook.root();
    for args in [["branch", "private"], ["tag", "v1"]] {
        run(git::command(root, &args), &[]).unwrap();
    }
    for shown in [
        refs_to_fetch(&shared.notebook).unwrap(),
        refs_to_push(&shared.notebook).unwrap(),
    ] {
        let shown = String::from_utf8_lossy(&shown).into_owned();
        assert!(shown.contains("refs/heads/main"), "{shown}");
        assert!(
            !shown.contains("private") && !shown.contains("tags"),
            "{shown}"
        );
    }
}

/// Give the copy a commit whose tree is `tree`, and push it.
fn push_tree(shared: &Shared, tree: &str) -> String {
    let head = shared.copy_head();
    let commit = String::from_utf8(shared.git(&["commit-tree", tree, "-p", &head, "-m", "x"]));
    match shared.push_to("refs/heads/main", commit.unwrap().trim()) {
        Outcome::Refused(why) => why,
        other => panic!("{other:?}"),
    }
}

fn make(shared: &Shared, args: &[&str], input: &[u8]) -> String {
    let out = run(git::command(&shared.copy, args), input).unwrap();
    String::from_utf8(out).unwrap().trim().to_owned()
}

#[test]
fn a_small_push_that_is_large_once_unpacked_is_refused() {
    let shared = Shared::new("share-bomb");
    let blob = make(
        &shared,
        &["hash-object", "-w", "--stdin"],
        &vec![b'x'; FILE_LIMIT as usize],
    );
    let many: String = (0..CHANGES_LIMIT / FILE_LIMIT + 1)
        .map(|n| format!("100644 blob {blob}\t{n:03}.md\n"))
        .collect();
    let tree = make(&shared, &["mktree"], many.as_bytes());
    assert!(push_tree(&shared, &tree).contains("together"));

    let empty = make(&shared, &["hash-object", "-w", "--stdin"], b"x");
    let mut tree = make(
        &shared,
        &["mktree"],
        format!("100644 blob {empty}\tx.md\n").as_bytes(),
    );
    for _ in 0..6 {
        let ten: String = (0..10)
            .map(|n| format!("040000 tree {tree}\t{n}\n"))
            .collect();
        tree = make(&shared, &["mktree"], ten.as_bytes());
    }
    assert!(push_tree(&shared, &tree).contains("too many files"));
}

#[test]
fn a_notebook_named_relative_to_where_the_program_runs_takes_a_push() {
    let shared = Shared::new("share-relative");
    write(&shared.copy, "memory/core.md", "# 心得\n\n- 一 {id: m1}\n");
    shared.commit();
    let here = std::env::current_dir().unwrap();
    let up = here.components().skip(1).map(|_| "..").collect::<PathBuf>();
    let root = shared.notebook.root().canonicalize().unwrap();
    // On Windows a path on another drive cannot be named relative to here.
    let Ok(rest) = root.strip_prefix(here.ancestors().last().unwrap()) else {
        return;
    };
    let notebook = Notebook::new(up.join(rest));

    let request = shared.request(&shared.head(), &shared.copy_head(), "refs/heads/main");
    let pushed = push(&notebook, &request, || Ok(true)).unwrap();

    assert!(
        matches!(pushed.outcome, Outcome::Taken(_)),
        "{:?}",
        pushed.outcome
    );
}

#[test]
fn what_is_huge_once_unpacked_never_reaches_git() {
    use flate2::write::ZlibEncoder;
    let pack = |kind: u8, size: u64, body: &[u8], base: &[u8]| {
        let mut pack = b"PACK\0\0\0\x02\0\0\0\x01".to_vec();
        let mut rest = size >> 4;
        let more = |rest: u64| if rest > 0 { 0x80 } else { 0 };
        pack.push(more(rest) | kind << 4 | (size & 0x0f) as u8);
        while rest > 0 {
            let low = (rest & 0x7f) as u8;
            rest >>= 7;
            pack.push(more(rest) | low);
        }
        pack.extend(base);
        let mut zip = ZlibEncoder::new(pack, flate2::Compression::fast());
        zip.write_all(body).unwrap();
        zip.finish().unwrap()
    };
    let check = |pack: &[u8]| super::pack::bounded(pack, 20, 1000, 1500);
    let base = [0; 20];
    check(&pack(3, 1000, &[0; 1000], &[])).unwrap();
    assert!(check(&pack(3, 1001, &[0; 1001], &[])).is_err());
    assert!(check(&pack(3, 10, &[0; 1001], &[])).is_err(), "it lies");
    check(&pack(7, 4, &[0xe8, 7, 0xe8, 7], &base)).unwrap();
    let huge_delta = [0xe8, 7, 0xff, 0xff, 0xff, 0x7f];
    assert!(check(&pack(7, 6, &huge_delta, &base)).is_err());
    assert!(check(b"PACK").is_err());

    let mut twice = pack(3, 1000, &[0; 1000], &[]);
    let one = twice[12..].to_vec();
    twice.extend(one);
    twice[11] = 2;
    assert!(check(&twice).is_err(), "too much together");
}

#[test]
fn a_push_still_on_its_way_when_sharing_ends_is_not_taken() {
    let shared = Shared::new("share-ended");
    write(&shared.copy, "memory/core.md", "# 心得\n\n- 一 {id: m1}\n");
    shared.commit();
    let head = shared.head();
    let request = shared.request(&head, &shared.copy_head(), "refs/heads/main");
    let pushed = push(&shared.notebook, &request, || Ok(false)).unwrap();
    assert!(matches!(pushed.outcome, Outcome::Refused(_)));
    assert_eq!(shared.head(), head);
}
