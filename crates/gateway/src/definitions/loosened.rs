//! Whether a proposed definition would let a tool run more freely than the
//! one in use.

use crate::permissions::{AutoCourse, auto_course};

use super::{CommandTool, CommandTransport, Definition, Method, PermissionName};

/// How freely a tool runs before the owner chooses, from the freest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Freedom {
    /// `allow`, or auto for a tool that only reads.
    Unasked,
    /// Auto for a tool the assessor weighs, which may run without the owner.
    Assessed,
    /// `confirm`, or auto for a tool that is always asked about.
    Asked,
    Forbidden,
}

impl Freedom {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unasked => "unasked",
            Self::Assessed => "assessed",
            Self::Asked => "asked",
            Self::Forbidden => "forbidden",
        }
    }
}

impl Definition {
    /// Whether the owner is asked of every call of a tool.
    fn each_time(&self, tool: &str) -> bool {
        self.commands
            .iter()
            .any(|t| t.name == tool && t.each_time.is_some())
    }

    /// What a command tool runs, its program included.
    fn runs(&self, tool: &str) -> Option<(&CommandTransport, &CommandTool)> {
        let command = self.commands.iter().find(|t| t.name == tool)?;
        Some((self.transport.command.as_ref()?, command))
    }

    /// How freely the definition itself lets a tool run before the owner
    /// chooses: its `[permissions]` entry, else what auto does with a
    /// written tool's marks. `None` for a tool it says nothing of.
    fn own_freedom(&self, tool: &str) -> Option<Freedom> {
        if let Some(permission) = self.permissions.get(tool) {
            return Some(match permission {
                PermissionName::Allow => Freedom::Unasked,
                PermissionName::Confirm => Freedom::Asked,
                PermissionName::Forbid => Freedom::Forbidden,
            });
        }
        let marks = match self.tools.iter().find(|t| t.name == tool) {
            Some(written) => written.marks(),
            None => self.commands.iter().find(|t| t.name == tool)?.marks(),
        };
        Some(match auto_course(marks, tool) {
            AutoCourse::Run => Freedom::Unasked,
            AutoCourse::Assess => Freedom::Assessed,
            AutoCourse::Ask => Freedom::Asked,
        })
    }
}

/// How `proposed` would loosen what `current` lets run without the owner,
/// one reason per tool; empty when it loosens nothing. A new tool may run
/// unasked only when it is a written `GET`; any other new tool must ask the
/// owner, since the assessor could let it run unasked too. No tool becomes
/// looser than `current` had it.
pub fn loosened(current: Option<&Definition>, proposed: &Definition) -> Vec<String> {
    let mut names: Vec<&str> = proposed
        .tools
        .iter()
        .map(|t| t.name.as_str())
        .chain(proposed.commands.iter().map(|t| t.name.as_str()))
        .chain(proposed.permissions.keys().map(String::as_str))
        .collect();
    if let Some(current) = current {
        names.extend(current.permissions.keys().map(String::as_str));
        names.extend(current.tools.iter().map(|t| t.name.as_str()));
        names.extend(current.commands.iter().map(|t| t.name.as_str()));
    }
    names.sort_unstable();
    names.dedup();
    let mut reasons = Vec::new();
    for name in names {
        let before = current.and_then(|c| c.own_freedom(name));
        let after = proposed.own_freedom(name);
        let reads = proposed
            .tools
            .iter()
            .any(|t| t.name == name && t.method == Method::Get);
        let written = |d: &Definition| d.tools.iter().find(|t| t.name == name).cloned();
        let spends = written(proposed).is_some_and(|t| proposed.spends(&t));
        let same = current.is_some_and(|c| {
            written(c) == written(proposed) && written(c).is_some_and(|t| c.spends(&t))
        });
        match (before, after) {
            (_, Some(after)) if spends && !same && after < Freedom::Asked => {
                reasons.push(format!(
                    "{name} would carry a token traded for at each use without asking: give it \"confirm\" under [permissions]"
                ));
            }
            (Some(before), Some(after)) if after < before => {
                reasons.push(format!(
                    "{name} would go from {} to {}",
                    before.as_str(),
                    after.as_str()
                ));
            }
            // What a command runs decides what it does, so one run unasked
            // may not become another.
            (Some(_), Some(after))
                if (after < Freedom::Asked || current.is_some_and(|c| c.each_time(name)))
                    && current.is_some_and(|c| c.runs(name) != proposed.runs(name)) =>
            {
                reasons.push(format!(
                    "{name} would run something else without asking: give it \"confirm\" under [permissions]"
                ));
            }
            (Some(before), None) if before != Freedom::Unasked => {
                reasons.push(format!(
                    "{name} would lose being {} and fall to what its server says",
                    before.as_str()
                ));
            }
            (None, Some(Freedom::Unasked)) if !reads => {
                reasons.push(format!(
                    "{name} would run without asking, but only a written GET tool may start at allow"
                ));
            }
            (None, Some(Freedom::Assessed)) => {
                reasons.push(format!(
                    "{name} is new and writes, so it must start by asking the owner: give it \"confirm\" under [permissions]"
                ));
            }
            _ => {}
        }
    }
    reasons
}
