//! Delegated tasks run by sessions in the owner's Adroit, reached through
//! the gateway as everything outside is: every start is judged, and held for
//! the owner, as their permissions say.

use serde_json::{Value, json};

use super::WORKER;
use super::order::Order;
use super::runners::{Began, Looked};
use super::runs::{Run, State};
use super::sight::Sight;
use crate::app::App;
use crate::approvals::Answer;
use crate::connections::{Connection, Transport};
use crate::gateway::{Call, OWNER, Outcome, Ticket};
use crate::permissions::SECRETARY;

/// The bundled definition of Adroit.
pub const ADROIT: &str = "adroit";
/// Its tool that starts a session.
pub const START: &str = "start_session";
/// Its tools that start a session, whether anew or from an ended one. Neither
/// the secretary nor a worker uses them.
pub const LAUNCH: &[&str] = &[START, "resume_session"];
/// Its tools that answer what a session waits on the owner for.
const PUT_QUESTION: &str = "put_question";
const ANSWER_PERMISSION: &str = "answer_permission";
/// How much of a session's answer is kept.
const RESULT_CHARS: usize = 6000;
/// How long a session just started may look as if it had ended its turn
/// before it took up its first prompt.
const TAKES_UP_SECS: i64 = 180;
/// How many turns are read for whoever watches a session.
const WATCHED_TURNS: u32 = 20;

pub fn is_adroit(c: &Connection) -> bool {
    matches!(&c.transport, Transport::Defined { definition, .. } if definition == ADROIT)
}

/// The Adroit connection an order names, or the only one there is.
fn resolve(app: &App, order: &Order) -> anyhow::Result<String> {
    let mut all = app.connections.list()?.into_iter().filter(is_adroit);
    if !order.connection.is_empty() {
        return all
            .find(|c| c.id == order.connection || c.name == order.connection)
            .map(|c| c.id)
            .ok_or_else(|| anyhow::anyhow!("no Adroit connection {}", order.connection));
    }
    let only = all
        .next()
        .ok_or_else(|| anyhow::anyhow!("Adroit is not connected"))?;
    anyhow::ensure!(
        all.next().is_none(),
        "several Adroits are connected; its Delegation names no Connection"
    );
    Ok(only.id)
}

fn connection_kept(app: &App, run: &Run) -> anyhow::Result<String> {
    if !run.connection.is_empty() {
        return Ok(run.connection.clone());
    }
    Ok(app
        .workers
        .runs
        .get(run.id)?
        .map(|r| r.connection)
        .unwrap_or_default())
}

/// The connection a run is started through: worked out once, so that what
/// is connected or renamed later leads it nowhere else.
fn connection(app: &App, run: &Run) -> anyhow::Result<String> {
    let kept = connection_kept(app, run)?;
    if !kept.is_empty() {
        return Ok(kept);
    }
    let connection = resolve(app, &run.order)?;
    app.workers.runs.through(run.id, &connection)?;
    Ok(connection)
}

/// What names a run's session, and no other's: it stands at the head of the
/// session's name, so a start whose answer was lost is found again.
fn token(run: &Run) -> String {
    format!("{}-run{}", run.order.code, run.id)
}

/// The call that starts a run's session. The same run makes the same call, so
/// what the owner approved is what is started, and no other run may use it.
fn start_call(app: &App, run: &Run) -> anyhow::Result<Call> {
    let order = &run.order;
    let mut input = json!({
        "dir": order.directory,
        "prompt": super::finish::prompt(order),
        "name": format!("{} {}", token(run), order.title.replace('\n', " ")),
    });
    if order.worktree {
        input["worktree"] = json!(true);
    }
    for (name, value) in [
        ("base", &order.base),
        ("agent", &order.agent),
        ("model", &order.model),
        ("permissions", &order.permissions),
    ] {
        if !value.is_empty() {
            input[name] = json!(value);
        }
    }
    let connection = connection(app, run)?;
    Ok(Call {
        // A connection outside the task's workgroup is asked about, as it is
        // when the secretary calls it.
        ask: app.permissions.ask_outside(&connection, &order.scopes)?,
        ..Call::new(WORKER, connection, START, input)
    })
}

pub(super) async fn begin(app: &App, run: &Run) -> anyhow::Result<Began> {
    match app.gateway.call(&start_call(app, run)?).await? {
        Outcome::Held(ticket) => Ok(Began::Asking(ticket.id)),
        Outcome::Done(answer) if answer.is_error => anyhow::bail!("{}", answer.text),
        Outcome::Done(answer) => {
            let started: Value = serde_json::from_str(&answer.text)?;
            let id = started["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("the session's id was not told"))?;
            Ok(Began::Running(id.to_owned()))
        }
    }
}

/// Whether a start whose answer was lost went through, or is asked already.
/// An error says that it cannot be told.
pub(super) async fn find(app: &App, run: &Run) -> anyhow::Result<Option<Began>> {
    // It is kept before the first call is made through it.
    if connection_kept(app, run)?.is_empty() {
        return Ok(None);
    }
    let call = start_call(app, run)?;
    if app.connections.get(&call.connection)?.is_none() {
        return Ok(None);
    }
    let sessions = read(
        app,
        &call.connection,
        "list_sessions",
        json!({ "ended": true }),
    )
    .await?;
    let token = token(run);
    let found = sessions
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| {
            s["name"]
                .as_str()
                .and_then(|n| n.strip_prefix(&token))
                .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_digit()))
        })
        .and_then(|s| s["id"].as_str());
    if let Some(id) = found {
        return Ok(Some(Began::Running(id.to_owned())));
    }
    Ok(app
        .gateway
        .ticket_for(&call)?
        .filter(|t| was_asked(&t.status))
        .map(|t| Began::Asking(t.id)))
}

/// Whether a request was put to the owner and not granted: one refused or
/// left unanswered is not asked anew.
fn was_asked(status: &str) -> bool {
    matches!(status, "held" | "rejected" | "lapsed")
}

/// How a run's session stands. An error says that Adroit cannot be asked; a
/// session that is gone is no error.
pub(super) async fn look(app: &App, run: &Run) -> anyhow::Result<Looked> {
    let connection = connection(app, run)?;
    if app.connections.get(&connection)?.is_none() {
        return Ok(Looked::Failed(format!(
            "its Adroit connection {connection} was removed; the session may go on there"
        )));
    }
    // Those held before it is read: one held since may be about what it did
    // not ask then.
    let held = held_of(app, &connection, run)?;
    let session = read(
        app,
        &connection,
        "read_session",
        json!({ "session": run.session, "turns": 1 }),
    )
    .await;
    match session {
        Ok(session) => {
            take_in(app, run, &session, &held)?;
            let age = crate::store::now() - run.started_at.unwrap_or_default();
            Ok(read_standing(&session, age))
        }
        Err(error) => {
            let all = read(app, &connection, "list_sessions", json!({ "ended": true })).await?;
            let known = all
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("the sessions could not be read"))?
                .iter()
                .any(|s| s["id"] == run.session.as_str());
            anyhow::ensure!(!known, "{error:#}");
            withdraw(app, &held)?;
            Ok(Looked::Failed(format!(
                "Adroit knows no session {} any more",
                run.session
            )))
        }
    }
}

/// Read what a run's session wrote, for whoever watches it.
pub(super) async fn watch(app: &App, run: &Run) -> anyhow::Result<()> {
    let connection = connection_kept(app, run)?;
    let held = held_of(app, &connection, run)?;
    let session = read(
        app,
        &connection,
        "read_session",
        json!({ "session": run.session, "turns": WATCHED_TURNS }),
    )
    .await?;
    take_in(app, run, &session, &held)
}

/// Take in what was read of a run's session, and take back the requests held
/// before it was read about what it asks no more.
fn take_in(app: &App, run: &Run, session: &Value, held: &[Ticket]) -> anyhow::Result<()> {
    app.workers.sights.read(run.id, session);
    let asked = &session["question"]["id"];
    withdraw(
        app,
        held.iter().filter(|t| t.shown["decides"]["id"] != *asked),
    )
}

fn withdraw<'a>(app: &App, held: impl IntoIterator<Item = &'a Ticket>) -> anyhow::Result<()> {
    let mut over = Vec::new();
    for ticket in held {
        if app.gateway.withdraw_ticket(ticket.id)? {
            over.push(ticket.id);
        }
    }
    if !over.is_empty() {
        crate::approvals::withdraw_apart(app, over);
    }
    Ok(())
}

/// The requests that put to the owner what a run's session asks. A caller may
/// name a session otherwise than the run does, so what it was last seen to ask
/// tells them too.
fn held_of(app: &App, connection: &str, run: &Run) -> anyhow::Result<Vec<Ticket>> {
    let seen = app.workers.sights.of(run.id).question.unwrap_or_default();
    let mut held = app.gateway.held_tickets()?;
    held.retain(|t| {
        t.connection == connection
            && t.shown["decides"]["id"].is_string()
            && (t.shown["call"]["session"] == run.session.as_str()
                || t.shown["decides"]["id"] == seen["id"])
    });
    Ok(held)
}

/// What the session of a run asks the owner.
#[derive(Debug, Clone)]
pub struct Question {
    pub id: String,
    /// The tool that answers it, as requests name theirs.
    pub tool: String,
    /// What a request of it shows.
    pub shown: Value,
    /// It is not told whole or cannot be read, so it is answered in Adroit alone.
    pub truncated: bool,
    /// The request that puts it to the owner already, or 0.
    pub ticket: i64,
    call: Call,
}

/// What a run's session asks the owner, when it can be answered from here.
pub fn question(app: &App, run: &Run, sight: &Sight) -> anyhow::Result<Option<Question>> {
    let Some(asked) = sight
        .question
        .as_ref()
        .filter(|_| run.state == State::Waiting)
    else {
        return Ok(None);
    };
    let tool = match asked["kind"].as_str() {
        Some("question") => PUT_QUESTION,
        Some("permission" | "plan") => ANSWER_PERMISSION,
        _ => return Ok(None),
    };
    let id = asked["id"].as_str().unwrap_or_default().to_owned();
    let connection = connection_kept(app, run)?;
    let call = Call::new(
        SECRETARY,
        &connection,
        tool,
        json!({
            "session": run.session,
            "question": id,
            "summary": format!("「{}」（{}）のセッションが答えを待っています。", run.order.title, run.order.code),
        }),
    );
    let shown = json!({ "decides": asked, "call": call.input });
    let unreadable = tool == PUT_QUESTION
        && !app
            .gateway
            .questions_of(&connection, tool, &shown)
            .is_ok_and(|questions| !questions.is_empty());
    Ok(Some(Question {
        tool: crate::wording::tool_of(&connection, tool),
        shown,
        truncated: unreadable || asked["truncated"] == true,
        ticket: app
            .gateway
            .held_decided(&connection, tool, &id)?
            .map(|t| t.id)
            .unwrap_or_default(),
        id,
        call,
    }))
}

/// The run and what it asks, which must be `id` still.
fn asked(app: &App, run: i64, id: &str) -> anyhow::Result<(Run, Question)> {
    let run = app
        .workers
        .runs
        .get(run)?
        .ok_or_else(|| anyhow::anyhow!("no run #{run}"))?;
    let question = question(app, &run, &app.workers.sights.of(run.id))?
        .filter(|q| q.id == id)
        .ok_or_else(|| anyhow::anyhow!("この問いは、もう待たれていません"))?;
    anyhow::ensure!(
        !question.truncated,
        "長くて全部は読めないので、Adroit で答えてください"
    );
    Ok((run, question))
}

/// Give a run's session the owner's answer to what it asks, and take back a
/// request that puts it to them, so that no second answer comes.
pub async fn answer(
    app: &App,
    run: i64,
    id: &str,
    answer: Answer,
    replies: Vec<String>,
) -> anyhow::Result<()> {
    let (run, question) = asked(app, run, id)?;
    anyhow::ensure!(
        answer != Answer::AlwaysAllow,
        "the owner is asked of every one"
    );
    anyhow::ensure!(
        question.call.tool != PUT_QUESTION || answer == Answer::Approve && !replies.is_empty(),
        "a question is answered in words"
    );
    anyhow::ensure!(
        replies.iter().all(|r| !r.trim().is_empty()),
        "a question is not answered"
    );
    let call = Call {
        actor: OWNER.to_owned(),
        ..question.call.clone()
    };
    // Not through a request held already: how its call went would reach the
    // secretary, not the owner who answers here. Taken back before, so that
    // it is not answered meanwhile.
    let held = app.gateway.withdraw_decided(&call.connection, id)?;
    let asked = !held.is_empty();
    crate::approvals::withdraw_apart(app, held);
    if let Err(error) = app
        .gateway
        .answer_unasked(&call, answer.into(), replies)
        .await
    {
        if asked && let Err(error) = hold(app, &question.call).await {
            tracing::warn!(%error, run = run.id, "could not put a question to the owner again");
        }
        return Err(error);
    }
    // It is answered, whatever becomes of the rest: the next look sees it.
    app.workers.sights.answered(run.id, id);
    let seen = app
        .workers
        .runs
        .answered(run.id)
        .and_then(|()| app.workers.heard_of(&run.session));
    if let Err(error) = seen {
        tracing::warn!(%error, run = run.id, "could not keep that a session was answered");
    }
    Ok(())
}

/// Put what a run's session asks to the owner anew, in place of any request
/// that does already.
pub async fn ask_again(app: &App, run: i64, id: &str) -> anyhow::Result<()> {
    let (_, question) = asked(app, run, id)?;
    let asked = hold(app, &question.call).await?;
    if asked == question.ticket {
        // Only now that it is known to be asked still and let be asked.
        let before = app
            .gateway
            .withdraw_decided(&question.call.connection, id)?;
        crate::approvals::withdraw_apart(app, before);
        hold(app, &question.call).await?;
    }
    Ok(())
}

async fn hold(app: &App, call: &Call) -> anyhow::Result<i64> {
    match app.gateway.call(call).await? {
        Outcome::Held(ticket) => Ok(ticket.id),
        Outcome::Done(answer) => anyhow::bail!("{}", answer.text),
    }
}

/// How a session stands, `age` seconds after it was started.
fn read_standing(session: &Value, age: i64) -> Looked {
    let reply = session["turns"]
        .as_array()
        .and_then(|t| t.last())
        .and_then(|t| t["reply"].as_str());
    let delivered = session["undelivered"].as_u64().unwrap_or(0) == 0;
    let answered = reply.is_some() && delivered;
    let result: String = reply
        .unwrap_or_default()
        .chars()
        .take(RESULT_CHARS)
        .collect();
    match session["state"].as_str().unwrap_or_default() {
        "waiting" => Looked::Waiting,
        "done" | "idle" if answered || delivered && age > TAKES_UP_SECS => Looked::Done(result),
        "failed" => Looked::Failed(format!(
            "the session's turn failed: {}",
            session["message"].as_str().unwrap_or(&result)
        )),
        "ended" if answered => Looked::Done(result),
        "ended" => Looked::Failed("the session was ended before it answered".into()),
        _ => Looked::Working,
    }
}

/// Read something of Adroit. The owner is never asked of it: a read they
/// keep to themselves fails instead.
async fn read(app: &App, connection: &str, tool: &str, input: Value) -> anyhow::Result<Value> {
    let call = Call {
        never_ask: true,
        ..Call::new(WORKER, connection, tool, input)
    };
    match app.gateway.call(&call).await? {
        Outcome::Done(answer) if !answer.is_error => Ok(serde_json::from_str(&answer.text)?),
        Outcome::Done(answer) => anyhow::bail!("{tool}: {}", answer.text),
        Outcome::Held(_) => anyhow::bail!("{tool} was held"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_left_unanswered_is_found_again_and_one_granted_is_not() {
        for (status, asked) in [
            ("held", true),
            ("rejected", true),
            ("lapsed", true),
            ("approved", false),
            ("always_allowed", false),
        ] {
            assert_eq!(was_asked(status), asked, "{status}");
        }
    }

    #[test]
    fn a_session_is_done_once_its_turn_ended_with_an_answer() {
        let session = |state: &str, reply: Option<&str>| json!({ "state": state, "turns": [{ "prompt": "p", "reply": reply }], "undelivered": 0 });
        for (state, reply, age, expected) in [
            ("starting", None, 1, Looked::Working),
            ("working", None, 9999, Looked::Working),
            ("background", Some("so far"), 9999, Looked::Working),
            ("waiting", None, 1, Looked::Waiting),
            ("idle", None, 1, Looked::Working),
            ("idle", None, TAKES_UP_SECS + 1, Looked::Done(String::new())),
            ("idle", Some("ok"), 1, Looked::Done("ok".into())),
            ("done", Some("ok"), 1, Looked::Done("ok".into())),
            ("ended", Some("ok"), 1, Looked::Done("ok".into())),
        ] {
            assert_eq!(
                read_standing(&session(state, reply), age),
                expected,
                "{state} {reply:?} {age}"
            );
        }
        let mut waits = session("idle", None);
        waits["undelivered"] = json!(1);
        let long = TAKES_UP_SECS + 1;
        assert_eq!(read_standing(&waits, long), Looked::Working, "not read yet");
        for state in ["failed", "ended"] {
            assert!(matches!(
                read_standing(&session(state, None), 1),
                Looked::Failed(_)
            ));
        }
    }
}
