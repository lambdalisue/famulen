//! Executors as Famulen starts them: processes of their own, here of this
//! very test program.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::time::Duration;

use famulen_executor::{Executor, Executors, Next, Program, Records, Standing, Start, What, Why};
use serde_json::json;

/// Not a test: what [`program`] runs.
#[test]
fn the_executor() {
    if std::env::args().any(|a| a == "be-an-executor") {
        famulen_executor::execute().ok();
        std::process::exit(0);
    }
}

fn program() -> Program {
    Program {
        path: std::env::current_exe().unwrap(),
        args: ["the_executor", "--exact", "--nocapture", "be-an-executor"]
            .map(String::from)
            .into(),
    }
}

struct Dir(PathBuf);

impl Dir {
    /// Short, as the path of a socket must be.
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("fx-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn executors(&self, key: &str) -> Executors {
        Executors::new(self.0.join("x"), key, program())
    }

    async fn run(&self, script: &str) -> Executor {
        self.executors("k")
            .start("1", self.script(script))
            .await
            .unwrap()
    }

    fn script(&self, script: &str) -> Start {
        Start {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: self.0.clone(),
            limit: Duration::from_secs(60),
            limit_bytes: 1 << 20,
            meta: json!({ "agent": "sh" }),
            ..Start::default()
        }
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

async fn next(records: &mut Records) -> Next {
    tokio::time::timeout(Duration::from_secs(20), records.next())
        .await
        .expect("a record in time")
        .unwrap()
}

/// Everything up to the end: the numbers, and the lines written.
async fn rest(records: &mut Records) -> (Vec<u64>, Vec<String>, Next) {
    let (mut seqs, mut lines) = (Vec::new(), Vec::new());
    loop {
        match next(records).await {
            Next::Record(record) => {
                seqs.push(record.seq);
                if let What::Out { line } = record.what {
                    lines.push(line);
                }
            }
            end => return (seqs, lines, end),
        }
    }
}

async fn until(records: &mut Records, line: &str) -> u64 {
    loop {
        match next(records).await {
            Next::Record(record) if record.what == (What::Out { line: line.into() }) => {
                return record.seq;
            }
            Next::Record(_) => {}
            end => panic!("{line} never came: {end:?}"),
        }
    }
}

async fn exit(executor: &Executor) -> Why {
    let mut records = executor.attach(0);
    let mut why = None;
    while let Next::Record(record) = next(&mut records).await {
        if let What::Exit { why: told, .. } = record.what {
            why = Some(told);
        }
    }
    why.expect("an end")
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .output()
        .unwrap()
        .status
        .success()
}

async fn gone(pid: &str) {
    for _ in 0..200 {
        if !alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{pid} is still there");
}

#[tokio::test]
async fn what_is_written_is_passed_on_numbered_and_the_rest_to_whoever_comes_back() {
    let dir = Dir::new("again");
    let executor = dir
        .run("echo one; read a; echo got $a; read b; echo got $b")
        .await;
    let mut records = executor.attach(0);
    let seen = until(&mut records, "one").await;
    drop(records);

    // Nobody is there while it goes on.
    assert!(executor.write("a", "x").await.unwrap());
    drop(executor);

    let again = dir.executors("k").of("1");
    assert_eq!(again.standing().await.unwrap(), Standing::Working);
    let mut records = again.attach(seen);
    until(&mut records, "got x").await;
    assert!(again.write("b", "y").await.unwrap());
    let (_, lines, end) = rest(&mut records).await;
    assert_eq!((lines, end), (vec!["got y".to_owned()], Next::Over));

    // All of it is there once it is over, whoever is asked.
    let (seqs, lines, end) = rest(&mut again.attach(0)).await;
    assert_eq!(lines, ["one", "got x", "got y"]);
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());
    assert_eq!(end, Next::Over);
    assert_eq!(again.standing().await.unwrap(), Standing::Over);
}

#[tokio::test]
async fn what_it_was_started_with_is_written_first_and_read_back() {
    let dir = Dir::new("input");
    let mut start = dir.script("read a; read b; echo $a $b $X; echo oh >&2");
    start.input = vec!["1".into(), "2".into()];
    start.env = [("X".to_owned(), "3".to_owned())].into();
    let executor = dir.executors("k").start("1", start).await.unwrap();
    let mut records = executor.attach(0);
    let Next::Record(first) = next(&mut records).await else {
        panic!("nothing");
    };
    assert!(
        matches!(&first.what, What::Start { meta, born: Some(_), .. } if meta["agent"] == "sh"),
        "{first:?}"
    );
    let (mut out, mut err) = (None, None);
    while let Next::Record(record) = next(&mut records).await {
        match record.what {
            What::Out { line } => out = Some(line),
            What::Err { line } => err = Some(line),
            _ => {}
        }
    }
    assert_eq!(out.as_deref(), Some("1 2 3"));
    assert_eq!(err.as_deref(), Some("oh"));
}

#[tokio::test]
async fn a_line_or_mark_by_the_same_key_is_taken_once() {
    let dir = Dir::new("once");
    let executor = dir.run("while read a; do echo got $a; done").await;
    assert!(executor.write("a", "x").await.unwrap());
    assert!(!executor.write("a", "x").await.unwrap());
    assert!(executor.mark("a").await.unwrap(), "marks are apart");
    assert!(!dir.executors("k").of("1").mark("a").await.unwrap());
    assert!(executor.write("b", "y").await.unwrap());
    let mut records = executor.attach(0);
    until(&mut records, "got x").await;
    until(&mut records, "got y").await;
    executor.forget().await.unwrap();
}

#[tokio::test]
async fn whoever_has_another_key_is_told_and_does_nothing() {
    let dir = Dir::new("key");
    let executor = dir.run("read a; echo got $a").await;
    let other = dir.executors("other").of("1");
    for refused in [
        other.standing().await.err(),
        other.write("a", "x").await.err(),
        other.stop(Duration::ZERO).await.err(),
    ] {
        assert!(refused.unwrap().to_string().contains("not known"));
    }
    // Nor one that does not mind who answers.
    use std::io::{BufRead as _, Write as _};
    let mut stream =
        std::os::unix::net::UnixStream::connect(executor.dir().join("socket")).unwrap();
    for line in [
        r#"{"op":"hello","from":1,"to":1,"nonce":"a"}"#,
        r#"{"op":"prove","proof":"00"}"#,
        r#"{"op":"stop","grace_ms":0}"#,
    ] {
        writeln!(stream, "{line}").unwrap();
    }
    let told: Vec<_> = std::io::BufReader::new(stream)
        .lines()
        .map_while(Result::ok)
        .collect();
    assert!(told[1].contains("not known"), "{told:?}");
    assert!(told.iter().all(|line| !line.contains("\"k\"")));
    assert_eq!(executor.standing().await.unwrap(), Standing::Working);
    executor.forget().await.unwrap();
}

#[tokio::test]
async fn only_the_owner_can_reach_what_it_keeps_and_listens_on() {
    let dir = Dir::new("mode");
    let executor = dir.run("read a").await;
    let mode = |path: PathBuf| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(dir.0.join("x")), 0o700);
    assert_eq!(mode(executor.dir().to_owned()), 0o700);
    assert_eq!(mode(executor.dir().join("socket")), 0o600);
    assert_eq!(mode(executor.dir().join("journal")), 0o600);
    executor.forget().await.unwrap();
}

#[tokio::test]
async fn stopping_ends_everything_it_started_and_forgetting_leaves_nothing() {
    let dir = Dir::new("stop");
    let executor = dir.run("sleep 60 & echo $!; wait").await;
    let mut records = executor.attach(0);
    let pid = loop {
        if let Next::Record(record) = next(&mut records).await
            && let What::Out { line } = record.what
        {
            break line;
        }
    };
    assert!(alive(&pid));
    executor.stop(Duration::ZERO).await.unwrap();
    gone(&pid).await;
    assert_eq!(exit(&executor).await, Why::Stopped);
    assert!(executor.write("a", "x").await.is_err());
    executor.forget().await.unwrap();
    assert_eq!(executor.standing().await.unwrap(), Standing::None);
    assert!(dir.executors("k").names().unwrap().is_empty());
    executor.forget().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn stopping_ends_what_left_the_group_too() {
    let dir = Dir::new("left-group");
    let executor = dir
        .run("perl -MPOSIX -e '$| = 1; POSIX::setsid(); print \"$$\\n\"; sleep 60' & wait")
        .await;
    let mut records = executor.attach(0);
    let pid = loop {
        if let Next::Record(record) = next(&mut records).await
            && let What::Out { line } = record.what
        {
            break line;
        }
    };
    assert!(alive(&pid));
    executor.stop(Duration::ZERO).await.unwrap();
    gone(&pid).await;
}

#[tokio::test]
async fn what_outlives_the_program_ends_with_it() {
    let dir = Dir::new("left");
    let executor = dir.run("sleep 60 & echo $!").await;
    let (_, lines, _) = rest(&mut executor.attach(0)).await;
    gone(&lines[0]).await;
    assert_eq!(exit(&executor).await, Why::Ended);
}

#[tokio::test]
async fn it_may_end_as_it_likes_within_the_grace() {
    let dir = Dir::new("grace");
    let executor = dir
        .run("trap 'echo bye; exit 3' TERM; echo up; while :; do sleep 0.05; done")
        .await;
    let mut records = executor.attach(0);
    until(&mut records, "up").await;
    executor.stop(Duration::from_secs(10)).await.unwrap();
    let (_, lines, end) = rest(&mut records).await;
    assert_eq!((lines, end), (vec!["bye".to_owned()], Next::Over));
}

#[tokio::test]
async fn it_ends_when_it_took_too_long_or_wrote_too_much() {
    let dir = Dir::new("limits");
    let executors = dir.executors("k");
    let mut long = dir.script("sleep 60");
    long.limit = Duration::from_secs(1);
    let long = executors.start("long", long).await.unwrap();
    assert_eq!(exit(&long).await, Why::TookTooLong);
    let much = executors
        .start("much", dir.script("while :; do echo 0123456789; done"))
        .await
        .unwrap();
    assert_eq!(exit(&much).await, Why::WroteTooMuch);
}

#[tokio::test]
async fn a_program_that_cannot_be_started_says_so_and_leaves_nothing() {
    let dir = Dir::new("none");
    let mut start = dir.script("");
    start.program = "/nowhere/agent".into();
    let executors = dir.executors("k");
    let error = executors.start("1", start).await.unwrap_err();
    assert!(error.to_string().contains("/nowhere/agent"), "{error:#}");
    assert_eq!(executors.of("1").standing().await.unwrap(), Standing::None);
}

#[tokio::test]
async fn the_same_work_is_not_started_twice() {
    let dir = Dir::new("twice");
    let executor = dir.run("read a").await;
    let again = dir.executors("k").start("1", dir.script("echo no")).await;
    assert!(again.is_err());
    assert_eq!(executor.standing().await.unwrap(), Standing::Working);
    executor.forget().await.unwrap();
}

#[tokio::test]
async fn one_whose_executor_went_away_is_told_gone_and_its_program_ends() {
    let dir = Dir::new("gone");
    let executor = dir
        .run("echo $PPID; trap '' HUP PIPE; while :; do sleep 0.05; done")
        .await;
    let mut records = executor.attach(0);
    let mut pid = None;
    let parent = loop {
        if let Next::Record(record) = next(&mut records).await {
            match record.what {
                What::Start { pid: p, .. } => pid = Some(p.to_string()),
                What::Out { line } => break line,
                _ => {}
            }
        }
    };
    std::process::Command::new("kill")
        .args(["-9", &parent])
        .status()
        .unwrap();
    assert_eq!(rest(&mut records).await.2, Next::Gone);
    assert_eq!(executor.standing().await.unwrap(), Standing::Gone);
    let pid = pid.unwrap();
    assert!(alive(&pid));
    executor.forget().await.unwrap();
    gone(&pid).await;
}

#[tokio::test]
async fn an_executor_told_to_end_by_the_system_takes_its_program_along() {
    let dir = Dir::new("term");
    let executor = dir.run("echo $PPID; sleep 60").await;
    let mut records = executor.attach(0);
    let parent = loop {
        if let Next::Record(record) = next(&mut records).await
            && let What::Out { line } = record.what
        {
            break line;
        }
    };
    std::process::Command::new("kill")
        .arg(&parent)
        .status()
        .unwrap();
    assert_eq!(exit(&executor).await, Why::Signalled);
}

#[tokio::test]
async fn lines_come_whole_to_whoever_gives_up_waiting_and_asks_again() {
    let dir = Dir::new("cancel");
    let executor = dir
        .run("for i in 1 2 3 4 5 6 7 8; do printf 'a%05000d' $i; sleep 0.02; echo; done")
        .await;
    let mut records = executor.attach(0);
    let mut seqs = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_millis(1), records.next()).await {
            Err(_) => {}
            Ok(next) => match next.unwrap() {
                Next::Record(record) => seqs.push(record.seq),
                end => {
                    assert_eq!(end, Next::Over);
                    break;
                }
            },
        }
    }
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());
    assert!(seqs.len() >= 10);
}

#[tokio::test]
async fn what_happened_before_is_told_apart_from_what_comes() {
    let dir = Dir::new("caught");
    let executor = dir.run("echo one; read a; echo two").await;
    until(&mut executor.attach(0), "one").await;
    let mut records = executor.attach(0);
    until(&mut records, "one").await;
    assert!(records.caught_up());
    executor.write("a", "x").await.unwrap();
    until(&mut records, "two").await;
    while next(&mut records).await != Next::Over {}

    let mut again = executor.attach(0);
    until(&mut again, "one").await;
    assert!(!again.caught_up());
    while next(&mut again).await != Next::Over {}
    assert!(again.caught_up());
}

#[tokio::test]
async fn one_whose_directory_is_taken_away_ends() {
    let dir = Dir::new("away");
    let executor = dir.run("echo $$; sleep 60").await;
    let mut records = executor.attach(0);
    let pid = loop {
        if let Next::Record(record) = next(&mut records).await
            && let What::Out { line } = record.what
        {
            break line;
        }
    };
    std::fs::remove_dir_all(executor.dir()).unwrap();
    gone(&pid).await;
}

#[tokio::test]
async fn it_is_reached_however_long_its_path_is() {
    let dir = Dir::new(&"long".repeat(30));
    let executor = dir.run("read a").await;
    assert_eq!(executor.standing().await.unwrap(), Standing::Working);
    executor.forget().await.unwrap();
}

#[tokio::test]
async fn what_the_program_started_ends_though_both_it_and_its_executor_went_away() {
    let dir = Dir::new("both");
    let executor = dir.run("echo $PPID; sleep 60 & echo $!; read a").await;
    let mut records = executor.attach(0);
    let mut lines = Vec::new();
    let mut pid = None;
    while lines.len() < 2 {
        if let Next::Record(record) = next(&mut records).await {
            match record.what {
                What::Start { pid: p, .. } => pid = Some(p.to_string()),
                What::Out { line } => lines.push(line),
                _ => {}
            }
        }
    }
    let killed = std::process::Command::new("kill")
        .args(["-9", &lines[0]])
        .status()
        .unwrap();
    assert!(killed.success());
    // Nobody writes to it any more, so it ends.
    gone(&pid.unwrap()).await;
    assert!(alive(&lines[1]));
    executor.forget().await.unwrap();
    gone(&lines[1]).await;
}
