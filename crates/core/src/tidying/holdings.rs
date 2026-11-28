//! What is wrong with the holdings that needs no model to tell: the same thing
//! kept twice, and an order long past still waiting to arrive. What may be the
//! same thing and may not is left to the dream, which reads them.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use famulen_notebook::body::same;
use rusqlite::OptionalExtension as _;

use super::Tidied;
use super::merging::{MERGED, merge, word};
use crate::dream_settings::DreamSettings;
use crate::memory::Change;
use crate::notebook::Writer;
use crate::purchases::SOURCE;
use crate::records::{
    HOLDINGS, Patch, QUANTITY, Record, Records, details, fields, log_lines, quantity, sections,
};
use crate::store::Store;

/// An order older than this is taken to have arrived.
const ARRIVED_DAYS: i64 = 30;

/// Make one of what is surely one thing, take long past orders to have
/// arrived.
pub fn holdings(
    store: &Store,
    records: &Records,
    today: NaiveDate,
) -> anyhow::Result<Option<Tidied>> {
    if !DreamSettings::load(store)?.tidy_records {
        return Ok(None);
    }
    let changes = merge_same(store, records)?;
    let arrived = arrive(store, records, today)?;
    let mut done = Vec::new();
    if !changes.is_empty() {
        done.push(format!("重なっていた物を {} 組まとめた", changes.len()));
    }
    if arrived > 0 {
        done.push(format!(
            "{ARRIVED_DAYS}日より前の注文の物 {arrived} 件を owned にした"
        ));
    }
    Ok((!done.is_empty()).then(|| Tidied {
        changes,
        status: done.join("、"),
    }))
}

fn value(record: &Record, section: &str, name: &str) -> String {
    let fields = if section.is_empty() {
        fields(record)
    } else {
        sections(record)
            .into_iter()
            .find(|s| same(&s.name, section))
            .map(|s| s.fields)
            .unwrap_or_default()
    };
    fields
        .into_iter()
        .find(|f| same(&f.name, name))
        .map(|f| f.value)
        .unwrap_or_default()
}

/// Whether it is at hand or on its way; what was lent or let go is never taken
/// for another.
fn held_now(record: &Record) -> bool {
    matches!(
        record.body.value("Status").as_deref(),
        Some("ordered" | "owned")
    )
}

/// What two records of one thing of one order say alike. None for what names
/// no order.
fn same_key(record: &Record) -> Option<Vec<String>> {
    let order = word(&value(record, "Acquired", "Order"));
    if order.is_empty() || !held_now(record) {
        return None;
    }
    let mut other: Vec<String> = details(record)
        .iter()
        .filter(|d| d.name != QUANTITY)
        .map(|d| format!("{}\n{}", word(&d.name), word(&d.value)))
        .collect();
    other.sort();
    let mut key = vec![
        order,
        word(&record.title()),
        word(&value(record, "Acquired", "Ordered as")),
        word(&value(record, "", "Model")),
        word(&value(record, "", "Maker")),
        word(&value(record, "Acquired", "Price")),
    ];
    key.extend(other);
    Some(key)
}

/// What tells two of a kind apart when both say it.
const OWN: [(&str, &str); 5] = [
    ("", "Serial"),
    ("", "Place"),
    ("Acquired", "From"),
    ("Acquired", "How"),
    ("Acquired", "Reason"),
];

fn may_be_one(a: &Record, b: &Record) -> bool {
    OWN.iter().all(|(section, name)| {
        let (a, b) = (value(a, section, name), value(b, section, name));
        a.is_empty() || b.is_empty() || word(&a) == word(&b)
    })
}

fn merge_same(store: &Store, records: &Records) -> anyhow::Result<Vec<Change>> {
    let mut groups: BTreeMap<Vec<String>, Vec<String>> = BTreeMap::new();
    for record in records.list(HOLDINGS)? {
        if let Some(key) = same_key(&record) {
            groups.entry(key).or_default().push(record.id);
        }
    }
    let mut changes = Vec::new();
    for (key, ids) in groups.into_iter().filter(|(_, ids)| ids.len() > 1) {
        // Held for one group at a time, so that the owner can write meanwhile.
        let held = records.hold()?;
        let mut alike = Vec::new();
        for id in ids {
            if let Some(record) = held.records().get(HOLDINGS, &id)?
                && same_key(&record).as_ref() == Some(&key)
            {
                alike.push(record);
            }
        }
        alike.sort_by_key(|r| {
            (
                r.meta.text("source").as_deref() == Some(SOURCE),
                Reverse(r.body.text().len()),
                r.id.clone(),
            )
        });
        let mut apart: Vec<Vec<Record>> = Vec::new();
        for record in alike {
            match apart
                .iter_mut()
                .find(|one| one.iter().all(|r| may_be_one(r, &record)))
            {
                Some(one) => one.push(record),
                None => apart.push(vec![record]),
            }
        }
        for one in apart.iter().filter(|one| one.len() > 1) {
            match merge(store, &held, one, "同じ注文の同じ物".into()) {
                Ok(change) => changes.push(change),
                Err(error) => {
                    tracing::warn!(%error, holding = %one[0].id, "holdings were not merged");
                }
            }
        }
    }
    Ok(changes)
}

/// The records a merge named in the log of another has already counted: one
/// stopped midway leaves them behind.
fn already_counted(alike: &[Record]) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for line in alike.iter().flat_map(log_lines) {
        if let Some(rest) = line.text.strip_prefix(MERGED)
            && let Some((_, named)) = rest.split_once('：')
        {
            ids.extend(named.split(", ").map(str::to_owned));
        }
    }
    ids
}

/// How many there are of what these records say. What one mail told of adds up,
/// unless each of its records only repeats how many records there are; of
/// several mails telling of one order, the one that tells of the most counts.
pub(super) fn how_many(store: &Store, alike: &[Record]) -> anyhow::Result<u32> {
    let done = already_counted(alike);
    let mut told: BTreeMap<Option<i64>, Vec<u32>> = BTreeMap::new();
    for record in alike.iter().filter(|r| !done.contains(&r.id)) {
        let mail = store
            .conn()
            .query_row(
                "SELECT mail FROM purchases WHERE holding = ?1",
                [&record.id],
                |r| r.get(0),
            )
            .optional()?;
        told.entry(mail).or_default().push(quantity(record));
    }
    // Records no mail told of are the same thing written again, not more of it.
    let unbought = told.remove(&None).and_then(|said| said.into_iter().max());
    let of_one_mail = |said: Vec<u32>| {
        let records = said.len() as u32;
        if said.iter().all(|n| *n == 1 || *n == records) {
            records
        } else {
            said.iter().sum()
        }
    };
    Ok(told
        .into_values()
        .map(of_one_mail)
        .chain(unbought)
        .max()
        .unwrap_or(1))
}

fn date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.get(..10)?, "%Y-%m-%d").ok()
}

fn bought_on(record: &Record) -> Option<NaiveDate> {
    date(&value(record, "Acquired", "On"))
}

fn arrive(store: &Store, records: &Records, today: NaiveDate) -> anyhow::Result<usize> {
    let mut arrived = 0;
    for record in records.list(HOLDINGS)? {
        if record.body.value("Status").as_deref() != Some("ordered") {
            continue;
        }
        let told: Option<String> = store
            .conn()
            .query_row(
                "SELECT bought_on FROM purchases WHERE holding = ?1",
                [&record.id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(on) = bought_on(&record).or_else(|| date(&told?)) else {
            continue;
        };
        if (today - on).num_days() <= ARRIVED_DAYS {
            continue;
        }
        let patch = Patch {
            log: vec![format!("{ARRIVED_DAYS}日より前の注文なので owned にした")],
            ..Patch::default().with("Status", "owned")
        };
        match records.change(Writer::Secretary, HOLDINGS, &record.id, patch) {
            Ok(_) => arrived += 1,
            Err(error) => {
                tracing::warn!(%error, holding = %record.id, "an old order was left as it was");
            }
        }
    }
    Ok(arrived)
}

#[cfg(test)]
mod tests {
    use chrono::{Days, Local};
    use rusqlite::params;

    use super::*;
    use crate::notebook::testing::TempDir;
    use crate::records::testing::records;
    use crate::records::{Entry, Field, MEMOS, SectionContent};

    const ME: Writer = Writer::Owner;

    struct Book {
        _tmp: TempDir,
        records: Records,
        store: Store,
    }

    fn book(name: &str) -> Book {
        let (_tmp, records) = records(name);
        Book {
            _tmp,
            records,
            store: Store::open_in_memory().unwrap(),
        }
    }

    fn today() -> NaiveDate {
        Local::now().date_naive()
    }

    fn field(name: &str, value: &str) -> Field {
        Field::new(name, value)
    }

    /// A thing of the order A-1, ordered today.
    fn thing(title: &str) -> Patch {
        let mut patch = Patch::titled(title).with("Status", "ordered");
        patch.source = SOURCE.into();
        patch.sections = vec![acquired("A-1", "980 JPY", today())];
        patch
    }

    fn acquired(order: &str, price: &str, on: NaiveDate) -> SectionContent {
        SectionContent {
            name: "Acquired".into(),
            fields: [
                ("On", on.to_string()),
                ("From", "本の店".into()),
                ("Price", price.into()),
                ("Order", order.into()),
            ]
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(n, v)| field(n, &v))
            .collect(),
            items: vec![],
        }
    }

    impl Book {
        fn add(&self, patch: Patch) -> Record {
            self.records.add(ME, HOLDINGS, patch).unwrap()
        }

        /// As added from a mail.
        fn bought(&self, mail: i64, patch: Patch) -> Record {
            let r = self.add(patch);
            self.store
                .conn()
                .execute(
                    "INSERT INTO purchases (holding, mail, shop, number, name, bought_on)
                     VALUES (?1, ?2, '本の店', 'A-1', ?3, '')",
                    params![r.id, mail, r.title()],
                )
                .unwrap();
            r
        }

        fn tidy(&self) -> Option<Tidied> {
            holdings(&self.store, &self.records, today()).unwrap()
        }

        fn list(&self) -> Vec<Record> {
            self.records.list(HOLDINGS).unwrap()
        }

        fn only(&self) -> Record {
            let mut list = self.list();
            assert_eq!(list.len(), 1);
            list.remove(0)
        }
    }

    fn detail(r: &Record, name: &str) -> Option<String> {
        details(r)
            .into_iter()
            .find(|f| f.name == name)
            .map(|f| f.value)
    }

    #[test]
    fn the_same_thing_of_one_order_becomes_one_with_how_many() {
        let b = book("same-merge");
        for _ in 0..3 {
            b.bought(1, thing("タイル"));
        }
        let done = b.tidy().unwrap();
        assert_eq!(detail(&b.only(), "数量").as_deref(), Some("3"));
        assert_eq!(done.changes.len(), 1);
        assert!(b.tidy().is_none());
    }

    #[test]
    fn what_a_record_put_into_another_was_marked_with_stays_on_that_one() {
        let b = book("same-marked");
        b.bought(1, thing("タイル"));
        let other = b.bought(1, thing("タイル"));
        crate::disclosure::marking::mark(&b.store, &other.id);
        b.tidy();
        let kept = b.only();
        assert_ne!(kept.id, other.id);
        assert!(crate::disclosure::marking::marked(&b.store, &kept.id));
    }

    #[test]
    fn what_two_mails_each_told_of_is_not_counted_twice() {
        let b = book("same-mails");
        for mail in [1, 1, 2, 2] {
            b.bought(mail, thing("タイル"));
        }
        b.tidy();
        assert_eq!(detail(&b.only(), "数量").as_deref(), Some("2"));
    }

    #[test]
    fn nothing_the_others_say_is_lost_and_what_the_owner_made_is_kept() {
        let b = book("same-keeps");
        let mut first = thing("鍵盤").with("Category", "機器");
        first.notes = Some("取り込んだ控え".into());
        first.labels = Some(vec!["仕事".into()]);
        first.sections.push(SectionContent {
            name: "Deadlines".into(),
            fields: vec![],
            items: vec![Entry {
                text: "保証".into(),
                fields: vec![field("What", "保証"), field("On", "2027-01-01")],
            }],
        });
        b.bought(1, first);
        let mut mine = thing("鍵盤")
            .with("Category", "入力装置")
            .with("Place", "机");
        mine.source = String::new();
        mine.notes = Some("手で書いた".into());
        let mine = b.add(mine);
        b.tidy();
        let kept = b.only();
        assert_eq!(kept.id, mine.id);
        assert_eq!(kept.body.value("Category").as_deref(), Some("入力装置"));
        assert_eq!(kept.body.value("Place").as_deref(), Some("机"));
        assert!(kept.notes().contains("手で書いた"));
        assert!(kept.notes().contains("取り込んだ控え"));
        assert_eq!(kept.labels(), ["仕事"]);
        assert!(kept.body.text().contains("2027-01-01"));
    }

    #[test]
    fn what_two_shops_sold_under_one_number_is_left_apart() {
        let b = book("same-shops");
        b.add(thing("鍵盤"));
        let mut other = thing("鍵盤");
        other.sections[0].fields[1] = field("From", "別の店");
        b.add(other);
        b.tidy();
        assert_eq!(b.list().len(), 2);
    }

    #[test]
    fn what_the_one_that_stays_says_otherwise_goes_to_its_log() {
        let b = book("same-otherwise");
        let mut mine = thing("鍵盤").with("Category", "入力装置");
        mine.source = String::new();
        b.add(mine);
        let mut other = thing("鍵盤").with("Category", "機器");
        other.sections = vec![acquired("A-1", "980 JPY", today() - Days::new(1))];
        b.add(other);
        b.tidy();
        let said: Vec<String> = log_lines(&b.only()).into_iter().map(|l| l.text).collect();
        let said = said.join("\n");
        assert!(said.contains("Category: 機器"), "{said}");
        let before = format!("Acquired の On: {}", today() - Days::new(1));
        assert!(said.contains(&before), "{said}");
    }

    #[test]
    fn names_that_differ_only_in_case_name_the_same_field() {
        let b = book("same-case");
        b.add(thing("無線機").with("Serial", "S1"));
        let other = b.add(thing("無線機").with("Serial", "S2"));
        let path = b.records.notebook().root().join(other.rel());
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("Serial: S2", "serial: S2")).unwrap();
        b.tidy();
        assert_eq!(b.list().len(), 2);
    }

    /// A thing of which there are several.
    fn several(title: &str, many: &str) -> Patch {
        let mut p = thing(title);
        p.details = Some(vec![field("数量", many)]);
        p
    }

    #[test]
    fn how_many_the_records_of_one_mail_say_are_added_up() {
        let b = book("same-sum");
        b.bought(1, several("タイル", "2"));
        b.bought(1, several("タイル", "3"));
        b.tidy();
        assert_eq!(detail(&b.only(), "数量").as_deref(), Some("5"));
    }

    #[test]
    fn a_total_each_record_of_one_mail_repeats_is_counted_once() {
        let b = book("same-total");
        b.bought(1, several("タイル", "2点"));
        b.bought(1, several("タイル", "2点中の2つ目"));
        b.tidy();
        assert_eq!(detail(&b.only(), "数量").as_deref(), Some("2"));
    }

    #[test]
    fn what_a_merge_stopped_midway_had_counted_is_not_counted_again() {
        let b = book("same-midway");
        let left = b.bought(1, thing("タイル"));
        let mut counted = several("タイル", "2");
        counted.log = vec![format!("{MERGED}（数量 2）：{}", left.id)];
        b.add(counted);
        b.tidy();
        assert_eq!(detail(&b.only(), "数量").as_deref(), Some("2"));
    }

    #[test]
    fn what_was_written_of_how_many_goes_to_the_log() {
        let b = book("same-written");
        let mut mine = thing("鍵盤");
        mine.source = String::new();
        b.add(mine);
        b.add(several("鍵盤", "2個（うち1個は予備）"));
        b.tidy();
        let said = said(&b.only());
        assert!(said.contains("数量: 2個（うち1個は予備）"), "{said}");
    }

    #[test]
    fn frontmatter_and_addresses_not_carried_over_go_to_the_log() {
        let b = book("same-meta");
        let with_url = |url: &str| {
            let mut p = thing("鍵盤");
            p.url = Some(url.into());
            p
        };
        let mut mine = with_url("https://example.com/mine");
        mine.source = String::new();
        let mine = b.add(mine);
        let other = b.add(with_url("https://example.com/other"));
        let path = b.records.notebook().root().join(other.rel());
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replacen("---\n", "---\nrating: 5\n", 1)).unwrap();
        b.tidy();
        let kept = b.only();
        assert_eq!(kept.id, mine.id);
        let said = said(&kept);
        assert!(said.contains("url: https://example.com/other"), "{said}");
        assert!(said.contains("rating: 5"), "{said}");
    }

    fn said(r: &Record) -> String {
        let said: Vec<String> = log_lines(r).into_iter().map(|l| l.text).collect();
        said.join("\n")
    }

    #[test]
    fn a_picture_is_not_given_up_for_one_that_was_not_found() {
        let b = book("same-picture");
        let mut mine = thing("鍵盤");
        mine.source = String::new();
        let mine = b.add(mine);
        let other = b.add(thing("鍵盤"));
        for (holding, bytes) in [(&mine.id, &b""[..]), (&other.id, &b"image"[..])] {
            b.store
                .conn()
                .execute(
                    "INSERT INTO holding_pictures (holding, id, mime, bytes)
                     VALUES (?1, 'p', 'image/png', ?2)",
                    params![holding, bytes],
                )
                .unwrap();
        }
        b.tidy();
        let bytes: Vec<u8> = b
            .store
            .conn()
            .query_row(
                "SELECT bytes FROM holding_pictures WHERE holding = ?1",
                [&mine.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bytes, b"image");
    }

    #[test]
    fn links_to_what_was_merged_lead_to_what_is_left() {
        let b = book("same-links");
        let one = b.bought(1, thing("鍵盤"));
        let two = b.bought(1, thing("鍵盤"));
        let memo = |to: &Record| {
            b.records
                .add(
                    ME,
                    MEMOS,
                    Patch {
                        notes: Some(format!("{} のこと", to.link())),
                        ..Patch::titled(to.id.clone())
                    },
                )
                .unwrap()
        };
        let memos = [memo(&one), memo(&two)];
        let remembered = b.records.notebook().memory_dir().join("remembered.md");
        let with_extension = format!("[[{}.md|控え]]", two.target());
        std::fs::write(
            &remembered,
            format!("- {}\n- {with_extension}\n", two.link()),
        )
        .unwrap();
        b.tidy();
        let kept = b.only();
        for m in memos {
            let m = b.records.need(MEMOS, &m.id).unwrap();
            assert!(m.notes().contains(&kept.target()), "{}", m.notes());
        }
        let remembered = std::fs::read_to_string(remembered).unwrap();
        assert!(!remembered.contains(&two.id), "{remembered}");
        assert!(remembered.contains(&kept.target()));
    }

    #[test]
    fn the_mail_of_what_was_merged_is_still_that_of_what_is_left() {
        let b = book("same-ledger");
        let mine = b.add({
            let mut p = thing("鍵盤");
            p.source = String::new();
            p
        });
        b.bought(7, thing("鍵盤"));
        b.tidy();
        let mail: i64 = b
            .store
            .conn()
            .query_row(
                "SELECT mail FROM purchases WHERE holding = ?1",
                [&mine.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(mail, 7);
    }

    #[test]
    fn things_that_may_differ_are_left_apart() {
        let b = book("same-apart");
        let with_detail = |title: &str, name: &str, value: &str| {
            let mut p = thing(title);
            p.details = Some(vec![field(name, value)]);
            p
        };
        b.add(thing("本 1巻"));
        b.add(thing("本 2巻"));
        b.add(with_detail("靴", "サイズ", "26"));
        b.add(with_detail("靴", "サイズ", "27"));
        b.add(thing("無線機").with("Serial", "S1"));
        b.add(thing("無線機").with("Serial", "S2"));
        let mut dear = thing("ケーブル");
        dear.sections = vec![acquired("A-1", "1595 JPY", today())];
        b.add(dear);
        b.add(thing("ケーブル"));
        let mut again = thing("電池");
        again.sections = vec![acquired("B-2", "980 JPY", today() - Days::new(20))];
        b.add(again);
        b.add(thing("電池"));
        b.add(thing("椅子").with("Status", "released"));
        b.add(thing("椅子"));
        b.tidy();
        assert_eq!(b.list().len(), 12);
    }

    #[test]
    fn what_was_ordered_over_thirty_days_ago_is_owned() {
        let b = book("same-arrived");
        let aged = |title: &str, days: u64| {
            let mut p = thing(title);
            p.sections = vec![acquired(title, "980 JPY", today() - Days::new(days))];
            b.add(p)
        };
        let old = aged("古い注文", 31);
        let new = aged("新しい注文", 30);
        b.tidy();
        let status = |r: &Record| {
            b.records
                .need(HOLDINGS, &r.id)
                .unwrap()
                .body
                .value("Status")
        };
        assert_eq!(status(&old).as_deref(), Some("owned"));
        assert_eq!(status(&new).as_deref(), Some("ordered"));
    }

    #[test]
    fn nothing_is_touched_when_records_are_not_to_be_tidied() {
        let b = book("same-off");
        b.add(thing("鍵盤"));
        b.add(thing("鍵盤"));
        DreamSettings {
            tidy_records: false,
            ..Default::default()
        }
        .save(&b.store)
        .unwrap();
        assert!(b.tidy().is_none());
        assert_eq!(b.list().len(), 2);
    }
}
