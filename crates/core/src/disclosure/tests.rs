use std::collections::BTreeMap;

use futures_util::FutureExt;
use serde_json::json;

use super::sources::is_slack;
use super::*;
use crate::approvals::Answer;
use crate::connections::Connection;
use crate::notebook::testing::TempDir;
use crate::slack::fake::FakeSlack;

const PUBLIC: &str = "C1";
const PRIVATE: &str = "G1";
const OTHER_PRIVATE: &str = "G2";
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

    fn turn(&self, source: Source, text: &str) {
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
        if room.private != Some(false) {
            note(&self.store, vec![Taint::room("w1", channel, &room)]).unwrap();
        }
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
    Source::Console {
        message: 1,
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
    assert_eq!(world.gate().of_runs(&listed).unwrap().len(), 1);
    assert!(world.gate().of_runs("done").unwrap().is_empty());
    let done = Source::Worker {
        run: 3,
        task: "task-aaaaaa".into(),
        outcome: "done".into(),
        workgroup: "all".into(),
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
    assert!(!reads("Read", json!({ "file_path": "/work/a.md" })));
    assert!(!reads("WebFetch", json!({ "url": "https://example.com" })));
}

#[tokio::test]
async fn what_slack_and_mail_answer_is_told_apart_by_who_can_read_it() {
    let world = World::new(console());
    let connection = |id: &str, kind: &str| {
        let transport = crate::connections::Transport::McpRemote {
            url: format!("https://{id}.example.com"),
        };
        Connection {
            id: id.into(),
            name: id.into(),
            kind: kind.into(),
            transport,
            secret: Default::default(),
            account: String::new(),
            status: String::new(),
        }
    };
    let slack = connection("w1", "slack");
    assert!(is_slack(&slack));
    let read = async |c: &Connection, tool: &str, arguments: Value, answer: Value| {
        let message =
            json!({ "method": "tools/call", "params": { "name": tool, "arguments": arguments } });
        let response =
            json!({ "result": { "content": [{ "type": "text", "text": answer.to_string() }] } });
        of_connection(world.approvals.slack(), c, &message, &response).await
    };
    for (channel, expected) in [(PUBLIC, 0), (PRIVATE, 1), ("D9", 1)] {
        let taints = read(
            &slack,
            "read_channel",
            json!({ "channel": channel }),
            json!({}),
        )
        .await;
        assert_eq!(taints.len(), expected, "{channel}");
    }
    let found = json!({ "messages": { "matches": [
        { "channel": { "id": "C1", "name": "general", "is_private": false } },
        { "channel": { "id": "G1", "name": "secret", "is_private": true } },
        { "channel": { "id": "D1", "name": "U2", "is_im": true, "is_private": false } },
    ] } });
    let taints = read(&slack, "search_messages", json!({}), found).await;
    let places: Vec<&str> = taints.iter().map(|t| t.place.as_str()).collect();
    assert_eq!(places, ["G1", "D1"]);
    let strange = read(&slack, "search_messages", json!({}), json!("?")).await;
    assert_eq!(strange.len(), 1);
    assert!(
        read(&slack, "list_channels", json!({}), json!({}))
            .await
            .is_empty()
    );

    let google = connection("gmail", "google");
    for (tool, expected) in [
        ("get_message", 1),
        ("search_messages", 1),
        ("list_events", 0),
    ] {
        assert_eq!(
            read(&google, tool, json!({}), json!({})).await.len(),
            expected,
            "{tool}"
        );
    }
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
