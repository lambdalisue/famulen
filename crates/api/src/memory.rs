//! What the secretary has learned, in layers of Markdown kept in Git.

use chrono::Utc;
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::dream_settings::{self as ds, DreamSettings as Plan, Frequency};
use famulen_core::dreaming;
use famulen_core::judging;
use famulen_core::memory::{self as m, Entry, EpisodeState, Src};
use famulen_core::memory::{Layer as Place, Snapshot};
use famulen_core::note_origin::{Origin, Told};
use famulen_core::notebook::{self, CommitInfo, Writer};
use famulen_core::workgroups::ALL;

use crate::pb::*;
use crate::records::Names;
use crate::{Api, OrFail, changed, fail, given, writer};

const LAYERS: [(&str, &str, &str); 7] = [
    ("dream", "夢", "夜の整理と、その結果"),
    ("pending", "保留中の判断", "急がない。根拠を待っている"),
    ("episodic", "気づき", "雑に書く受信箱。夢が整理したら消える"),
    ("working", "作業記憶", "いま進めている仕事の途中経過"),
    ("core", "心得", "毎回の会話で必ず読む"),
    ("long", "長期記憶", "意味記憶・手続き記憶・展望記憶"),
    ("forgotten", "忘却", "古くなった記憶。戻せる"),
];

fn item(e: &Entry, file: &str) -> MemoryItem {
    MemoryItem {
        id: e.id().unwrap_or_default().to_owned(),
        text: e.body().to_owned(),
        src: e.src().map(|s| s.as_str().to_owned()).unwrap_or_default(),
        attrs: e
            .attrs()
            .iter()
            .filter(|(k, _)| k != "id" && k != "src")
            .map(|(k, v)| (k.clone(), v.to_text()))
            .collect(),
        file: file.to_owned(),
        time: e
            .time()
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_default(),
        ..Default::default()
    }
}

fn origin(told: Told) -> NoteOrigin {
    let kind = match told.origin {
        Some(Origin::Mail { .. }) => "mail",
        Some(Origin::Slack { .. }) => "slack",
        None => "",
    };
    NoteOrigin {
        kind: kind.into(),
        account: told.account,
        channel: told.channel,
        from: told.from,
        subject: told.subject,
        at: told.at.unwrap_or_default(),
        url: told.url,
        excerpt: told.excerpt,
        outcome: told
            .outcome
            .map(|o| o.label().to_owned())
            .unwrap_or_default(),
        reason: told.reason,
        ..Default::default()
    }
}

fn notebook_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The memories an item names, and those that replaced it.
fn related(all: &Snapshot, entry: &Entry) -> Vec<RelatedMemory> {
    let id = entry.id().unwrap_or_default();
    let named = entry
        .list("supersedes")
        .into_iter()
        .map(|id| ("supersedes", id))
        .chain(entry.against().into_iter().map(|id| ("against", id)));
    let replaced = all
        .entries
        .iter()
        .filter(|(_, at)| !id.is_empty() && at.entry.list("supersedes").iter().any(|s| s == id))
        .map(|(id, _)| ("superseded", id.clone()));
    named
        .chain(replaced)
        .filter_map(|(how, id)| {
            let at = all.entries.get(&id)?;
            Some(RelatedMemory {
                how: how.into(),
                item: item(&at.entry, &notebook_path(&at.file)).into(),
                ..Default::default()
            })
        })
        .collect()
}

/// How many commits of an item or a file are shown.
const HISTORY: usize = 30;
/// How far back the commit that made an item is looked for.
const LONGEST: usize = 2000;

fn file_change(c: CommitInfo) -> FileChange {
    FileChange {
        commit: c.id.chars().take(8).collect(),
        at: c.time.timestamp(),
        author: c.author.name,
        message: c.message,
        ..Default::default()
    }
}

/// What a line of the history holding an item's id looks like, be it in an
/// item's attributes or in the frontmatter of a file.
fn line_of(id: &str) -> String {
    let id: String = id
        .chars()
        .flat_map(|c| {
            let plain = c.is_alphanumeric() || c == '-' || c == '_';
            (!plain).then_some('\\').into_iter().chain([c])
        })
        .collect();
    format!(r#"(^|[{{ ,])"?id"?: *"?{id}"?([,}} ]|$)"#)
}

fn op_name(op: m::Op) -> String {
    serde_json::to_value(op)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Refuse anyone but the owner.
fn owner_only(ctx: &RequestContext) -> anyhow::Result<()> {
    anyhow::ensure!(
        writer(ctx) == Writer::Owner,
        "only the owner decides on a pending judgment"
    );
    Ok(())
}

pub(crate) fn dream(n: famulen_core::dreaming::Night) -> Dream {
    let c = &n.summary.counts;
    Dream {
        id: n.id,
        at: n.at,
        status: n.status,
        commit: n.commit.unwrap_or_default(),
        added: c.added as i32,
        edited: c.edited as i32,
        replaced: c.replaced as i32,
        archived: c.archived as i32,
        confirmed: c.confirmed as i32,
        pending_added: c.pending_added as i32,
        task_made: c.task_made as i32,
        doubted: c.doubted as i32,
        discarded: c.discarded as i32,
        changes: n
            .summary
            .changes
            .iter()
            .map(|ch| DreamChange {
                op: op_name(ch.op),
                id: ch.id.clone(),
                file: ch.file.clone(),
                text: ch.text.clone(),
                reason: ch.reason.clone().unwrap_or_default(),
                previous: ch.previous.clone().unwrap_or_default(),
                from_file: ch.from_file.clone().unwrap_or_default(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

impl Api {
    fn dream_settings(&self) -> anyhow::Result<DreamSettings> {
        let plan = Plan::load(&self.app.store)?;
        let (frequency, weekday) = match plan.frequency {
            Frequency::Nightly => (DreamFrequency::DREAM_FREQUENCY_NIGHTLY, 0),
            Frequency::EveryOtherDay => (DreamFrequency::DREAM_FREQUENCY_EVERY_OTHER_DAY, 0),
            Frequency::Weekly { weekday } => (DreamFrequency::DREAM_FREQUENCY_WEEKLY, weekday),
        };
        Ok(DreamSettings {
            at: format!("{:02}:{:02}", plan.hour, plan.minute),
            frequency: frequency.into(),
            weekday: weekday as i32,
            early_at: plan.early_at as i32,
            core_lines: plan.core_lines as i32,
            next: self.next_dream()?,
            tidy_records: Some(plan.tidy_records),
            rest_after: Some(plan.rest_after as i32),
            ..Default::default()
        })
    }

    /// When the scheduled dream runs next, in words.
    fn next_dream(&self) -> anyhow::Result<String> {
        let plan = Plan::load(&self.app.store)?;
        let now = Utc::now().with_timezone(&self.app.tz()).naive_local();
        Ok(plan.next_in_words(ds::last(&self.app.store)?, now))
    }

    fn layer_count(&self, id: &str) -> anyhow::Result<String> {
        let dir = &self.app.memory_dir;
        Ok(match id {
            "pending" => m::pending_entries(dir)?.len().to_string(),
            "episodic" => dreaming::waiting_notes(dir)?.len().to_string(),
            "working" => m::working_memories(dir)?.len().to_string(),
            "core" => format!(
                "{} / {} 行",
                m::core_entries(dir)?.len(),
                Plan::load(&self.app.store)?.core_lines
            ),
            "long" => m::long_term(dir)?
                .iter()
                .map(|g| g.files.len())
                .sum::<usize>()
                .to_string(),
            "forgotten" => m::forgotten_months(dir)?.len().to_string(),
            _ => String::new(),
        })
    }

    /// A pending judgment, with the answer on it that waits.
    fn pending_item(&self, e: &Entry) -> anyhow::Result<MemoryItem> {
        let mut out = item(e, m::PENDING);
        let waiting = match e.id() {
            Some(id) => judging::waiting_answer(&self.app.memory_dir, id)?,
            None => None,
        };
        if let Some((date, note)) = waiting {
            let shown = item(&note, "");
            out.answer = PendingAnswer {
                adopted: note.get(judging::VERDICT) == Some(judging::ADOPTED),
                words: judging::words(e, &note).to_owned(),
                note: shown.id,
                date: date.to_string(),
                time: shown.time,
                ..Default::default()
            }
            .into();
        }
        Ok(out)
    }

    fn memory_detail(&self, id: &str, path: &str) -> anyhow::Result<MemoryDetail> {
        let dir = &self.app.memory_dir;
        let file = std::path::Path::new(path);
        let found = Snapshot::take(dir)?;
        let (file, entry) = match found.entries.get(id) {
            Some(at) => (at.file.clone(), at.entry.clone()),
            None => {
                anyhow::ensure!(
                    id.is_empty() && matches!(Place::of(file), Some(Place::Long(_))),
                    "no memory {id}{path}"
                );
                // A missing file reads as an empty one.
                m::read_file(dir, file)?;
                let entry = m::read_doc(dir, file)?.entries().next().cloned();
                (file.to_owned(), entry.unwrap_or_else(|| Entry::new("")))
            }
        };
        let rel = notebook_path(&file);
        let place = Place::of(&file);
        let (layer, group) = match place {
            Some(Place::Long(group)) => ("long", group.dir_name()),
            Some(Place::Core) => ("core", ""),
            Some(Place::Pending) => ("pending", ""),
            _ => ("forgotten", ""),
        };
        let prose = m::read_doc(dir, &file)?
            .long_text()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let id = entry.id().unwrap_or_default();
        let history = self.history_of(id, (layer == "long").then_some(rel.as_str()))?;
        Ok(MemoryDetail {
            item: match place {
                Some(Place::Pending) => self.pending_item(&entry)?,
                _ => item(&entry, &rel),
            }
            .into(),
            layer: layer.into(),
            group: group.into(),
            nights: self.nights_of(id)?,
            created: history.last().cloned().into(),
            history: history.into_iter().take(HISTORY).collect(),
            related: related(&found, &entry),
            links: self.links_of(&entry, &prose),
            prose,
            ..Default::default()
        })
    }

    /// What the dreams did with an item, newest first.
    fn nights_of(&self, id: &str) -> anyhow::Result<Vec<MemoryNight>> {
        let nights = match id {
            "" => Vec::new(),
            _ => dreaming::dreams_of(&self.app.store, id)?,
        };
        Ok(nights
            .into_iter()
            .flat_map(|night| {
                let (dream, at) = (night.id, night.at);
                night
                    .summary
                    .changes
                    .into_iter()
                    .filter(|c| c.id == id)
                    .map(move |c| MemoryNight {
                        dream,
                        at,
                        op: op_name(c.op),
                        reason: c.reason.unwrap_or_default(),
                        previous: c.previous.unwrap_or_default(),
                        basis: c
                            .basis
                            .into_iter()
                            .map(|b| MemoryBasis {
                                id: b.id,
                                date: b.date,
                                time: b.time,
                                text: b.text,
                                src: b.src,
                                origin: b.origin,
                                ..Default::default()
                            })
                            .collect(),
                        ..Default::default()
                    })
            })
            .collect())
    }

    /// The commits that changed an item, wherever it was kept, and those that
    /// changed the file it is.
    fn history_of(&self, id: &str, file: Option<&str>) -> anyhow::Result<Vec<FileChange>> {
        let dir = &self.app.memory_dir;
        let mut commits = match file {
            Some(file) => notebook::file_log(dir, file, LONGEST)?,
            None => Vec::new(),
        };
        if !id.is_empty() {
            let kept = [m::CORE, m::PENDING, m::LONG, m::FORGOTTEN];
            for c in notebook::line_log(dir, &line_of(id), &kept, LONGEST)? {
                if commits.iter().all(|known| known.id != c.id) {
                    commits.push(c);
                }
            }
        }
        commits.sort_by_key(|c| std::cmp::Reverse(c.time));
        Ok(commits.into_iter().map(file_change).collect())
    }

    /// The records a memory links to or is tagged with, each once.
    fn links_of(&self, entry: &Entry, prose: &str) -> Vec<RecordLink> {
        let mut targets = m::links(entry.text());
        targets.extend(m::links(prose));
        if let Some(party) = entry.get("party") {
            targets.push(format!("records/parties/{party}"));
        }
        let mut names = Names::default();
        let mut seen = std::collections::BTreeSet::new();
        targets
            .into_iter()
            .filter(|t| seen.insert(t.clone()))
            .filter_map(|target| {
                let (kind, id) = target.strip_prefix("records/")?.split_once('/')?;
                let records = &self.app.records;
                let link = names.link(records, kind, id, String::new());
                Some(link.unwrap_or_else(|| RecordLink {
                    kind: kind.into(),
                    id: id.into(),
                    missing: true,
                    ..Default::default()
                }))
            })
            .collect()
    }

    /// Refuse to take back what a running dream may have read.
    fn not_dreaming(&self) -> anyhow::Result<()> {
        let dreaming = self.app.store.state(famulen_core::runtime::DREAMING_KEY)?;
        anyhow::ensure!(
            dreaming.is_none(),
            "a dream is reading the answer now; try again once it is over"
        );
        Ok(())
    }

    /// Commit what others wrote before the owner's decision is written.
    fn commit_others(&self) -> anyhow::Result<()> {
        let dir = &self.app.memory_dir;
        notebook::commit_all(dir, &notebook::famulen(), "その日の記憶")?;
        Ok(())
    }

    /// Commit what the owner decided.
    fn commit_judged(&self, message: &str, days: &[chrono::NaiveDate]) -> anyhow::Result<()> {
        let mut paths = vec![
            m::PENDING.to_owned(),
            m::CORE.to_owned(),
            m::FORGOTTEN.to_owned(),
        ];
        paths.extend(days.iter().map(|d| format!("{}/{d}.md", m::EPISODIC)));
        notebook::commit_paths(&self.app.memory_dir, &notebook::owner(), message, &paths)?;
        changed(self);
        Ok(())
    }

    fn memory_layer(&self, id: &str) -> anyhow::Result<MemoryLayer> {
        let (id, label, help) = LAYERS
            .iter()
            .find(|(l, _, _)| *l == id)
            .ok_or_else(|| anyhow::anyhow!("no layer {id}"))?;
        Ok(MemoryLayer {
            id: (*id).into(),
            label: (*label).into(),
            help: (*help).into(),
            count: self.layer_count(id)?,
            ..Default::default()
        })
    }
}

impl MemoryService for Api {
    async fn list_layers(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListLayersResponse> {
        let layers = LAYERS
            .iter()
            .map(|(id, _, _)| self.memory_layer(id))
            .collect::<anyhow::Result<Vec<_>>>()
            .or_fail()?;
        Response::ok(ListLayersResponse {
            layers,
            ..Default::default()
        })
    }

    async fn get_layer(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetLayerRequest>,
    ) -> ServiceResult<Layer> {
        let r = request.to_owned_message();
        let dir = &self.app.memory_dir;
        let mut out = Layer {
            layer: self.memory_layer(&r.layer).or_fail()?.into(),
            ..Default::default()
        };
        match r.layer.as_str() {
            "pending" => {
                out.items = m::pending_entries(dir)
                    .or_fail()?
                    .iter()
                    .map(|e| self.pending_item(e))
                    .collect::<anyhow::Result<_>>()
                    .or_fail()?;
            }
            "core" => {
                out.items = m::core_entries(dir)
                    .or_fail()?
                    .iter()
                    .map(|e| item(e, "core.md"))
                    .collect();
            }
            "episodic" => {
                let mut notes = dreaming::waiting_notes(dir).or_fail()?;
                // The newest day first, each day in the order it was written.
                notes.sort_by_key(|(date, _)| std::cmp::Reverse(*date));
                out.items = notes
                    .iter()
                    .map(|(date, e)| item(e, &format!("{}/{date}.md", m::EPISODIC)))
                    .collect();
            }
            "working" => {
                out.working = m::working_memories(dir)
                    .or_fail()?
                    .into_iter()
                    .map(|w| WorkingMemory {
                        name: w.name,
                        title: w.title,
                        done: w.done,
                        next: w.next,
                        waiting: w.waiting,
                        clues: w.clues,
                        ..Default::default()
                    })
                    .collect();
            }
            "long" => {
                out.files = m::long_term(dir)
                    .or_fail()?
                    .into_iter()
                    .flat_map(|g| g.files)
                    .map(|f| {
                        let path = f.path.to_string_lossy().into_owned();
                        MemoryFile {
                            entries: f.entries.iter().map(|e| item(e, &path)).collect(),
                            title: if f.summary.is_empty() {
                                f.name
                            } else {
                                f.summary
                            },
                            items: f.entries.len() as i32,
                            path,
                            ..Default::default()
                        }
                    })
                    .collect();
            }
            "forgotten" => {
                let months = m::forgotten_months(dir).or_fail()?;
                let month = given(&r.at).map(str::to_owned).or(months.last().cloned());
                if let Some(month) = month {
                    let file = format!("forgotten/{month}.md");
                    out.items = m::forgotten(dir, &month)
                        .or_fail()?
                        .iter()
                        .map(|e| item(e, &file))
                        .collect();
                }
                out.periods = months.into_iter().rev().collect();
            }
            _ => {}
        }
        Response::ok(out)
    }

    async fn get_note_origin(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<NoteOrigin> {
        let id = request.id.to_owned();
        if id.contains('/') {
            return Response::ok(origin(self.app.told_of_source(&id).await.or_fail()?));
        }
        let notes = dreaming::waiting_notes(&self.app.memory_dir).or_fail()?;
        let Some((_, note)) = notes.iter().find(|(_, n)| n.id() == Some(id.as_str())) else {
            return Err(fail(format!("no note {id} waits in the inbox")));
        };
        let told = match note.get(Origin::ATTR).and_then(Origin::parse) {
            Some(origin) => self.app.told_of(origin).await.or_fail()?,
            None => Told::default(),
        };
        Response::ok(origin(told))
    }

    async fn get_memory(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetMemoryRequest>,
    ) -> ServiceResult<MemoryDetail> {
        Response::ok(self.memory_detail(request.id, request.path).or_fail()?)
    }

    async fn answer_pending(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AnswerPendingRequest>,
    ) -> ServiceResult<MemoryItem> {
        owner_only(&ctx).or_fail()?;
        let r = request.to_owned_message();
        let dir = &self.app.memory_dir;
        let at = Utc::now().with_timezone(&self.app.tz()).naive_local();
        let _hold = self.app.notebook.hold().or_fail()?;
        if judging::waiting_answer(dir, &r.id).or_fail()?.is_some() {
            self.not_dreaming().or_fail()?;
        }
        self.commit_others().or_fail()?;
        let words = self.app.records.names().canonical_text(&r.words, "");
        let (_, mut days) =
            judging::answer(dir, at, &r.id, r.adopt, &words.or_fail()?).or_fail()?;
        days.push(at.date());
        let message = if r.adopt {
            "あなたが保留中の判断を採用した"
        } else {
            "あなたが保留中の判断に答えた"
        };
        self.commit_judged(message, &days).or_fail()?;
        let judged = m::pending_entries(dir).or_fail()?;
        let judged = judged.iter().find(|e| e.id() == Some(r.id.as_str()));
        let judged = judged.ok_or_else(|| fail("the judgment is gone"))?;
        Response::ok(self.pending_item(judged).or_fail()?)
    }

    async fn withdraw_answer(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Empty> {
        owner_only(&ctx).or_fail()?;
        let _hold = self.app.notebook.hold().or_fail()?;
        self.not_dreaming().or_fail()?;
        self.commit_others().or_fail()?;
        let days = judging::withdraw(&self.app.memory_dir, request.id).or_fail()?;
        if days.is_empty() {
            return Err(fail("no answer waits for the dream"));
        }
        self.commit_judged("あなたが保留中の判断への答えを取り消した", &days)
            .or_fail()?;
        Response::ok(Empty::default())
    }

    async fn reject_pending(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RejectPendingRequest>,
    ) -> ServiceResult<Empty> {
        owner_only(&ctx).or_fail()?;
        let r = request.to_owned_message();
        let dir = &self.app.memory_dir;
        let today = Utc::now().with_timezone(&self.app.tz()).date_naive();
        let _hold = self.app.notebook.hold().or_fail()?;
        if judging::waiting_answer(dir, &r.id).or_fail()?.is_some() {
            self.not_dreaming().or_fail()?;
        }
        self.commit_others().or_fail()?;
        let reason = self.app.records.names().canonical_text(&r.reason, "");
        let days = judging::reject(dir, today, &r.id, &reason.or_fail()?).or_fail()?;
        self.commit_judged("あなたが保留中の判断を却下した", &days)
            .or_fail()?;
        Response::ok(Empty::default())
    }

    async fn restore_memory(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Empty> {
        owner_only(&ctx).or_fail()?;
        let _hold = self.app.notebook.hold().or_fail()?;
        self.commit_others().or_fail()?;
        judging::restore(&self.app.memory_dir, request.id).or_fail()?;
        self.commit_judged("あなたが忘れた記憶を戻した", &[])
            .or_fail()?;
        Response::ok(Empty::default())
    }

    async fn read_file(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ReadFileRequest>,
    ) -> ServiceResult<FileContent> {
        let path = request.path.to_owned();
        let dir = &self.app.memory_dir;
        let text = m::read_file(dir, &path).or_fail()?;
        let history = notebook::file_log(dir, &path, HISTORY)
            .or_fail()?
            .into_iter()
            .map(file_change)
            .collect();
        Response::ok(FileContent {
            path,
            text,
            history,
            ..Default::default()
        })
    }

    async fn write_file(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, WriteFileRequest>,
    ) -> ServiceResult<FileContent> {
        writer(&ctx).may_write().or_fail()?;
        let r = request.to_owned_message();
        let dir = &self.app.memory_dir;
        let _hold = self.app.notebook.hold().or_fail()?;
        let before = m::read_file(dir, &r.path).unwrap_or_default();
        let text = self
            .app
            .records
            .names()
            .canonical_memory(&r.text, before)
            .or_fail()?;
        m::write_file(dir, &r.path, text.as_bytes()).or_fail()?;
        notebook::commit_all(
            dir,
            &writer(&ctx).author(),
            &format!("あなたが {} を直した", r.path),
        )
        .or_fail()?;
        changed(self);
        Response::ok(FileContent {
            path: r.path,
            text,
            ..Default::default()
        })
    }

    async fn remember(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RememberRequest>,
    ) -> ServiceResult<MemoryItem> {
        writer(&ctx).may_write().or_fail()?;
        let r = request.to_owned_message();
        let dir = &self.app.memory_dir;
        let at = Utc::now().with_timezone(&self.app.tz()).naive_local();
        let workgroup = if r.workgroup.is_empty() {
            self.app.workgroups.working().or_fail()?
        } else {
            r.workgroup.clone()
        };
        let text = r.text.trim();
        if text.is_empty() {
            return Err(fail("nothing to remember"));
        }
        let _hold = self.app.notebook.hold().or_fail()?;
        let names = self.app.records.names();
        let text = names.canonical_text(text, "").or_fail()?;
        let text = text.as_str();
        let src = if r.said { Src::Said } else { Src::Inferred };
        let entry = m::append_episode_from(dir, at, text, EpisodeState::Waiting, src).or_fail()?;
        let id = entry.id().unwrap_or_default().to_owned();
        if workgroup != ALL {
            m::tag_episode(dir, at.date(), &id, "workgroup", &workgroup).or_fail()?;
        }
        if !r.party.is_empty() {
            m::tag_episode(dir, at.date(), &id, "party", &r.party).or_fail()?;
        }
        changed(self);
        Response::ok(item(&entry, &format!("{}/{}.md", m::EPISODIC, at.date())))
    }

    async fn set_working(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetWorkingRequest>,
    ) -> ServiceResult<WorkingMemory> {
        writer(&ctx).may_write().or_fail()?;
        let r = request.to_owned_message();
        if r.name.is_empty()
            || !r
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(fail("name is lowercase letters, digits and -"));
        }
        let _hold = self.app.notebook.hold().or_fail()?;
        let names = self.app.records.names();
        let before = m::working_memories(&self.app.memory_dir)
            .or_fail()?
            .into_iter()
            .find(|w| w.name == r.name)
            .unwrap_or_default();
        let linked = |text: &str, before: &str| names.canonical_text(text, before).or_fail();
        let working = m::WorkingMemory {
            name: r.name.clone(),
            title: r.name,
            done: linked(&r.done, &before.done)?,
            next: linked(&r.next, &before.next)?,
            waiting: linked(&r.waiting, &before.waiting)?,
            clues: linked(&r.clues, &before.clues)?,
        };
        m::write_working(&self.app.memory_dir, &working).or_fail()?;
        Response::ok(WorkingMemory {
            name: working.name,
            done: working.done,
            next: working.next,
            waiting: working.waiting,
            clues: working.clues,
            ..Default::default()
        })
    }

    async fn close_working(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Empty> {
        writer(&ctx).may_write().or_fail()?;
        let _hold = self.app.notebook.hold().or_fail()?;
        if !m::remove_working(&self.app.memory_dir, request.id).or_fail()? {
            return Err(fail(format!("no working memory {}", request.id)));
        }
        Response::ok(Empty::default())
    }

    async fn list_party_memories(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<ListPartyMemoriesResponse> {
        let items = m::party_memories(&self.app.memory_dir, request.id)
            .or_fail()?
            .iter()
            .map(|(file, e)| item(e, &file.to_string_lossy()))
            .collect();
        Response::ok(ListPartyMemoriesResponse {
            items,
            ..Default::default()
        })
    }

    async fn list_dreams(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListDreamsResponse> {
        let dreams = famulen_core::dreaming::recent_dreams(&self.app.store, 14)
            .or_fail()?
            .into_iter()
            .map(dream)
            .collect();
        let role = famulen_core::models::Role::Dream;
        let blocked = match self.app.models.assignment(role).or_fail()? {
            None => "夢のモデルが決まっていない。設定の夢で決める".to_owned(),
            Some(a)
                if !self
                    .app
                    .models
                    .within_cap(role, self.app.tz(), Utc::now())
                    .or_fail()? =>
            {
                format!(
                    "今日の上限（{} 回）に達した。設定の夢で上限を変えられる",
                    a.cap.unwrap_or_default()
                )
            }
            Some(_) => String::new(),
        };
        Response::ok(ListDreamsResponse {
            dreams,
            next: self.next_dream().or_fail()?,
            dreaming: self
                .app
                .store
                .state(famulen_core::runtime::DREAMING_KEY)
                .or_fail()?
                .is_some(),
            blocked,
            waiting_notes: dreaming::waiting_notes(&self.app.memory_dir)
                .or_fail()?
                .len() as i32,
            ..Default::default()
        })
    }

    async fn dream_now(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<Empty> {
        writer(&ctx).may_write().or_fail()?;
        let app = self.app.clone();
        tokio::spawn(async move {
            famulen_core::runtime::dream_and_record(&app).await;
        });
        Response::ok(Empty::default())
    }

    async fn get_dream_settings(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<DreamSettings> {
        Response::ok(self.dream_settings().or_fail()?)
    }

    async fn set_dream_settings(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DreamSettings>,
    ) -> ServiceResult<DreamSettings> {
        let r = request.to_owned_message();
        let (hour, minute) =
            r.at.split_once(':')
                .and_then(|(h, m)| Some((h.parse().ok()?, m.parse().ok()?)))
                .ok_or_else(|| fail("the time looks like 04:00"))?;
        let frequency = match r.frequency.as_known() {
            Some(DreamFrequency::DREAM_FREQUENCY_NIGHTLY) => Frequency::Nightly,
            Some(DreamFrequency::DREAM_FREQUENCY_EVERY_OTHER_DAY) => Frequency::EveryOtherDay,
            Some(DreamFrequency::DREAM_FREQUENCY_WEEKLY) => Frequency::Weekly {
                weekday: u32::try_from(r.weekday).map_err(|_| fail("no such weekday"))?,
            },
            _ => return Err(fail("say how often to dream")),
        };
        let count = |n: i32| u32::try_from(n).map_err(|_| fail("a count is never negative"));
        let kept = Plan::load(&self.app.store).or_fail()?;
        let plan = Plan {
            hour,
            minute,
            frequency,
            early_at: count(r.early_at)?,
            core_lines: count(r.core_lines)?,
            tidy_records: r.tidy_records.unwrap_or(kept.tidy_records),
            rest_after: r.rest_after.map_or(Ok(kept.rest_after), count)?,
        };
        plan.save(&self.app.store).or_fail()?;
        self.app.journal.record("setting", "dream changed");
        // A new core limit reaches the secretary once its prompt is built again.
        self.app.changed.notify_one();
        changed(self);
        Response::ok(self.dream_settings().or_fail()?)
    }
}
