//! Delegated tasks run by sessions in the owner's Adroit, reached through
//! the gateway as everything outside is: every start is judged, and held for
//! the owner, as their permissions say.

use serde_json::{Value, json};

use super::WORKER;
use super::order::Order;
use super::runners::{Began, Looked};
use super::runs::Run;
use crate::app::App;
use crate::connections::{Connection, Transport};
use crate::gateway::{Call, Outcome};

/// The bundled definition of Adroit.
pub const ADROIT: &str = "adroit";
/// Its tool that starts a session.
pub const START: &str = "start_session";
/// Its tools that start a session, whether anew or from an ended one. Neither
/// the secretary nor a worker uses them.
pub const LAUNCH: &[&str] = &[START, "resume_session"];
/// How much of a session's answer is kept.
const RESULT_CHARS: usize = 6000;
/// How long a session just started may look as if it had ended its turn
/// before it took up its first prompt.
const TAKES_UP_SECS: i64 = 180;

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
    let session = read(
        app,
        &connection,
        "read_session",
        json!({ "session": run.session, "turns": 1 }),
    )
    .await;
    match session {
        Ok(session) => {
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
            Ok(Looked::Failed(format!(
                "Adroit knows no session {} any more",
                run.session
            )))
        }
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
