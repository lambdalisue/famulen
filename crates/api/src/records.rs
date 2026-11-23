//! The owner's records.

use std::collections::BTreeMap;

use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::notebook::Writer;
use famulen_core::records::{self as core, Patch, ROUTINES, SECRETARY, TASKS};
use famulen_core::workgroups::ALL;

use crate::pb::*;
use crate::{Api, OrFail, by_secretary, changed, fail, given, workgroups_for, writer};

/// The titles and codes of records, read once for a listing.
#[derive(Default)]
struct Names {
    titles: BTreeMap<String, BTreeMap<String, String>>,
    codes: BTreeMap<String, BTreeMap<String, String>>,
}

impl Names {
    fn link(
        &mut self,
        records: &core::Records,
        kind: &str,
        id: &str,
        from: String,
    ) -> Option<RecordLink> {
        if !self.titles.contains_key(kind) {
            let titles = records
                .list(kind)
                .unwrap_or_default()
                .into_iter()
                .map(|r| (r.id.clone(), r.title()))
                .collect();
            self.titles.insert(kind.to_owned(), titles);
            self.codes
                .insert(kind.to_owned(), records.codes(kind).unwrap_or_default());
        }
        Some(RecordLink {
            title: self.titles[kind].get(id)?.clone(),
            code: self.codes[kind].get(id).cloned().unwrap_or_default(),
            kind: kind.to_owned(),
            id: id.to_owned(),
            from,
            ..Default::default()
        })
    }
}

impl Api {
    /// Tell the secretary a task is now its to do.
    fn hand_to_secretary(&self, t: &core::Record) -> anyhow::Result<()> {
        let code = self.app.records.code(TASKS, &t.id)?;
        let title = t.title();
        let input = famulen_core::input::Input {
            sender: famulen_core::input::Sender::Owner,
            source: famulen_core::input::Source::Console {
                // Negative, so it never meets a console message's id.
                message: -chrono::Utc::now().timestamp_millis(),
                workgroup: t
                    .workgroups()
                    .first()
                    .cloned()
                    .unwrap_or_else(|| ALL.to_owned()),
                view: "タスク".to_owned(),
                selection: Some(format!("タスク {code}：{title}")),
            },
            text: format!(
                "タスク {code}「{title}」をあなたに任せます。進めて、状態と経過を save_record で書いてください。"
            ),
        };
        self.app.store.enqueue(&input)?;
        self.app.inputs.notify_one();
        Ok(())
    }

    fn record(&self, r: &core::Record, names: &mut Names) -> Record {
        let records = &self.app.records;
        let mut links: Vec<RecordLink> = Vec::new();
        for (kind, id) in core::linked(r) {
            let link = names.link(records, &kind, &id, String::new());
            links.push(link.unwrap_or_else(|| RecordLink {
                kind,
                id,
                missing: true,
                ..Default::default()
            }));
        }
        let code = names
            .link(records, &r.kind, &r.id, String::new())
            .map(|l| l.code)
            .unwrap_or_default();
        let obsolete = r.obsolete();
        let moved_to = r
            .moved()
            .and_then(|(kind, id)| names.link(records, &kind, &id, String::new()));
        let meta = |name: &str| r.meta.text(name).unwrap_or_default();
        Record {
            kind: r.kind.clone(),
            id: r.id.clone(),
            code,
            title: r.title(),
            labels: r.labels(),
            workgroups: r.workgroups(),
            source: meta("source"),
            created: meta("created"),
            updated: meta("updated"),
            url: meta("url"),
            google: meta("google"),
            fields: core::fields(r).into_iter().map(field).collect(),
            sections: core::sections(r)
                .into_iter()
                .map(|s| RecordSection {
                    name: s.name,
                    fields: s.fields.into_iter().map(field).collect(),
                    items: s
                        .items
                        .into_iter()
                        .map(|i| RecordItem {
                            text: i.text,
                            fields: i.fields.into_iter().map(field).collect(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                })
                .collect(),
            details: core::details(r).into_iter().map(field).collect(),
            notes: r.notes(),
            log: core::log_lines(r)
                .into_iter()
                .map(|l| LogLine {
                    at: l.at,
                    who: l.who,
                    text: l.text,
                    ..Default::default()
                })
                .collect(),
            links,
            obsolete: obsolete.as_ref().map(|m| m.on.clone()).unwrap_or_default(),
            obsolete_reason: obsolete.map(|m| m.reason).unwrap_or_default(),
            moved_to: moved_to.into(),
            ..Default::default()
        }
    }

    fn whole(&self, kind: &str, id: &str) -> anyhow::Result<Record> {
        let records = &self.app.records;
        let r = records.need(kind, id)?;
        let mut names = Names::default();
        let mut out = self.record(&r, &mut names);
        let to_it = self.app.index.backlinks(kind, &r.id)?;
        for b in to_it.records {
            out.backlinks
                .extend(names.link(records, &b.kind, &b.id, b.from));
        }
        out.memories = to_it
            .memories
            .into_iter()
            .map(|m| MemoryLink {
                file: m
                    .path
                    .split_once('/')
                    .map_or(m.path.clone(), |(_, file)| file.to_owned()),
                id: m.id,
                title: m.title,
                ..Default::default()
            })
            .collect();
        if kind == TASKS {
            let all = by_id(records.list(TASKS)?);
            for id in core::Records::blockers(&all, &r) {
                out.blockers
                    .extend(names.link(records, TASKS, &id, String::new()));
            }
        }
        out.markdown = r.render();
        Ok(out)
    }
}

fn by_id(records: Vec<core::Record>) -> BTreeMap<String, core::Record> {
    records.into_iter().map(|r| (r.id.clone(), r)).collect()
}

fn field(f: core::Field) -> Field {
    Field {
        name: f.name,
        value: f.value,
        values: f.values,
        ..Default::default()
    }
}

fn field_from(f: Field) -> core::Field {
    core::Field {
        name: f.name,
        value: f.value,
        values: f.values,
    }
}

fn field_type(name: &str, ty: &core::FieldType) -> FieldType {
    use core::FieldType as T;
    let (word, choices, of) = match ty {
        T::Text => ("text", Vec::new(), None),
        T::Number => ("number", Vec::new(), None),
        T::Flag => ("flag", Vec::new(), None),
        T::Date => ("date", Vec::new(), None),
        T::DateOrMinute => ("date-or-minute", Vec::new(), None),
        T::Money => ("money", Vec::new(), None),
        T::Url => ("url", Vec::new(), None),
        T::List => ("list", Vec::new(), None),
        T::Link(of) => ("link", Vec::new(), of.clone()),
        T::Links(of) => ("links", Vec::new(), of.clone()),
        T::Any => ("any", Vec::new(), None),
        T::OneOf(choices) => ("choice", choices.clone(), None),
    };
    FieldType {
        name: name.to_owned(),
        r#type: word.to_owned(),
        choices,
        of: of.unwrap_or_default(),
        ..Default::default()
    }
}

fn in_workgroup(workgroups: &[String], workgroup: &str) -> bool {
    workgroup.is_empty() || workgroup == ALL || core::Records::in_workgroup(workgroups, workgroup)
}

impl RecordService for Api {
    async fn list_kinds(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListKindsResponse> {
        let kinds = self
            .app
            .records
            .kinds()
            .or_fail()?
            .into_iter()
            .map(|k| RecordKind {
                fields: k.fields.iter().map(|(n, t)| field_type(n, t)).collect(),
                sections: k
                    .sections
                    .iter()
                    .map(|(name, ty)| {
                        let (many, fields) = match ty {
                            core::SectionType::Fields(f) => (false, f),
                            core::SectionType::Items(f) => (true, f),
                        };
                        SectionType {
                            name: name.clone(),
                            many,
                            fields: fields.iter().map(|(n, t)| field_type(n, t)).collect(),
                            ..Default::default()
                        }
                    })
                    .collect(),
                free: k.layout.free,
                name: k.name,
                code: k.code,
                ..Default::default()
            })
            .collect();
        Response::ok(ListKindsResponse {
            kinds,
            ..Default::default()
        })
    }

    async fn list_records(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListRecordsRequest>,
    ) -> ServiceResult<ListRecordsResponse> {
        let r = request.to_owned_message();
        let all = self.app.records.list(&r.kind).or_fail()?;
        let tasks = if r.kind == TASKS {
            by_id(all.clone())
        } else {
            BTreeMap::new()
        };
        let mut names = Names::default();
        let records = all
            .iter()
            .filter(|t| in_workgroup(&t.workgroups(), &r.workgroup))
            .filter(|t| !r.open_only || !core::is_over(t))
            .filter(|t| r.label.is_empty() || t.labels().contains(&r.label))
            .map(|t| {
                let mut out = self.record(t, &mut names);
                for id in core::Records::blockers(&tasks, t) {
                    out.blockers
                        .extend(names.link(&self.app.records, TASKS, &id, String::new()));
                }
                out
            })
            .collect();
        Response::ok(ListRecordsResponse {
            records,
            ..Default::default()
        })
    }

    async fn get_record(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, RecordRef>,
    ) -> ServiceResult<Record> {
        Response::ok(self.whole(request.kind, request.id).or_fail()?)
    }

    async fn save_record(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SaveRecordRequest>,
    ) -> ServiceResult<Record> {
        let r = request.to_owned_message();
        let adding = r.id.is_empty();
        let source = match (r.source.trim(), adding) {
            ("", true) if by_secretary(&ctx) => format!("{}が足した", self.app.aide_name()),
            ("", true) => "コンソールで足した".to_owned(),
            (given, _) => given.to_owned(),
        };
        let workgroups = if adding {
            Some(workgroups_for(self, &ctx, r.workgroups))
        } else {
            r.set_workgroups.then_some(r.workgroups)
        };
        let patch = Patch {
            title: r.title,
            notes: r.notes,
            labels: (r.set_labels || !r.labels.is_empty()).then_some(r.labels),
            workgroups,
            source,
            fields: r.fields.into_iter().map(field_from).collect(),
            sections: r
                .sections
                .into_iter()
                .map(|s| core::SectionContent {
                    name: s.name,
                    fields: s.fields.into_iter().map(field_from).collect(),
                    items: s
                        .items
                        .into_iter()
                        .map(|i| core::Entry {
                            text: i.text,
                            fields: i.fields.into_iter().map(field_from).collect(),
                        })
                        .collect(),
                })
                .collect(),
            details: (r.set_details || !r.details.is_empty())
                .then(|| r.details.into_iter().map(field_from).collect()),
            log: r.log,
            google: given(&r.google).map(str::to_owned),
            url: given(&r.url).map(str::to_owned),
        };
        let by = writer(&ctx);
        let (before, saved) = if adding {
            if patch.title.as_deref().unwrap_or_default().trim().is_empty() {
                return Err(fail("a record needs a title"));
            }
            (None, self.app.records.add(by, &r.kind, patch).or_fail()?)
        } else {
            let before = self.app.records.need(&r.kind, &r.id).or_fail()?;
            let saved = self
                .app
                .records
                .change(by, &r.kind, &r.id, patch)
                .or_fail()?;
            (Some(before), saved)
        };
        let is_secretarys =
            |t: &core::Record| t.body.value("Assignee").as_deref() == Some(SECRETARY);
        let handed = r.kind == TASKS
            && is_secretarys(&saved)
            && !before.as_ref().is_some_and(is_secretarys)
            && !core::is_over(&saved);
        if handed && by == Writer::Owner {
            self.hand_to_secretary(&saved).or_fail()?;
        }
        if r.kind == ROUTINES {
            // Its next run is worked out now, not when it would have come due.
            self.app.scheduler().list(chrono::Utc::now()).or_fail()?;
        }
        changed(self);
        Response::ok(self.whole(&r.kind, &saved.id).or_fail()?)
    }

    async fn clear_obsolete(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RecordRef>,
    ) -> ServiceResult<Record> {
        let records = &self.app.records;
        let marked = records.need(request.kind, request.id).or_fail()?;
        let cleared = records
            .clear_obsolete(writer(&ctx), request.kind, request.id)
            .or_fail()?;
        if marked.obsolete().is_some() {
            let today = chrono::Utc::now()
                .with_timezone(&self.app.tz())
                .date_naive();
            famulen_core::tidying::confirmed(&self.app.store, &cleared, today).or_fail()?;
        }
        changed(self);
        Response::ok(self.whole(&cleared.kind, &cleared.id).or_fail()?)
    }

    async fn delete_record(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RecordRef>,
    ) -> ServiceResult<Empty> {
        self.app
            .records
            .remove(writer(&ctx), request.kind, request.id)
            .or_fail()?;
        changed(self);
        Response::ok(Empty::default())
    }

    async fn link_account(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, LinkAccountRequest>,
    ) -> ServiceResult<Record> {
        let r = request.to_owned_message();
        self.app
            .records
            .link_account(
                writer(&ctx),
                &r.party,
                &r.service,
                &r.handle,
                r.confirmed,
                &r.how,
            )
            .or_fail()?;
        changed(self);
        if !r.confirmed {
            self.app
                .console
                .notify(famulen_core::console_log::Event::Approvals);
        }
        Response::ok(self.whole(core::PARTIES, &r.party).or_fail()?)
    }

    async fn answer_account(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AnswerAccountRequest>,
    ) -> ServiceResult<Empty> {
        self.app
            .records
            .answer_account(writer(&ctx), request.account, request.same)
            .or_fail()?;
        changed(self);
        self.app
            .console
            .notify(famulen_core::console_log::Event::Approvals);
        Response::ok(Empty::default())
    }
}
