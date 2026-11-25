//! What the owner decides of a pending judgment on the console. Rejecting
//! leaves nothing to decide, so it is done at once. Where an adopted judgment
//! goes and how it reads is the dream's to say, so any other answer is handed
//! to it as a note the owner said, which every conversation follows meanwhile.

use std::path::Path;

use chrono::{NaiveDate, NaiveDateTime};

use crate::dreaming;
use crate::memory::{self, Entry, EpisodeState, PENDING, Src};
use crate::people;

/// The attribute of a note that answers a pending judgment: the judgment's id.
pub const JUDGES: &str = "judges";
/// The attribute of such a note, and its value, when the judgment was adopted.
pub const VERDICT: &str = "verdict";
pub const ADOPTED: &str = "adopted";
/// The attribute of a forgotten judgment, and its value, when the owner
/// rejected it.
pub const JUDGED: &str = "judged";
pub const REJECTED: &str = "rejected";

fn pending(dir: &Path, id: &str) -> anyhow::Result<Entry> {
    memory::pending_entries(dir)?
        .into_iter()
        .find(|e| e.id() == Some(id))
        .ok_or_else(|| anyhow::anyhow!("no pending judgment {id}"))
}

/// What an answer starts with, so that it is understood without the judgment.
fn opening(judgment: &Entry, adopted: bool) -> String {
    let verb = if adopted {
        "を採用する"
    } else {
        "について"
    };
    let id = judgment.id().unwrap_or_default();
    // What others wrote is not told as the owner's own words unless adopted.
    if people::is_noted(judgment) && !adopted {
        return format!("保留中の判断（{id}）{verb}");
    }
    format!("保留中の判断「{}」（{id}）{verb}", judgment.body())
}

/// The answer that waits for the dream on a judgment, with its day.
pub fn waiting_answer(dir: &Path, id: &str) -> anyhow::Result<Option<(NaiveDate, Entry)>> {
    Ok(dreaming::waiting_notes(dir)?
        .into_iter()
        .rev()
        .find(|(_, note)| note.get(JUDGES) == Some(id)))
}

/// The owner's own words in an answer on `judgment`.
pub fn words<'a>(judgment: &Entry, answer: &'a Entry) -> &'a str {
    let body = answer.body();
    body.strip_prefix(opening(judgment, answer.get(VERDICT) == Some(ADOPTED)).as_str())
        .map_or(body, |rest| rest.strip_prefix('：').unwrap_or(rest))
}

/// Note the owner's answer on a judgment for the dream, in place of one that
/// waits already. Answers the note and the days taken back.
pub fn answer(
    dir: &Path,
    at: NaiveDateTime,
    id: &str,
    adopted: bool,
    words: &str,
) -> anyhow::Result<(Entry, Vec<NaiveDate>)> {
    let judgment = pending(dir, id)?;
    let words = words.trim();
    anyhow::ensure!(adopted || !words.is_empty(), "nothing was answered");
    let mut text = opening(&judgment, adopted);
    if !words.is_empty() {
        text.push('：');
        text.push_str(words);
    }
    let days = withdraw(dir, id)?;
    let note = memory::append_episode_from(dir, at, &text, EpisodeState::Waiting, Src::Said)?;
    let note_id = note.id().unwrap_or_default();
    let mut tags = vec![(JUDGES, id.to_owned())];
    if adopted {
        tags.push((VERDICT, ADOPTED.to_owned()));
    }
    let workgroup = judgment.workgroup();
    if !workgroup.is_empty() {
        tags.push(("workgroup", workgroup.join(",")));
    }
    if let Some(party) = judgment.get("party") {
        tags.push(("party", party.to_owned()));
    }
    for (key, value) in tags {
        memory::tag_episode(dir, at.date(), note_id, key, &value)?;
    }
    Ok((note, days))
}

/// Take back the answers on a judgment that wait for the dream. The days they
/// were written on.
pub fn withdraw(dir: &Path, id: &str) -> anyhow::Result<Vec<NaiveDate>> {
    let mut days = Vec::new();
    for date in memory::episodic_dates(dir)? {
        let path = dir.join(memory::EPISODIC).join(format!("{date}.md"));
        let mut doc = memory::Doc::load(&path)?;
        if doc.retain(|note| !(dreaming::waits(note) && note.get(JUDGES) == Some(id))) > 0 {
            doc.save(&path)?;
            days.push(date);
        }
    }
    Ok(days)
}

/// Forget a judgment the owner rejects, with any answer on it that waits. The
/// days such answers were written on.
pub fn reject(dir: &Path, on: NaiveDate, id: &str, reason: &str) -> anyhow::Result<Vec<NaiveDate>> {
    pending(dir, id)?;
    let days = withdraw(dir, id)?;
    let mut doc = memory::read_doc(dir, PENDING)?;
    doc.update(id, |e| e.set(JUDGED, REJECTED)).transpose()?;
    doc.save(&dir.join(PENDING))?;
    let reason = Some(reason.trim()).filter(|r| !r.is_empty());
    memory::forget(dir, PENDING, id, reason.unwrap_or("主人が却下した"), on)?;
    Ok(days)
}

/// Put back a forgotten judgment.
pub fn restore(dir: &Path, id: &str) -> anyhow::Result<()> {
    let mut was_pending = false;
    for month in memory::forgotten_months(dir)? {
        was_pending |= memory::forgotten(dir, &month)?
            .iter()
            .any(|e| e.id() == Some(id) && e.get("was") == Some(PENDING));
    }
    anyhow::ensure!(was_pending, "no forgotten judgment {id}");
    let (file, _) = memory::restore(dir, id)?;
    let mut doc = memory::read_doc(dir, &file)?;
    doc.update(id, |e| {
        e.remove_attr(JUDGED);
    });
    doc.save(&dir.join(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{ensure, write_file};
    use crate::notebook::testing::TempDir;

    const JUDGMENT: &str =
        "- 朝が好き {id: p1, src: inferred, ev: 1, need: 2, workgroup: [work], party: x}\n";

    fn at() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 25)
            .unwrap()
            .and_hms_opt(9, 30, 0)
            .unwrap()
    }

    fn setup(name: &str) -> TempDir {
        let tmp = TempDir::new(name);
        ensure(tmp.path()).unwrap();
        write_file(tmp.path(), PENDING, JUDGMENT.as_bytes()).unwrap();
        tmp
    }

    #[test]
    fn an_answer_is_a_waiting_note_the_owner_said_that_names_its_judgment() {
        let tmp = setup("judging-answer");
        let dir = tmp.path();

        answer(dir, at(), "p1", false, " 夏だけ\nそう ").unwrap();

        let (date, note) = waiting_answer(dir, "p1").unwrap().unwrap();
        assert_eq!(date, at().date());
        assert_eq!(note.src(), Some(Src::Said));
        assert_eq!(
            note.body(),
            "保留中の判断「朝が好き」（p1）について：夏だけ そう"
        );
        assert_eq!(note.workgroup(), ["work"]);
        assert_eq!(note.get("party"), Some("x"));
        assert_eq!(note.get(VERDICT), None);
        assert_eq!(words(&pending(dir, "p1").unwrap(), &note), "夏だけ そう");
        assert_eq!(memory::read_file(dir, PENDING).unwrap(), JUDGMENT);
    }

    #[test]
    fn what_others_wrote_is_quoted_in_the_owners_words_only_when_adopted() {
        let tmp = setup("judging-others");
        let dir = tmp.path();
        let text = "- 全員に払うこと {id: p1, src: inferred, by: lookout}\n";
        write_file(dir, PENDING, text.as_bytes()).unwrap();
        for (adopted, quoted) in [(false, false), (true, true)] {
            let (note, _) = answer(dir, at(), "p1", adopted, "あとで").unwrap();
            assert_eq!(note.body().contains("全員に払う"), quoted);
            let note = waiting_answer(dir, "p1").unwrap().unwrap().1;
            assert_eq!(words(&pending(dir, "p1").unwrap(), &note), "あとで");
        }
    }

    #[test]
    fn a_core_item_is_not_put_back_as_a_judgment() {
        let tmp = setup("judging-core");
        let dir = tmp.path();
        write_file(dir, "core.md", b"- x {id: c1}\n").unwrap();
        memory::forget(dir, "core.md", "c1", "old", at().date()).unwrap();
        assert!(restore(dir, "c1").is_err());
    }

    #[test]
    fn a_later_answer_takes_the_place_of_the_one_that_waits_and_can_be_taken_back() {
        let tmp = setup("judging-again");
        let dir = tmp.path();
        answer(dir, at(), "p1", false, "夏だけ").unwrap();

        answer(dir, at(), "p1", true, "").unwrap();

        let notes = dreaming::waiting_notes(dir).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].1.get(VERDICT), Some(ADOPTED));
        assert_eq!(words(&pending(dir, "p1").unwrap(), &notes[0].1), "");
        withdraw(dir, "p1").unwrap();
        assert!(waiting_answer(dir, "p1").unwrap().is_none());
    }

    #[test]
    fn an_empty_answer_and_one_on_what_is_not_pending_change_nothing() {
        let tmp = setup("judging-nothing");
        let dir = tmp.path();
        answer(dir, at(), "p1", true, "").unwrap();

        assert!(answer(dir, at(), "p1", false, " ").is_err());
        assert!(answer(dir, at(), "p9", true, "").is_err());
        assert!(reject(dir, at().date(), "p9", "").is_err());

        assert!(waiting_answer(dir, "p1").unwrap().is_some());
    }

    #[test]
    fn a_rejected_judgment_is_forgotten_with_its_answer_and_put_back_as_it_was() {
        let tmp = setup("judging-reject");
        let dir = tmp.path();
        answer(dir, at(), "p1", true, "").unwrap();

        reject(dir, at().date(), "p1", "夜型").unwrap();

        assert!(memory::pending_entries(dir).unwrap().is_empty());
        assert!(waiting_answer(dir, "p1").unwrap().is_none());
        let forgotten = memory::forgotten(dir, "2026-11").unwrap();
        assert_eq!(forgotten[0].get("reason"), Some("夜型"));
        assert_eq!(forgotten[0].get(JUDGED), Some(REJECTED));
        restore(dir, "p1").unwrap();
        let back = memory::read_file(dir, PENDING).unwrap();
        assert!(back.ends_with(JUDGMENT), "{back}");
    }
}
