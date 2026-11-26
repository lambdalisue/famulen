//! Dreaming, and recording how it went.

use std::collections::BTreeSet;
use std::time::SystemTime;

use chrono::{NaiveDate, Utc};
use serde_json::Value;

use crate::app::App;
use crate::console_log::Event;
use crate::disclosure::{self, ModelReader};
use crate::dream_settings::DreamSettings;
use crate::dreaming;
use crate::input::{Input, Sender, Source};
use crate::memory;
use crate::models::Role;
use crate::notebook_place;
use crate::people;
use crate::tidying;

use super::keep;

/// Set while a dream runs, so the console can say so and a second one waits.
pub const DREAMING_KEY: &str = "dream.running";

/// How a dream that was asked for went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dreamt {
    /// The model's answer was applied, changing something or not.
    Applied,
    Failed,
    /// Another dream is running, or the notebook is moving: none was seen.
    Busy,
}

/// Dream once and record how it went, unless a dream is already running.
pub async fn dream_and_record(app: &App) -> Dreamt {
    if notebook_place::moving(&app.store).is_ok_and(|to| to.is_some()) {
        return Dreamt::Busy;
    }
    // Two asked for together would both see none running if it were looked
    // at before it is set.
    match app
        .store
        .claim_state(DREAMING_KEY, Utc::now().timestamp().to_string())
    {
        Ok(true) => {}
        Ok(false) => return Dreamt::Busy,
        Err(error) => {
            tracing::warn!(%error, "could not mark that a dream runs");
            app.journal
                .record("dream", format!("見られなかった（{error}）"));
            return Dreamt::Failed;
        }
    }
    app.console.notify(Event::Records);
    let dreamt = match dream(app).await {
        Ok((status, applied)) => {
            app.journal.record("dream", status);
            if applied {
                Dreamt::Applied
            } else {
                Dreamt::Failed
            }
        }
        Err(error) => {
            tracing::warn!(%error, "the dream failed");
            app.journal
                .record("dream", format!("見られなかった（{error}）"));
            Dreamt::Failed
        }
    };
    let _ = app.store.clear_state(DREAMING_KEY);
    app.console.notify(Event::Records);
    dreamt
}

async fn dream(app: &App) -> anyhow::Result<(String, bool)> {
    let tz = app.tz();
    let today = Utc::now().with_timezone(&tz).date_naive();
    let mut outcome = dreaming::dream(
        &app.store,
        &app.records,
        &app.memory_dir,
        today,
        |system, prompt| ask(app, system, prompt, dreaming::plan_schema()),
    )
    .await?;
    if !outcome.applied {
        return Ok((outcome.status, false));
    }
    // Before anything reads the memory and carries its marks on.
    let reader = ModelReader::new(app.models.clone(), app.settings.clone());
    if let Err(error) = disclosure::settle_doubts(&app.store, Some(&reader), &outcome.doubts).await
    {
        tracing::warn!(%error, "what the dream wrote stays marked");
    }
    match ask_about_working(app, today) {
        Ok(None) => {}
        Ok(Some(status)) => {
            outcome.status = format!("{}。{status}", outcome.status);
            if let Err(error) =
                dreaming::add_to_night(&app.store, outcome.night, Vec::new(), &status)
            {
                tracing::warn!(%error, "what was asked of working memory was not added to its night");
            }
        }
        Err(error) => tracing::warn!(%error, "could not check the working memory"),
    }
    let mut unkept = None;
    match keep_people_seen(app, today) {
        Ok((None, missed)) => unkept = Some(missed),
        Ok((Some(status), missed)) => {
            unkept = Some(missed);
            outcome.status = format!("{}。{status}", outcome.status);
            if let Err(error) =
                dreaming::add_to_night(&app.store, outcome.night, Vec::new(), &status)
            {
                tracing::warn!(%error, "what was done of parties was not added to its night");
            }
        }
        Err(error) => tracing::warn!(%error, "could not keep when parties were last seen"),
    }
    match ask_about_people(app, today) {
        Ok(None) => {}
        Ok(Some(status)) => {
            outcome.status = format!("{}。{status}", outcome.status);
            if let Err(error) =
                dreaming::add_to_night(&app.store, outcome.night, Vec::new(), &status)
            {
                tracing::warn!(%error, "what was asked of people was not added to its night");
            }
        }
        Err(error) => tracing::warn!(%error, "could not ask about people"),
    }
    // Whom the notes speak of was kept above; without that they are still
    // needed.
    if let Some(unkept) = unkept {
        sweep(app, today, &unkept);
    }
    let holdings = {
        let (store, records) = (app.store.clone(), app.records.clone());
        tokio::task::spawn_blocking(move || tidying::holdings(&store, &records, today)).await?
    };
    match holdings {
        Ok(None) => {}
        Ok(Some(tidied)) => {
            outcome.status = format!("{}。{}", outcome.status, tidied.status);
            if let Err(error) =
                dreaming::add_to_night(&app.store, outcome.night, tidied.changes, &tidied.status)
            {
                tracing::warn!(%error, "what was done of holdings was not added to its night");
            }
        }
        Err(error) => tracing::warn!(%error, "the holdings were not tidied"),
    }
    // Memory stands by now, whatever becomes of the records.
    let tidied = tidying::tidy(
        &app.store,
        &app.records,
        &app.memory_dir,
        today,
        |system, prompt| ask(app, system, prompt, tidying::schema()),
    )
    .await;
    let (changes, status) = match tidied {
        Ok(None) => return Ok((outcome.status, true)),
        Ok(Some(tidied)) => (tidied.changes, tidied.status),
        Err(error) => {
            tracing::warn!(%error, "the records were not tidied");
            (Vec::new(), format!("記録は整えられなかった（{error}）"))
        }
    };
    if let Err(error) = dreaming::add_to_night(&app.store, outcome.night, changes, &status) {
        tracing::warn!(%error, "what tidying did was not added to its night");
    }
    Ok((format!("{}。{status}", outcome.status), true))
}

/// Take away the notes the dream is done with, once whom they speak of is kept.
pub(super) fn sweep_notes(app: &App) {
    let today = Utc::now().with_timezone(&app.tz()).date_naive();
    if notebook_place::moving(&app.store).is_ok_and(|to| to.is_some()) {
        return;
    }
    match people::keep_seen(&app.records, &app.memory_dir, today, 0) {
        Ok((_, _, unkept)) => sweep(app, today, &unkept),
        Err(error) => tracing::warn!(%error, "could not keep when parties were last seen"),
    }
}

fn sweep(app: &App, today: NaiveDate, unkept: &BTreeSet<String>) {
    let swept = app
        .notebook
        .hold()
        .and_then(|_hold| dreaming::sweep(&app.memory_dir, today, unkept));
    match swept {
        Ok(0) => {}
        Ok(gone) => {
            tracing::info!(gone, "took away the notes dealt with");
            app.console.notify(Event::Records);
        }
        Err(error) => tracing::warn!(%error, "could not take away the notes dealt with"),
    }
}

/// Ask the secretary of the working memory that may have outlived its work,
/// once a day at most. What was asked, if anything was.
fn ask_about_working(app: &App, today: NaiveDate) -> anyhow::Result<Option<String>> {
    let found = memory::leftovers(&app.records, &app.memory_dir, SystemTime::now())?;
    if found.is_empty() {
        return Ok(None);
    }
    let list: Vec<String> = found
        .iter()
        .map(|l| format!("- {}: {}", l.name, l.why))
        .collect();
    let input = Input {
        sender: Sender::System,
        source: Source::Schedule {
            name: "working-memory".into(),
            at: today.to_string(),
            routine: String::new(),
        },
        text: format!(
            "This working memory may have outlived its work. Check each: close it with close_working when the work is finished; when it goes on, write where it stands with set_working. Do not close what you could not check.\n{}",
            list.join("\n")
        ),
    };
    let Some(id) = keep(&app.store, &app.inputs, &input)? else {
        return Ok(None);
    };
    let paths = found
        .iter()
        .map(|l| format!("memory/working/{}.md", l.name));
    disclosure::carry_to_input(&app.store, paths, id)?;
    Ok(Some(format!(
        "作業記憶 {} 件を、閉じるか秘書に尋ねた",
        found.len()
    )))
}

/// Keep when parties were last seen, before the notes of this night are
/// handed over. What came of it, if anyone rests from now, and the parties
/// that could not be kept.
fn keep_people_seen(
    app: &App,
    today: NaiveDate,
) -> anyhow::Result<(Option<String>, BTreeSet<String>)> {
    let rest_after = DreamSettings::load(&app.store)?.rest_after;
    let (_, rested, unkept) = people::keep_seen(&app.records, &app.memory_dir, today, rest_after)?;
    let status = (rested > 0).then(|| format!("長く出てこない相手 {rested} 人を休みにした"));
    Ok((status, unkept))
}

/// Have the secretary bring the parties up to date with the people noted, once
/// a day at most. What was asked, if anything was.
fn ask_about_people(app: &App, today: NaiveDate) -> anyhow::Result<Option<String>> {
    let Some((text, notes)) = people::to_ask(&app.memory_dir, today)? else {
        return Ok(None);
    };
    let input = Input {
        sender: Sender::System,
        source: Source::Schedule {
            name: "people".into(),
            at: today.to_string(),
            routine: String::new(),
        },
        text,
    };
    let Some(id) = keep(&app.store, &app.inputs, &input)? else {
        return Ok(None);
    };
    disclosure::carry_to_input(&app.store, notes.iter().map(|(_, note)| note), id)?;
    let _hold = app.notebook.hold()?;
    people::mark_told(&app.memory_dir, today, &notes)?;
    Ok(Some(format!(
        "人の気づき {} 件を、相手に反映するよう秘書に頼んだ",
        notes.len()
    )))
}

async fn ask(app: &App, system: String, prompt: String, schema: Value) -> anyhow::Result<Value> {
    let completion = app
        .models
        .complete(Role::Dream, app.tz(), &system, &prompt, Some(&schema))
        .await?;
    completion
        .json
        .ok_or_else(|| anyhow::anyhow!("the dream answered without JSON: {}", completion.text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disclosure::marking;
    use crate::memory::{WorkingMemory, ensure, working_memories, write_working};
    use crate::notebook::testing::TempDir;

    #[test]
    fn the_secretary_brings_the_parties_up_to_date_once_a_day() {
        let dir = TempDir::new("dream-people");
        let app = crate::runtime::testing::app(dir.path());
        ensure(&app.memory_dir).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
        assert_eq!(ask_about_people(&app, today).unwrap(), None);
        let note = |who: &str| {
            people::write_down(
                &app.records,
                &app.memory_dir,
                today.and_hms_opt(9, 0, 0).unwrap(),
                &people::Place {
                    feed: crate::lookout::Feed::Chat,
                    name: "会社",
                    connection: "slack",
                    workgroups: &[],
                    names: &Default::default(),
                },
                &[crate::lookout::Item {
                    title: String::new(),
                    body: String::new(),
                }],
                &[people::Noted {
                    item: 0,
                    who: who.into(),
                    note: "経理の担当".into(),
                    tie: people::Tie::WorksWith,
                }],
            )
            .unwrap();
        };
        note("田中");
        for (_, noted) in people::to_ask(&app.memory_dir, today).unwrap().unwrap().1 {
            marking::mark(&app.store, &noted);
        }

        let asked = ask_about_people(&app, today).unwrap().unwrap();

        assert!(asked.contains("人の気づき 1 件"), "{asked}");
        let first = app.store.recent_inputs(0, 10).unwrap()[0].0.id;
        assert!(marking::marked_input(&app.store, first));
        note("佐藤");
        assert_eq!(ask_about_people(&app, today).unwrap(), None);
        ask_about_people(&app, today.succ_opt().unwrap()).unwrap();
        let inputs = app.store.recent_inputs(0, 10).unwrap();
        assert_eq!(inputs.len(), 2);
        for who in ["田中", "佐藤"] {
            let with = inputs.iter().filter(|(i, ..)| i.input.text.contains(who));
            assert_eq!(with.count(), 1, "{who}");
        }
        assert!(
            inputs
                .iter()
                .all(|(i, ..)| i.input.sender == Sender::System)
        );
    }

    #[test]
    fn a_night_that_has_people_rest_says_how_many() {
        let dir = TempDir::new("dream-rest");
        let app = crate::runtime::testing::app(dir.path());
        ensure(&app.memory_dir).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
        assert_eq!(keep_people_seen(&app, today).unwrap().0, None);
        app.records
            .add(
                crate::notebook::Writer::Secretary,
                crate::records::PARTIES,
                crate::records::Patch::titled("田中").with("Last seen", "2026-08-01"),
            )
            .unwrap();

        let status = keep_people_seen(&app, today).unwrap().0.unwrap();

        assert!(status.contains("1 人"), "{status}");
    }

    #[test]
    fn the_secretary_is_asked_once_a_day_of_working_memory_that_may_be_over() {
        let dir = TempDir::new("dream-working");
        let app = crate::runtime::testing::app(dir.path());
        ensure(&app.memory_dir).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 11, 24).unwrap();
        assert_eq!(ask_about_working(&app, today).unwrap(), None);
        let memory = WorkingMemory {
            name: "trip".into(),
            ..WorkingMemory::default()
        };
        let rel = write_working(&app.memory_dir, &memory).unwrap();
        std::fs::File::options()
            .write(true)
            .open(app.memory_dir.join(rel))
            .unwrap()
            .set_modified(SystemTime::now() - std::time::Duration::from_secs(8 * 86_400))
            .unwrap();
        marking::mark(&app.store, "memory/working/trip.md");

        let asked = ask_about_working(&app, today).unwrap();

        assert!(asked.unwrap().contains("作業記憶 1 件"));
        assert_eq!(ask_about_working(&app, today).unwrap(), None);
        let inputs = app.store.recent_inputs(0, 10).unwrap();
        assert_eq!(inputs.len(), 1);
        let input = &inputs[0].0.input;
        assert_eq!(input.sender, Sender::System);
        assert!(marking::marked_input(&app.store, inputs[0].0.id));
        assert!(input.text.contains("- trip: not written for 8 days"));
        assert!(input.text.contains("close_working"));
        assert_eq!(working_memories(&app.memory_dir).unwrap().len(), 1);
        let tomorrow = today.succ_opt().unwrap();
        assert!(ask_about_working(&app, tomorrow).unwrap().is_some());
    }
}
