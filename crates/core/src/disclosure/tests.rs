use std::collections::BTreeMap;

use futures_util::FutureExt;
use rusqlite::params;
use serde_json::json;

use super::sources::{Looks, is_slack};
use super::*;
use crate::approvals::Answer;
use crate::connections::Connection;
use crate::notebook::testing::TempDir;
use crate::slack::fake::FakeSlack;

const PUBLIC: &str = "C1";
const PRIVATE: &str = "G1";
const OTHER_PRIVATE: &str = "G2";
const SHARED: &str = "C2";
const OWNERS_DM: &str = "DM-U1";

/// Answers as told, whatever is read.
struct Told(anyhow::Result<Reading, String>);

impl Reader for Told {
    fn read<'a>(&'a self, _: &'a Saying) -> BoxFuture<'a, anyhow::Result<Reading>> {
        let told = self.0.clone().map_err(anyhow::Error::msg);
        async move { told }.boxed()
    }
}

struct World {
    store: Store,
    permissions: Permissions,
    approvals: Approvals<&'static FakeSlack>,
    reader: Arc<dyn Reader>,
    memory: std::path::PathBuf,
    _dir: TempDir,
}

fn room(name: &str, private: bool) -> Room {
    Room {
        name: name.into(),
        private: Some(private),
        ..Room::default()
    }
}

impl World {
    fn new(source: Source) -> Self {
        Self::reading(source, Ok(Reading::default()))
    }

    fn reading(source: Source, reading: anyhow::Result<Reading, String>) -> Self {
        let dir = TempDir::new("disclosure");
        let memory = dir.path().join("memory");
        std::fs::create_dir_all(memory.join("long")).unwrap();
        std::fs::create_dir_all(memory.join("episodic")).unwrap();
        let slack: &'static FakeSlack = Box::leak(Box::default());
        *slack.rooms.lock().unwrap() = vec![
            (PUBLIC.into(), room("general", false)),
            (PRIVATE.into(), room("secret", true)),
            (OTHER_PRIVATE.into(), room("other", true)),
            (
                SHARED.into(),
                Room {
                    shared: true,
                    ..room("with", false)
                },
            ),
        ];
        let store = Store::open_in_memory().unwrap();
        let permissions = Permissions::new(
            store.clone(),
            crate::permissions::test_gateway(
                famulen_gateway::store::Store::open_in_memory().unwrap(),
            ),
        );
        let clients = SlackClients::new([("w1".to_owned(), slack)], "w1").unwrap();
        let owners = BTreeMap::from([("w1".to_owned(), "U1".to_owned())]);
        let approvals = Approvals::new(Some(clients), store.clone(), permissions.clone(), owners);
        let world = Self {
            store,
            permissions,
            approvals,
            reader: Arc::new(Told(reading)),
            memory,
            _dir: dir,
        };
        world.turn(source, "x");
        world
    }

    fn turn(&self, source: Source, text: &str) -> i64 {
        self.store.end_turn().unwrap();
        let sender = match source {
            Source::Slack { .. } | Source::Console { .. } | Source::Approval { .. } => {
                Sender::Owner
            }
            _ => Sender::System,
        };
        let input = Input {
            sender,
            source,
            text: text.into(),
        };
        let id = self.store.enqueue(&input).unwrap().unwrap();
        self.store.start_turn(id, "all").unwrap();
        id
    }

    fn running(&self) -> i64 {
        self.store.running().unwrap().unwrap().input
    }

    fn input(&self, id: i64) -> Input {
        self.store.get(id).unwrap().unwrap().input
    }

    fn add_run(&self, run: i64, task: &str, spec: &str) {
        self.store
            .conn()
            .execute(
                "INSERT INTO worker_runs (id, task, runner, state, attempt, spec, created_at)
                 VALUES (?1, ?2, 'agent', 'ended', 1, ?3, 0)",
                params![run, task, spec],
            )
            .unwrap();
    }

    fn gate(&self) -> Gate<'_, &'static FakeSlack> {
        Gate {
            store: &self.store,
            memory_dir: &self.memory,
            permissions: &self.permissions,
            approvals: &self.approvals,
            reader: Some(&self.reader),
        }
    }

    fn read_party(&self, code: &str) {
        let answer = json!({ "kind": "parties", "code": code, "title": "A" }).to_string();
        let taints = of_own(&self.memory, "get_record", &json!({}), &answer);
        assert!(!taints.is_empty());
        note(&self.store, taints).unwrap();
    }

    async fn read_channel(&self, channel: &str) {
        let room = self.approvals.room("w1", channel).await;
        note(&self.store, vec![of_room("w1", "", channel, &room)]).unwrap();
    }

    async fn asks(&self, channel: &str) -> Option<Asking> {
        self.asks_of(channel, "本文").await
    }

    async fn asks_of(&self, channel: &str, text: &str) -> Option<Asking> {
        let input = said(channel, text);
        self.gate().asks("w1", channel, text, &input).await
    }

    /// Hold a saying the way the session does.
    async fn held(&self, channel: &str, text: &str) -> i64 {
        let asking = self.asks_of(channel, text).await.unwrap();
        let bound = crate::permissions::Bound {
            ask: Some(asking.why.clone()),
            waive: false,
        };
        self.permissions
            .set_scoped(
                "owner",
                "conn:famulen",
                "send_slack",
                Some(crate::permissions::Permission::Allow),
            )
            .unwrap();
        let decision = self
            .permissions
            .decide_bound(
                crate::permissions::Caller::Secretary,
                "mcp__famulen__send_slack",
                &said(channel, text),
                bound,
            )
            .await
            .unwrap();
        let crate::permissions::Decision::Held(id) = decision else {
            panic!("{decision:?}");
        };
        asking.held(&self.store, id);
        let ticket = self.permissions.gateway().ticket(id).unwrap().unwrap();
        self.approvals.present(&ticket).await.unwrap();
        id
    }

    fn after(&self, request: i64) {
        self.turn(
            Source::Approval {
                id: request,
                user: "U1".into(),
            },
            "approved",
        );
    }
}

fn said(channel: &str, text: &str) -> Value {
    json!({ "workspace": "w1", "channel": channel, "text": text })
}

fn console() -> Source {
    console_at(1)
}

fn console_at(message: i64) -> Source {
    Source::Console {
        message,
        workgroup: "all".into(),
        view: String::new(),
        selection: None,
        reply_to: None,
        attached: Vec::new(),
    }
}

fn slack_in(channel: &str) -> Source {
    Source::Slack {
        workspace: "w1".into(),
        channel: channel.into(),
        ts: "1.0".into(),
        thread_ts: None,
        user: "U1".into(),
        unmentioned: false,
        others_before: false,
        attached: Vec::new(),
    }
}

fn discloses() -> Reading {
    Reading {
        discloses: Some("人の評価".into()),
        allowed_by: None,
    }
}

#[tokio::test]
async fn what_a_turn_read_of_people_is_asked_about_before_it_is_said_in_public() {
    let world = World::new(slack_in(PUBLIC));
    world.read_party("party-aaaaaa");
    let asking = world.asks(PUBLIC).await.unwrap();
    for part in ["公開のチャンネル #general", "相手の記録", "party-aaaaaa"] {
        assert!(asking.why.contains(part), "{}", asking.why);
    }
    assert!(reason_of(&asking.why).is_some());
}

#[tokio::test]
async fn an_ordinary_answer_is_not_asked_about() {
    assert_eq!(World::new(slack_in(PUBLIC)).asks(PUBLIC).await, None);
}

#[tokio::test]
async fn the_owners_dm_is_never_asked_about() {
    let world = World::reading(console(), Ok(discloses()));
    world.read_party("party-aaaaaa");
    assert_eq!(world.asks(OWNERS_DM).await, None);
    let main = world.gate().asks("", OWNERS_DM, "x", &Value::Null).await;
    assert_eq!(main, None);
}

#[tokio::test]
async fn what_was_said_in_a_private_channel_may_go_back_there_and_nowhere_else() {
    let world = World::new(console());
    world.read_channel(PRIVATE).await;
    assert_eq!(world.asks(PRIVATE).await, None);
    for wider in [PUBLIC, OTHER_PRIVATE] {
        let why = world.asks(wider).await.unwrap().why;
        assert!(why.contains("#secret"), "{why}");
    }
}

#[tokio::test]
async fn what_the_owner_asks_in_a_private_channel_is_of_that_channel() {
    let world = World::new(slack_in(PRIVATE));
    assert_eq!(world.asks(PRIVATE).await, None);
    assert!(world.asks(PUBLIC).await.is_some());
}

#[tokio::test]
async fn what_was_said_in_public_is_not_confidential() {
    let world = World::new(slack_in(PUBLIC));
    world.read_channel(PUBLIC).await;
    assert_eq!(world.asks(OTHER_PRIVATE).await, None);
}

#[tokio::test]
async fn a_place_slack_tells_nothing_of_is_taken_for_public() {
    let world = World::new(console());
    world.read_party("party-aaaaaa");
    for unknown in ["C9", "D9", "#general", "U2"] {
        let why = world.asks(unknown).await.unwrap().why;
        assert!(why.contains("公開として扱う"), "{why}");
    }
}

#[tokio::test]
async fn an_approval_lets_it_be_said_and_what_goes_on_there_is_not_asked_again() {
    let world = World::new(slack_in(PUBLIC));
    world.read_party("party-aaaaaa");
    let id = world.held(PUBLIC, "本文").await;
    assert!(
        world.asks_of(PUBLIC, "続き").await.is_some(),
        "not answered yet"
    );
    world
        .approvals
        .answer_by_id(id, Answer::Approve)
        .await
        .unwrap()
        .unwrap();
    world.after(id);

    // The approved words use up what the owner's answer gave.
    let asking = world.asks(PUBLIC).await.unwrap();
    let bound = crate::permissions::Bound {
        ask: Some(asking.why),
        waive: false,
    };
    let caller = crate::permissions::Caller::Secretary;
    let tool = "mcp__famulen__send_slack";
    let decision = world
        .permissions
        .decide_bound(caller, tool, &said(PUBLIC, "本文"), bound)
        .await
        .unwrap();
    assert_eq!(decision, crate::permissions::Decision::Allow);

    assert_eq!(world.asks_of(PUBLIC, "続き").await, None);
    assert!(world.asks_of(OTHER_PRIVATE, "続き").await.is_some());
    world.read_party("party-bbbbbb");
    assert!(world.asks_of(PUBLIC, "続き").await.is_some());
}

#[tokio::test]
async fn a_rejection_lets_nothing_be_said() {
    let world = World::new(slack_in(PUBLIC));
    world.read_party("party-aaaaaa");
    let id = world.held(PUBLIC, "本文").await;
    world
        .approvals
        .answer_by_id(id, Answer::Reject)
        .await
        .unwrap();
    world.after(id);
    assert!(world.asks(PUBLIC).await.is_some());
    assert!(world.asks_of(PUBLIC, "言い換え").await.is_some());
}

#[tokio::test]
async fn a_request_is_never_allowed_from_now_on_and_is_kept_to_the_dm() {
    let world = World::new(slack_in(PUBLIC));
    world.read_party("party-aaaaaa");
    let id = world.held(PUBLIC, "本文").await;
    let ticket = world.permissions.gateway().ticket(id).unwrap().unwrap();
    assert!(crate::approvals::may_show_secret(&ticket));
    assert!(
        crate::approvals::summary(&ticket)
            .unwrap()
            .contains("相手の記録")
    );
    world
        .approvals
        .answer_by_id(id, Answer::AlwaysAllow)
        .await
        .unwrap();
    let ticket = world.permissions.gateway().ticket(id).unwrap().unwrap();
    assert_eq!(ticket.status, "approved");
    let log: Vec<String> = Journal::new(world.store.clone())
        .recent(10)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == JOURNAL)
        .map(|e| e.detail)
        .collect();
    assert_eq!(log.len(), 2, "{log:?}");
}

#[tokio::test]
async fn the_owners_own_words_allow_that_saying_when_they_are_really_theirs() {
    for (quoted, sender_said, asked) in [
        ("公開で言ってよい", "これは公開で言ってよいです", false),
        ("公開で言ってよい", "まとめて", true),
        ("公", "公開で言ってよいです", true),
    ] {
        let reading = Reading {
            discloses: Some("人の評価".into()),
            allowed_by: Some(quoted.into()),
        };
        let world = World::reading(console(), Ok(reading));
        world.turn(slack_in(PUBLIC), sender_said);
        world.read_party("party-aaaaaa");
        assert_eq!(world.asks(PUBLIC).await.is_some(), asked, "{sender_said}");
    }
}

#[tokio::test]
async fn what_an_earlier_turn_read_is_found_in_what_is_said() {
    let world = World::reading(slack_in(PUBLIC), Ok(discloses()));
    let why = world.asks(PUBLIC).await.unwrap().why;
    assert!(why.contains("人の評価"), "{why}");
    let broken = World::reading(slack_in(PUBLIC), Err("上限".into()));
    assert!(broken.asks(PUBLIC).await.is_some());
    let none = Gate {
        reader: None,
        ..broken.gate()
    };
    let asking = none.asks("w1", PUBLIC, "x", &Value::Null).await;
    assert!(asking.is_some());
}

#[tokio::test]
async fn what_a_turn_read_goes_with_what_it_leaves_for_later() {
    let world = World::new(console());
    world.read_party("party-aaaaaa");
    let gate = world.gate();
    gate.hand_on("wake_at", r#"{"id":"7"}"#).await.unwrap();
    gate.hand_on("delegate_task", r#"{"kind":"tasks","id":"T1"}"#)
        .await
        .unwrap();
    world
        .store
        .conn()
        .execute(
            "INSERT INTO worker_runs (id, task, runner, state, attempt, spec, created_at)
             VALUES (3, 'T1', 'agent', 'ended', 1, '{}', 0)",
            [],
        )
        .unwrap();
    world.turn(
        Source::Wake {
            id: 7,
            at: "now".into(),
        },
        "x",
    );
    assert!(world.asks(PUBLIC).await.is_some());
    world.turn(
        Source::Worker {
            run: 3,
            task: "task-aaaaaa".into(),
            outcome: "done".into(),
            workgroup: "all".into(),
            again: 0,
        },
        "x",
    );
    assert!(world.asks(PUBLIC).await.is_some());
    world.turn(
        Source::Wake {
            id: 8,
            at: "now".into(),
        },
        "x",
    );
    assert_eq!(world.asks(PUBLIC).await, None);
}

#[tokio::test]
async fn what_a_handed_file_holds_goes_with_the_task() {
    let world = World::new(console());
    let file = world.memory.join("long/a.md");
    std::fs::write(&file, "[[records/parties/x|A]]").unwrap();
    world
        .store
        .conn()
        .execute(
            "INSERT INTO worker_runs (id, task, runner, state, attempt, spec, created_at)
             VALUES (3, 'T1', 'agent', 'ended', 1, ?1, 0)",
            [json!({ "files": ["memory/long/a.md"] }).to_string()],
        )
        .unwrap();
    let listed = json!({ "ended": [{ "id": "3", "task": "T1" }] }).to_string();
    let kinds: Vec<Kind> = world
        .gate()
        .of_runs(&listed)
        .unwrap()
        .iter()
        .map(|t| t.kind)
        .collect();
    assert!(kinds.contains(&Kind::People), "{kinds:?}");
    assert!(world.gate().of_runs("done").unwrap().is_empty());
    let done = Source::Worker {
        run: 3,
        task: "task-aaaaaa".into(),
        outcome: "done".into(),
        workgroup: "all".into(),
        again: 0,
    };
    world.turn(done, "x");
    assert!(world.asks(PUBLIC).await.is_some());
}

#[tokio::test]
async fn mail_may_be_answered_by_mail_and_what_the_notebook_holds_is_asked_about() {
    let world = World::new(console());
    note(
        &world.store,
        vec![Taint::of_connection("gmail", "のやりとり")],
    )
    .unwrap();
    assert_eq!(world.gate().asks_outside("gmail", &Value::Null).await, None);
    assert!(
        world
            .gate()
            .asks_outside("bluesky", &Value::Null)
            .await
            .is_some()
    );
    world.read_party("party-aaaaaa");
    assert!(
        world
            .gate()
            .asks_outside("gmail", &Value::Null)
            .await
            .is_some()
    );
}

#[test]
fn records_of_people_and_of_the_owners_own_are_told_from_those_of_work() {
    let dir = TempDir::new("disclosure-kinds");
    let memory = dir.path().join("memory");
    let kind_of = |kind: &str| {
        let answer = json!({ "kind": kind, "id": "1", "title": "t" }).to_string();
        let taints = of_own(&memory, "get_record", &json!({}), &answer);
        taints.first().map(|t| t.kind)
    };
    for (kind, expected) in [
        ("parties", Some(Kind::People)),
        ("contracts", Some(Kind::Belongings)),
        ("holdings", Some(Kind::Belongings)),
        ("recipes", Some(Kind::Belongings)),
        ("tasks", None),
        ("events", None),
        ("memos", None),
    ] {
        assert_eq!(kind_of(kind), expected, "{kind}");
    }
}

#[test]
fn what_is_found_in_the_notebook_is_judged_by_where_it_is_and_whom_it_tells_of() {
    let dir = TempDir::new("disclosure-search");
    let memory = dir.path().join("memory");
    std::fs::create_dir_all(memory.join("long")).unwrap();
    std::fs::create_dir_all(memory.join("episodic")).unwrap();
    std::fs::write(memory.join("long/a.md"), "[[records/parties/x|A]] は速い").unwrap();
    std::fs::write(memory.join("long/b.md"), "ビルドの手順").unwrap();
    std::fs::write(
        memory.join("episodic/d.md"),
        "- 速い [[records/parties/x|A]] {id: n1}\n- ビルド {id: n2}",
    )
    .unwrap();
    let found = |scope: &str, path: &str, id: &str| {
        let hits = json!({ "hits": [{ "scope": scope, "path": path, "id": id, "title": "t" }] });
        let taints = of_own(&memory, "search_notebook", &json!({}), &hits.to_string());
        taints.first().map(|t| t.kind)
    };
    let people = Some(Kind::People);
    for (scope, path, id, expected) in [
        ("records/parties", "records/parties/x.md", "x", people),
        (
            "records/contracts",
            "records/contracts/x.md",
            "x",
            Some(Kind::Belongings),
        ),
        ("records/tasks", "records/tasks/x.md", "x", None),
        ("memory/long", "memory/long/a.md", "a", people),
        ("memory/long", "memory/long/b.md", "b", None),
        ("memory/episodic", "memory/episodic/d.md", "n1", people),
        ("memory/episodic", "memory/episodic/d.md", "n2", None),
    ] {
        assert_eq!(found(scope, path, id), expected, "{path} {id}");
    }
    assert!(of_own(&memory, "search_notebook", &json!({}), "done").is_empty());
    assert!(!of_own(&memory, "search_notebook", &json!({}), "cut off {").is_empty());
}

#[test]
fn an_apis_answer_is_of_its_connection() {
    let arguments = json!({ "connection": "x" });
    let taints = of_own(Path::new("/n/memory"), "call_api", &arguments, "{}");
    assert!(taints[0].came_by("x") && !taints[0].came_by("y"));
}

#[test]
fn a_new_tool_of_famulens_own_counts_as_private_until_it_is_told_plain() {
    let memory = Path::new("/n/memory");
    let reads = |tool: &str| !of_own(memory, tool, &json!({}), "{}").is_empty();
    for tool in ["brand_new", "list_messages", "list_inputs", "list_secrets"] {
        assert!(reads(tool), "{tool}");
    }
    for tool in ["list_events", "post_message", "send_slack", "remember"] {
        assert!(!reads(tool), "{tool}");
    }
}

#[test]
fn files_of_the_notebook_read_by_hand_count_too() {
    let dir = TempDir::new("disclosure-files");
    let memory = dir.path().join("memory");
    std::fs::create_dir_all(&memory).unwrap();
    for kind in ["parties", "tasks"] {
        let records = dir.path().join("records").join(kind);
        std::fs::create_dir_all(&records).unwrap();
        std::fs::write(records.join("x.md"), "x").unwrap();
    }
    let at = |path: &str| dir.path().join(path).display().to_string();
    let reads = |tool: &str, input: Value| {
        !of_builtin(&memory, Path::new("/work"), tool, &input).is_empty()
    };
    for (path, expected) in [
        ("records/parties/x.md", true),
        ("records/tasks/../parties/x.md", true),
        ("records/parties", true),
        ("memory", true),
        ("", true),
        ("records/tasks/x.md", false),
    ] {
        assert_eq!(
            reads("Read", json!({ "file_path": at(path) })),
            expected,
            "{path}"
        );
        assert_eq!(
            reads("Grep", json!({ "path": at(path) })),
            expected,
            "{path}"
        );
    }
    let above = dir.path().parent().unwrap().display().to_string();
    assert!(reads("Grep", json!({ "path": above })));
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    for (tool, input) in [
        ("Read", json!({ "file_path": "../records/parties/x.md" })),
        ("Glob", json!({ "pattern": "../records/parties/*.md" })),
    ] {
        let taints = of_builtin(&memory, &work, tool, &input);
        assert!(!taints.is_empty(), "{tool}");
    }
    assert!(reads(
        "Glob",
        json!({ "pattern": at("records/parties/*.md") })
    ));
    assert!(reads("Bash", json!({ "command": "ls" })));
    assert!(reads("Read", json!({ "file_path": "/work/a.md" })));
}

#[tokio::test]
async fn what_slack_answers_is_told_apart_by_who_can_read_it() {
    let world = World::new(console());
    let slack = remote("w1", "slack");
    assert!(is_slack(&slack));
    let read = async |c: &Connection, tool: &str, arguments: Value, answer: Value| {
        let message =
            json!({ "method": "tools/call", "params": { "name": tool, "arguments": arguments } });
        let response =
            json!({ "result": { "content": [{ "type": "text", "text": answer.to_string() }] } });
        of_connection(
            world.approvals.slack(),
            &REPOSITORIES,
            c,
            &message,
            &response,
        )
        .await
    };
    let kinds = |taints: Vec<Taint>| taints.into_iter().map(|t| t.kind).collect::<Vec<_>>();
    for (channel, expected) in [
        (PUBLIC, vec![Kind::Internal]),
        (PRIVATE, vec![Kind::Private]),
        ("D9", vec![Kind::Private]),
    ] {
        for tool in ["read_channel", "channel_info"] {
            let arguments = json!({ "channel": channel });
            let taints = read(&slack, tool, arguments, json!({})).await;
            assert_eq!(kinds(taints), expected, "{tool} {channel}");
        }
    }
    let found = json!({ "messages": { "matches": [
        { "channel": { "id": "C1", "name": "general", "is_private": false } },
        { "channel": { "id": "G1", "name": "secret", "is_private": true } },
        { "channel": { "id": "D1", "name": "U2", "is_im": true, "is_private": false } },
    ] } });
    let taints = read(&slack, "search_messages", json!({}), found).await;
    let places: Vec<(&str, Kind)> = taints.iter().map(|t| (t.place.as_str(), t.kind)).collect();
    assert_eq!(
        places,
        [
            ("C1", Kind::Internal),
            ("G1", Kind::Private),
            ("D1", Kind::Private)
        ]
    );
    let strange = read(&slack, "search_messages", json!({}), json!("?")).await;
    assert_eq!(kinds(strange), [Kind::Private]);
    for tool in ["list_channels", "user_info"] {
        let taints = read(&slack, tool, json!({}), json!({})).await;
        assert_eq!(kinds(taints), [Kind::Internal], "{tool}");
    }
    for tool in ["post_message", "add_reaction", "get_permalink"] {
        assert!(
            read(&slack, tool, json!({}), json!({})).await.is_empty(),
            "{tool}"
        );
    }
    assert_eq!(
        read(&slack, "download_file", json!({}), json!({}))
            .await
            .len(),
        1
    );
}

#[test]
fn a_room_is_described_by_who_reads_it() {
    let described = |channel: Value| Room::of(&channel).described();
    for (channel, expected) in [
        (
            json!({ "name": "general", "is_private": false, "num_members": 120 }),
            "公開のチャンネル #general（120 人）",
        ),
        (
            json!({ "name": "few", "is_private": true, "num_members": 3 }),
            "非公開のチャンネル #few（3 人）",
        ),
        (
            json!({ "name": "with", "is_private": false, "is_ext_shared": true }),
            "公開のチャンネル #with（外部と共有）",
        ),
        (json!({ "name": "U2", "is_im": true }), "ほかの人との DM"),
        (json!({}), "公開かどうか分からない場（公開として扱う）"),
    ] {
        assert_eq!(described(channel), expected);
    }
}

#[test]
fn workers_say_nothing_on_slack() {
    const SLACK: &str = include_str!("../../../gateway/services/slack/slack.toml");
    let store = famulen_gateway::store::Store::open_in_memory().unwrap();
    let gateway = famulen_gateway::Gateway::new(
        store.clone(),
        crate::connections::Connections::new(store),
        reqwest::Client::new(),
        None,
    )
    .with_definitions(None, &[("slack.toml".into(), SLACK.into())]);
    gateway.add_defined("slack", BTreeMap::new(), None).unwrap();
    let permissions = Permissions::new(Store::open_in_memory().unwrap(), gateway);
    for tool in ["post_message", "post_ephemeral", "update_message"] {
        assert!(says_on_slack(&permissions, "slack", tool), "{tool}");
    }
    assert!(!says_on_slack(&permissions, "slack", "read_channel"));
    assert!(!says_on_slack(&permissions, "other", "post_message"));
}

#[tokio::test]
async fn an_approval_does_not_cover_what_is_read_after_it() {
    let world = World::new(slack_in(PUBLIC));
    let read = |answer: &str| {
        let answer = format!(r#"{{"items":[{{"text":"{answer}","party":"x"}}],"party":"x"}}"#);
        let taints = of_own(&world.memory, "get_layer", &json!({}), &answer);
        assert!(!taints.is_empty());
        note(&world.store, taints).unwrap();
    };
    read("A");
    let id = world.held(PUBLIC, "本文").await;
    world
        .approvals
        .answer_by_id(id, Answer::Approve)
        .await
        .unwrap();
    world.after(id);
    assert_eq!(world.asks_of(PUBLIC, "続き").await, None);
    read("A");
    assert_eq!(world.asks_of(PUBLIC, "続き").await, None);
    read("B");
    assert!(world.asks_of(PUBLIC, "続き").await.is_some());
}

#[tokio::test]
async fn outside_a_turn_everything_is_asked_about_and_nothing_in_confidence_is_read() {
    let world = World::new(console());
    world.store.end_turn().unwrap();
    assert!(world.asks(PUBLIC).await.is_some());
    assert!(world.gate().asks_outside("x", &Value::Null).await.is_some());
    let taints = vec![Taint::of_connection("gmail", "のやりとり")];
    assert!(note(&world.store, taints).is_err());
}

#[test]
fn tools_that_put_something_before_others_are_told_by_their_names() {
    let permissions = World::new(console()).permissions;
    for (tool, expected) in [
        ("add_issue_comment", true),
        ("createPullRequest", true),
        ("http_post", true),
        ("send_message", true),
        ("insert_event", false),
    ] {
        assert_eq!(reaches_others(&permissions, "x", tool), expected, "{tool}");
    }
}

/// Tells of repositories as it was told, and of no other.
struct Repositories(&'static [(&'static str, bool)]);

impl Looks for Repositories {
    fn repository<'a>(&'a self, full_name: &'a str) -> BoxFuture<'a, Option<bool>> {
        let found = self.0.iter().find(|(name, _)| *name == full_name);
        async move { found.map(|(_, public)| *public) }.boxed()
    }
}

const REPOSITORIES: Repositories = Repositories(&[("o/open", true), ("o/closed", false)]);

fn connection_of(id: &str, kind: &str, transport: crate::connections::Transport) -> Connection {
    Connection {
        id: id.into(),
        name: format!("{id} の名前"),
        kind: kind.into(),
        transport,
        secret: Default::default(),
        account: String::new(),
        status: String::new(),
    }
}

fn remote(id: &str, kind: &str) -> Connection {
    let transport = crate::connections::Transport::McpRemote {
        url: format!("https://{id}.example.com"),
    };
    connection_of(id, kind, transport)
}

fn defined(id: &str, definition: &str, values: &[(&str, &str)], public: bool) -> Connection {
    let transport = crate::connections::Transport::Defined {
        definition: definition.into(),
        values: values
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        public,
    };
    connection_of(id, definition, transport)
}

async fn answered(c: &Connection, tool: &str, arguments: Value, answer: &str) -> Vec<Taint> {
    let message =
        json!({ "method": "tools/call", "params": { "name": tool, "arguments": arguments } });
    let response = json!({ "result": { "content": [{ "type": "text", "text": answer }] } });
    of_connection(
        None::<&SlackClients<&FakeSlack>>,
        &REPOSITORIES,
        c,
        &message,
        &response,
    )
    .await
}

#[tokio::test]
async fn what_a_connection_nobody_told_of_answers_is_confidential_whatever_it_is() {
    let wiki = remote("wiki", "notes");
    for tool in ["get-note", "search-notes", "create-note", "brand_new"] {
        let taints = answered(&wiki, tool, json!({}), "{}").await;
        assert_eq!(taints.len(), 1, "{tool}");
        assert!(
            taints[0].label.contains("wiki の名前"),
            "{}",
            taints[0].label
        );
        assert!(taints[0].came_by("wiki"));
    }
    let api = connection_of(
        "api",
        "api",
        crate::connections::Transport::Api {
            base_url: "https://api.example.com".into(),
            openapi_url: String::new(),
        },
    );
    assert_eq!(answered(&api, "list_items", json!({}), "[]").await.len(), 1);
    let google = defined("g", "google", &[], false);
    for tool in ["list_events", "get_profile", "insert_event"] {
        assert_eq!(
            answered(&google, tool, json!({}), "{}").await.len(),
            1,
            "{tool}"
        );
    }
}

#[tokio::test]
async fn only_what_a_public_repository_holds_is_public() {
    let github = defined("gh", "github", &[], false);
    let of = |repo: &str| {
        let (owner, name) = repo.split_once('/').unwrap();
        json!({ "owner": owner, "repo": name })
    };
    for (tool, arguments, answer, expected) in [
        ("get_file_contents", of("o/open"), "{}", 0),
        ("issue_read", of("o/open"), "{}", 0),
        ("list_pull_requests", of("O/Open"), "{}", 0),
        ("get_file_contents", of("o/closed"), "{}", 1),
        ("get_file_contents", of("o/unknown"), "{}", 1),
        // A draft review is seen only by the one who writes it.
        (
            "pull_request_read",
            of("o/open"),
            r#"{"state":"PENDING"}"#,
            1,
        ),
        ("get_file_contents", json!({ "owner": "o" }), "{}", 1),
        ("search_code", json!({ "query": "repo:o/open x" }), "{}", 1),
        ("list_notifications", of("o/open"), "{}", 1),
        ("get_me", json!({}), "{}", 1),
        ("issue_write", of("o/open"), "{}", 1),
    ] {
        let taints = answered(&github, tool, arguments.clone(), answer).await;
        assert_eq!(taints.len(), expected, "{tool} {arguments}");
    }
    let lookalike = remote("gh2", "github");
    let taints = answered(&lookalike, "get_file_contents", of("o/open"), "{}").await;
    assert_eq!(taints.len(), 1, "a server only named so is not GitHub's");
}

#[tokio::test]
async fn a_feed_is_public_when_anyone_may_read_it_as_it_is_given() {
    for (url, public, expected) in [
        ("https://example.com/feed.xml", true, 0),
        ("https://example.com/feed.xml", false, 1),
        ("https://example.com/feed.xml?token=abc", true, 1),
        ("https://example.com/feed?private_key=abc", true, 1),
        ("https://me:pw@example.com/feed.xml", true, 1),
    ] {
        let feed = defined("f", "rss", &[("url", url)], public);
        let taints = answered(&feed, "get_feed", json!({}), "<rss/>").await;
        assert_eq!(taints.len(), expected, "{url} {public}");
    }
    let mut keyed = defined("f", "rss", &[("url", "https://example.com/f")], true);
    keyed.secret.bearer = Some("k".into());
    assert_eq!(answered(&keyed, "get_feed", json!({}), "").await.len(), 1);
    let posts = defined("b", "bluesky", &[], false);
    for tool in ["search_posts", "get_post_thread"] {
        assert!(
            answered(&posts, tool, json!({}), "{}").await.is_empty(),
            "{tool}"
        );
    }
}

#[tokio::test]
async fn what_was_said_in_public_stays_in_its_workspace() {
    let world = World::new(slack_in(PUBLIC));
    world.read_channel(PUBLIC).await;
    assert_eq!(world.asks(OTHER_PRIVATE).await, None);
    assert_eq!(world.asks(PUBLIC).await, None);
    let why = world.asks(SHARED).await.unwrap().why;
    assert!(why.contains("外部と共有"), "{why}");
    assert!(
        world.asks("C9").await.is_some(),
        "a place Slack tells nothing of"
    );
    assert!(
        world
            .gate()
            .asks_outside("gh", &Value::Null)
            .await
            .is_some()
    );
}

#[tokio::test]
async fn what_an_unknown_connection_gave_is_asked_about_wherever_others_read_it() {
    let world = World::new(console());
    let taints = answered(&remote("wiki", "notes"), "get-note", json!({}), "x").await;
    note(&world.store, taints).unwrap();
    let why = world.asks(PUBLIC).await.unwrap().why;
    assert!(why.contains("「wiki の名前」"), "{why}");
    assert!(world.asks(PRIVATE).await.is_some());
    assert_eq!(world.asks(OWNERS_DM).await, None);
    assert_eq!(world.gate().asks_outside("wiki", &Value::Null).await, None);
    assert!(
        world
            .gate()
            .asks_outside("gh", &Value::Null)
            .await
            .is_some()
    );
}

#[test]
fn files_outside_the_notebook_count_too_but_the_skills_do_not() {
    let memory = Path::new("/n/memory");
    let work = Path::new("/work");
    let reads = |tool: &str, input: Value| !of_builtin(memory, work, tool, &input).is_empty();
    for (tool, input, expected) in [
        ("Read", json!({ "file_path": "/elsewhere/repo/a.rs" }), true),
        (
            "Read",
            json!({ "file_path": "/work/attachments/a.pdf" }),
            true,
        ),
        ("Grep", json!({ "pattern": "x" }), true),
        (
            "Glob",
            json!({ "pattern": "**/*.rs", "path": "/elsewhere" }),
            true,
        ),
        ("Edit", json!({ "file_path": "/elsewhere/a.md" }), true),
        (
            "Read",
            json!({ "file_path": "/work/.claude/skills/a/SKILL.md" }),
            false,
        ),
        (
            "Read",
            json!({ "file_path": "/work/.claude/skills/../x.md" }),
            true,
        ),
        ("Skill", json!({ "skill": "a" }), false),
        ("TodoWrite", json!({}), false),
        ("Write", json!({ "file_path": "/elsewhere/a.md" }), false),
        ("WebSearch", json!({ "query": "x" }), false),
        ("ReadMcpResourceTool", json!({}), true),
        ("BrandNew", json!({}), true),
        ("mcp__famulen__get_record", json!({}), false),
    ] {
        assert_eq!(reads(tool, input.clone()), expected, "{tool} {input}");
    }
}

#[test]
fn a_page_is_public_only_where_anyone_on_the_internet_reads_it_with_no_key() {
    let memory = Path::new("/n/memory");
    let fetched = |url: &str| {
        let input = json!({ "url": url, "prompt": "x" });
        !of_builtin(memory, Path::new("/work"), "WebFetch", &input).is_empty()
    };
    for (url, expected) in [
        ("https://93.184.215.14/a", false),
        ("http://10.0.0.1/wiki", true),
        ("http://127.0.0.1:8080/", true),
        ("http://[::1]/", true),
        ("http://localhost/", true),
        ("http://intranet/page", true),
        ("https://wiki.corp/page", true),
        ("https://nowhere.invalid/page", true),
        ("https://u:p@93.184.215.14/", true),
        ("https://93.184.215.14/?access_token=x", true),
        ("file:///etc/passwd", true),
        ("not a url", true),
    ] {
        assert_eq!(fetched(url), expected, "{url}");
    }
}

fn link(world: &World, conversation: i64, input: i64) {
    world
        .store
        .conn()
        .execute(
            "INSERT INTO conversation_keys (key, conversation) VALUES (?1, ?2)",
            params![format!("input:{input}"), conversation],
        )
        .unwrap();
}

fn read_closed_repository(world: &World) {
    note(&world.store, vec![Taint::of_connection("gh", "の内容")]).unwrap();
}

#[tokio::test]
async fn what_a_conversation_read_is_asked_about_in_its_later_turns_and_nowhere_else() {
    let world = World::new(console());
    link(&world, 1, world.running());
    read_closed_repository(&world);
    let later = world.turn(console_at(10), "それを #general に");
    link(&world, 1, later);
    let why = world.asks(PUBLIC).await.unwrap().why;
    assert!(why.contains("「gh」"), "{why}");
    assert_eq!(world.asks(OWNERS_DM).await, None);
    let elsewhere = world.turn(console_at(11), "x");
    link(&world, 2, elsewhere);
    assert_eq!(world.asks(PUBLIC).await, None);
}

#[tokio::test]
async fn what_a_turn_was_given_stays_with_its_conversation() {
    let world = World::new(console());
    world.add_run(3, "T1", "{}");
    let done = world.turn(worker_ended(3), "x");
    link(&world, 1, done);
    world.gate().begun(done, &world.input(done)).await.unwrap();
    let later = world.turn(console_at(12), "まとめて #general に");
    link(&world, 1, later);
    assert!(world.asks(PUBLIC).await.is_some());
}

#[tokio::test]
async fn what_was_handed_on_from_other_conversations_is_marked_too() {
    let world = World::new(console());
    link(&world, 1, world.running());
    read_closed_repository(&world);
    let other = world.turn(console_at(13), "x");
    link(&world, 2, other);
    carry(&world.store, &[1], &[], 2).unwrap();
    assert!(world.asks(PUBLIC).await.is_some());
}

#[tokio::test]
async fn what_a_worker_did_is_confidential_whatever_it_was_handed() {
    let world = World::new(console());
    world.add_run(3, "T1", "{}");
    world.turn(worker_ended(3), "x");
    let why = world.asks(PUBLIC).await.unwrap().why;
    assert!(why.contains("手代"), "{why}");
}

#[tokio::test]
async fn a_routine_made_in_a_turn_carries_what_it_read_to_every_run() {
    let world = World::new(console());
    read_closed_repository(&world);
    world
        .gate()
        .hand_on("save_record", r#"{"kind":"routines","id":"R1"}"#)
        .await
        .unwrap();
    world.turn(scheduled("R1"), "x");
    assert!(world.asks(PUBLIC).await.is_some());
    carry_routine(&world.store, "R1", "T9").unwrap();
    let held = taint::held_by(&world.store, &Holder::Task("T9".into())).unwrap();
    assert!(!held.is_empty());
    world.turn(scheduled("R2"), "x");
    assert_eq!(world.asks(PUBLIC).await, None);
}

#[tokio::test]
async fn what_was_written_into_a_record_is_read_again_with_it() {
    let world = World::new(console());
    read_closed_repository(&world);
    let saved = r#"{"kind":"memos","id":"01memo00000000000000000001","title":"t"}"#;
    world.gate().hand_on("save_record", saved).await.unwrap();
    world.turn(console_at(20), "x");
    assert!(world.gate().of_marked_in(saved).unwrap().len() == 1);
    let listed =
        json!({ "records": [{ "kind": "memos", "id": "01memo00000000000000000001" }] }).to_string();
    assert_eq!(world.gate().of_marked_in(&listed).unwrap().len(), 1);
    let found =
        json!({ "hits": [{ "scope": "records/memos", "id": "01memo00000000000000000001" }] })
            .to_string();
    assert_eq!(world.gate().of_marked_in(&found).unwrap().len(), 1);
    let other = json!({ "kind": "memos", "id": "01memo00000000000000000002" }).to_string();
    assert!(world.gate().of_marked_in(&other).unwrap().is_empty());
    assert_eq!(world.asks(PUBLIC).await, None);
}

fn worker_ended(run: i64) -> Source {
    Source::Worker {
        run,
        task: "task-aaaaaa".into(),
        outcome: "done".into(),
        workgroup: "all".into(),
        again: 0,
    }
}

fn scheduled(routine: &str) -> Source {
    Source::Schedule {
        name: "毎朝".into(),
        at: routine.into(),
        routine: routine.into(),
    }
}

#[tokio::test]
async fn what_a_server_no_connection_is_known_for_answers_is_confidential() {
    let world = World::new(console());
    let answer = async |method: &str| {
        let message = json!({ "method": method, "params": { "name": "get", "arguments": {} } });
        let response = json!({ "result": { "content": [{ "type": "text", "text": "x" }] } });
        let clients = None::<&SlackClients<&FakeSlack>>;
        of_answer(clients, &world.permissions, "ghost", &message, &response).await
    };
    assert_eq!(answer("tools/call").await.len(), 1);
    assert!(answer("tools/list").await.is_empty());
}

#[tokio::test]
async fn a_private_repository_is_not_told_through_the_same_connection_to_another() {
    let world = World::new(console());
    let github = defined("gh", "github", &[], false);
    let read = |repo: &str| json!({ "owner": "o", "repo": repo });
    let taints = answered(&github, "get_file_contents", read("closed"), "{}").await;
    note(&world.store, taints).unwrap();
    let gate = world.gate();
    assert_eq!(gate.asks_outside("gh", &read("closed")).await, None);
    assert!(gate.asks_outside("gh", &read("open")).await.is_some());
    assert!(gate.asks_outside("gh", &json!({})).await.is_some());
    let found = answered(&github, "search_code", json!({ "query": "x" }), "{}").await;
    note(&world.store, found).unwrap();
    assert!(gate.asks_outside("gh", &read("closed")).await.is_some());
}

#[tokio::test]
async fn a_draft_a_public_repository_keeps_is_not_public() {
    let github = defined("gh", "github", &[], false);
    let open = json!({ "owner": "o", "repo": "open" });
    for answer in [r#"[{"draft":true}]"#, r#"[{"draft": true}]"#] {
        let taints = answered(&github, "list_releases", open.clone(), answer).await;
        assert_eq!(taints.len(), 1, "{answer}");
    }
    assert!(
        answered(&github, "list_releases", open, r#"[{"draft":false}]"#)
            .await
            .is_empty()
    );
}

#[test]
fn tools_that_send_or_push_reach_others_too() {
    let permissions = World::new(console()).permissions;
    for tool in [
        "send_message",
        "push_files",
        "create_or_update_file",
        "send_mail",
        "invite_user",
        "merge_pull_request",
        "upload_file",
    ] {
        assert!(reaches_others(&permissions, "x", tool), "{tool}");
    }
}

#[test]
fn a_link_that_holds_its_own_key_is_not_public() {
    let memory = Path::new("/n/memory");
    let fetched = |url: &str| {
        let input = json!({ "url": url, "prompt": "x" });
        !of_builtin(memory, Path::new("/work"), "WebFetch", &input).is_empty()
    };
    for url in [
        "https://docs.google.com/document/d/abc/edit",
        "https://drive.google.com/file/d/abc",
        "https://gist.github.com/someone/0123abcd",
        "https://files.slack.com/files-pri/T1-F1/a.pdf",
        "https://www.dropbox.com/s/abc/a.pdf",
        "https://x.notion.site/page",
    ] {
        assert!(fetched(url), "{url}");
    }
}

#[test]
fn a_pattern_that_climbs_out_of_the_skills_counts() {
    let input = json!({ "pattern": "*/../../x*", "path": "/work/.claude/skills" });
    assert!(!of_builtin(Path::new("/n/memory"), Path::new("/work"), "Glob", &input).is_empty());
}

#[test]
fn an_attached_file_is_of_the_place_it_was_attached_in_and_no_other_is() {
    let kept = |path: &str| {
        crate::attachments::Attached::Kept(crate::attachments::Attachment {
            id: "1".into(),
            name: "a.pdf".into(),
            mime: "application/pdf".into(),
            size: 1,
            path: path.into(),
        })
    };
    let input = Input {
        sender: Sender::Owner,
        source: Source::Slack {
            workspace: "w1".into(),
            channel: PRIVATE.into(),
            ts: "1.0".into(),
            thread_ts: None,
            user: "U1".into(),
            unmentioned: false,
            others_before: false,
            attached: vec![kept("/work/attachments/1/a.pdf")],
        },
        text: String::new(),
    };
    let read = |path: &str| {
        let called = json!({ "file_path": path });
        let taints = of_builtin(Path::new("/n/memory"), Path::new("/work"), "Read", &called);
        attached_in(taints, &input, Path::new("/work"), &called)
    };
    let here = read("/work/attachments/1/a.pdf");
    assert!(here[0].stays_in("w1", PRIVATE, &room("secret", true)));
    assert!(!here[0].stays_in("w1", OTHER_PRIVATE, &room("other", true)));
    for other in ["/work/attachments/2/b.pdf", "/work/elsewhere.md"] {
        let taints = read(other);
        assert!(
            !taints[0].stays_in("w1", PRIVATE, &room("secret", true)),
            "{other}"
        );
    }
}

#[tokio::test]
async fn only_what_anyone_sees_on_bluesky_without_logging_in_is_public() {
    let posts = defined("b", "bluesky", &[], false);
    for (tool, answer, expected) in [
        ("search_posts", "{}", 0),
        ("get_post_thread", "{}", 0),
        (
            "get_post_thread",
            r#"{"labels":[{"val":"!no-unauthenticated"}]}"#,
            1,
        ),
        ("get_timeline", "{}", 1),
        ("list_notifications", "{}", 1),
    ] {
        assert_eq!(
            answered(&posts, tool, json!({}), answer).await.len(),
            expected,
            "{tool} {answer}"
        );
    }
}

#[test]
fn a_grep_pattern_is_not_a_path() {
    let dir = TempDir::new("disclosure-grep");
    let memory = dir.path().join("memory");
    std::fs::create_dir_all(dir.path().join("records/tasks")).unwrap();
    let input = json!({ "pattern": "a..b", "path": dir.path().join("records/tasks").display().to_string() });
    assert!(of_builtin(&memory, Path::new("/work"), "Grep", &input).is_empty());
}

#[tokio::test]
async fn what_was_written_into_the_notebook_is_read_again_however_it_is_read() {
    let world = World::new(console());
    read_closed_repository(&world);
    let gate = world.gate();
    let routine = r#"{"kind":"routines","id":"01routine0000000000000000a"}"#;
    gate.hand_on("save_record", routine).await.unwrap();
    let remembered = r#"{"id":"mremember01","text":"x"}"#;
    gate.hand_on("remember", remembered).await.unwrap();
    world.turn(console_at(30), "x");
    for text in [
        "records/routines/01routine0000000000000000a.md",
        r#"{"kind":"routines","id":"01routine0000000000000000a"}"#,
        "- 速い {id: mremember01}",
    ] {
        assert!(!gate.of_marked_in(text).unwrap().is_empty(), "{text}");
    }
    assert!(gate.of_marked_in("{id: m1}").unwrap().is_empty());
    assert!(!gate.of_all_marked().unwrap().is_empty());
}

#[test]
fn a_link_whose_path_looks_like_a_key_is_not_public() {
    for (url, expected) in [
        ("https://example.org/blog/2026/11/hello-world", true),
        ("https://example.org/docs/getting-started", true),
        ("https://app.box.com/s/abc", false),
        ("https://www.notion.so/page", false),
        ("https://1drv.ms/x", false),
        ("https://example.org/share/k3J9xQ2mZ7pL4vR8tY1w", false),
        ("https://example.org/d/0123456789abcdef0123", false),
    ] {
        assert_eq!(web::opens_to_anyone(url), expected, "{url}");
    }
}

#[test]
fn a_tool_that_reads_does_not_reach_others_by_its_other_words() {
    let permissions = World::new(console()).permissions;
    for tool in [
        "get_file_contents",
        "read_mail",
        "list_messages",
        "search_files",
    ] {
        assert!(!reaches_others(&permissions, "x", tool), "{tool}");
    }
}

#[tokio::test]
async fn a_task_a_routine_made_is_read_again_with_what_the_routine_was_made_with() {
    let world = World::new(console());
    read_closed_repository(&world);
    world
        .gate()
        .hand_on("save_record", r#"{"kind":"routines","id":"R1"}"#)
        .await
        .unwrap();
    carry_routine(&world.store, "R1", "01task000000000000000000009").unwrap();
    let read = world
        .gate()
        .of_marked_in("01task000000000000000000009")
        .unwrap();
    assert!(!read.is_empty());
}

#[tokio::test]
async fn a_turn_whose_marks_cannot_be_told_is_marked_as_holding_something() {
    let world = World::new(console());
    let id = world.running();
    link(&world, 1, id);
    world
        .store
        .conn()
        .execute(
            "INSERT INTO taints (holder, taint) VALUES ('conversation:1', 'broken')",
            [],
        )
        .unwrap();
    assert!(world.gate().begun(id, &world.input(id)).await.is_err());
    world.gate().begun_or_held(id, &world.input(id)).await;
    let rows: i64 = world
        .store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM taints WHERE holder = 'conversation:1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 2, "the fallback mark is kept beside the broken one");
}
