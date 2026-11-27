use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

pub(super) fn rule() -> Rule {
    Rule {
        repository: "/repo".into(),
        checks: vec!["just ci".into()],
        reviews: 2,
        merge: true,
        deploy: None,
        forge: gh(),
    }
}

pub(super) fn pinned(rule: Rule) -> Pinned {
    Pinned {
        rule,
        remote: "/remote.git".into(),
        target: "main".into(),
        branch: "famulen/t1".into(),
    }
}

fn finishing(rule: Rule) -> Finishing {
    Finishing::new(pinned(rule))
}

fn content(commit: &str) -> Content {
    Content {
        change: "c".into(),
        commit: commit.into(),
        tree: format!("tree-{commit}"),
        base: "base".into(),
        title: "title".into(),
        body: String::new(),
    }
}

/// Hands that answer as scripted, and tell what was done to the outside.
pub(super) struct Fake {
    pub(super) snapshot: Mutex<Snapshot>,
    pub(super) checks: Mutex<VecDeque<Result<(), String>>>,
    pub(super) verdicts: Mutex<VecDeque<Verdict>>,
    pub(super) ci: Mutex<VecDeque<Ci>>,
    pub(super) answer: Mutex<Answer>,
    pub(super) deployed: Mutex<Option<Result<(), String>>>,
    /// Another deployment is under way.
    pub(super) busy: AtomicBool,
    pub(super) merge_fails: bool,
    pub(super) lost: bool,
    pub(super) did: Mutex<Vec<String>>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(Snapshot::Changed(content("one"))),
            checks: Mutex::default(),
            verdicts: Mutex::default(),
            ci: Mutex::default(),
            answer: Mutex::new(Answer::Waiting(7)),
            deployed: Mutex::new(Some(Ok(()))),
            busy: AtomicBool::new(false),
            merge_fails: false,
            lost: false,
            did: Mutex::default(),
        }
    }
}

impl Fake {
    fn did(&self) -> Vec<String> {
        self.did.lock().unwrap().clone()
    }

    fn note(&self, what: impl Into<String>) {
        self.did.lock().unwrap().push(what.into());
    }

    fn now(&self, commit: &str) {
        *self.snapshot.lock().unwrap() = Snapshot::Changed(content(commit));
    }
}

impl Hands for Fake {
    fn keep(&self, _: &Finishing) -> anyhow::Result<()> {
        Ok(())
    }

    async fn snapshot(&self, _: &Pinned) -> anyhow::Result<Snapshot> {
        anyhow::ensure!(!self.lost, "no workspace");
        Ok(self.snapshot.lock().unwrap().clone())
    }

    async fn check(&self, _: &str) -> anyhow::Result<Result<(), String>> {
        Ok(self.checks.lock().unwrap().pop_front().unwrap_or(Ok(())))
    }

    async fn review(&self, _: &Pinned, content: &Content) -> anyhow::Result<Verdict> {
        self.note(format!("review {}", content.commit));
        Ok(self
            .verdicts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Verdict::Clean))
    }

    async fn publish(
        &self,
        _: &Pinned,
        content: &Content,
        _: Option<&str>,
        remaining: Option<&str>,
    ) -> anyhow::Result<PullRequest> {
        self.note(format!("publish {}", content.commit));
        if let Some(remaining) = remaining {
            self.note(format!("left {remaining}"));
        }
        Ok(PullRequest {
            number: 1,
            url: "https://forge.example/pull/1".into(),
            head: content.commit.clone(),
        })
    }

    async fn ci(&self, _: &Pinned, _: &PullRequest) -> anyhow::Result<Ci> {
        let mut ci = self.ci.lock().unwrap();
        Ok(match ci.len() {
            0 => Ci::Passed,
            1 => ci[0].clone(),
            _ => ci.pop_front().unwrap(),
        })
    }

    async fn merged(&self, _: &Pinned, _: &PullRequest) -> anyhow::Result<bool> {
        Ok(self.did().iter().any(|d| d.starts_with("merge ")))
    }

    async fn merge(&self, _: &Pinned, pull: &PullRequest) -> anyhow::Result<()> {
        anyhow::ensure!(!self.merge_fails, "conflict");
        self.note(format!("merge {}", pull.head));
        Ok(())
    }

    async fn confirm_merge(&self, _: &PullRequest, _: Option<i64>) -> anyhow::Result<Answer> {
        Ok(*self.answer.lock().unwrap())
    }

    async fn deploy(&self, _: &Pinned, _: &PullRequest, command: &str) -> anyhow::Result<Started> {
        if self.busy.load(Ordering::SeqCst) {
            return Ok(Started::Busy);
        }
        self.note(format!("deploy {command}"));
        Ok(Started::Yes)
    }

    async fn deployed(&self) -> anyhow::Result<Option<Result<(), String>>> {
        Ok(self.deployed.lock().unwrap().clone())
    }

    async fn tidy(&self, _: &Pinned, merged: Option<&str>) -> anyhow::Result<()> {
        self.note(match merged {
            Some(head) => format!("tidy {head}"),
            None => "tidy".to_owned(),
        });
        Ok(())
    }
}

/// Step until it is something other than going on.
async fn run(finishing: &mut Finishing, hands: &Fake) -> Next {
    for _ in 0..50 {
        match finishing.step(hands, 0).await.unwrap() {
            Next::Again => {}
            next => return next,
        }
    }
    panic!("it never came to rest: {finishing:?}");
}

/// What was done to the outside, reviews aside.
fn outside(hands: &Fake) -> Vec<String> {
    hands
        .did()
        .into_iter()
        .filter(|d| d.starts_with("publish") || d.starts_with("merge") || d.starts_with("deploy"))
        .collect()
}

#[tokio::test]
async fn work_that_passes_is_published_merged_and_tidied() {
    let (mut f, hands) = (finishing(rule()), Fake::default());
    f.turned();
    assert_eq!(run(&mut f, &hands).await, Next::Done);
    assert_eq!(
        hands.did(),
        ["review one", "publish one", "merge one", "tidy one"]
    );
    assert!(f.summary().contains("Merged: https://forge.example/pull/1"));
}

#[tokio::test]
async fn work_that_changes_nothing_goes_nowhere() {
    let (mut f, hands) = (finishing(rule()), Fake::default());
    *hands.snapshot.lock().unwrap() = Snapshot::Unchanged;
    f.turned();
    assert_eq!(run(&mut f, &hands).await, Next::Done);
    assert_eq!(outside(&hands), Vec::<String>::new());
}

#[tokio::test]
async fn faults_handed_back_and_left_as_they_were_are_still_not_pushed() {
    let (mut f, hands) = (finishing(rule()), Fake::default());
    hands.verdicts.lock().unwrap().push_back(Verdict::Findings {
        text: "loses data".into(),
        serious: true,
    });
    f.turned();
    assert!(matches!(run(&mut f, &hands).await, Next::Turn(_)));
    f.turned();
    assert!(matches!(run(&mut f, &hands).await, Next::Stop(why) if why.contains("loses data")));
    assert_eq!(outside(&hands), Vec::<String>::new());
}

#[tokio::test]
async fn other_findings_left_as_they_were_go_with_the_pull_request() {
    let (mut f, hands) = (finishing(rule()), Fake::default());
    hands.verdicts.lock().unwrap().push_back(Verdict::Findings {
        text: "could be simpler".into(),
        serious: false,
    });
    f.turned();
    assert!(matches!(run(&mut f, &hands).await, Next::Turn(_)));
    f.turned();
    assert_eq!(run(&mut f, &hands).await, Next::Done);
    assert_eq!(
        hands.did()[..3],
        ["review one", "publish one", "left could be simpler"]
    );
}

#[tokio::test]
async fn a_failed_check_is_told_to_the_agent_and_the_third_stops() {
    let (mut f, hands) = (finishing(rule()), Fake::default());
    hands
        .checks
        .lock()
        .unwrap()
        .extend(std::iter::repeat_n(Err("boom at line 3".to_owned()), 3));
    for _ in 0..2 {
        f.turned();
        let Next::Turn(message) = run(&mut f, &hands).await else {
            panic!("the failure is not handed back");
        };
        assert!(message.contains("just ci") && message.contains("boom at line 3"));
        assert_eq!(f.stage, Stage::Implementing);
    }
    f.turned();
    let Next::Stop(why) = run(&mut f, &hands).await else {
        panic!("it goes on after the third");
    };
    assert!(why.contains("3 times"), "{why}");
    assert_eq!(outside(&hands), Vec::<String>::new());
}

#[tokio::test]
async fn ci_that_fails_is_handed_back_and_nothing_is_merged() {
    let (mut f, hands) = (finishing(rule()), Fake::default());
    hands.ci.lock().unwrap().push_back(Ci::Failed {
        name: "test".into(),
        log: "assertion failed".into(),
    });
    f.turned();
    let Next::Turn(message) = run(&mut f, &hands).await else {
        panic!("the failure is not handed back");
    };
    assert!(message.contains("assertion failed"));
    assert_eq!(outside(&hands), ["publish one"]);
}

#[tokio::test]
async fn ci_that_is_not_done_merges_nothing_and_is_given_up_in_the_end() {
    let (mut f, hands) = (finishing(rule()), Fake::default());
    hands.ci.lock().unwrap().push_back(Ci::Pending);
    f.turned();
    assert_eq!(run(&mut f, &hands).await, Next::Wait(CI_EVERY));
    assert!(matches!(
        f.step(&hands, CI_LIMIT_SECONDS + 1).await.unwrap(),
        Next::Stop(_)
    ));
    assert_eq!(outside(&hands), ["publish one"]);
}

#[tokio::test]
async fn content_that_changed_after_ci_is_checked_and_published_anew() {
    let (mut f, hands) = (finishing(rule()), Fake::default());
    hands.ci.lock().unwrap().push_back(Ci::Pending);
    f.turned();
    run(&mut f, &hands).await;
    hands.now("two");
    hands.ci.lock().unwrap()[0] = Ci::Passed;
    assert_eq!(run(&mut f, &hands).await, Next::Done);
    assert_eq!(outside(&hands), ["publish one", "publish two", "merge two"]);
}

#[tokio::test]
async fn findings_are_handed_back_once_and_what_is_left_goes_with_the_pull_request() {
    let (mut f, hands) = (finishing(rule()), Fake::default());
    let findings = |text: &str| Verdict::Findings {
        text: text.into(),
        serious: false,
    };
    hands
        .verdicts
        .lock()
        .unwrap()
        .extend([findings("first"), findings("second")]);
    f.turned();
    assert!(matches!(run(&mut f, &hands).await, Next::Turn(m) if m.contains("first")));
    hands.now("two");
    f.turned();
    assert_eq!(run(&mut f, &hands).await, Next::Done);
    assert_eq!(
        hands.did()[..4],
        ["review one", "review two", "publish two", "left second"]
    );
}

#[tokio::test]
async fn faults_of_correctness_left_by_the_last_review_push_nothing() {
    let mut once = rule();
    once.reviews = 1;
    let (mut f, hands) = (finishing(once), Fake::default());
    hands.verdicts.lock().unwrap().push_back(Verdict::Findings {
        text: "loses data".into(),
        serious: true,
    });
    f.turned();
    assert!(matches!(run(&mut f, &hands).await, Next::Stop(why) if why.contains("loses data")));
    assert_eq!(outside(&hands), Vec::<String>::new());
}

#[tokio::test]
async fn a_merge_the_owner_did_not_allow_waits_for_them() {
    let mut asks = rule();
    asks.merge = false;
    asks.deploy = Some("deploy it".into());
    let (mut f, hands) = (finishing(asks.clone()), Fake::default());
    f.turned();
    assert_eq!(run(&mut f, &hands).await, Next::Wait(ASKED_EVERY));
    assert_eq!(f.ticket, Some(7));
    assert_eq!(outside(&hands), ["publish one"]);
    *hands.answer.lock().unwrap() = Answer::Yes;
    assert_eq!(run(&mut f, &hands).await, Next::Done);
    assert_eq!(
        outside(&hands),
        ["publish one", "merge one", "deploy deploy it"]
    );

    let (mut f, hands) = (finishing(asks), Fake::default());
    *hands.answer.lock().unwrap() = Answer::No;
    f.turned();
    assert_eq!(run(&mut f, &hands).await, Next::Done);
    assert_eq!(outside(&hands), ["publish one"]);
    assert!(f.summary().contains("not merged"), "{}", f.summary());
}

#[tokio::test]
async fn an_approval_to_merge_does_not_carry_over_to_changed_content() {
    let mut asks = rule();
    asks.merge = false;
    let (mut f, hands) = (finishing(asks), Fake::default());
    f.turned();
    run(&mut f, &hands).await;
    *hands.answer.lock().unwrap() = Answer::Yes;
    assert_eq!(f.step(&hands, 0).await.unwrap(), Next::Again);
    hands.now("two");
    *hands.answer.lock().unwrap() = Answer::Waiting(8);
    assert_eq!(run(&mut f, &hands).await, Next::Wait(ASKED_EVERY));
    assert_eq!(outside(&hands), ["publish one", "publish two"]);
    assert_eq!(f.ticket, Some(8));
}

#[tokio::test]
async fn a_merge_that_fails_is_handed_back() {
    let (mut f, hands) = (
        finishing(rule()),
        Fake {
            merge_fails: true,
            ..Fake::default()
        },
    );
    f.turned();
    assert!(matches!(run(&mut f, &hands).await, Next::Turn(m) if m.contains("conflict")));
}

#[tokio::test]
async fn deploying_waits_for_the_one_under_way_and_gives_up_in_time() {
    let mut deploys = rule();
    deploys.deploy = Some("deploy it".into());
    let (mut f, hands) = (finishing(deploys), Fake::default());
    hands.busy.store(true, Ordering::SeqCst);
    f.stage = Stage::Deploying;
    f.merged = true;
    f.pull = Some(PullRequest {
        number: 1,
        url: "https://forge.example/pull/1".into(),
        head: "one".into(),
    });
    assert!(matches!(f.step(&hands, 0).await.unwrap(), Next::Wait(_)));
    assert_eq!(f.deploy_started, None);
    assert_eq!(outside(&hands), Vec::<String>::new());

    let mut late = f.clone();
    assert!(matches!(
        late.step(&hands, DEPLOY_LIMIT_SECONDS + 1).await.unwrap(),
        Next::Stop(why) if why.contains("not deployed")
    ));

    hands.busy.store(false, Ordering::SeqCst);
    assert_eq!(f.step(&hands, 60).await.unwrap(), Next::Again);
    assert_eq!(f.deploy_started, Some(60));
    assert_eq!(outside(&hands), ["deploy deploy it"]);
}

#[tokio::test]
async fn deploying_that_was_cut_off_is_not_run_again() {
    let mut deploys = rule();
    deploys.deploy = Some("deploy it".into());
    let (mut f, hands) = (finishing(deploys), Fake::default());
    *hands.deployed.lock().unwrap() = None;
    f.stage = Stage::Deploying;
    f.deploy_started = Some(0);
    assert!(matches!(f.step(&hands, 0).await.unwrap(), Next::Wait(_)));
    assert!(matches!(
        f.step(&hands, DEPLOY_LIMIT_SECONDS + 1).await.unwrap(),
        Next::Stop(_)
    ));
    assert_eq!(outside(&hands), Vec::<String>::new());
}

#[tokio::test]
async fn what_is_kept_goes_on_where_it_was_after_a_restart() {
    let dir = crate::notebook::testing::TempDir::new("finishing");
    let path = dir.path().join("store.db");
    let hands = Fake::default();
    hands.ci.lock().unwrap().push_back(Ci::Pending);
    {
        let store = Store::open(&path).unwrap();
        let mut f = finishing(rule());
        f.turned();
        run(&mut f, &hands).await;
        f.save(&store, 3).unwrap();
    }
    let store = Store::open(&path).unwrap();
    let mut f = Finishing::load(&store, 3).unwrap().unwrap();
    assert_eq!(f.stage, Stage::Ci);
    hands.ci.lock().unwrap()[0] = Ci::Passed;
    assert_eq!(run(&mut f, &hands).await, Next::Done);
    assert_eq!(outside(&hands), ["publish one", "merge one"]);
}

#[tokio::test]
async fn a_turn_not_yet_taken_is_told_again_after_a_restart() {
    let (mut f, hands) = (finishing(rule()), Fake::default());
    hands.checks.lock().unwrap().push_back(Err("boom".into()));
    f.turned();
    let told = run(&mut f, &hands).await;
    assert_eq!(f.step(&hands, 0).await.unwrap(), told);
}

#[tokio::test]
async fn what_changed_after_the_last_review_allowed_is_not_pushed() {
    let mut once = rule();
    once.reviews = 1;
    let (mut f, hands) = (finishing(once), Fake::default());
    hands.ci.lock().unwrap().extend([
        Ci::Failed {
            name: "ci".into(),
            log: "broke".into(),
        },
        Ci::Passed,
    ]);
    f.turned();
    assert!(matches!(run(&mut f, &hands).await, Next::Turn(_)));
    hands.now("two");
    f.turned();
    let next = run(&mut f, &hands).await;
    assert!(
        matches!(&next, Next::Stop(why) if why.contains("review") && why.contains("/pull/1")),
        "{next:?}"
    );
    assert_eq!(outside(&hands), ["publish one"]);
}
