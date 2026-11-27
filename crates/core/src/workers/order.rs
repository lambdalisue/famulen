//! What a delegated task orders.

use std::path::Path;

use anyhow::Context as _;
use famulen_notebook::records::{flag, record_of};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::hub;
use crate::notebook::link_target;
use crate::permissions::split_tool;
use crate::records::{Field, Record, SectionContent, sections};

/// The built-in tools a worker may be handed. Any other may start agents,
/// outlive the run or reach what is not looked at here.
pub const HANDABLE: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "MultiEdit",
    "NotebookEdit",
    "Grep",
    "Glob",
    "LS",
    "Bash",
    "WebSearch",
    "WebFetch",
    "TodoWrite",
];

/// Whether a tool may be handed to a worker: a built-in of [`HANDABLE`], or a
/// connection's, by a well-formed name, that is not Famulen's own.
pub fn may_hand(tool: &str) -> bool {
    let word = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    };
    match split_tool(tool) {
        Some((connection, name)) => {
            connection != hub::SERVER_NAME && word(connection) && word(name)
        }
        None => HANDABLE.contains(&tool),
    }
}

/// The section of a task that says how it is delegated.
pub const DELEGATION: &str = "Delegation";

/// How much of the notebook a run reads, each level with what is below it.
/// No level writes it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reading {
    /// Nothing.
    None,
    /// What the request quotes of it, and no more.
    Excerpt,
    /// The files the order names.
    #[default]
    Named,
    /// Besides, the records of the kinds the order names, found by search.
    Search,
    /// Besides, proposing what the secretary may write.
    Propose,
}

impl Reading {
    const ALL: [Reading; 5] = [
        Reading::None,
        Reading::Excerpt,
        Reading::Named,
        Reading::Search,
        Reading::Propose,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Reading::None => "none",
            Reading::Excerpt => "excerpt",
            Reading::Named => "named",
            Reading::Search => "search",
            Reading::Propose => "propose",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Reading::ALL.into_iter().find(|r| r.as_str() == text)
    }

    fn is_default(&self) -> bool {
        *self == Reading::default()
    }
}

/// What runs a delegated task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runner {
    /// A session in the owner's Adroit.
    #[default]
    Adroit,
    /// An agent CLI session Famulen starts, with nothing but the tools and
    /// files handed to it.
    Agent,
    /// Claude Code, started by Famulen itself to work on code in a workspace
    /// of its own; tried in the one repository the owner names.
    Direct,
}

impl Runner {
    pub fn as_str(self) -> &'static str {
        match self {
            Runner::Adroit => "adroit",
            Runner::Agent => "agent",
            Runner::Direct => "direct",
        }
    }

    /// Whether Famulen runs the agent itself, under an executor.
    pub fn runs_here(self) -> bool {
        matches!(self, Runner::Agent | Runner::Direct)
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Runner::Adroit, Runner::Agent, Runner::Direct]
            .into_iter()
            .find(|r| r.as_str() == text)
    }

    /// The name the owner reads.
    pub fn label(self) -> &'static str {
        match self {
            Runner::Adroit => "Adroit",
            Runner::Agent => "手代",
            Runner::Direct => "直接",
        }
    }
}

/// A delegated task as a run takes it: kept with the run, so that what the
/// owner approved is what is started.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Order {
    pub code: String,
    pub title: String,
    pub workgroup: String,
    /// Every workgroup the task is in.
    pub workgroups: Vec<String>,
    /// The workgroups its connections must be in to be used without asking,
    /// each of them; none when it is not known who delegated it, and then
    /// every connection is asked about. Decided when a run is made.
    #[serde(default)]
    pub scopes: Vec<String>,
    pub runner: Runner,
    /// The request: the task's notes.
    pub request: String,
    pub connection: String,
    pub directory: String,
    pub worktree: bool,
    pub base: String,
    pub agent: String,
    pub model: String,
    pub permissions: String,
    pub tools: Vec<String>,
    pub files: Vec<String>,
    /// Whether a call that needs the owner's approval waits for it instead
    /// of being refused.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wait: bool,
    #[serde(default, skip_serializing_if = "Reading::is_default")]
    pub notebook: Reading,
    /// The kinds of record it may search.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search: Vec<String>,
    /// The owner's steps that finish it. Decided when a run is made.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub finish: String,
}

impl Order {
    /// What `task` orders, or why it cannot be run as it is written.
    pub fn of(task: &Record, code: &str) -> anyhow::Result<Self> {
        let section = sections(task)
            .into_iter()
            .find(|s| s.name.eq_ignore_ascii_case(DELEGATION))
            .unwrap_or_default();
        let field = |name: &str| {
            section
                .fields
                .iter()
                .find(|f| f.name.eq_ignore_ascii_case(name))
        };
        let text = |name: &str| {
            field(name)
                .map(|f| f.value.trim().to_owned())
                .unwrap_or_default()
        };
        let list = |name: &str| -> Vec<String> {
            field(name)
                .map(|f| f.values.clone())
                .unwrap_or_default()
                .into_iter()
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
                .collect()
        };
        let runner = Runner::parse(&text("Runner"))
            .context("its Delegation names no Runner: adroit, agent or direct")?;
        let order = Self {
            code: code.to_owned(),
            title: task.title(),
            workgroup: task
                .workgroups()
                .first()
                .cloned()
                .unwrap_or_else(|| crate::workgroups::ALL.to_owned()),
            workgroups: task.workgroups(),
            scopes: Vec::new(),
            runner,
            request: task.notes().trim().to_owned(),
            connection: text("Connection"),
            directory: text("Directory"),
            worktree: flag(&text("Worktree")).unwrap_or(false),
            base: text("Base"),
            agent: text("Agent"),
            model: text("Agent model"),
            permissions: text("Permissions"),
            tools: list("Tools"),
            files: list("Files").iter().map(|f| file_of(f)).collect(),
            wait: flag(&text("Wait")).unwrap_or(false),
            notebook: match text("Notebook").as_str() {
                "" => Reading::default(),
                level => Reading::parse(level)
                    .context("its Notebook is none, excerpt, named, search or propose")?,
            },
            search: list("Search"),
            finish: String::new(),
        };
        order.check()?;
        Ok(order)
    }

    /// What tells this order from another: all that is done, and where.
    pub fn digest(&self) -> anyhow::Result<String> {
        let same = Self {
            code: String::new(),
            title: String::new(),
            scopes: Vec::new(),
            finish: String::new(),
            ..self.clone()
        };
        let digest = Sha256::digest(serde_json::to_vec(&same)?);
        Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
    }

    /// The section of a task that orders this.
    pub fn section(&self) -> SectionContent {
        let worktree = if self.worktree { "yes" } else { "" };
        let wait = if self.wait { "yes" } else { "" };
        let notebook = if self.notebook.is_default() {
            ""
        } else {
            self.notebook.as_str()
        };
        let mut fields: Vec<Field> = [
            ("Runner", self.runner.as_str()),
            ("Connection", &self.connection),
            ("Directory", &self.directory),
            ("Worktree", worktree),
            ("Base", &self.base),
            ("Agent", &self.agent),
            ("Agent model", &self.model),
            ("Permissions", &self.permissions),
            ("Wait", wait),
            ("Notebook", notebook),
        ]
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| Field::new(name, value))
        .collect();
        for (name, values) in [
            ("Tools", &self.tools),
            ("Files", &self.files),
            ("Search", &self.search),
        ] {
            if !values.is_empty() {
                fields.push(Field {
                    name: name.to_owned(),
                    value: String::new(),
                    values: values.clone(),
                });
            }
        }
        SectionContent {
            name: DELEGATION.to_owned(),
            fields,
            items: Vec::new(),
        }
    }

    /// Refuse what could not be run.
    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.request.is_empty(),
            "its notes hold no request to hand over"
        );
        anyhow::ensure!(
            self.directory.is_empty() || Path::new(&self.directory).is_absolute(),
            "Directory is an absolute path, not {}",
            self.directory
        );
        anyhow::ensure!(
            !self.wait || self.runner.runs_here(),
            "Wait is for work Famulen runs itself"
        );
        anyhow::ensure!(
            self.runner.runs_here() || (self.notebook.is_default() && self.search.is_empty()),
            "Notebook and Search are for work Famulen runs itself"
        );
        anyhow::ensure!(
            self.files.is_empty() || !self.runner.runs_here() || self.notebook >= Reading::Named,
            "Files are for a Notebook of named or more"
        );
        anyhow::ensure!(
            self.search.is_empty() || self.notebook >= Reading::Search,
            "Search is for a Notebook of search or propose"
        );
        anyhow::ensure!(
            self.notebook != Reading::Search || !self.search.is_empty(),
            "a Notebook of search needs Search, the kinds of record"
        );
        for kind in &self.search {
            anyhow::ensure!(
                kind.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "{kind:?} is not a kind of record"
            );
        }
        match self.runner {
            Runner::Adroit => anyhow::ensure!(
                !self.directory.is_empty(),
                "a task for Adroit needs a Directory"
            ),
            Runner::Agent => self.check_tools()?,
            Runner::Direct => {
                anyhow::ensure!(
                    !self.directory.is_empty(),
                    "a task run directly needs a Directory"
                );
                anyhow::ensure!(
                    self.agent.is_empty() || self.agent == "claude",
                    "only Claude runs directly, not {}",
                    self.agent
                );
                for (field, set) in [
                    ("Worktree", self.worktree),
                    ("Connection", !self.connection.is_empty()),
                    ("Permissions", !self.permissions.is_empty()),
                    ("Agent model", !self.model.is_empty()),
                ] {
                    anyhow::ensure!(!set, "{field} is for Adroit, not for work run directly");
                }
                self.check_tools()?;
            }
        }
        Ok(())
    }

    fn check_tools(&self) -> anyhow::Result<()> {
        for tool in &self.tools {
            anyhow::ensure!(
                may_hand(tool),
                "{tool:?} is never handed to a worker; it may have {}, and a connection's tools as mcp__<connection>__<tool>",
                HANDABLE.join(", ")
            );
        }
        Ok(())
    }
}

/// The path in the notebook a file handed over is written as: a path, or a
/// wiki link to a record or a memory.
pub(super) fn file_of(text: &str) -> String {
    if !text.starts_with("[[") {
        return text.to_owned();
    }
    let target = link_target(text);
    match record_of(target) {
        Some((kind, id)) => format!("records/{kind}/{id}.md"),
        None => format!("{target}.md"),
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::notebook::Writer;
    use crate::paths::at;
    use crate::records::{MEMOS, Patch, Records, TASKS};

    /// A delegated task's patch, its `Delegation` as given.
    pub(in crate::workers) fn delegated(title: &str, fields: &[(&str, &str)]) -> Patch {
        Patch {
            notes: Some("Do it.".into()),
            sections: vec![SectionContent {
                name: DELEGATION.into(),
                fields: fields.iter().map(|(n, v)| Field::new(n, *v)).collect(),
                items: Vec::new(),
            }],
            ..Patch::titled(title)
        }
        .with("Assignee", super::super::WORKER)
    }

    fn order(records: &Records, patch: Patch) -> anyhow::Result<Order> {
        let t = records.add(Writer::Secretary, TASKS, patch).unwrap();
        Order::of(&t, "task-1")
    }

    #[test]
    fn an_order_is_read_from_the_task_as_it_is_written() {
        let (_tmp, r) = crate::records::testing::records("order-read");
        let got = order(
            &r,
            delegated(
                "直す",
                &[
                    ("Runner", "adroit"),
                    ("Directory", &at("/src/api")),
                    ("Worktree", "yes"),
                    ("Agent", "codex"),
                ],
            ),
        )
        .unwrap();
        assert_eq!(got.runner, Runner::Adroit);
        assert_eq!(
            (got.directory.as_str(), got.worktree, got.agent.as_str()),
            (at("/src/api").as_str(), true, "codex")
        );
        assert_eq!(got.request, "Do it.");

        let memo = r.add(Writer::Owner, MEMOS, Patch::titled("メモ")).unwrap();
        let files = format!("memory/long/semantic/a.md, {}", memo.link());
        let got = order(
            &r,
            delegated(
                "調べる",
                &[
                    ("Runner", "agent"),
                    ("Tools", "Read, mcp__mail__search_threads"),
                    ("Files", &files),
                ],
            ),
        )
        .unwrap();
        assert_eq!(got.tools, ["Read", "mcp__mail__search_threads"]);
        assert_eq!(
            got.files,
            [
                "memory/long/semantic/a.md".to_owned(),
                format!("records/memos/{}.md", memo.id)
            ]
        );
    }

    #[test]
    fn what_is_ordered_reads_back_from_the_section_it_is_written_in() {
        let (_tmp, r) = crate::records::testing::records("order-written");
        let wanted = Order {
            code: "task-1".into(),
            title: "直す".into(),
            workgroup: crate::workgroups::ALL.into(),
            runner: Runner::Agent,
            request: "Do it.".into(),
            directory: at("/src"),
            tools: vec!["Read".into(), "mcp__mail__search".into()],
            files: vec!["memory/core.md".into()],
            wait: true,
            notebook: Reading::Propose,
            search: vec!["tasks".into(), "memos".into()],
            ..Order::default()
        };
        let mut patch = delegated("直す", &[]);
        patch.sections = vec![wanted.section()];
        assert_eq!(order(&r, patch).unwrap(), wanted);
    }

    #[test]
    fn work_run_directly_is_handed_connections_and_the_named_notebook() {
        let (_tmp, r) = crate::records::testing::records("order-direct");
        let src = at("/src");
        let got = order(
            &r,
            delegated(
                "直す",
                &[
                    ("Runner", "direct"),
                    ("Directory", &src),
                    ("Tools", "mcp__mail__search"),
                    ("Files", "memory/core.md"),
                ],
            ),
        )
        .unwrap();
        assert_eq!(got.tools, ["mcp__mail__search"]);
        assert_eq!(got.files, ["memory/core.md"]);
        assert_eq!(got.notebook, Reading::Named, "named unless it says");
    }

    #[test]
    fn an_order_that_says_nothing_of_the_notebook_keeps_its_digest() {
        let written = serde_json::to_string(&Order::default()).unwrap();
        assert!(!written.contains("notebook") && !written.contains("search"));
    }

    #[test]
    fn a_task_that_does_not_say_how_it_is_delegated_orders_nothing() {
        let (_tmp, r) = crate::records::testing::records("order-refused");
        for (fields, why) in [
            (&[][..], "no Runner"),
            (&[("Runner", "adroit")][..], "needs a Directory"),
            (
                &[("Runner", "adroit"), ("Directory", "src")][..],
                "an absolute path",
            ),
            (
                &[("Runner", "agent"), ("Tools", "mcp__famulen__save_record")][..],
                "never handed",
            ),
            (
                &[("Runner", "agent"), ("Tools", "Read, Task")][..],
                "never handed",
            ),
            (
                &[("Runner", "agent"), ("Tools", "default")][..],
                "never handed",
            ),
            (
                &[("Runner", "agent"), ("Tools", "Read Bash")][..],
                "never handed",
            ),
            (
                &[("Runner", "agent"), ("Tools", "mcp__mail__a b")][..],
                "never handed",
            ),
            (&[("Runner", "direct")][..], "needs a Directory"),
            (
                &[("Runner", "adroit"), ("Directory", "/src"), ("Wait", "yes")][..],
                "runs itself",
            ),
            (
                &[
                    ("Runner", "direct"),
                    ("Directory", "/src"),
                    ("Worktree", "yes"),
                ][..],
                "for Adroit",
            ),
            (
                &[
                    ("Runner", "direct"),
                    ("Directory", "/src"),
                    ("Agent", "codex"),
                ][..],
                "only Claude",
            ),
            (
                &[
                    ("Runner", "adroit"),
                    ("Directory", "/src"),
                    ("Notebook", "none"),
                ][..],
                "runs itself",
            ),
            (
                &[
                    ("Runner", "agent"),
                    ("Notebook", "excerpt"),
                    ("Files", "memory/core.md"),
                ][..],
                "named",
            ),
            (
                &[("Runner", "agent"), ("Notebook", "search")][..],
                "needs Search",
            ),
            (
                &[("Runner", "agent"), ("Search", "tasks")][..],
                "search or propose",
            ),
            (
                &[
                    ("Runner", "agent"),
                    ("Notebook", "search"),
                    ("Search", "../memory"),
                ][..],
                "kind of record",
            ),
        ] {
            let error = order(&r, delegated("x", fields)).unwrap_err().to_string();
            assert!(error.contains(why), "{error}");
        }
        let mut empty = delegated("x", &[("Runner", "agent")]);
        empty.notes = None;
        assert!(order(&r, empty).is_err(), "a request is needed");
    }
}
