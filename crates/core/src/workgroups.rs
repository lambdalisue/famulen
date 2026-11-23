//! Workgroups such as 仕事 and 個人 that scope what the console shows and what
//! the secretary may use. `all` always exists and cannot be removed.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use famulen_notebook::workgroups as written;

use crate::notebook::{Notebook, Writer};
use crate::store::Store;

pub const ALL: &str = "all";

const ALL_COLOR: &str = "#0F6E68";

/// What a member of a workgroup is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberKind {
    /// A channel the owner talks through: `console`, `slack:<workspace>`.
    Channel,
    /// A service connection.
    Connection,
    /// A lookout source.
    Lookout,
}

impl MemberKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Connection => "connection",
            Self::Lookout => "lookout",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "channel" => Self::Channel,
            "connection" => Self::Connection,
            "lookout" => Self::Lookout,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarMode {
    Show,
    Faint,
    Hide,
}

impl CalendarMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Faint => "faint",
            Self::Hide => "hide",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "show" => Some(Self::Show),
            "faint" => Some(Self::Faint),
            "hide" => Some(Self::Hide),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Show => "出す",
            Self::Faint => "薄く出す",
            Self::Hide => "出さない",
        }
    }
}

/// What a workgroup's account choice is for.
pub const PURPOSES: [(&str, &str); 3] = [
    ("mail", "メールを送るとき"),
    ("calendar", "予定を入れるとき"),
    ("slack", "Slack に投稿するとき"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workgroup {
    pub id: String,
    pub name: String,
    pub color: String,
}

impl Workgroup {
    pub fn is_all(&self) -> bool {
        self.id == ALL
    }
}

/// The palette a workgroup may take its theme color from.
pub const COLORS: [(&str, &str); 5] = [
    ("青", "#2a78d6"),
    ("橙", "#E0622F"),
    ("青緑", "#0F6E68"),
    ("紫", "#6250d6"),
    ("緑", "#008300"),
];

/// The workgroups: which there are is written in the notebook, and what each
/// takes in of Famulen's own (channels, connections) is kept as settings.
#[derive(Clone)]
pub struct Workgroups {
    store: Store,
    notebook: Notebook,
}

impl Workgroups {
    /// Open the workgroups, creating `all`, 仕事 and 個人 the first time.
    pub fn new(store: Store, notebook: Notebook) -> anyhow::Result<Self> {
        let this = Self { store, notebook };
        if this.store.state("workgroups.seeded")?.is_none() {
            if written::list(&this.notebook)?.is_empty() {
                for (id, name, color) in [
                    (ALL, "すべて", ALL_COLOR),
                    ("work", "仕事", "#2a78d6"),
                    ("personal", "個人", "#E0622F"),
                ] {
                    this.write(Writer::Secretary, id, name, color)?;
                }
            }
            this.store.set_state("workgroups.seeded", "1")?;
        }
        Ok(this)
    }

    fn write(&self, writer: Writer, id: &str, name: &str, color: &str) -> anyhow::Result<()> {
        written::set(
            &self.notebook,
            &writer.signature(),
            &written::Workgroup {
                id: id.to_owned(),
                name: name.to_owned(),
                color: color.to_owned(),
            },
        )
    }

    /// The workgroup of the input the secretary is working on.
    pub fn working(&self) -> anyhow::Result<String> {
        Ok(self
            .store
            .running()?
            .map(|turn| turn.workgroup)
            .unwrap_or_else(|| ALL.to_owned()))
    }

    /// Every workgroup, `all` first whatever the notebook says.
    pub fn list(&self) -> anyhow::Result<Vec<Workgroup>> {
        let mut all: Vec<Workgroup> = written::list(&self.notebook)?
            .into_iter()
            .map(|w| Workgroup {
                color: if check_color(&w.color).is_ok() {
                    w.color
                } else {
                    ALL_COLOR.to_owned()
                },
                id: w.id,
                name: w.name,
            })
            .collect();
        let first = match all.iter().position(Workgroup::is_all) {
            Some(at) => all.remove(at),
            None => Workgroup {
                id: ALL.to_owned(),
                name: "すべて".to_owned(),
                color: ALL_COLOR.to_owned(),
            },
        };
        all.insert(0, first);
        Ok(all)
    }

    pub fn get(&self, id: impl AsRef<str>) -> anyhow::Result<Option<Workgroup>> {
        Ok(self.list()?.into_iter().find(|w| w.id == id.as_ref()))
    }

    /// The workgroup an id names, falling back to `all` for unknown ids.
    pub fn resolve(&self, id: Option<&str>) -> anyhow::Result<Workgroup> {
        if let Some(id) = id
            && let Some(workgroup) = self.get(id)?
        {
            return Ok(workgroup);
        }
        self.get(ALL)?
            .ok_or_else(|| anyhow::anyhow!("the all workgroup is missing"))
    }

    pub fn add(
        &self,
        writer: Writer,
        name: impl AsRef<str>,
        color: impl AsRef<str>,
    ) -> anyhow::Result<Workgroup> {
        let name = name.as_ref().trim();
        anyhow::ensure!(!name.is_empty(), "a workgroup needs a name");
        check_color(color.as_ref())?;
        let id = {
            let base: String = name
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            let base = if base.is_empty() {
                "workgroup".to_owned()
            } else {
                base
            };
            let mut id = base.clone();
            let mut n = 2;
            while self.get(&id)?.is_some() {
                id = format!("{base}{n}");
                n += 1;
            }
            id
        };
        self.write(writer, &id, name, color.as_ref())?;
        self.get(&id)?
            .ok_or_else(|| anyhow::anyhow!("workgroup {id} vanished"))
    }

    pub fn update(
        &self,
        writer: Writer,
        id: impl AsRef<str>,
        name: Option<&str>,
        color: Option<&str>,
    ) -> anyhow::Result<()> {
        let id = id.as_ref();
        let now = self
            .get(id)?
            .ok_or_else(|| anyhow::anyhow!("no workgroup {id}"))?;
        let name = name.map_or(now.name.as_str(), str::trim);
        anyhow::ensure!(!name.is_empty(), "a workgroup needs a name");
        let color = color.unwrap_or(&now.color);
        check_color(color)?;
        self.write(writer, id, name, color)
    }

    /// Remove a workgroup. Its members and settings go; records stay.
    pub fn remove(&self, writer: Writer, id: impl AsRef<str>) -> anyhow::Result<()> {
        let id = id.as_ref();
        anyhow::ensure!(id != ALL, "the all workgroup cannot be removed");
        written::remove(&self.notebook, &writer.signature(), id)?;
        let conn = self.store.conn();
        conn.execute("DELETE FROM workgroup_members WHERE workgroup = ?1", [id])?;
        conn.execute("DELETE FROM workgroup_accounts WHERE workgroup = ?1", [id])?;
        Ok(())
    }

    pub fn set_member(
        &self,
        workgroup: impl AsRef<str>,
        kind: MemberKind,
        member: impl AsRef<str>,
        on: bool,
    ) -> anyhow::Result<()> {
        let conn = self.store.conn();
        if on {
            conn.execute(
                "INSERT OR IGNORE INTO workgroup_members (workgroup, kind, ref) VALUES (?1, ?2, ?3)",
                params![workgroup.as_ref(), kind.as_str(), member.as_ref()],
            )?;
        } else {
            conn.execute(
                "DELETE FROM workgroup_members WHERE workgroup = ?1 AND kind = ?2 AND ref = ?3",
                params![workgroup.as_ref(), kind.as_str(), member.as_ref()],
            )?;
        }
        Ok(())
    }

    /// Take a member out of every workgroup, as when it is removed.
    pub fn forget_member(&self, kind: MemberKind, member: impl AsRef<str>) -> anyhow::Result<()> {
        self.store.conn().execute(
            "DELETE FROM workgroup_members WHERE kind = ?1 AND ref = ?2",
            params![kind.as_str(), member.as_ref()],
        )?;
        Ok(())
    }

    /// Whether a member belongs to `workgroup`. Everything belongs to `all`.
    pub fn has_member(
        &self,
        workgroup: impl AsRef<str>,
        kind: MemberKind,
        member: impl AsRef<str>,
    ) -> anyhow::Result<bool> {
        if workgroup.as_ref() == ALL {
            return Ok(true);
        }
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT 1 FROM workgroup_members WHERE workgroup = ?1 AND kind = ?2 AND ref = ?3",
                params![workgroup.as_ref(), kind.as_str(), member.as_ref()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The workgroups other than `all` that a member belongs to.
    pub fn workgroups_of(
        &self,
        kind: MemberKind,
        member: impl AsRef<str>,
    ) -> anyhow::Result<Vec<String>> {
        let within: Vec<String> = {
            let conn = self.store.conn();
            let mut stmt = conn
                .prepare("SELECT workgroup FROM workgroup_members WHERE kind = ?1 AND ref = ?2")?;
            stmt.query_map(params![kind.as_str(), member.as_ref()], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        Ok(self
            .list()?
            .into_iter()
            .map(|w| w.id)
            .filter(|id| within.contains(id))
            .collect())
    }

    /// The workgroup a request through a channel starts in: the channel's only
    /// workgroup, or `all` when it is in none or several.
    pub fn workgroup_for_channel(&self, channel: impl AsRef<str>) -> anyhow::Result<String> {
        let workgroups = self.workgroups_of(MemberKind::Channel, channel)?;
        Ok(match workgroups.as_slice() {
            [only] => only.clone(),
            _ => ALL.to_owned(),
        })
    }

    pub fn account(
        &self,
        workgroup: impl AsRef<str>,
        purpose: impl AsRef<str>,
    ) -> anyhow::Result<Option<String>> {
        Ok(self
            .store
            .conn()
            .query_row(
                "SELECT connection FROM workgroup_accounts WHERE workgroup = ?1 AND purpose = ?2",
                params![workgroup.as_ref(), purpose.as_ref()],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_account(
        &self,
        workgroup: impl AsRef<str>,
        purpose: impl AsRef<str>,
        connection: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.store.conn();
        match connection {
            Some(c) if !c.is_empty() => {
                conn.execute(
                    "INSERT INTO workgroup_accounts (workgroup, purpose, connection) VALUES (?1, ?2, ?3)
                     ON CONFLICT (workgroup, purpose) DO UPDATE SET connection = ?3",
                    params![workgroup.as_ref(), purpose.as_ref(), c],
                )?;
            }
            _ => {
                conn.execute(
                    "DELETE FROM workgroup_accounts WHERE workgroup = ?1 AND purpose = ?2",
                    params![workgroup.as_ref(), purpose.as_ref()],
                )?;
            }
        }
        Ok(())
    }

    /// The header the secretary reads about the workgroup of an input.
    pub fn describe_for_prompt(&self, workgroup: &Workgroup) -> anyhow::Result<String> {
        if workgroup.is_all() {
            return Ok("workgroup: すべて (every connection and record may be used)".to_owned());
        }
        let members: Vec<String> = {
            let conn = self.store.conn();
            let mut stmt = conn.prepare(
                "SELECT ref FROM workgroup_members WHERE workgroup = ?1 AND kind = 'connection' ORDER BY ref",
            )?;
            stmt.query_map([&workgroup.id], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        let mut accounts = Vec::new();
        for (purpose, _) in PURPOSES {
            if let Some(conn) = self.account(&workgroup.id, purpose)? {
                accounts.push(format!("{purpose}={conn}"));
            }
        }
        Ok(format!(
            "workgroup: {} (id {}). Connections in this workgroup: {}. Tools of other connections need the owner's confirmation. Events of calendars outside it are busy time only. Put records you add in this workgroup.{}",
            workgroup.name,
            workgroup.id,
            if members.is_empty() {
                "none".to_owned()
            } else {
                members.join(", ")
            },
            if accounts.is_empty() {
                String::new()
            } else {
                format!(" Accounts to use: {}.", accounts.join(", "))
            }
        ))
    }
}

fn check_color(color: &str) -> anyhow::Result<()> {
    let ok = color.len() == 7
        && color.starts_with('#')
        && color[1..].chars().all(|c| c.is_ascii_hexdigit());
    anyhow::ensure!(ok, "a color is #RRGGBB: {color}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: Writer = Writer::Owner;

    fn workgroups() -> (crate::notebook::testing::TempDir, Workgroups) {
        static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tmp, records) = crate::records::testing::records(&format!("workgroups-{n}"));
        let c =
            Workgroups::new(Store::open_in_memory().unwrap(), records.notebook().clone()).unwrap();
        (tmp, c)
    }

    #[test]
    fn a_forgotten_member_leaves_every_workgroup() {
        let (_tmp, c) = workgroups();
        c.set_member("work", MemberKind::Connection, "gmail-work", true)
            .unwrap();
        c.set_member("personal", MemberKind::Connection, "gmail-work", true)
            .unwrap();
        c.forget_member(MemberKind::Connection, "gmail-work")
            .unwrap();
        assert!(
            !c.has_member("work", MemberKind::Connection, "gmail-work")
                .unwrap()
        );
        assert!(
            !c.has_member("personal", MemberKind::Connection, "gmail-work")
                .unwrap()
        );
    }

    #[test]
    fn all_work_and_personal_exist_at_first() {
        let (_tmp, c) = workgroups();
        let names: Vec<_> = c.list().unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["すべて", "仕事", "個人"]);
    }

    #[test]
    fn all_cannot_be_removed_but_others_can() {
        let (_tmp, c) = workgroups();
        assert!(c.remove(ME, ALL).is_err());
        let hobby = c.add(ME, "Hobby 2", "#6250d6").unwrap();
        assert_eq!(hobby.id, "hobby2");
        c.remove(ME, &hobby.id).unwrap();
        assert!(c.get(&hobby.id).unwrap().is_none());
    }

    #[test]
    fn a_channel_in_one_workgroup_starts_requests_there() {
        let (_tmp, c) = workgroups();
        c.set_member("work", MemberKind::Channel, "slack:w1", true)
            .unwrap();
        assert_eq!(c.workgroup_for_channel("slack:w1").unwrap(), "work");
        c.set_member("personal", MemberKind::Channel, "slack:w1", true)
            .unwrap();
        assert_eq!(c.workgroup_for_channel("slack:w1").unwrap(), ALL);
        assert_eq!(c.workgroup_for_channel("console").unwrap(), ALL);
    }

    #[test]
    fn everything_belongs_to_all() {
        let (_tmp, c) = workgroups();
        assert!(
            c.has_member(ALL, MemberKind::Connection, "gmail-work")
                .unwrap()
        );
        assert!(
            !c.has_member("work", MemberKind::Connection, "gmail-work")
                .unwrap()
        );
        c.set_member("work", MemberKind::Connection, "gmail-work", true)
            .unwrap();
        assert!(
            c.has_member("work", MemberKind::Connection, "gmail-work")
                .unwrap()
        );
    }

    #[test]
    fn the_prompt_names_the_accounts_to_use() {
        let (_tmp, c) = workgroups();
        c.set_account("work", "mail", Some("gmail-work")).unwrap();
        c.set_member("work", MemberKind::Connection, "gmail-work", true)
            .unwrap();
        let work = c.get("work").unwrap().unwrap();
        let text = c.describe_for_prompt(&work).unwrap();
        assert!(text.contains("mail=gmail-work"), "{text}");
        assert!(
            text.contains("Connections in this workgroup: gmail-work."),
            "{text}"
        );
    }

    #[test]
    fn colors_are_checked() {
        let (_tmp, c) = workgroups();
        assert!(c.add(ME, "x", "blue").is_err());
        assert!(c.update(ME, "work", None, Some("#12345")).is_err());
    }
}
