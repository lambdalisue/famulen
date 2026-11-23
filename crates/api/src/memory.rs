//! What the secretary has learned, in layers of Markdown kept in Git.

use chrono::{NaiveDate, Utc};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::dream_settings::{self as ds, DreamSettings as Plan, Frequency};
use famulen_core::memory::{self as m, Entry, EpisodeState, Layer as CoreLayer, Src};
use famulen_core::notebook;
use famulen_core::workgroups::ALL;

use crate::pb::*;
use crate::{Api, OrFail, changed, fail, given, writer};

const LAYERS: [(&str, &str, &str); 7] = [
    ("dream", "夢", "夜の整理と、その結果"),
    ("pending", "保留中の判断", "急がない。根拠を待っている"),
    (
        "episodic",
        "エピソード記憶",
        "その日の出来事と気づき。夢を待っている",
    ),
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
        changes: n
            .summary
            .changes
            .iter()
            .map(|ch| DreamChange {
                op: serde_json::to_value(ch.op)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default(),
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

fn waiting(e: &Entry) -> bool {
    e.get("dreamt").is_none() && e.state() == Some(EpisodeState::Waiting)
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
            ..Default::default()
        })
    }

    /// When the scheduled dream runs next, in words.
    fn next_dream(&self) -> anyhow::Result<String> {
        let plan = Plan::load(&self.app.store)?;
        let now = Utc::now().with_timezone(&self.app.tz()).naive_local();
        Ok(plan.next_in_words(ds::last(&self.app.store)?, now))
    }

    fn waiting_notes(&self) -> usize {
        let dir = &self.app.memory_dir;
        m::episodic_dates(dir)
            .unwrap_or_default()
            .into_iter()
            .flat_map(|d| m::episodes(dir, d).unwrap_or_default())
            .filter(waiting)
            .count()
    }

    fn layer_count(&self, id: &str) -> anyhow::Result<String> {
        let dir = &self.app.memory_dir;
        Ok(match id {
            "pending" => m::pending_entries(dir)?.len().to_string(),
            "episodic" => self.waiting_notes().to_string(),
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

    /// Write an explicit rule or correction the owner gave straight into a
    /// kept file, instead of waiting for the dream.
    fn keep_now(
        &self,
        file: &str,
        text: &str,
        workgroup: &str,
        today: NaiveDate,
    ) -> anyhow::Result<()> {
        let rel = std::path::PathBuf::from(file);
        anyhow::ensure!(
            CoreLayer::of(&rel).is_some_and(CoreLayer::is_kept),
            "keep_in must be core.md, pending.md or long/<semantic|procedural|prospective>/<name>.md"
        );
        let edit = famulen_core::dreaming::Edit {
            op: "add".into(),
            id: None,
            file: Some(file.into()),
            text: Some(text.into()),
            attrs: [
                ("src".to_owned(), "said".to_owned()),
                ("from".to_owned(), today.to_string()),
            ]
            .into_iter()
            .chain((workgroup != ALL).then(|| ("workgroup".to_owned(), workgroup.to_owned())))
            .collect(),
            reason: None,
            record: None,
        };
        let skipped = famulen_core::dreaming::apply(&self.app.memory_dir, &[edit], today)?;
        anyhow::ensure!(
            skipped.is_empty(),
            "could not keep it: {}",
            skipped.join("; ")
        );
        Ok(())
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
                    .map(|e| item(e, "pending.md"))
                    .collect();
            }
            "core" => {
                out.items = m::core_entries(dir)
                    .or_fail()?
                    .iter()
                    .map(|e| item(e, "core.md"))
                    .collect();
            }
            "episodic" => {
                let dates = m::episodic_dates(dir).or_fail()?;
                let date = NaiveDate::parse_from_str(&r.at, "%Y-%m-%d")
                    .ok()
                    .or(dates.last().copied());
                if let Some(date) = date {
                    let file = format!("{}/{date}.md", m::EPISODIC);
                    out.items = m::episodes(dir, date)
                        .or_fail()?
                        .iter()
                        .map(|e| item(e, &file))
                        .collect();
                }
                out.periods = dates.iter().rev().take(30).map(|d| d.to_string()).collect();
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

    async fn read_file(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ReadFileRequest>,
    ) -> ServiceResult<FileContent> {
        let path = request.path.to_owned();
        let dir = &self.app.memory_dir;
        let text = m::read_file(dir, &path).or_fail()?;
        let mut history = Vec::new();
        for c in notebook::log(dir, 200).or_fail()? {
            if history.len() >= 30 {
                break;
            }
            let touched = notebook::diff_stat(dir, &c.id)
                .map(|s| s.iter().any(|f| f.path == path))
                .unwrap_or(false);
            if touched {
                history.push(FileChange {
                    commit: c.id.chars().take(8).collect(),
                    at: c.time.timestamp(),
                    author: c.author.name.clone(),
                    message: c.message.clone(),
                    ..Default::default()
                });
            }
        }
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
        let names = self.app.records.names();
        let text = names.canonical_text(text, "").or_fail()?;
        let text = text.as_str();
        let kept = !r.keep_in.is_empty() && r.said;
        if kept {
            self.keep_now(&r.keep_in, text, &workgroup, at.date())
                .or_fail()?;
        }
        let state = if kept {
            EpisodeState::Applied
        } else {
            EpisodeState::Waiting
        };
        let src = if r.said { Src::Said } else { Src::Inferred };
        let entry = m::append_episode_from(dir, at, text, state, src).or_fail()?;
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
            waiting_notes: self.waiting_notes() as i32,
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
        let plan = Plan {
            hour,
            minute,
            frequency,
            early_at: count(r.early_at)?,
            core_lines: count(r.core_lines)?,
            tidy_records: match r.tidy_records {
                Some(on) => on,
                None => Plan::load(&self.app.store).or_fail()?.tidy_records,
            },
        };
        plan.save(&self.app.store).or_fail()?;
        self.app.journal.record("setting", "dream changed");
        // A new core limit reaches the secretary once its prompt is built again.
        self.app.changed.notify_one();
        changed(self);
        Response::ok(self.dream_settings().or_fail()?)
    }
}
