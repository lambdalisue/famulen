//! The owner's skills, given to the secretary without what they grant.
//!
//! Claude Code runs with project settings only, so the owner's own skills
//! (and their permission rules) are not read from their home. Each start
//! copies the owner's skills into the working directory, where Claude Code
//! finds them as project skills, and the skills of the owner's enabled
//! plugins into plugins of their own that hold nothing but skills.
//!
//! A skill's `allowed-tools` lets the tools it names run without asking,
//! and its `hooks` run commands without asking at all, so both are left out
//! of every copy: every tool call a skill leads to is still put to Famulen.
//! A skill's `context` and `agent` would run it in an agent of its own, which
//! the secretary may not start, so they are left out too.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Frontmatter keys that would let something run without Famulen's answer, or
/// run the skill in an agent of its own.
const GRANTS: &[&str] = &["allowed-tools", "hooks", "context", "agent"];

/// Where the copies of the owner's own skills go, under the working directory.
pub const SKILLS_DIR: &str = ".claude/skills";
/// Where the plugins made from the owner's enabled plugins go, under the
/// working directory; Claude Code is pointed at it with `--plugin-dir`.
pub const PLUGINS_DIR: &str = ".claude/owner-plugins";

/// Where the owner's skills are found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sources {
    /// Claude Code's configuration directory: `skills/`, `settings.json`
    /// for the enabled plugins and `plugins/` for where they are installed.
    pub claude_dir: PathBuf,
    /// Further directories of skills, read after `claude_dir/skills`; a
    /// name found earlier wins.
    pub skill_dirs: Vec<PathBuf>,
}

/// Where Claude Code keeps what is its own, as `env` gives it.
pub(crate) fn claude_dir(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let given = |name| env(name).filter(|d| !d.is_empty()).map(PathBuf::from);
    given("CLAUDE_CONFIG_DIR").or_else(|| Some(given("HOME")?.join(".claude")))
}

impl Sources {
    /// The owner's as `env` gives them: `$CLAUDE_CONFIG_DIR` or
    /// `~/.claude`, and `~/.agents/skills`.
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let home = env("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
        let claude_dir = claude_dir(&env)?;
        Some(Self {
            claude_dir,
            skill_dirs: home
                .map(|h| vec![h.join(".agents/skills")])
                .unwrap_or_default(),
        })
    }
}

/// What was put in place for one start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Installed {
    /// The owner's own skills, by name.
    pub skills: Vec<String>,
    /// The plugins skills came from, by name.
    pub plugins: Vec<String>,
}

/// Put the owner's skills into `workdir`, replacing what an earlier start
/// put there.
pub fn install(sources: &Sources, workdir: &Path) -> anyhow::Result<Installed> {
    let skills_dir = workdir.join(SKILLS_DIR);
    let plugins_dir = workdir.join(PLUGINS_DIR);
    for dir in [&skills_dir, &plugins_dir] {
        if dir.symlink_metadata().is_ok() {
            std::fs::remove_dir_all(dir)?;
        }
    }
    std::fs::create_dir_all(&skills_dir)?;

    let mut installed = Installed::default();
    let mut seen = BTreeSet::new();
    let own = std::iter::once(sources.claude_dir.join("skills")).chain(sources.skill_dirs.clone());
    for dir in own {
        for (name, from) in skills_in(&dir)? {
            if seen.insert(name.clone()) {
                copy_skill(&from, &skills_dir.join(&name))?;
                installed.skills.push(name);
            }
        }
    }

    for (plugin, root) in enabled_plugins(&sources.claude_dir)? {
        let skills = skills_in(&root.join("skills"))?;
        if skills.is_empty() {
            continue;
        }
        let dest = plugins_dir.join(&plugin);
        std::fs::create_dir_all(dest.join(".claude-plugin"))?;
        std::fs::write(
            dest.join(".claude-plugin/plugin.json"),
            serde_json::json!({ "name": plugin }).to_string(),
        )?;
        for (name, from) in skills {
            copy_skill(&from, &dest.join("skills").join(name))?;
        }
        installed.plugins.push(plugin);
    }
    Ok(installed)
}

/// The skills directly in `dir`, by name, in name order: each directory
/// (or link to one) that holds a `SKILL.md`. A missing `dir` has none.
fn skills_in(dir: &Path) -> anyhow::Result<Vec<(String, PathBuf)>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut skills = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.') || !entry.path().join("SKILL.md").is_file() {
            continue;
        }
        skills.push((name, entry.path()));
    }
    skills.sort();
    Ok(skills)
}

/// Copy the skill at `from` to `to`: `SKILL.md` without its grants, and
/// every other entry as a link to the original, so the files a skill reads
/// are the owner's own.
fn copy_skill(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    let skill = std::fs::read_to_string(from.join("SKILL.md"))?;
    std::fs::write(to.join("SKILL.md"), without_grants(&skill))?;
    let from = std::fs::canonicalize(from)?;
    for entry in std::fs::read_dir(&from)? {
        let entry = entry?;
        if entry.file_name() != "SKILL.md" {
            symlink(&entry.path(), &to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(not(unix))]
fn symlink(original: &Path, link: &Path) -> std::io::Result<()> {
    if original.is_dir() {
        std::os::windows::fs::symlink_dir(original, link)
    } else {
        std::os::windows::fs::symlink_file(original, link)
    }
}

/// `skill` with the frontmatter keys in [`GRANTS`] left out, values that
/// run over several lines included. The rest is kept as written.
pub fn without_grants(skill: &str) -> String {
    let skill = skill.trim_start_matches('\u{feff}');
    let mut lines = skill.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return String::new();
    };
    if first.trim_end() != "---" {
        return skill.to_owned();
    }
    let mut out = String::from(first);
    let mut in_frontmatter = true;
    let mut dropping = false;
    // How far the keys are indented: YAML lets all of them be.
    let mut indent = None;
    for line in lines {
        if in_frontmatter {
            if line.trim_end() == "---" {
                in_frontmatter = false;
            } else {
                let text = line.trim_start_matches([' ', '\t']);
                let depth = line.len() - text.len();
                let blank = text.trim().is_empty() || text.starts_with('#');
                if !blank && indent.is_none() {
                    indent = Some(depth);
                }
                let continues =
                    blank || indent.is_some_and(|keys| depth > keys) || text.starts_with('-');
                if !continues {
                    let key = text.split(':').next().unwrap_or_default();
                    let key = key.trim().trim_matches(['"', '\'']);
                    // Ways to write a key that are not read here.
                    let odd = text.starts_with(['{', '?', ':', '&', '!'])
                        || key == "<<"
                        || key.contains('\\');
                    dropping = odd || GRANTS.iter().any(|g| g.eq_ignore_ascii_case(key));
                }
                if dropping {
                    continue;
                }
            }
        }
        out.push_str(line);
    }
    out
}

/// The owner's enabled plugins, by name, with where each is installed for
/// the owner: `enabledPlugins` in `settings.json`, looked up in
/// `plugins/installed_plugins.json`. Nothing else of the settings is read.
fn enabled_plugins(claude_dir: &Path) -> anyhow::Result<Vec<(String, PathBuf)>> {
    let read = |path: PathBuf| -> Value {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null)
    };
    let settings = read(claude_dir.join("settings.json"));
    let installed = read(claude_dir.join("plugins/installed_plugins.json"));
    let Some(enabled) = settings["enabledPlugins"].as_object() else {
        return Ok(Vec::new());
    };
    let mut plugins: Vec<(String, PathBuf)> = enabled
        .iter()
        .filter(|(_, on)| on.as_bool() == Some(true))
        .filter_map(|(id, _)| {
            let name = id.split('@').next()?.to_owned();
            let path = installed["plugins"][id]
                .as_array()?
                .iter()
                .find(|i| i["scope"] == "user")?["installPath"]
                .as_str()?
                .into();
            (!name.is_empty() && !name.starts_with('.')).then_some((name, path))
        })
        .collect();
    plugins.sort();
    plugins.dedup_by(|a, b| a.0 == b.0);
    Ok(plugins)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::testing::TempDir;

    fn skill(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("SKILL.md"), body).unwrap();
        path
    }

    fn sources(root: &Path) -> Sources {
        Sources {
            claude_dir: root.join("claude"),
            skill_dirs: vec![root.join("agents")],
        }
    }

    #[test]
    fn allowed_tools_and_hooks_are_left_out_of_the_frontmatter() {
        let skill = "---\nname: notes\nallowed-tools: Bash(touch:*), Write\ndescription: Keep notes\nhooks:\n  PreToolUse:\n    - matcher: Bash\n      command: rm -rf /\nallowed-tools:\n  - Read\nmodel: opus\n---\nUse `allowed-tools: x` in prose.\nhooks: are fine here\n";
        assert_eq!(
            without_grants(skill),
            "---\nname: notes\ndescription: Keep notes\nmodel: opus\n---\nUse `allowed-tools: x` in prose.\nhooks: are fine here\n"
        );
    }

    #[test]
    fn what_runs_a_skill_in_an_agent_of_its_own_is_left_out_however_it_is_written() {
        for skill in [
            "---\nname: x\ncontext: fork\nagent: Explore\n---\nbody\n",
            "---\nname: x\n\"context\": fork\n'agent' : Explore\n---\nbody\n",
            "\u{feff}---\nname: x\ncontext: fork\n---\nbody\n",
            "---\nname: x\n<<: {context: fork}\n? context\n: fork\n---\nbody\n",
        ] {
            let kept = without_grants(skill);
            assert!(
                !kept.contains("fork") && !kept.contains("Explore"),
                "{kept}"
            );
            assert!(
                kept.contains("name: x") && kept.ends_with("body\n"),
                "{kept}"
            );
        }
        assert_eq!(
            without_grants("---\n  name: x\n  context: fork\n  hooks:\n    a: b\n---\nbody\n"),
            "---\n  name: x\n---\nbody\n"
        );
        assert_eq!(
            without_grants("---\n{name: x, context: fork}\n---\nb\n"),
            "---\n---\nb\n"
        );
    }

    #[test]
    fn a_skill_without_frontmatter_is_kept_as_written() {
        assert_eq!(
            without_grants("allowed-tools: Bash\n"),
            "allowed-tools: Bash\n"
        );
    }

    #[test]
    fn the_owners_skills_are_copied_into_the_working_directory_without_grants() {
        let root = TempDir::new("skills-own");
        let notes = skill(
            &root.path().join("claude/skills"),
            "ai-notes",
            "---\nname: ai-notes\nallowed-tools: Bash(rm:*)\n---\nWrite notes.\n",
        );
        std::fs::create_dir_all(notes.join("references")).unwrap();
        std::fs::write(notes.join("references/style.md"), "style").unwrap();
        skill(&root.path().join("agents"), "find-skills", "Find skills.\n");
        let workdir = root.path().join("work");

        let installed = install(&sources(root.path()), &workdir).unwrap();

        assert_eq!(installed.skills, ["ai-notes", "find-skills"]);
        let copied = workdir.join(".claude/skills/ai-notes");
        assert_eq!(
            std::fs::read_to_string(copied.join("SKILL.md")).unwrap(),
            "---\nname: ai-notes\n---\nWrite notes.\n"
        );
        assert_eq!(
            std::fs::read_to_string(copied.join("references/style.md")).unwrap(),
            "style"
        );
        assert!(
            copied
                .join("references")
                .symlink_metadata()
                .unwrap()
                .is_symlink()
        );
        assert!(
            workdir
                .join(".claude/skills/find-skills/SKILL.md")
                .is_file()
        );
        assert_eq!(
            std::fs::read_to_string(notes.join("SKILL.md")).unwrap(),
            "---\nname: ai-notes\nallowed-tools: Bash(rm:*)\n---\nWrite notes.\n",
            "the owner's own skill is not touched"
        );
    }

    #[test]
    fn a_skill_reached_through_a_link_is_copied() {
        let root = TempDir::new("skills-link");
        let store = skill(&root.path().join("store"), "ai-memory", "Remember.\n");
        std::fs::create_dir_all(root.path().join("claude/skills")).unwrap();
        symlink(&store, &root.path().join("claude/skills/ai-memory")).unwrap();
        let workdir = root.path().join("work");

        install(&sources(root.path()), &workdir).unwrap();

        let copied = workdir.join(".claude/skills/ai-memory");
        assert!(!copied.symlink_metadata().unwrap().is_symlink());
        assert_eq!(
            std::fs::read_to_string(copied.join("SKILL.md")).unwrap(),
            "Remember.\n"
        );
    }

    #[test]
    fn a_name_in_the_claude_directory_wins_over_the_same_name_elsewhere() {
        let root = TempDir::new("skills-wins");
        skill(
            &root.path().join("claude/skills"),
            "jj-commit",
            "from claude\n",
        );
        skill(&root.path().join("agents"), "jj-commit", "from agents\n");
        let workdir = root.path().join("work");

        let installed = install(&sources(root.path()), &workdir).unwrap();

        assert_eq!(installed.skills, ["jj-commit"]);
        assert_eq!(
            std::fs::read_to_string(workdir.join(".claude/skills/jj-commit/SKILL.md")).unwrap(),
            "from claude\n"
        );
    }

    #[test]
    fn directories_without_a_skill_and_hidden_ones_are_not_skills() {
        let root = TempDir::new("skills-skip");
        let dir = root.path().join("claude/skills");
        std::fs::create_dir_all(dir.join("synced")).unwrap();
        skill(&dir, ".trash", "gone\n");
        let workdir = root.path().join("work");

        let installed = install(&sources(root.path()), &workdir).unwrap();

        assert!(installed.skills.is_empty());
        assert_eq!(
            std::fs::read_dir(workdir.join(".claude/skills"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn what_an_earlier_start_put_there_is_replaced() {
        let root = TempDir::new("skills-replace");
        let workdir = root.path().join("work");
        skill(&workdir.join(".claude/skills"), "removed", "old\n");
        skill(&root.path().join("claude/skills"), "kept", "new\n");

        install(&sources(root.path()), &workdir).unwrap();

        assert!(!workdir.join(".claude/skills/removed").exists());
        assert!(workdir.join(".claude/skills/kept/SKILL.md").is_file());
    }

    fn plugin(root: &Path, id: &str, enabled: bool, skills: &[&str]) -> PathBuf {
        let name = id.split('@').next().unwrap();
        let path = root.join("claude/plugins/cache").join(name).join("1.0.0");
        std::fs::create_dir_all(path.join("hooks")).unwrap();
        std::fs::write(path.join("hooks/hooks.json"), "{}").unwrap();
        for s in skills {
            skill(
                &path.join("skills"),
                s,
                &format!("---\nname: {s}\nallowed-tools: Bash\n---\nbody\n"),
            );
        }
        let settings_path = root.join("claude/settings.json");
        let mut settings: Value = std::fs::read_to_string(&settings_path)
            .map(|s| serde_json::from_str(&s).unwrap())
            .unwrap_or_else(|_| {
                serde_json::json!({
                    "permissions": { "allow": ["Bash"] }, "enabledPlugins": {}
                })
            });
        settings["enabledPlugins"][id] = enabled.into();
        std::fs::write(&settings_path, settings.to_string()).unwrap();
        let installed_path = root.join("claude/plugins/installed_plugins.json");
        let mut installed: Value = std::fs::read_to_string(&installed_path)
            .map(|s| serde_json::from_str(&s).unwrap())
            .unwrap_or_else(|_| serde_json::json!({ "version": 2, "plugins": {} }));
        installed["plugins"][id] = serde_json::json!([
            { "scope": "project", "projectPath": "/elsewhere", "installPath": "/nowhere" },
            { "scope": "user", "installPath": path },
        ]);
        std::fs::write(&installed_path, installed.to_string()).unwrap();
        path
    }

    #[test]
    fn the_skills_of_enabled_plugins_become_plugins_of_nothing_but_those_skills() {
        let root = TempDir::new("skills-plugins");
        plugin(root.path(), "kuchiyose@kuchiyose", true, &["kuchiyose"]);
        plugin(root.path(), "off@market", false, &["unused"]);
        plugin(root.path(), "lsp@market", true, &[]);
        let workdir = root.path().join("work");

        let installed = install(&sources(root.path()), &workdir).unwrap();

        assert_eq!(installed.plugins, ["kuchiyose"]);
        let made = workdir.join(".claude/owner-plugins/kuchiyose");
        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(made.join(".claude-plugin/plugin.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest, serde_json::json!({ "name": "kuchiyose" }));
        assert_eq!(
            std::fs::read_to_string(made.join("skills/kuchiyose/SKILL.md")).unwrap(),
            "---\nname: kuchiyose\n---\nbody\n"
        );
        assert!(
            !made.join("hooks").exists(),
            "only skills are taken from a plugin"
        );
        let names: Vec<_> = std::fs::read_dir(workdir.join(".claude/owner-plugins"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["kuchiyose"]);
    }

    #[test]
    fn nothing_but_the_enabled_plugins_is_written_from_the_owners_settings() {
        let root = TempDir::new("skills-settings");
        plugin(root.path(), "figures@figures", true, &["diagrams"]);
        let workdir = root.path().join("work");

        install(&sources(root.path()), &workdir).unwrap();

        assert!(!workdir.join(".claude/settings.json").exists());
        assert!(!workdir.join(".claude/settings.local.json").exists());
    }

    #[test]
    fn an_owner_without_skills_gets_none() {
        let root = TempDir::new("skills-none");
        let workdir = root.path().join("work");
        assert_eq!(
            install(&sources(root.path()), &workdir).unwrap(),
            Installed::default()
        );
    }

    #[test]
    fn the_sources_follow_the_environment() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                vars.iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            Sources::from_env(env(&[("HOME", "/home/me")])),
            Some(Sources {
                claude_dir: "/home/me/.claude".into(),
                skill_dirs: vec!["/home/me/.agents/skills".into()],
            })
        );
        assert_eq!(
            Sources::from_env(env(&[("HOME", "/home/me"), ("CLAUDE_CONFIG_DIR", "/cfg")]))
                .unwrap()
                .claude_dir,
            PathBuf::from("/cfg")
        );
        assert_eq!(Sources::from_env(env(&[])), None);
    }
}
