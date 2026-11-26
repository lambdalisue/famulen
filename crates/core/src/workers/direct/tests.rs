use super::*;
use crate::notebook::testing::TempDir;

/// A checkout jj would know: its repository is kept in `.jj/repo`.
fn checkout(dir: &TempDir) -> PathBuf {
    let checkout = dir.path().join("src");
    std::fs::create_dir_all(checkout.join(".jj/repo")).unwrap();
    std::fs::create_dir_all(checkout.join("api")).unwrap();
    checkout.canonicalize().unwrap()
}

fn order(directory: &Path) -> Order {
    Order {
        code: "task-ab12cd".into(),
        runner: super::super::Runner::Direct,
        request: "Do it.".into(),
        directory: directory.display().to_string(),
        ..Order::default()
    }
}

#[test]
fn work_runs_directly_in_the_repository_the_owner_named_and_nowhere_else() {
    let dir = TempDir::new("direct-where");
    let checkout = checkout(&dir);
    let other = dir.path().join("other");
    std::fs::create_dir_all(other.join(".jj/repo")).unwrap();
    let direct = fenced(checkout.clone());

    let (at, inside) = super::checkout(Some(&direct), &order(&checkout.join("api"))).unwrap();
    assert_eq!((at, inside), (checkout.clone(), PathBuf::from("api")));

    let refused = |direct: Option<&Direct>, directory: &Path| {
        super::checkout(direct, &order(directory))
            .unwrap_err()
            .to_string()
    };
    assert!(refused(None, &checkout).contains("no repository is set"));
    assert!(refused(Some(&direct), &other).contains("only in"));
    let worktree = checkout.join(".claude/worktrees/task-x-run1");
    std::fs::create_dir_all(&worktree).unwrap();
    assert!(refused(Some(&direct), &worktree).contains("not in another workspace"));
    let plain = fenced(other.join("nothing"));
    assert!(refused(Some(&plain), &checkout).contains("no repository"));
    std::fs::remove_dir_all(other.join(".jj")).unwrap();
    assert!(refused(Some(&fenced(other.clone())), &other).contains("no checkout of jj"));
}

#[test]
fn a_run_works_in_a_workspace_of_its_own_named_as_delegated_worktrees_are() {
    let dir = TempDir::new("direct-place");
    let checkout = checkout(&dir);
    let runs = super::super::Runs::new(crate::store::Store::open_in_memory().unwrap());
    let run = runs.begin("t", &order(&checkout.join("api"))).unwrap();
    let store = crate::store::Store::open_in_memory().unwrap();
    let place = super::place(Some(&fenced(checkout.clone())), &run, &store, None).unwrap();
    let name = format!("task-ab12cd-run{}", run.id);
    assert_eq!(place.name, name);
    assert_eq!(
        place.workspace,
        checkout.join(".claude/worktrees").join(&name)
    );
    assert_eq!(place.directory, place.workspace.join("api"));
}

#[cfg(unix)]
#[tokio::test]
async fn the_workspace_is_made_by_jj_once_from_the_base_and_given_the_checkouts_build() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = TempDir::new("direct-prepare");
    let checkout = checkout(&dir);
    std::fs::create_dir_all(checkout.join("target/debug")).unwrap();
    std::fs::write(checkout.join("target/debug/built"), "x").unwrap();
    let jj = dir.path().join("jj");
    let told = dir.path().join("told");
    // Writes what it was asked, and makes the workspace as jj does.
    std::fs::write(
        &jj,
        format!(
            "#!/bin/sh\necho \"$@\" >> {}\ncase \"$*\" in *\"workspace add\"*) ;; *) exit 0 ;; esac\nfor last; do :; done\n[ -d \"$(dirname \"$last\")\" ] || exit 1\nmkdir \"$last\" \"$last/.jj\"\ntouch \"$last/README.md\"\n",
            told.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&jj, std::fs::Permissions::from_mode(0o755)).unwrap();
    let direct = Direct {
        repository: checkout.clone(),
        jj: jj.display().to_string(),
        fenced: true,
    };
    let workspace = checkout.join(".claude/worktrees/task-ab12cd-run1");
    let place = Place {
        checkout: checkout.clone(),
        workspace: workspace.clone(),
        name: "task-ab12cd-run1".into(),
        directory: workspace.clone(),
    };

    prepare(&direct, &place, "").await.unwrap();
    prepare(&direct, &place, "").await.unwrap();

    let told = std::fs::read_to_string(&told).unwrap();
    let added = told.lines().filter(|l| l.contains("workspace add")).count();
    assert_eq!(added, 1, "made once: {told}");
    assert!(
        told.contains("workspace add --name task-ab12cd-run1 -r trunk()"),
        "{told}"
    );
    assert!(
        told.contains(&format!(
            "snapshot.auto-track=\"none()\" -R {}",
            checkout.display()
        )),
        "{told}"
    );
    // Where blocks cannot be shared, as on most Linux file systems, nothing
    // is copied; APFS shares them.
    let copied = std::fs::read_to_string(workspace.join("target/debug/built")).ok();
    if cfg!(target_os = "macos") {
        assert_eq!(copied.as_deref(), Some("x"));
    } else {
        assert!(copied.is_none() || copied.as_deref() == Some("x"));
    }

    let failing = Direct {
        jj: "/nowhere/jj".into(),
        ..direct
    };
    let other = Place {
        workspace: checkout.join(".claude/worktrees/task-ab12cd-run2"),
        ..place
    };
    assert!(prepare(&failing, &other, "main").await.is_err());
}

#[test]
fn its_shell_writes_what_jj_and_git_keep_of_the_repository_but_nothing_the_owner_would_run() {
    let dir = TempDir::new("direct-fence");
    let src = checkout(&dir);
    let place = Place {
        checkout: src.clone(),
        workspace: src.join(".claude/worktrees/w"),
        name: "w".into(),
        directory: src.join(".claude/worktrees/w"),
    };
    let kept = Kept {
        places: vec!["/data/notebook".into()],
        hosts: vec!["127.0.0.1".into()],
    };
    let fence = |place: &Place| fence(&kept, Path::new("/data/workers"), place);
    let apart = fence(&place);
    assert_eq!(apart.sealed, [PathBuf::from("/data/notebook")]);
    assert_eq!(apart.hosts, ["127.0.0.1"]);
    assert_eq!(apart.unread, [PathBuf::from("/data/workers")]);
    assert_eq!(
        apart.writable,
        [place.workspace.clone(), src.join(".jj")],
        "no Git directory is there to write"
    );
    std::fs::create_dir_all(src.join(".git")).unwrap();
    let fence = fence(&place);
    assert_eq!(
        fence.writable,
        [place.workspace.clone(), src.join(".jj"), src.join(".git")]
    );
    let w = &place.workspace;
    for never in [
        src.join(".jj/repo/config.toml"),
        src.join(".jj/repo/store/git_target"),
        src.join(".jj/repo/store/git/config"),
        src.join(".git/config"),
        src.join(".git/hooks"),
        src.join(".git/info"),
        src.join(".git/modules"),
        src.join(".git/commondir"),
        src.join(".git/worktrees/w/gitdir"),
        w.join(".jj/repo"),
        w.join(".git"),
        w.join(".claude"),
        w.join(".mcp.json"),
    ] {
        assert!(fence.unwritable.contains(&never), "{}", never.display());
    }
}

#[test]
fn what_went_wrong_is_told_apart_and_only_what_passes_or_comes_back_is_waited_for() {
    for (why, fault) in [
        ("API Error: 529 overloaded", Fault::Passing),
        ("Claude AI usage limit reached|1764000000", Fault::Limit),
        ("You've hit your limit · resets 3pm", Fault::Limit),
        ("Invalid API key · Please run /login", Fault::Login),
        ("OAuth token has expired", Fault::Login),
        ("it could not decide", Fault::Other),
        (
            "failed to log into the changelog of an oauth library",
            Fault::Other,
        ),
    ] {
        assert_eq!(super::fault(why), fault, "{why}");
    }
    assert!(waits(Fault::Passing).is_some());
    assert!(waits(Fault::Limit).is_some_and(|(wait, _)| wait >= Duration::from_secs(60)));
    assert_eq!(waits(Fault::Login), None);
    assert_eq!(waits(Fault::Other), None);
}

#[test]
fn a_conversation_is_named_anew_each_time_in_the_form_the_agent_takes() {
    let (a, b) = (new_conversation(), new_conversation());
    assert_ne!(a, b);
    let parts: Vec<usize> = a.split('-').map(str::len).collect();
    assert_eq!(parts, [8, 4, 4, 4, 12]);
    assert_eq!(&a[14..15], "4");
}

#[test]
fn a_run_that_begins_again_goes_on_in_the_workspace_the_run_cut_off_before_it_left() {
    let dir = TempDir::new("direct-again");
    let checkout = checkout(&dir);
    let below = checkout.join(".claude/worktrees");
    for made in ["task-ab12cd-run1", "task-ab12cd-run3", "task-ab12cd-run30"] {
        std::fs::create_dir_all(below.join(made).join(".jj")).unwrap();
    }
    let store = crate::store::Store::open_in_memory().unwrap();
    // Runs 1, 3, 4 and 30 ran directly; 4 left no workspace.
    for id in [1, 3, 4, 30] {
        let name = format!("task-ab12cd-run{id}");
        let place = Place {
            checkout: checkout.clone(),
            workspace: below.join(&name),
            name,
            directory: below.join("x"),
        };
        store
            .set_state(place_key(id), serde_json::to_string(&place).unwrap())
            .unwrap();
    }
    let runs = super::super::Runs::new(crate::store::Store::open_in_memory().unwrap());
    let first = runs.begin("t", &order(&checkout)).unwrap();
    let again = |id, attempt| Run {
        id,
        attempt,
        ..first.clone()
    };
    let direct = fenced(checkout.clone());
    let name = |run: Run, cut| super::place(Some(&direct), &run, &store, cut).unwrap().name;
    assert_eq!(
        name(again(5, 2), Some(3)),
        "task-ab12cd-run3",
        "the run cut off"
    );
    assert_eq!(
        name(again(5, 1), Some(3)),
        "task-ab12cd-run5",
        "a first run has its own"
    );
    assert_eq!(
        name(again(5, 2), None),
        "task-ab12cd-run5",
        "none was cut off"
    );
    assert_eq!(
        name(again(5, 2), Some(4)),
        "task-ab12cd-run5",
        "the run cut off left nothing, and no older one is taken"
    );
    assert_eq!(
        name(again(5, 2), Some(2)),
        "task-ab12cd-run5",
        "a run that did not run directly"
    );
    assert_eq!(
        name(again(30, 1), None),
        "task-ab12cd-run30",
        "a run that began works where it began"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_workspace_made_from_a_revision_that_holds_nothing_is_refused_and_names_the_base() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = TempDir::new("direct-empty");
    let checkout = checkout(&dir);
    let jj = dir.path().join("jj");
    std::fs::write(
        &jj,
        "#!/bin/sh\ncase \"$*\" in *\"workspace add\"*) ;; *) exit 0 ;; esac\nfor last; do :; done\nmkdir -p \"$last/.jj\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&jj, std::fs::Permissions::from_mode(0o755)).unwrap();
    let direct = Direct {
        repository: checkout.clone(),
        jj: jj.display().to_string(),
        fenced: true,
    };
    let workspace = checkout.join(".claude/worktrees/task-ab12cd-run1");
    let place = Place {
        checkout,
        workspace: workspace.clone(),
        name: "task-ab12cd-run1".into(),
        directory: workspace,
    };
    let why = prepare(&direct, &place, "").await.unwrap_err().to_string();
    assert!(
        why.contains("holds nothing") && why.contains("trunk()") && why.contains("Base"),
        "{why}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_workspace_jj_failed_to_make_is_taken_away_so_that_the_next_start_makes_it_anew() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = TempDir::new("direct-failed");
    let checkout = checkout(&dir);
    let jj = dir.path().join("jj");
    let told = dir.path().join("told");
    // Makes some of the workspace, then fails on the revision, as jj may.
    std::fs::write(
        &jj,
        format!(
            "#!/bin/sh\necho \"$@\" >> {}\ncase \"$*\" in *\"workspace add\"*) ;; *) exit 0 ;; esac\nfor last; do :; done\nmkdir -p \"$last/.jj\"\necho 'no such revision' >&2\nexit 1\n",
            told.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&jj, std::fs::Permissions::from_mode(0o755)).unwrap();
    let direct = Direct {
        repository: checkout.clone(),
        jj: jj.display().to_string(),
        fenced: true,
    };
    let workspace = checkout.join(".claude/worktrees/task-ab12cd-run1");
    let place = Place {
        checkout,
        workspace: workspace.clone(),
        name: "task-ab12cd-run1".into(),
        directory: workspace.clone(),
    };
    let why = format!(
        "{:#}",
        prepare(&direct, &place, "nowhere").await.unwrap_err()
    );
    assert!(why.contains("no such revision"), "{why}");
    assert!(!workspace.exists());
    let told = std::fs::read_to_string(&told).unwrap();
    assert!(told.contains("workspace forget task-ab12cd-run1"), "{told}");
}

#[cfg(unix)]
#[test]
fn a_directory_yet_to_come_is_told_inside_a_repository_named_through_a_link() {
    let dir = TempDir::new("direct-linked");
    let real = checkout(&dir);
    let link = dir.path().join("linked");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let direct = fenced(link.clone());
    let (at, inside) = super::checkout(Some(&direct), &order(&link.join("crates/new"))).unwrap();
    assert_eq!((at, inside), (real, PathBuf::from("crates/new")));
    let outside = dir.path().join("elsewhere/new");
    assert!(super::checkout(Some(&direct), &order(&outside)).is_err());
    assert!(super::checkout(Some(&direct), &order(&link.join("../x"))).is_err());
}

#[test]
fn work_runs_directly_nowhere_the_sandbox_is_not_known_to_keep_it_from_what_it_must_not_write() {
    let dir = TempDir::new("direct-unfenced");
    let checkout = checkout(&dir);
    let unfenced = Direct {
        fenced: false,
        ..fenced(checkout.clone())
    };
    let why = super::checkout(Some(&unfenced), &order(&checkout))
        .unwrap_err()
        .to_string();
    assert!(why.contains("only on macOS"), "{why}");
    assert_eq!(Direct::new(&checkout).fenced, cfg!(target_os = "macos"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_workspace_jj_knows_by_the_name_already_is_neither_made_over_nor_forgotten() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = TempDir::new("direct-known");
    let checkout = checkout(&dir);
    let jj = dir.path().join("jj");
    let told = dir.path().join("told");
    std::fs::write(
        &jj,
        format!(
            "#!/bin/sh\necho \"$@\" >> {}\ncase \"$*\" in *\"workspace list\"*) echo 'default: abc'; echo 'task-ab12cd-run1: def' ;; esac\n",
            told.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&jj, std::fs::Permissions::from_mode(0o755)).unwrap();
    let direct = Direct {
        repository: checkout.clone(),
        jj: jj.display().to_string(),
        fenced: true,
    };
    let workspace = checkout.join(".claude/worktrees/task-ab12cd-run1");
    let place = Place {
        checkout,
        workspace: workspace.clone(),
        name: "task-ab12cd-run1".into(),
        directory: workspace,
    };
    let why = format!("{:#}", prepare(&direct, &place, "").await.unwrap_err());
    assert!(why.contains("already"), "{why}");
    let told = std::fs::read_to_string(&told).unwrap();
    assert!(
        !told.contains("workspace add") && !told.contains("forget"),
        "{told}"
    );
}
