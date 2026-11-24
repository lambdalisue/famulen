//! `famulen-notebook`: read and write a notebook by its rules, for people and
//! AI alike.

mod args;

use std::path::{Component, Path};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser as _;
use famulen_notebook::body::{Value, split_commas};
use famulen_notebook::check::{self, Severity};
use famulen_notebook::index::Index;
use famulen_notebook::memory::{self, Layer, LongGroup};
use famulen_notebook::records::{METADATA, Names, Record, Records, record_of};
use famulen_notebook::search::Query;
use famulen_notebook::sync::{self, Sync, Trouble};
use famulen_notebook::{Author, Doc, Entry, EpisodeState, MEMORY, Notebook, Signature};
use serde_json::json;

use args::{Cli, Command, SearchArgs, Sign};

const NAME: &str = env!("CARGO_PKG_NAME");
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn parse_author(text: &str) -> anyhow::Result<Author> {
    let (name, rest) = text
        .split_once('<')
        .with_context(|| format!("expected `Name <email>`, not {text:?}"))?;
    let email = rest
        .strip_suffix('>')
        .with_context(|| format!("expected `Name <email>`, not {text:?}"))?;
    Ok(Author::new(name.trim(), email.trim()))
}

fn git_user(root: &Path) -> Option<Author> {
    let get = |key: &str| {
        std::process::Command::new("git")
            .args(["config", key])
            .current_dir(root)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .filter(|s| !s.is_empty())
    };
    Some(Author::new(get("user.name")?, get("user.email")?))
}

impl Sign {
    fn signature(&self, root: &Path) -> anyhow::Result<Signature> {
        let author = match &self.author {
            Some(text) => parse_author(text)?,
            None => {
                git_user(root).context("give --author, or set Git's user.name and user.email")?
            }
        };
        Ok(Signature {
            author,
            co_authors: self
                .co_authors
                .iter()
                .map(|a| parse_author(a))
                .collect::<anyhow::Result<_>>()?,
            agent: self.agent.clone(),
            model: self.model.clone(),
            sources: self.sources.clone(),
        })
    }
}

/// Write a field from the command line. A value in `[a, b]` is a list, and
/// an empty one removes the field.
fn put(record: &mut Record, name: &str, text: &str) -> anyhow::Result<()> {
    let list = text
        .strip_prefix('[')
        .filter(|t| !t.starts_with('['))
        .and_then(|t| t.strip_suffix(']'))
        .map(split_commas);
    if METADATA.contains(&name) {
        match list {
            Some(items) => record.meta.set_list(name, &items),
            None => record
                .meta
                .set_opt(name, Some(text).filter(|t| !t.is_empty())),
        }
        return Ok(());
    }
    let value = list.map_or_else(|| Value::One(text.to_owned()), Value::Many);
    match name.split_once('/') {
        Some((section, name)) => record.body.set_in(section, name, value),
        None => record.body.set(name, value),
    }
}

/// A memory not yet written, with an id of its own.
fn new_entry(dir: &Path, text: &str, attrs: &[(String, String)]) -> anyhow::Result<Entry> {
    let mut entry = Entry::new(text).with("id", memory::fresh_id(dir)?)?;
    for (k, v) in attrs {
        entry.set(k, v.as_str())?;
    }
    Ok(entry)
}

fn print_json(value: &serde_json::Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("famulen-notebook: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// How long what was read from the shared notebook is taken as new.
const FRESH: Duration = Duration::from_secs(60);

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    if let Command::Clone { url, directory } = &cli.command {
        sync::clone(url, directory)?;
        return Ok(ExitCode::SUCCESS);
    }
    let notebook = Notebook::new(&cli.notebook).with_tool(NAME, VERSION);
    let Some(sync) = Sync::of(&notebook) else {
        anyhow::ensure!(
            !matches!(cli.command, Command::Sync),
            "not a copy made by `famulen-notebook clone`"
        );
        return here(cli, notebook);
    };
    match touches(&cli.command) {
        Touches::Nothing => here(cli, notebook),
        Touches::Reading => {
            warn(sync.refresh(FRESH));
            here(cli, notebook)
        }
        Touches::Shared => {
            sync.sync()?;
            Ok(ExitCode::SUCCESS)
        }
        Touches::Writing => {
            let away = warn(sync.pull());
            let code = here(cli, notebook)?;
            if !away {
                let sent = sync
                    .push()
                    .context("it is written and committed here, but not sent");
                // Tried again by whoever reads a failure, it would be written twice.
                if is_away(&sent) {
                    warn(sent);
                } else {
                    sent?;
                }
            }
            Ok(code)
        }
    }
}

/// Say what went wrong with a copy, which is used as it is. Whether the
/// shared notebook is away.
fn warn(synced: anyhow::Result<()>) -> bool {
    if let Err(error) = &synced {
        eprintln!("famulen-notebook: warning: {error:#}");
    }
    is_away(&synced)
}

fn is_away(synced: &anyhow::Result<()>) -> bool {
    synced
        .as_ref()
        .is_err_and(|error| error.downcast_ref::<Trouble>().is_some_and(Trouble::passes))
}

/// What a command has to do with the shared notebook.
enum Touches {
    Nothing,
    Reading,
    Writing,
    Shared,
}

fn touches(command: &Command) -> Touches {
    match command {
        Command::Clone { .. } | Command::Index | Command::Use { .. } => Touches::Nothing,
        Command::Sync => Touches::Shared,
        Command::Check { fix: false, .. }
        | Command::Search(_)
        | Command::Get { .. }
        | Command::List { .. }
        | Command::Links { .. } => Touches::Reading,
        Command::Init
        | Command::Check { fix: true, .. }
        | Command::Add { .. }
        | Command::Set { .. }
        | Command::Rm { .. }
        | Command::Log { .. }
        | Command::Remember { .. }
        | Command::Keep { .. }
        | Command::Commit { .. } => Touches::Writing,
    }
}

/// Do a command on the notebook as it is here.
fn here(cli: Cli, notebook: Notebook) -> anyhow::Result<ExitCode> {
    let _hold = match touches(&cli.command) {
        Touches::Writing => Some(notebook.hold()?),
        _ => None,
    };
    let app = App {
        records: Records::new(notebook.clone()),
        notebook,
        sign: cli.sign,
    };
    match cli.command {
        Command::Clone { .. } | Command::Sync => {}
        Command::Init => app.notebook.ensure(&app.signature()?)?,
        Command::Check { paths, fix, json } => return app.check(&paths, fix, json),
        Command::Search(args) => app.search(args)?,
        Command::Index => Index::open(&app.notebook)?.refresh()?,
        Command::Get { path } => app.get(&path)?,
        Command::List { kind: None, json } => app.list_kinds(json)?,
        Command::List {
            kind: Some(kind),
            json,
        } => app.list(&kind, json)?,
        Command::Add {
            kind,
            title,
            text,
            fields,
        } => app.add(&kind, &title, &text, &fields)?,
        Command::Set {
            kind,
            id,
            title,
            text,
            fields,
            unset,
        } => {
            let mut record = app.named(&kind, &id)?;
            if let Some(title) = title {
                record.body.set_title(&title)?;
            }
            if let Some(text) = text {
                record.body.set_notes(&text)?;
            }
            for (k, v) in &fields {
                put(&mut record, k, v)?;
            }
            for k in &unset {
                put(&mut record, k, "")?;
            }
            app.write(&record, "を直した")?;
        }
        Command::Links { kind, id, json } => app.links(&kind, &id, json)?,
        Command::Rm { kind, id } => {
            let record = app.named(&kind, &id)?;
            let subject = format!("{} を消した", record.title());
            app.records
                .remove(&app.signature()?, &kind, &record.id, &subject)?;
        }
        Command::Log {
            kind,
            id,
            text,
            who,
        } => {
            let mut record = app.named(&kind, &id)?;
            let at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
            record.body.log(&at, &who, &text)?;
            app.write(&record, "の経過を書いた")?;
        }
        Command::Remember { layer, text, attrs } => app.remember(&layer, &text, &attrs)?,
        Command::Keep {
            group,
            name,
            summary,
            text,
            attrs,
        } => app.keep(&group, &name, &summary, &text, &attrs)?,
        Command::Use { id } => {
            Index::open(&app.notebook)?.record_use(&id, chrono::Local::now().date_naive())?
        }
        Command::Commit { message, paths } => app.commit(&message, paths)?,
    }
    Ok(ExitCode::SUCCESS)
}

struct App {
    notebook: Notebook,
    records: Records,
    sign: Sign,
}

impl App {
    fn signature(&self) -> anyhow::Result<Signature> {
        self.sign.signature(self.notebook.root())
    }

    /// The record an id or a code names.
    fn named(&self, kind: &str, text: &str) -> anyhow::Result<Record> {
        self.records
            .resolve(kind, text)?
            .map(|id| self.records.get(kind, &id))
            .transpose()?
            .flatten()
            .with_context(|| format!("no record {kind}/{text}"))
    }

    fn write(&self, record: &Record, did: &str) -> anyhow::Result<()> {
        let subject = format!("{} {did}", record.title());
        self.records.write(&self.signature()?, record, &subject)
    }

    fn check(&self, paths: &[String], fix: bool, json: bool) -> anyhow::Result<ExitCode> {
        if fix {
            let changed = check::fix(&self.notebook, paths)?;
            if !changed.is_empty() {
                self.notebook
                    .commit(&self.signature()?, "手帳の書き方を直した", &changed)?;
            }
        }
        let findings = if paths.is_empty() {
            check::check(&self.notebook)?
        } else {
            check::check_files(&self.notebook, paths)?
        };
        if json {
            let out: Vec<_> = findings
                .iter()
                .map(|f| {
                    json!({
                        "path": f.path, "line": f.line, "rule": f.rule,
                        "severity": f.severity.as_str(), "why": f.why,
                    })
                })
                .collect();
            print_json(&json!(out))?;
        } else {
            for f in &findings {
                let severity = f.severity.as_str();
                println!("{}:{}: {severity}[{}] {}", f.path, f.line, f.rule, f.why);
            }
        }
        Ok(if findings.iter().any(|f| f.severity == Severity::Error) {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        })
    }

    fn search(&self, args: SearchArgs) -> anyhow::Result<()> {
        let hits = Index::open(&self.notebook)?.search(&Query {
            text: args.words.join(" "),
            scopes: args.scopes,
            workgroup: args.workgroup,
            fields: args.fields,
            from: args.from,
            to: args.to,
            forgotten: args.forgotten,
            limit: args.limit,
        })?;
        if args.json {
            let out: Vec<_> = hits
                .iter()
                .map(|h| {
                    json!({
                        "path": h.path, "scope": h.scope, "id": h.id,
                        "title": h.title, "snippet": h.snippet, "score": h.score,
                    })
                })
                .collect();
            return print_json(&json!(out));
        }
        for h in &hits {
            println!("{}#{}\t{}\n\t{}", h.path, h.id, h.title, h.snippet);
        }
        Ok(())
    }

    fn get(&self, path: &str) -> anyhow::Result<()> {
        let inside = Path::new(path).components().all(|c| match c {
            Component::Normal(name) => name != ".git",
            Component::CurDir => true,
            _ => false,
        });
        anyhow::ensure!(inside, "not a file of the notebook: {path}");
        print!(
            "{}",
            std::fs::read_to_string(self.notebook.root().join(path))?
        );
        Ok(())
    }

    fn list_kinds(&self, json: bool) -> anyhow::Result<()> {
        let kinds = self.records.kind_names()?;
        if json {
            return print_json(&json!(kinds));
        }
        for kind in kinds {
            println!("{kind}");
        }
        Ok(())
    }

    fn list(&self, kind: &str, json: bool) -> anyhow::Result<()> {
        let records = self.records.list(kind)?;
        let codes = self.records.codes(kind)?;
        let code = |id: &str| codes.get(id).map(String::as_str).unwrap_or_default();
        if json {
            let out: Vec<_> = records
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id, "code": code(&r.id),
                        "title": r.title(), "path": r.rel(),
                    })
                })
                .collect();
            return print_json(&json!(out));
        }
        for r in &records {
            println!("{}\t{}", code(&r.id), r.title());
        }
        Ok(())
    }

    fn links(&self, kind: &str, id: &str, json: bool) -> anyhow::Result<()> {
        let record = self.named(kind, id)?;
        let to_it = Index::open(&self.notebook)?.backlinks(kind, &record.id)?;
        let title = |target: &str| {
            let (kind, id) = record_of(target)?;
            Some(self.records.get(kind, id).ok()??.title())
        };
        let out: Vec<_> = record
            .links()
            .into_iter()
            .map(|(from, target)| (from, title(&target), target))
            .collect();
        let inward: Vec<_> = to_it
            .records
            .into_iter()
            .map(|b| (b.from, b.title, format!("records/{}/{}", b.kind, b.id)))
            .chain(to_it.memories.into_iter().map(|m| {
                let path = m.path.trim_end_matches(".md").to_owned();
                (String::new(), m.title, path)
            }))
            .collect();
        if json {
            return print_json(&json!({
                "links": out.iter().map(|(from, title, target)| json!({
                    "from": from, "title": title, "target": target,
                })).collect::<Vec<_>>(),
                "backlinks": inward.iter().map(|(from, title, source)| json!({
                    "from": from, "title": title, "source": source,
                })).collect::<Vec<_>>(),
            }));
        }
        for (from, title, target) in &out {
            let title = title.as_deref().unwrap_or_default();
            println!("->\t{target}\t{title}\t{from}");
        }
        for (from, title, source) in &inward {
            println!("<-\t{source}\t{title}\t{from}");
        }
        Ok(())
    }

    /// A memory's text with its links in the form they are kept in.
    fn linked(&self, text: &str) -> anyhow::Result<String> {
        Names::new(self.records.clone()).canonical_text(text, "")
    }

    fn add(
        &self,
        kind: &str,
        title: &str,
        text: &str,
        fields: &[(String, String)],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.records.kind_names()?.iter().any(|k| k == kind),
            "no kind {kind}; add the directory records/{kind}/ first"
        );
        let mut record = self.records.draft(kind, title)?;
        for (k, v) in fields {
            put(&mut record, k, v)?;
        }
        record.body.set_notes(text)?;
        self.write(&record, "を足した")?;
        println!("{}", record.id);
        Ok(())
    }

    fn remember(&self, layer: &str, text: &str, attrs: &[(String, String)]) -> anyhow::Result<()> {
        // Signed first, so that nothing is written that cannot be committed.
        let signature = self.signature()?;
        let text = &self.linked(text)?;
        let dir = self.notebook.memory_dir();
        let (rel, id) = match layer {
            "episodic" => {
                let now = chrono::Local::now().naive_local();
                let entry = memory::append_episode(&dir, now, text, EpisodeState::Waiting)?;
                let id = entry.id().unwrap_or_default().to_owned();
                for (k, v) in attrs {
                    memory::tag_episode(&dir, now.date(), &id, k, v)?;
                }
                (format!("{}/{}.md", memory::EPISODIC, now.date()), id)
            }
            "core" | "pending" => {
                let rel = format!("{layer}.md");
                let entry = new_entry(&dir, text, attrs)?;
                let id = entry.id().unwrap_or_default().to_owned();
                let mut doc = memory::doc_to_add_to(&dir, &rel)?;
                doc.push(entry)?;
                doc.save(&dir.join(&rel))?;
                (rel, id)
            }
            other => anyhow::bail!("remember goes in core, pending or episodic, not {other}"),
        };
        self.notebook.commit(
            &signature,
            &format!("{rel} に書き留めた"),
            &[format!("{MEMORY}/{rel}")],
        )?;
        println!("{id}");
        Ok(())
    }

    fn keep(
        &self,
        group: &str,
        name: &str,
        summary: &str,
        text: &str,
        attrs: &[(String, String)],
    ) -> anyhow::Result<()> {
        // Signed first, so that nothing is written that cannot be committed.
        let signature = self.signature()?;
        let group = LongGroup::from_dir_name(group).with_context(|| {
            format!("a long-term group is semantic, procedural or prospective, not {group}")
        })?;
        let rel = format!("{}/{}/{name}.md", memory::LONG, group.dir_name());
        anyhow::ensure!(
            Layer::of(&rel).is_some(),
            "not a long-term memory file: {rel}"
        );
        let dir = self.notebook.memory_dir();
        let path = dir.join(&rel);
        anyhow::ensure!(!path.exists(), "{rel} already holds a memory");
        let summary = self.linked(summary)?;
        let text = match text.trim() {
            "" => summary.clone(),
            _ => self.linked(text)?,
        };
        let entry = new_entry(&dir, &summary, attrs)?;
        let id = entry.id().unwrap_or_default().to_owned();
        let mut doc = Doc::new_long();
        doc.set_long_text(&text)?;
        doc.push(entry)?;
        doc.save(&path)?;
        self.notebook.commit(
            &signature,
            &format!("{rel} を覚えた"),
            &[format!("{MEMORY}/{rel}")],
        )?;
        println!("{id}");
        Ok(())
    }

    fn commit(&self, message: &str, mut paths: Vec<String>) -> anyhow::Result<()> {
        if paths.is_empty() {
            paths.push(".".to_owned());
        }
        if self
            .notebook
            .commit(&self.signature()?, message, &paths)?
            .is_none()
        {
            eprintln!("nothing to commit");
        }
        Ok(())
    }
}
