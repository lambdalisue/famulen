//! What was read in confidence, and where it is kept.

use std::collections::BTreeSet;
use std::hash::{DefaultHasher, Hash, Hasher};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::slack::web::Room;
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    /// What the notebook says of people.
    People,
    /// The owner's things, contracts and other records of their own.
    Belongings,
    /// What was said where not everyone can read, and whatever is not known
    /// to be public.
    Private,
    /// What everyone in a workspace can read, and no one outside it.
    Internal,
    /// What has to do with the vault.
    Vault,
    /// What the conversation still holds from earlier turns.
    Earlier,
}

/// One thing read in confidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct Taint {
    pub(crate) kind: Kind,
    /// The connection and the channel it was read in, for what was said there.
    pub(crate) connection: String,
    pub(crate) place: String,
    /// What it is, in the owner's words.
    pub(crate) label: String,
    /// Tells it from another of the same name: a digest of what was read. An
    /// approval covers what the owner was asked about, and nothing read since.
    #[serde(default)]
    told: u64,
}

impl Taint {
    pub(crate) fn new(kind: Kind, label: impl Into<String>) -> Self {
        Self {
            kind,
            connection: String::new(),
            place: String::new(),
            label: label.into(),
            told: 0,
        }
    }

    /// As `words` told it. What was said in a channel stays what it is however
    /// it was read.
    pub(super) fn told_by(mut self, words: &str) -> Self {
        if self.place.is_empty() {
            let mut hasher = DefaultHasher::new();
            words.hash(&mut hasher);
            self.told = hasher.finish();
        }
        self
    }

    pub(super) fn earlier() -> Self {
        Self::new(Kind::Earlier, "前の話")
    }

    pub(super) fn of_connection(connection: &str, what: &str) -> Self {
        Self::named(connection, connection, what)
    }

    /// Read through `connection`, which the owner calls `name`.
    pub(super) fn named(connection: &str, name: &str, what: &str) -> Self {
        let name = if name.is_empty() { connection } else { name };
        Self {
            connection: connection.to_owned(),
            ..Self::new(Kind::Private, format!("「{name}」{what}"))
        }
    }

    /// What was said in a channel of Slack.
    pub(super) fn room(workspace: &str, channel: &str, room: &Room) -> Self {
        let what = match room.name.as_str() {
            "" => "公開でない場の内容".to_owned(),
            name => format!("公開でない場 #{name} の内容"),
        };
        Self {
            connection: workspace.to_owned(),
            place: channel.to_owned(),
            ..Self::new(Kind::Private, what)
        }
    }

    /// What was said where everyone in a Slack workspace can read.
    pub(super) fn internal(workspace: &str, name: &str, what: &str) -> Self {
        Self {
            kind: Kind::Internal,
            ..Self::named(workspace, name, what)
        }
    }

    /// Said in `channel`.
    pub(super) fn in_place(mut self, channel: &str) -> Self {
        self.place = channel.to_owned();
        self
    }

    /// Whether those who hear it in `channel`, which is `room`, are among
    /// those it was said to.
    pub(super) fn stays_in(&self, workspace: &str, channel: &str, room: &Room) -> bool {
        if self.connection != workspace {
            return false;
        }
        match self.kind {
            Kind::Private => !self.place.is_empty() && self.place == channel,
            // Those of other organizations may be in a channel shared with them.
            Kind::Internal => self.place == channel || (room.private.is_some() && !room.shared),
            _ => false,
        }
    }

    /// Whether it was read through `connection` from the repository named
    /// `repository`, so that it may go back there.
    pub(crate) fn of_repository(&self, connection: &str, repository: Option<&str>) -> bool {
        self.kind == Kind::Private
            && self.connection == connection
            && repository.is_some_and(|r| r == self.place)
    }

    /// Whether it was read through `connection`, outside any channel.
    pub(crate) fn came_by(&self, connection: &str) -> bool {
        self.kind == Kind::Private && self.place.is_empty() && self.connection == connection
    }
}

/// What carries marks: a turn, or what a turn leaves for later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Holder {
    Input(i64),
    Wake(i64),
    /// A delegated task, by its id.
    Task(String),
    /// A conversation: what its model still holds of every turn in it.
    Conversation(i64),
    /// A routine, by its record's id: what its runs are told.
    Routine(String),
    /// A record or a memory of the notebook, by its id: what was written into it.
    Record(String),
}

impl Holder {
    fn key(&self) -> String {
        match self {
            Self::Input(id) => format!("input:{id}"),
            Self::Wake(id) => format!("wake:{id}"),
            Self::Task(id) => format!("task:{id}"),
            Self::Conversation(id) => format!("conversation:{id}"),
            Self::Routine(id) => format!("routine:{id}"),
            Self::Record(id) => format!("record:{id}"),
        }
    }
}

/// The conversation an input is of, when it is of one.
pub(super) fn conversation_of(store: &Store, input: i64) -> anyhow::Result<Option<i64>> {
    Ok(store
        .conn()
        .query_row(
            "SELECT conversation FROM conversation_keys WHERE key = ?1",
            [format!("input:{input}")],
            |r| r.get(0),
        )
        .optional()?)
}

pub(super) fn mark(store: &Store, holder: &Holder, taints: &[Taint]) -> anyhow::Result<()> {
    for taint in taints {
        store.conn().execute(
            "INSERT OR IGNORE INTO taints (holder, taint) VALUES (?1, ?2)",
            params![holder.key(), serde_json::to_string(taint)?],
        )?;
    }
    Ok(())
}

pub(super) fn clear(store: &Store, holder: &Holder) -> anyhow::Result<()> {
    store
        .conn()
        .execute("DELETE FROM taints WHERE holder = ?1", [holder.key()])?;
    Ok(())
}

pub(super) fn held_by(store: &Store, holder: &Holder) -> anyhow::Result<BTreeSet<Taint>> {
    let conn = store.conn();
    let mut statement = conn.prepare("SELECT taint FROM taints WHERE holder = ?1")?;
    let rows = statement.query_map([holder.key()], |r| r.get::<_, String>(0))?;
    rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}

/// The files of the notebook a run was handed.
pub(super) fn files_of(store: &Store, run: i64) -> anyhow::Result<Vec<String>> {
    let spec: Option<String> = store
        .conn()
        .query_row("SELECT spec FROM worker_runs WHERE id = ?1", [run], |r| {
            r.get(0)
        })
        .optional()?;
    let spec: serde_json::Value = serde_json::from_str(spec.as_deref().unwrap_or("{}"))?;
    let files = spec["files"].as_array().into_iter().flatten();
    Ok(files
        .filter_map(|f| f.as_str().map(str::to_owned))
        .collect())
}

/// The task a worker's run was of.
pub(super) fn task_of(store: &Store, run: i64) -> anyhow::Result<Option<String>> {
    Ok(store
        .conn()
        .query_row("SELECT task FROM worker_runs WHERE id = ?1", [run], |r| {
            r.get(0)
        })
        .optional()?)
}

/// What the owner was asked to let be said, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Disclosure {
    pub(super) ticket: i64,
    pub(super) input: i64,
    pub(super) workspace: String,
    pub(super) channel: String,
    pub(super) taints: BTreeSet<Taint>,
}

pub(super) fn keep_disclosure(store: &Store, d: &Disclosure) -> anyhow::Result<()> {
    store.conn().execute(
        "INSERT OR REPLACE INTO disclosures (ticket, input, workspace, channel, taints)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            d.ticket,
            d.input,
            d.workspace,
            d.channel,
            serde_json::to_string(&d.taints)?
        ],
    )?;
    Ok(())
}

/// What the owner was asked about in the turn of `input`.
pub(super) fn disclosures_of(store: &Store, input: i64) -> anyhow::Result<Vec<Disclosure>> {
    let conn = store.conn();
    let mut statement = conn
        .prepare("SELECT ticket, workspace, channel, taints FROM disclosures WHERE input = ?1")?;
    let rows = statement.query_map([input], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, String>(3)?))
    })?;
    rows.map(|row| {
        let (ticket, workspace, channel, taints) = row?;
        Ok(Disclosure {
            ticket,
            input,
            workspace,
            channel,
            taints: serde_json::from_str(&taints)?,
        })
    })
    .collect()
}

/// Every record and memory of the notebook something was written into in
/// confidence, by its id, with what was written.
pub(super) fn marked_records(store: &Store) -> anyhow::Result<Vec<(String, Taint)>> {
    let conn = store.conn();
    let mut statement =
        conn.prepare("SELECT holder, taint FROM taints WHERE holder LIKE 'record:%'")?;
    let rows = statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    rows.map(|row| {
        let (holder, taint) = row?;
        let id = holder.trim_start_matches("record:").to_owned();
        Ok((id, serde_json::from_str(&taint)?))
    })
    .collect()
}
