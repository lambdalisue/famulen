//! Delegated tasks run by sessions in the owner's Adroit, reached through
//! the gateway as everything outside is: every start is judged, and held for
//! the owner, as their permissions say.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Duration;

use futures_util::future::join_all;
use serde_json::{Value, json};

use super::WORKER;
use super::order::Order;
use super::runners::{Began, Looked};
use super::runs::{Outcome as Ended, Run, State};
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
/// Its tool that asks a session to go on.
const SEND: &str = "send_to_session";
const ANSWER_PERMISSION: &str = "answer_permission";
/// How much of a session's answer is kept.
const RESULT_CHARS: usize = 6000;
/// How long a session just started may look as if it had ended its turn
/// before it took up its first prompt.
const TAKES_UP_SECS: i64 = 180;
/// How many turns are read for whoever watches a session, or to tell whether
/// an ended run's session went on: the most Adroit tells.
const WATCHED_TURNS: u32 = 20;
/// How long one answer is waited for that tells whether ended runs went on.
const LOOKS_BACK_FOR: Duration = Duration::from_secs(10);
/// What a session is asked after a passing fault.
const GO_ON: &str = "Your last turn was cut off by a passing fault of the model's service. \
Go on with your work from where it stopped, and check what was done before you do it again. \
If it was finished already, say so and report on it.";
/// How a session was when it was read: its state, and when it was last active.
pub(super) type Trace = (String, i64);

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
pub(super) fn token(run: &Run) -> String {
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

/// The session of a run, and every session that goes on in the Adroit it was
/// started in. Those that ended are not listed: they grow without end, and a
/// long answer is cut.
pub(super) async fn sessions(app: &App, run: &Run) -> anyhow::Result<(Value, Value)> {
    let kept = connection_kept(app, run)?;
    let connection = if kept.is_empty() {
        resolve(app, &run.order)?
    } else {
        kept
    };
    let own = json!({ "session": run.session, "turns": 1 });
    let own = read(app, &connection, "read_session", own).await?;
    let going_on = read(app, &connection, "list_sessions", json!({})).await?;
    Ok((own, going_on))
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
            let now = crate::store::now();
            let age = now - run.started_at.unwrap_or_default();
            let asked = app.workers.runs.again(run.id)?.continued_at;
            if taking_up(&session, asked, now) {
                return Ok(Looked::Working);
            }
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

/// Those of these ended runs whose sessions went on since, by their ids. One
/// that cannot be read is left for the next time, and an Adroit that does not
/// answer holds up no other for long.
pub(super) async fn went_on(
    app: &App,
    ended: &[(Run, bool)],
    read_before: &Mutex<HashMap<i64, Trace>>,
) -> Vec<i64> {
    let mut of: BTreeMap<String, Vec<&(Run, bool)>> = BTreeMap::new();
    for one in ended {
        match connection_kept(app, &one.0) {
            Ok(connection) if !connection.is_empty() => of.entry(connection).or_default().push(one),
            Ok(_) => {}
            Err(error) => tracing::warn!(run = one.0.id, %error, "could not tell where a run was"),
        }
    }
    join_all(
        of.iter()
            .map(|(connection, runs)| went_on_in(app, connection, runs, read_before)),
    )
    .await
    .concat()
}

async fn went_on_in(
    app: &App,
    connection: &str,
    runs: &[&(Run, bool)],
    read_before: &Mutex<HashMap<i64, Trace>>,
) -> Vec<i64> {
    let mut went = Vec::new();
    if !matches!(app.connections.get(connection), Ok(Some(_))) {
        return went;
    }
    let ask = async |tool, input| {
        tokio::time::timeout(LOOKS_BACK_FOR, read(app, connection, tool, input))
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("{tool} did not answer")))
    };
    let listed = match ask("list_sessions", json!({})).await {
        Ok(listed) => listed,
        Err(error) => {
            tracing::debug!(%error, "could not list the sessions of ended runs");
            // One that is closed is not listed: that it was told of is all
            // that leads to it.
            for (run, _) in runs.iter().filter(|(_, heard)| *heard) {
                app.workers.hear_again(&run.session);
            }
            return went;
        }
    };
    for (run, heard) in runs {
        let trace = traced(&listed, &run.session);
        let before = read_before
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&run.id)
            .cloned();
        let may_have = trace.as_ref().is_some_and(|t| {
            before.as_ref() != Some(t)
                && (t.1 / 1000 >= run.ended_at.unwrap_or_default() || works(&t.0))
        });
        if !heard && !may_have {
            continue;
        }
        let input = json!({ "session": run.session, "turns": WATCHED_TURNS });
        match ask("read_session", input).await {
            Ok(session) if has_gone_on(&session, run) => went.push(run.id),
            Ok(_) => {
                if let Some(trace) = trace {
                    read_before
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(run.id, trace);
                }
            }
            Err(error) => {
                tracing::debug!(run = run.id, %error, "could not read an ended run's session");
            }
        }
    }
    went
}

fn traced(listed: &Value, session: &str) -> Option<Trace> {
    let found = listed
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["id"] == session)?;
    Some((
        found["state"].as_str().unwrap_or_default().to_owned(),
        found["last_active_ms"].as_i64().unwrap_or_default(),
    ))
}

/// Whether the session of a run that ended went on with its work. One that was
/// done only takes in what other sessions tell it.
fn has_gone_on(session: &Value, run: &Run) -> bool {
    let ended_at = run.ended_at.unwrap_or_default();
    let at_work = works(session["state"].as_str().unwrap_or_default());
    // One begun in the very second the run ended is another turn than the one
    // that ended it when it is under way, or was active after that second.
    let mut later = turns(session).filter(|t| {
        let last = t["last_ms"].as_i64().unwrap_or_default() / 1000;
        began(t) > ended_at || began(t) == ended_at && (at_work || last > ended_at)
    });
    match run.outcome {
        Some(Ended::Failed) => at_work || later.next().is_some(),
        Some(Ended::Done) => later.any(|t| !only_tells(t["prompt"].as_str().unwrap_or_default())),
        _ => false,
    }
}

/// Whether a session in this state is in a turn.
fn works(state: &str) -> bool {
    matches!(state, "working" | "background" | "waiting")
}

fn turns(session: &Value) -> impl Iterator<Item = &Value> {
    session["turns"].as_array().into_iter().flatten()
}

/// When a turn began, in Unix seconds.
fn began(turn: &Value) -> i64 {
    turn["began_ms"].as_i64().unwrap_or_default() / 1000
}

/// Whether a prompt is only what another session tells, with nothing asked.
fn only_tells(prompt: &str) -> bool {
    prompt.starts_with("[message]")
        && !prompt
            .lines()
            .any(|l| l.starts_with("[request]") || l.starts_with("[user]"))
}

/// Ask a run's session to go on. The owner let it work when it was started,
/// so they are not asked again; what they forbid stays so.
pub(super) async fn go_on(app: &App, run: &Run) -> anyhow::Result<()> {
    let call = Call {
        waive: true,
        never_ask: true,
        ..Call::new(
            WORKER,
            connection(app, run)?,
            SEND,
            json!({ "session": run.session, "prompt": GO_ON }),
        )
    };
    match app.gateway.call(&call).await? {
        Outcome::Done(answer) if !answer.is_error => Ok(()),
        Outcome::Done(answer) => anyhow::bail!("{SEND}: {}", answer.text),
        Outcome::Held(_) => anyhow::bail!("{SEND} was held"),
    }
}

/// Whether a session that failed has yet to take up what it was asked at
/// `asked`.
fn taking_up(session: &Value, asked: i64, now: i64) -> bool {
    if session["state"] != "failed" {
        return false;
    }
    // Who else asked it, and when, is not told: it counts from its last sign.
    let active = session["last_active_ms"].as_i64().unwrap_or_default() / 1000;
    let undelivered = session["undelivered"].as_u64().unwrap_or(0) > 0;
    undelivered && now - asked.max(active) <= TAKES_UP_SECS
        || now - asked <= TAKES_UP_SECS && turns(session).last().is_none_or(|t| began(t) < asked)
}

/// Whether a turn that failed saying this was cut off by a fault that passes
/// by itself. What it is not known to be, it is not: a session whose allowance
/// is used up or that is logged out would fail again.
fn passes(why: &str) -> bool {
    let why = why.to_lowercase();
    if why.contains("not your usage limit") {
        return true;
    }
    let stays = [
        "usage limit",
        "session limit",
        "weekly limit",
        "hit your limit",
        "reached your limit",
        "spend limit",
        "quota",
        "out of extra usage",
        "credit",
        "billing",
        "/login",
        "log in",
        "logged out",
        "not logged in",
        "api key",
        "oauth",
        "authenticat",
        "unauthorized",
        "permission_error",
        "error: 400",
        "error: 401",
        "error: 403",
    ];
    let passing = [
        "the response stopped arriving",
        "response stalled",
        "stream disconnected",
        "stream closed",
        "stream ended",
        "stream idle timeout",
        "connection error",
        "connection closed",
        "connection lost",
        "connection reset",
        "unable to connect",
        "econnreset",
        "etimedout",
        "econnrefused",
        "enotfound",
        "epipe",
        "socket",
        "fetch failed",
        "network error",
        "request timed out",
        "request timeout",
        "overloaded",
        "temporarily limiting requests",
        "temporarily unavailable",
        "service unavailable",
        "internal server error",
        "bad gateway",
        "gateway timeout",
        "rate_limit_error",
        "error: 429",
        "error: 500",
        "error: 502",
        "error: 503",
        "error: 504",
        "error: 520",
        "error: 522",
        "error: 524",
        "error: 529",
    ];
    !stays.iter().any(|s| why.contains(s)) && passing.iter().any(|p| why.contains(p))
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
        "failed" => {
            let said = session["message"].as_str().unwrap_or(&result);
            let why = format!("the session's turn failed: {said}");
            if passes(said) {
                Looked::Stumbled(why)
            } else {
                Looked::Failed(why)
            }
        }
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

    fn ended(outcome: Ended, ended_at: i64) -> Run {
        let runs = super::super::Runs::new(crate::store::Store::open_in_memory().unwrap());
        let order = super::super::runs::tests::order(super::super::Runner::Adroit);
        Run {
            outcome: Some(outcome),
            ended_at: Some(ended_at),
            ..runs.begin("t", &order).unwrap()
        }
    }

    #[test]
    fn an_ended_run_goes_on_with_a_turn_of_its_session_begun_since() {
        let turn =
            |began: i64, prompt: &str| json!({ "began_ms": began * 1000 + 999, "prompt": prompt });
        let told = "[message] from another\nmain moved";
        let asked = "[request] from Famulen\ngo on";
        for (state, turns, outcome, expected) in [
            ("working", json!([turn(9, "p")]), Ended::Failed, true),
            ("waiting", json!([turn(9, "p")]), Ended::Failed, true),
            ("failed", json!([turn(9, "p")]), Ended::Failed, false),
            ("failed", json!([turn(10, "p")]), Ended::Failed, false),
            ("failed", json!([turn(11, "p")]), Ended::Failed, true),
            ("idle", json!([turn(11, told)]), Ended::Failed, true),
            ("done", json!([turn(9, asked)]), Ended::Done, false),
            ("done", json!([turn(11, asked)]), Ended::Done, true),
            ("done", json!([turn(10, asked)]), Ended::Done, false),
            ("working", json!([turn(10, asked)]), Ended::Done, true),
            (
                "done",
                json!([{ "began_ms": 10_500, "last_ms": 14_000, "prompt": asked }]),
                Ended::Done,
                true,
            ),
            ("working", json!([turn(11, "直して")]), Ended::Done, true),
            ("working", json!([turn(11, told)]), Ended::Done, false),
            (
                "idle",
                json!([turn(11, asked), turn(12, told)]),
                Ended::Done,
                true,
            ),
            (
                "working",
                json!([turn(11, &format!("{told}\n{asked}"))]),
                Ended::Done,
                true,
            ),
            ("working", json!([turn(11, "p")]), Ended::Stopped, false),
        ] {
            let session = json!({ "state": state, "turns": turns });
            assert_eq!(
                has_gone_on(&session, &ended(outcome, 10)),
                expected,
                "{session} {outcome:?}"
            );
        }
    }

    #[test]
    fn a_passing_fault_is_told_from_a_limit_reached_and_a_lost_login() {
        for why in [
            "API Error: The response stopped arriving. The response above may be incomplete.",
            "API Error: 529 {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\"}}",
            "API Error: 500 Internal server error",
            "API Error: Connection closed mid-response.",
            "Request timed out",
            "API Error: Server is temporarily limiting requests (not your usage limit) · Rate limited",
            "stream disconnected before completion",
        ] {
            assert!(passes(why), "{why}");
        }
        for why in [
            "Claude AI usage limit reached|1795627053",
            "You've hit your limit · resets 3pm",
            "You've hit your session limit · resets 3pm (API Error: 429)",
            "Credit balance is too low",
            "Not logged in · Please run /login",
            "API Error: 401 OAuth token has expired",
            "API Error: 400 invalid_request_error",
            "Prompt is too long",
            "",
        ] {
            assert!(!passes(why), "{why}");
        }
    }

    #[test]
    fn a_session_asked_to_go_on_is_not_taken_for_failed_before_it_takes_that_up() {
        let failed = |began: i64, undelivered: u64| json!({ "state": "failed", "last_active_ms": began * 1000, "turns": [{ "began_ms": began * 1000 }], "undelivered": undelivered });
        let late = 100 + TAKES_UP_SECS + 1;
        for (session, asked, now, expected) in [
            (failed(90, 0), 0, 200, false),
            (failed(90, 1), 0, 200, true),
            (failed(90, 1), 0, 9999, false),
            (failed(90, 1), 100, late, false),
            (failed(90, 0), 100, 101, true),
            (failed(100, 0), 100, 101, false),
            (failed(90, 0), 100, late, false),
        ] {
            assert_eq!(
                taking_up(&session, asked, now),
                expected,
                "{session} {asked} {now}"
            );
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
