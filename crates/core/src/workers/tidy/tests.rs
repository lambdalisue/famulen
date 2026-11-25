use std::sync::Mutex;

use chrono::TimeZone as _;
use futures_util::future::BoxFuture;

use super::*;
use crate::notebook::Writer;
use crate::notebook::testing::TempDir;
use crate::records::Patch;
use crate::workers::order::tests::delegated;
use crate::workers::{Order, Outcome};

type Meanwhile = Box<dyn FnOnce() + Send>;

/// A machine that answers as it is told to, and removes for real.
struct Fake {
    sessions: Mutex<anyhow::Result<Vec<Session>>>,
    /// What a program answers instead, by a part of its command line.
    says: Mutex<Vec<(&'static str, Ran)>>,
    /// Done once a part of a command line is run.
    then: Mutex<Option<(&'static str, Meanwhile)>>,
    ran: Mutex<Vec<String>>,
    /// The workspaces jj keeps, and where.
    workspaces: Mutex<Vec<(String, PathBuf)>>,
    vms: Mutex<Vec<String>>,
    /// The files other processes have open.
    open: Mutex<Vec<PathBuf>>,
}

fn says(out: &str) -> Ran {
    Ran {
        ok: true,
        out: out.into(),
        err: String::new(),
    }
}

fn fails(err: &str) -> Ran {
    Ran {
        ok: false,
        out: String::new(),
        err: err.into(),
    }
}

impl Fake {
    fn answer(&self, line: &str, args: &[&str]) -> Ran {
        let said = self
            .says
            .lock()
            .unwrap()
            .iter()
            .find(|(part, _)| line.contains(part))
            .map(|(_, ran)| ran.clone());
        if let Some(ran) = said {
            return ran;
        }
        if line.contains("workspace list") {
            let all = self.workspaces.lock().unwrap();
            return says(
                &all.iter()
                    .map(|(name, root)| format!("{name}\t{}\n", root.display()))
                    .collect::<String>(),
            );
        }
        if line.contains("workspace remove") {
            let name = *args.last().unwrap();
            let mut all = self.workspaces.lock().unwrap();
            let at = all.iter().position(|(n, _)| n == name).unwrap();
            std::fs::remove_dir_all(all.remove(at).1).unwrap();
            return says("");
        }
        if line.starts_with("jj") && line.ends_with("status") {
            return says("The working copy has no changes.\nWorking copy  (@) : a\n");
        }
        if line.contains("trunk() & remote_bookmarks()") {
            return says("push-x\torigin\nmain\tgit\nmain\torigin\n");
        }
        if line.starts_with("lsof") {
            let mut out = format!(
                "p{}\nn/usr/bin/famulen\np7\nn/bin/sh\np8\n",
                std::process::id()
            );
            for file in self.open.lock().unwrap().iter() {
                out.push_str(&format!("n{}\n", file.display()));
            }
            return says(&out);
        }
        if line.starts_with("ps") {
            return says(&format!(
                "    1     0\n{} 501\n  7 501\n  8 501\n",
                std::process::id()
            ));
        }
        if line == "vitro ls --json" {
            let vms: Vec<_> = self
                .vms
                .lock()
                .unwrap()
                .iter()
                .map(|n| serde_json::json!({ "name": n }))
                .collect();
            return says(&serde_json::json!({ "vms": vms }).to_string());
        }
        if line.starts_with("vitro destroy") {
            self.vms.lock().unwrap().retain(|n| n != args[1]);
        }
        says("")
    }
}

impl Ground for Fake {
    fn sessions<'a>(&'a self, _run: &'a Run) -> BoxFuture<'a, anyhow::Result<Vec<Session>>> {
        Box::pin(async move {
            match &*self.sessions.lock().unwrap() {
                Ok(all) => Ok(all.clone()),
                Err(error) => anyhow::bail!("{error}"),
            }
        })
    }

    fn run<'a>(
        &'a self,
        _dir: &'a Path,
        program: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, anyhow::Result<Ran>> {
        Box::pin(async move {
            let line = format!("{program} {}", args.join(" "));
            self.ran.lock().unwrap().push(line.clone());
            let ran = self.answer(&line, args);
            let mut then = self.then.lock().unwrap();
            if then.as_ref().is_some_and(|(part, _)| line.contains(part)) {
                (then.take().unwrap().1)();
            }
            Ok(ran)
        })
    }

    fn run_whole<'a>(
        &'a self,
        dir: &'a Path,
        program: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, anyhow::Result<Ran>> {
        self.run(dir, program, args)
    }
}

struct Bench {
    app: App,
    fake: Arc<Fake>,
    checkout: PathBuf,
    dir: TempDir,
}

/// A task delegated to a session, the session's run over.
struct Work {
    task: String,
    code: String,
    session: String,
    path: PathBuf,
}

fn bench(name: &str) -> Bench {
    let dir = TempDir::new(name);
    let app = crate::runtime::testing::app(&dir.path().join("data"));
    let checkout = dir.path().join("src");
    std::fs::create_dir_all(checkout.join(".jj/repo")).unwrap();
    let checkout = checkout.canonicalize().unwrap();
    std::fs::create_dir_all(checkout.join(".claude/worktrees")).unwrap();
    let fake = Fake {
        sessions: Mutex::new(Ok(Vec::new())),
        says: Mutex::default(),
        then: Mutex::default(),
        ran: Mutex::default(),
        workspaces: Mutex::new(vec![("default".into(), checkout.clone())]),
        vms: Mutex::default(),
        open: Mutex::default(),
    };
    Bench {
        app,
        fake: Arc::new(fake),
        checkout,
        dir,
    }
}

impl Bench {
    fn task(&self, title: &str) -> (String, String) {
        let patch = delegated(
            title,
            &[
                ("Runner", "adroit"),
                ("Directory", self.checkout.to_str().unwrap()),
                ("Worktree", "yes"),
            ],
        )
        .with("Status", "done");
        let id = self
            .app
            .records
            .add(Writer::Secretary, TASKS, patch)
            .unwrap()
            .id;
        let code = self.app.records.code(TASKS, &id).unwrap();
        (id, code)
    }

    /// A worktree as Adroit makes one, whose task is done and session closed.
    fn work(&self) -> Work {
        let (task, code) = self.task("直す");
        self.run(task, code)
    }

    fn run(&self, task: String, code: String) -> Work {
        let order = Order::of(&self.app.records.need(TASKS, &task).unwrap(), &code).unwrap();
        let run = self.app.workers.runs.begin(&task, &order).unwrap();
        let session = format!("s{}", run.id);
        self.app.workers.runs.running(run.id, &session).unwrap();
        self.app
            .workers
            .runs
            .ending(run.id, Outcome::Done, "")
            .unwrap();
        self.app.workers.runs.ended(run.id).unwrap();
        let name = format!("{code}-run{}-x", run.id);
        let path = self.worktree(&name);
        self.fake
            .sessions
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .push(Session {
                id: session.clone(),
                dir: path.clone(),
                ended: true,
            });
        Work {
            task,
            code,
            session,
            path,
        }
    }

    fn worktree(&self, name: &str) -> PathBuf {
        let path = place(&self.checkout, name);
        std::fs::create_dir_all(path.join(".jj")).unwrap();
        std::fs::write(path.join(".jj/repo"), "../../../../.jj/repo").unwrap();
        std::fs::write(path.join("main.rs"), "x").unwrap();
        self.fake
            .workspaces
            .lock()
            .unwrap()
            .push((name.into(), path.clone()));
        path
    }

    fn session(&self, id: &str, dir: &Path, ended: bool) {
        let mut sessions = self.fake.sessions.lock().unwrap();
        let sessions = sessions.as_mut().unwrap();
        sessions.retain(|s| s.id != id);
        sessions.push(Session {
            id: id.into(),
            dir: dir.into(),
            ended,
        });
    }

    fn status(&self, task: &str, status: &str) {
        self.app
            .records
            .change(
                Writer::Owner,
                TASKS,
                task,
                Patch::default().with("Status", status),
            )
            .unwrap();
    }

    fn says(&self, part: &'static str, ran: Ran) {
        self.fake.says.lock().unwrap().push((part, ran));
    }

    async fn tidy(&self, how: How) -> Vec<Left> {
        Tidy::new(&self.app, self.fake.clone())
            .all(how)
            .await
            .unwrap()
    }

    /// What became of the only thing there is, which was not removed.
    async fn stays(&self, work: &Work) -> Fate {
        let left = self.tidy(How::Remove).await;
        assert_eq!(left.len(), 1, "{left:?}");
        assert!(work.path.join("main.rs").exists(), "{left:?}");
        assert!(!self.ran("workspace remove"));
        left[0].fate.clone()
    }

    async fn is_kept(&self, work: &Work, why: &str) {
        match self.stays(work).await {
            Fate::Kept(said) if said.contains(why) => {}
            other => panic!("{why}: {other:?}"),
        }
    }

    async fn is_in_use(&self, work: &Work) {
        let fate = self.stays(work).await;
        assert!(matches!(fate, Fate::InUse(_)), "{fate:?}");
    }

    fn ran(&self, part: &str) -> bool {
        self.fake
            .ran
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains(part))
    }

    fn told(&self) -> Vec<Input> {
        self.app
            .store
            .recent_inputs(0, 50)
            .unwrap()
            .into_iter()
            .map(|(stored, _)| stored.input)
            .collect()
    }
}

#[tokio::test]
async fn a_worktree_whose_work_is_over_and_in_the_trunk_is_removed_and_written_down() {
    for status in ["done", "cancelled"] {
        let b = bench("tidy-removed");
        let w = b.work();
        b.status(&w.task, status);
        let left = b.tidy(How::Remove).await;
        assert_eq!(left[0].fate, Fate::Removed, "{status}");
        assert_eq!(left[0].bytes, Some("x../../../../.jj/repo".len() as u64));
        assert!(!w.path.exists());
        assert!(b.checkout.join(".jj/repo").exists());
        let written = b.app.journal.recent(5).unwrap();
        assert!(
            written
                .iter()
                .any(|e| e.detail.contains(w.path.to_str().unwrap())),
            "{written:?}"
        );
        // What was asked of jj: the trunk was fetched, and it alone.
        assert!(b.ran("git fetch --remote origin --branch main"));
        assert!(b.ran(r#""main"@"origin"..@ ~ empty()"#));
        assert!(!b.ran("forget"));
    }
}

#[tokio::test]
async fn a_preview_removes_nothing() {
    let b = bench("tidy-preview");
    let w = b.work();
    let vm = format!("{}-e2e", w.code);
    b.fake.vms.lock().unwrap().push(vm.clone());
    let left = b.tidy(How::Preview).await;
    assert!(left.iter().all(|l| l.fate == Fate::Removable), "{left:?}");
    assert_eq!(left.len(), 2);
    assert!(w.path.join("main.rs").exists());
    assert_eq!(*b.fake.vms.lock().unwrap(), [vm]);
    assert!(!b.ran("workspace remove") && !b.ran("vitro destroy"));
}

#[tokio::test]
async fn one_whose_task_or_session_may_go_on_is_left_without_a_word() {
    for status in ["pending", "in_progress", "waiting"] {
        let b = bench("tidy-task-open");
        let w = b.work();
        b.status(&w.task, status);
        b.is_in_use(&w).await;
    }
    let b = bench("tidy-session-open");
    let w = b.work();
    b.session(&w.session, &w.path, false);
    b.is_in_use(&w).await;
    assert!(!b.ran("jj"), "nothing is asked of it");
}

#[tokio::test]
async fn one_whose_run_is_open_is_left() {
    let b = bench("tidy-run-open");
    let w = b.work();
    b.app
        .store
        .conn()
        .execute("UPDATE worker_runs SET state = 'running'", [])
        .unwrap();
    b.is_in_use(&w).await;
}

#[tokio::test]
async fn one_whose_session_is_not_known_to_be_closed_is_kept() {
    let b = bench("tidy-session-gone");
    let w = b.work();
    *b.fake.sessions.lock().unwrap() = Ok(Vec::new());
    b.is_kept(&w, "知らない").await;
    *b.fake.sessions.lock().unwrap() = Err(anyhow::anyhow!("away"));
    b.is_kept(&w, "away").await;
    b.app
        .store
        .conn()
        .execute("UPDATE worker_runs SET session = ''", [])
        .unwrap();
    b.is_kept(&w, "どのセッション").await;
}

#[tokio::test]
async fn one_with_unsaved_changes_is_kept() {
    for status in [
        "Working copy changes:\nM a\nWorking copy  (@) : a\n",
        "The working copy has no changes.\nUntracked paths:\n? big\n",
        "",
    ] {
        let b = bench("tidy-unsaved");
        let w = b.work();
        b.says("--no-pager status", says(status));
        b.is_kept(&w, "未保存の変更がある").await;
    }
    let b = bench("tidy-too-big");
    let w = b.work();
    b.says(
        "--no-pager status",
        Ran {
            err: "Warning: Refused to snapshot some files:".into(),
            ..says("The working copy has no changes.\n")
        },
    );
    b.is_kept(&w, "未保存の変更がある").await;
}

#[tokio::test]
async fn one_git_sees_changes_in_is_kept() {
    let b = bench("tidy-git");
    let w = b.work();
    std::fs::write(w.path.join(".git"), "gitdir: x").unwrap();
    b.says("git --no-optional-locks status", says("?? nested/\n"));
    b.is_kept(&w, "未保存の変更がある").await;
}

#[tokio::test]
async fn one_with_changes_outside_the_trunk_is_kept() {
    let b = bench("tidy-apart");
    let w = b.work();
    b.says("~ empty()", says("vutnkkuz\n"));
    b.is_kept(&w, "vutnkkuz").await;
}

#[tokio::test]
async fn whatever_cannot_be_asked_keeps_it() {
    for part in [
        "workspace list",
        "--no-pager status",
        "trunk() & remote_bookmarks()",
        "git fetch",
        "~ empty()",
        "ps",
        "lsof",
    ] {
        let b = bench("tidy-fails");
        let w = b.work();
        // What it printed before it failed counts for nothing.
        b.says(
            part,
            Ran {
                out: "The working copy has no changes.\n".into(),
                ..fails("Operation not permitted")
            },
        );
        b.is_kept(&w, "Operation not permitted").await;
    }
}

#[tokio::test]
async fn one_whose_trunk_is_not_on_a_remote_is_kept() {
    let b = bench("tidy-no-trunk");
    let w = b.work();
    b.says(
        "trunk() & remote_bookmarks()",
        says("main\tgit\npush-x\torigin\n"),
    );
    b.is_kept(&w, "本流").await;
    assert!(!b.ran("git fetch"));
}

#[tokio::test]
async fn one_a_process_or_another_session_works_in_is_kept() {
    let b = bench("tidy-process");
    let w = b.work();
    b.fake.open.lock().unwrap().push(w.path.join("target/x"));
    b.is_kept(&w, "プロセスがここを使っている（8）").await;

    let b = bench("tidy-other-session");
    let w = b.work();
    b.session("other", &w.path.join("crates"), false);
    b.is_kept(&w, "ほかのセッション").await;
}

#[tokio::test]
async fn processes_that_cannot_be_seen_keep_it() {
    let own = std::process::id();
    let seen = format!("p{own}\nn/x\n");
    for (program, said, why) in [
        ("lsof", seen, "lsof"),
        ("ps ", format!("{own} 501\n"), "ps"),
        ("ps ", "1 0\n7 501\n".into(), "ps"),
    ] {
        let b = bench("tidy-blind");
        let w = b.work();
        b.says(program, says(&said));
        b.is_kept(&w, &format!("{why} does not see")).await;
    }
}

#[tokio::test]
async fn what_begins_while_it_is_asked_keeps_it() {
    let b = bench("tidy-meanwhile");
    let w = b.work();
    let (fake, dir) = (b.fake.clone(), w.path.clone());
    *b.fake.then.lock().unwrap() = Some((
        "~ empty()",
        Box::new(move || {
            fake.sessions
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .push(Session {
                    id: "resumed".into(),
                    dir,
                    ended: false,
                });
        }),
    ));
    b.is_kept(&w, "ほかのセッション").await;
}

#[tokio::test]
async fn a_repository_inside_it_keeps_it() {
    for inside in [".claude/worktrees/nest/.jj", "target/clone/.git"] {
        let b = bench("tidy-nested");
        let w = b.work();
        std::fs::create_dir_all(w.path.join(inside)).unwrap();
        b.is_kept(&w, "a repository of its own").await;
    }
    let b = bench("tidy-nested-listed");
    let w = b.work();
    let nest = w.path.join("nest");
    b.fake
        .workspaces
        .lock()
        .unwrap()
        .push(("nest".into(), nest));
    b.is_kept(&w, "another workspace").await;
}

#[tokio::test]
async fn one_delegated_work_is_yet_to_begin_in_is_left() {
    let b = bench("tidy-awaited");
    let w = b.work();
    let inside = delegated(
        "続き",
        &[
            ("Runner", "adroit"),
            ("Directory", w.path.to_str().unwrap()),
        ],
    );
    b.app.records.add(Writer::Secretary, TASKS, inside).unwrap();
    b.is_in_use(&w).await;
}

#[tokio::test]
async fn a_task_put_back_in_the_queue_meanwhile_keeps_what_it_left() {
    let b = bench("tidy-retried");
    let w = b.work();
    let (app, task) = (b.app.clone(), w.task.clone());
    *b.fake.then.lock().unwrap() = Some((
        "~ empty()",
        Box::new(move || {
            let patch = Patch::default().with("Status", "pending");
            app.records
                .change(Writer::Owner, TASKS, &task, patch)
                .unwrap();
        }),
    ));
    b.is_in_use(&w).await;

    // Once its worktree is gone, another run begins and makes a VM.
    let b = bench("tidy-vm-retried");
    let w = b.work();
    b.fake.vms.lock().unwrap().push(w.code.clone());
    let (app, task, code) = (b.app.clone(), w.task.clone(), w.code.clone());
    *b.fake.then.lock().unwrap() = Some((
        "vitro ls",
        Box::new(move || {
            let order = Order::of(&app.records.need(TASKS, &task).unwrap(), &code).unwrap();
            let run = app.workers.runs.begin(&task, &order).unwrap();
            app.workers.runs.running(run.id, "new").unwrap();
        }),
    ));
    let left = b.tidy(How::Remove).await;
    assert_eq!(left[0].fate, Fate::Removed);
    assert!(matches!(left[1].fate, Fate::InUse(_)), "{left:?}");
    assert_eq!(b.fake.vms.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_worktree_not_named_after_a_run_is_not_looked_at() {
    let b = bench("tidy-others");
    let w = b.work();
    let other = b.worktree("faster-ci");
    let longer = b.worktree(&format!("{}-run10000", w.code));
    let near = b.worktree(&format!("x-{}-run1", w.code));
    let left = b.tidy(How::Remove).await;
    assert_eq!(left.len(), 1, "{left:?}");
    assert!(other.exists() && longer.exists() && near.exists());
}

#[tokio::test]
async fn a_symbolic_link_is_not_followed() {
    let b = bench("tidy-link");
    let w = b.work();
    let elsewhere = b.dir.path().join("elsewhere");
    std::fs::rename(&w.path, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &w.path).unwrap();
    b.is_kept(&w, "no directory").await;
    assert!(elsewhere.join("main.rs").exists());

    let b = bench("tidy-link-above");
    let w = b.work();
    let moved = b.dir.path().join("worktrees");
    std::fs::rename(w.path.parent().unwrap(), &moved).unwrap();
    std::os::unix::fs::symlink(&moved, w.path.parent().unwrap()).unwrap();
    assert_eq!(b.tidy(How::Remove).await, []);
    assert!(w.path.join("main.rs").exists());
}

#[tokio::test]
async fn a_workspace_that_is_not_this_checkouts_is_kept() {
    let b = bench("tidy-foreign");
    let w = b.work();
    let foreign = b.dir.path().join("foreign/.jj/repo");
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(w.path.join(".jj/repo"), foreign.to_str().unwrap()).unwrap();
    b.is_kept(&w, "another repository").await;

    // A checkout of its own is no workspace.
    let b = bench("tidy-own-checkout");
    let w = b.work();
    std::fs::remove_file(w.path.join(".jj/repo")).unwrap();
    std::fs::create_dir(w.path.join(".jj/repo")).unwrap();
    b.is_kept(&w, "no workspace").await;

    // jj keeps another one there, or that of the checkout itself.
    for names in [vec!["default"], vec!["a", "b"], vec![]] {
        let b = bench("tidy-wrong-name");
        let w = b.work();
        *b.fake.workspaces.lock().unwrap() = names
            .iter()
            .map(|n| ((*n).to_owned(), w.path.clone()))
            .collect();
        b.is_kept(&w, "jj の作業場所").await;
    }
}

#[test]
fn a_checkout_is_never_its_own_worktree() {
    let b = bench("tidy-real-place");
    assert!(real_place(&b.checkout, &b.checkout).is_err());
    assert!(real_place(&b.checkout, &place(&b.checkout, "..")).is_err());
    assert!(real_place(&b.checkout, &place(&b.checkout, "")).is_err());
    let w = b.work();
    assert!(real_place(&b.checkout, &w.path).is_ok());
    assert_eq!(checkout_of(&w.path), None, "a workspace is no checkout");
    assert_eq!(
        checkout_of(&b.checkout.join(".claude")),
        Some(b.checkout.clone())
    );
}

#[tokio::test]
async fn a_removal_that_fails_is_told_as_kept() {
    let b = bench("tidy-remove-fails");
    let w = b.work();
    b.says("workspace remove", fails("busy"));
    let left = b.tidy(How::Remove).await;
    assert!(left[0].fate.why().contains("busy"), "{left:?}");
    assert!(w.path.exists());
}

#[tokio::test]
async fn only_vms_named_after_the_task_are_destroyed() {
    let b = bench("tidy-vms");
    let w = b.work();
    let mine = [w.code.clone(), format!("{}-e2e", w.code)];
    let others = [
        format!("{}x", w.code),
        format!("my-{}", w.code),
        "things-e2e".to_owned(),
        "task".to_owned(),
    ];
    *b.fake.vms.lock().unwrap() = [&mine[..], &others[..]].concat();
    let left = b.tidy(How::Remove).await;
    assert!(left.iter().all(|l| l.fate == Fate::Removed), "{left:?}");
    assert_eq!(*b.fake.vms.lock().unwrap(), others);
}

#[tokio::test]
async fn a_vm_stays_while_anything_of_its_task_does() {
    let b = bench("tidy-vm-kept");
    let w = b.work();
    b.fake.vms.lock().unwrap().push(w.code.clone());
    b.says("~ empty()", says("vutnkkuz\n"));
    let left = b.tidy(How::Remove).await;
    assert!(
        left.iter().all(|l| matches!(l.fate, Fate::Kept(_))),
        "{left:?}"
    );

    let b = bench("tidy-vm-open");
    let w = b.work();
    // Another run of the task, whose session is open and which has no worktree left.
    b.status(&w.task, "done");
    let again = b.run(w.task.clone(), w.code.clone());
    std::fs::remove_dir_all(&again.path).unwrap();
    b.session(&again.session, Path::new("/elsewhere"), false);
    b.fake.vms.lock().unwrap().push(w.code.clone());
    let left = b.tidy(How::Remove).await;
    let vm = left.iter().find(|l| l.kind == Kind::Vm).unwrap();
    assert!(matches!(vm.fate, Fate::InUse(_)), "{left:?}");
    assert_eq!(b.fake.vms.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_owner_is_told_of_what_stays_once_a_day_and_not_of_the_same_again() {
    let b = bench("tidy-told");
    let w = b.work();
    b.says("~ empty()", says("vutnkkuz\n"));
    let left = b.tidy(How::Remove).await;
    let at = |day, hour| {
        chrono_tz::Asia::Tokyo
            .with_ymd_and_hms(2026, 11, day, hour, 0, 0)
            .unwrap()
    };
    let seen: BTreeSet<_> = left.iter().filter_map(matter).collect();
    tell(&b.app, &left, &seen, at(26, 3)).unwrap();
    assert_eq!(b.told(), [], "not at night");
    tell(&b.app, &left, &BTreeSet::new(), at(26, 9)).unwrap();
    assert_eq!(b.told(), [], "it may pass by itself");
    tell(&b.app, &left, &seen, at(26, 9)).unwrap();
    let told = b.told();
    assert_eq!(told.len(), 1);
    for part in [w.path.to_str().unwrap(), &w.code, "vutnkkuz"] {
        assert!(told[0].text.contains(part), "{}", told[0].text);
    }
    tell(&b.app, &left, &seen, at(27, 9)).unwrap();
    assert_eq!(b.told().len(), 1, "the same is not told again");

    let mut other = left.clone();
    let with = |other: &mut Vec<Left>, why: &str| {
        other[0].fate = Fate::Kept(why.into());
        other.iter().filter_map(matter).collect::<BTreeSet<_>>()
    };
    let seen = with(&mut other, "プロセスがここを使っている（8）");
    tell(&b.app, &other, &seen, at(26, 20)).unwrap();
    assert_eq!(b.told().len(), 1, "once a day");
    tell(&b.app, &other, &seen, at(27, 9)).unwrap();
    assert_eq!(b.told().len(), 2);
    let seen = with(&mut other, "プロセスがここを使っている（9）");
    tell(&b.app, &other, &seen, at(28, 9)).unwrap();
    assert_eq!(b.told().len(), 2, "another process is no news");
    let seen = left.iter().filter_map(matter).collect();
    tell(&b.app, &left, &seen, at(29, 9)).unwrap();
    assert_eq!(b.told().len(), 2, "nor is a reason told before");
}

#[tokio::test]
async fn what_is_in_use_or_was_removed_is_not_told() {
    let b = bench("tidy-untold");
    let w = b.work();
    b.session(&w.session, &w.path, false);
    let left = b.tidy(How::Remove).await;
    assert_eq!(matter(&left[0]), None);
    let other = Left {
        fate: Fate::Removed,
        ..left[0].clone()
    };
    assert_eq!(matter(&other), None);
}
