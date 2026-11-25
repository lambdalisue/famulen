//! Parties: their accounts, who added them, and whether they rest.

use std::collections::BTreeSet;

use anyhow::Context as _;
use chrono::NaiveDate;
use famulen_notebook::body::{Item, NewItem};
use famulen_notebook::records::flag;

use super::{PARTIES, Patch, Record, Records};
use crate::notebook::Writer;

pub const LAST_SEEN: &str = "Last seen";
pub const ADDED_BY: &str = "Added by";
/// The `Status` of a party that was not seen for long.
pub const RESTING: &str = "resting";

/// The metadata that holds the day the owner put a party to rest.
const RESTED: &str = "rested";

pub fn is_resting(party: &Record) -> bool {
    party.kind == PARTIES && party.body.value("Status").as_deref() == Some(RESTING)
}

/// The day a party was last seen.
pub fn last_seen(party: &Record) -> Option<NaiveDate> {
    party.body.value(LAST_SEEN)?.trim().parse().ok()
}

/// What a party is called, its title first.
pub fn names_of(party: &Record) -> Vec<String> {
    std::iter::once(party.title())
        .chain(party.body.values("Aliases"))
        .collect()
}

/// A name without what does not tell one person from another.
pub fn bare(name: &str) -> String {
    let name: String = name.split_whitespace().collect::<String>().to_lowercase();
    ["さん", "様", "さま", "氏", "くん", "君", "ちゃん"]
        .iter()
        .find_map(|honorific| name.strip_suffix(honorific).filter(|n| !n.is_empty()))
        .unwrap_or(&name)
        .to_owned()
}

fn bare_names(party: &Record) -> BTreeSet<String> {
    names_of(party).iter().map(|n| bare(n)).collect()
}

fn kind_of(party: &Record) -> String {
    party
        .body
        .value("Kind")
        .filter(|k| !k.is_empty())
        .unwrap_or_else(|| "person".to_owned())
}

pub fn is_person(party: &Record) -> bool {
    kind_of(party) == "person"
}

/// Keep when a party was last seen, whatever order the days come in, and take
/// one that is seen again out of its rest. Whether it was.
pub(super) fn note_seen(
    party: &mut Record,
    before: Option<NaiveDate>,
    writer: Writer,
    status_given: bool,
    was_resting: bool,
) -> anyhow::Result<bool> {
    let now = last_seen(party);
    if writer != Writer::Owner {
        // A day of margin, as the owner's timezone may be ahead of this machine's.
        let latest = chrono::Local::now().date_naive() + chrono::Duration::days(1);
        anyhow::ensure!(
            now == before || now.is_none_or(|on| on <= latest),
            "{LAST_SEEN} is a day that has come"
        );
        if let Some(was) = before.filter(|was| now.is_none_or(|on| on < *was)) {
            party.body.set(LAST_SEEN, was.to_string())?;
        }
    }
    if writer == Writer::Owner && is_resting(party) && !was_resting {
        let today = chrono::Local::now().date_naive();
        party.meta.set(RESTED, today.to_string());
    }
    let rested: Option<NaiveDate> = party.meta.text(RESTED).and_then(|on| on.parse().ok());
    // What came before the owner put it to rest is no return.
    let returns = now > before && now > rested && !status_given && is_resting(party);
    if !is_resting(party) || returns {
        party.meta.set_opt(RESTED, None::<&str>);
    }
    if returns {
        party.body.set("Status", "")?;
    }
    Ok(returns)
}

/// Who added a party decides whether it may come to rest, so only the owner
/// changes it.
pub(super) fn may_set_added_by(
    writer: Writer,
    party: &Record,
    patch: &Patch,
) -> anyhow::Result<()> {
    let given = patch.given(ADDED_BY).filter(|_| party.kind == PARTIES);
    anyhow::ensure!(
        writer == Writer::Owner
            || given.is_none_or(|f| party.body.value(ADDED_BY).as_deref() == Some(&f.value)),
        "who added a party is the owner's to change"
    );
    Ok(())
}

/// An account of a party.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyAccount {
    /// `<service>:<handle>`.
    pub id: String,
    pub service: String,
    pub handle: String,
    pub confirmed: bool,
    pub how: String,
}

fn accounts(record: &Record) -> Vec<PartyAccount> {
    record
        .body
        .items("Accounts")
        .into_iter()
        .map(|item| {
            let of = |name: &str| item.nested(name).map(Item::value).unwrap_or_default();
            let service = item.plain();
            let handle = of("Handle");
            PartyAccount {
                id: account_id(&service, &handle),
                confirmed: flag(&of("Confirmed")).unwrap_or(false),
                how: of("How"),
                service,
                handle,
            }
        })
        .collect()
}

fn account_id(service: &str, handle: &str) -> String {
    format!("{service}:{handle}")
}

fn account_item(service: &str, handle: &str, confirmed: bool, how: &str) -> NewItem {
    NewItem::new(service)
        .with("Handle", handle)
        .with("Confirmed", if confirmed { "yes" } else { "no" })
        .with("How", how)
}

/// The accounts' handles of a party.
pub fn handles_of(party: &Record) -> Vec<String> {
    accounts(party).into_iter().map(|a| a.handle).collect()
}

impl Records {
    /// Refuse a name that makes a party the same as one there already, resting
    /// or not: the secretary would have one person twice. The owner may know
    /// two people of one name, and two may share an alias.
    pub(super) fn refuse_same_party(
        &self,
        writer: Writer,
        new: &Record,
        before: Option<&Record>,
    ) -> anyhow::Result<()> {
        if writer == Writer::Owner {
            return Ok(());
        }
        let was: BTreeSet<String> = before.map(bare_names).unwrap_or_default();
        let moved = before.is_some_and(|b| kind_of(b) != kind_of(new));
        let mut wanted = bare_names(new);
        wanted.retain(|n| moved || !was.contains(n));
        if wanted.is_empty() {
            return Ok(());
        }
        let title = bare(&new.title());
        for party in self.list(PARTIES)? {
            if party.id == new.id || kind_of(&party) != kind_of(new) {
                continue;
            }
            let same = wanted.contains(&bare(&party.title()))
                || (wanted.contains(&title) && bare_names(&party).contains(&title));
            anyhow::ensure!(
                !same,
                "a party of that name is there already: {} ({}). When it is the same, change that one; \
                 when it is surely another, give it a name that tells them apart",
                party.title(),
                self.code(PARTIES, &party.id)?
            );
        }
        Ok(())
    }

    /// Every account of every party, with the party.
    pub fn accounts(&self) -> anyhow::Result<Vec<(Record, Vec<PartyAccount>)>> {
        Ok(self
            .list(PARTIES)?
            .into_iter()
            .map(|r| {
                let accounts = accounts(&r);
                (r, accounts)
            })
            .collect())
    }

    /// Link an account to a party. An account already linked elsewhere moves.
    pub fn link_account(
        &self,
        writer: Writer,
        party: &str,
        service: &str,
        handle: &str,
        confirmed: bool,
        how: &str,
    ) -> anyhow::Result<()> {
        writer.may_write()?;
        anyhow::ensure!(
            !service.trim().is_empty() && !handle.trim().is_empty(),
            "an account needs a service and a handle"
        );
        let _writing = self.writing()?;
        let target = self.need(PARTIES, party)?.id;
        let wanted = account_id(service, handle);
        let mut was_confirmed = false;
        for (mut record, all) in self.accounts()? {
            let Some(at) = all.iter().position(|a| a.id == wanted) else {
                continue;
            };
            was_confirmed |= all[at].confirmed;
            if record.id != target {
                record.body.remove_item("Accounts", at)?;
                let subject = format!("{} から {wanted} を外した", record.title());
                self.notebook
                    .write(&writer.signature(), &record, &subject)?;
            }
        }
        let mut record = self.need(PARTIES, &target)?;
        let item = account_item(service, handle, confirmed || was_confirmed, how);
        match accounts(&record).iter().position(|a| a.id == wanted) {
            Some(at) => record.body.replace_item("Accounts", at, &item)?,
            None => record.body.add_item("Accounts", &item)?,
        }
        let subject = format!("{} に {wanted} をつないだ", record.title());
        self.notebook
            .write(&writer.signature(), &record, &subject)?;
        Ok(())
    }

    /// Answer a guessed account: the same party confirms it, another removes it.
    pub fn answer_account(&self, writer: Writer, account: &str, same: bool) -> anyhow::Result<()> {
        writer.may_write()?;
        let _writing = self.writing()?;
        let (mut record, at) = self
            .accounts()?
            .into_iter()
            .find_map(|(r, all)| Some((r, all.iter().position(|a| a.id == account)?)))
            .with_context(|| format!("no account {account}"))?;
        if same {
            record
                .body
                .set_item_field("Accounts", at, "Confirmed", "yes")?;
            record
                .body
                .set_item_field("Accounts", at, "How", "あなたが確かめた")?;
        } else {
            record.body.remove_item("Accounts", at)?;
        }
        let subject = format!("{} の {account} を確かめた", record.title());
        self.notebook
            .write(&writer.signature(), &record, &subject)?;
        Ok(())
    }

    /// The party a confirmed account belongs to. Guessed links are not used.
    pub fn party_of(&self, service: &str, handle: &str) -> anyhow::Result<Option<String>> {
        Ok(self.accounts()?.into_iter().find_map(|(r, all)| {
            all.iter()
                .any(|a| a.confirmed && a.service == service && a.handle == handle)
                .then_some(r.id)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::Patch;
    use crate::records::testing::records;
    use crate::records::tests::{ME, add};

    #[test]
    fn a_guessed_account_is_not_used_until_it_is_confirmed_and_one_account_has_one_party() {
        let (_tmp, r) = records("records-accounts");
        let a = add(&r, PARTIES, "田中");
        let b = add(&r, PARTIES, "佐藤");

        r.link_account(ME, &a.id, "slack", "U1", false, "名前が同じ")
            .unwrap();
        assert_eq!(r.party_of("slack", "U1").unwrap(), None);
        r.answer_account(ME, "slack:U1", true).unwrap();
        assert_eq!(r.party_of("slack", "U1").unwrap(), Some(a.id.clone()));
        r.link_account(ME, &b.id, "slack", "U1", false, "移った")
            .unwrap();
        assert_eq!(r.party_of("slack", "U1").unwrap(), Some(b.id.clone()));
        assert!(accounts(&r.need(PARTIES, &a.id).unwrap()).is_empty());
        r.answer_account(ME, "slack:U1", false).unwrap();
        assert!(accounts(&r.need(PARTIES, &b.id).unwrap()).is_empty());
    }

    fn seen(r: &Records, writer: Writer, id: &str, on: &str) -> anyhow::Result<Record> {
        let patch = Patch::default().with("Last seen", on);
        r.change(writer, PARTIES, id, patch)
    }

    #[test]
    fn a_party_at_rest_returns_when_seen_later_but_not_when_seen_no_later() {
        let (_tmp, r) = records("records-resting");
        let p = r
            .add(
                Writer::Secretary,
                PARTIES,
                Patch::titled("田中")
                    .with("Last seen", "2026-09-02")
                    .with("Status", RESTING),
            )
            .unwrap();

        let p = seen(&r, Writer::Secretary, &p.id, "2026-09-01").unwrap();
        assert!(is_resting(&p));
        assert_eq!(p.body.value("Last seen").as_deref(), Some("2026-09-02"));
        let p = seen(&r, Writer::Secretary, &p.id, "2026-09-03").unwrap();
        assert!(!is_resting(&p));
        assert!(seen(&r, Writer::Secretary, &p.id, "2999-01-01").is_err());
        let p = seen(&r, Writer::Owner, &p.id, "2999-01-01").unwrap();
        let alias = Patch::default().with("Aliases", "たなっち");
        r.change(Writer::Secretary, PARTIES, &p.id, alias).unwrap();
        let p = seen(&r, Writer::Owner, &p.id, "2026-09-01").unwrap();
        assert_eq!(p.body.value("Last seen").as_deref(), Some("2026-09-01"));
    }

    #[test]
    fn who_added_a_party_is_kept_and_only_the_owner_changes_it() {
        let (_tmp, r) = records("records-added-by");
        let by = |p: &Record| p.body.value("Added by");
        let owners = r
            .add(Writer::Owner, PARTIES, Patch::titled("田中"))
            .unwrap();
        let own = r
            .add(Writer::Secretary, PARTIES, Patch::titled("佐藤"))
            .unwrap();
        let asked = r
            .add(
                Writer::Secretary,
                PARTIES,
                Patch::titled("鈴木").with("Added by", "owner"),
            )
            .unwrap();
        assert_eq!(by(&owners).as_deref(), Some("owner"));
        assert_eq!(by(&own).as_deref(), Some("secretary"));
        assert_eq!(by(&asked).as_deref(), Some("owner"));

        let patch = || Patch::default().with("Added by", "secretary");
        for id in [&owners.id, &asked.id] {
            assert!(r.change(Writer::Secretary, PARTIES, id, patch()).is_err());
        }
        r.change(Writer::Secretary, PARTIES, &own.id, patch())
            .unwrap();
        r.change(Writer::Owner, PARTIES, &asked.id, patch())
            .unwrap();
    }

    #[test]
    fn the_secretary_adds_no_second_party_of_the_same_name() {
        let (_tmp, r) = records("records-same-party");
        let tanaka = r
            .add(
                Writer::Secretary,
                PARTIES,
                Patch::titled("田中 太郎")
                    .with("Aliases", "Tanaka Taro")
                    .with("Status", RESTING),
            )
            .unwrap();
        let add = |writer, patch| r.add(writer, PARTIES, patch);

        for name in ["田中太郎さん", "tanaka taro"] {
            let refused = add(Writer::Secretary, Patch::titled(name)).unwrap_err();
            let code = r.code(PARTIES, &tanaka.id).unwrap();
            assert!(refused.to_string().contains(&code), "{refused}");
        }
        let alias = Patch::titled("T").with("Aliases", "田中太郎");
        assert!(add(Writer::Secretary, alias).is_err());
        let other = Patch::titled("U123").with("Aliases", "Tanaka Taro");
        let other = add(Writer::Secretary, other).unwrap();
        let renamed = Patch::titled("田中太郎");
        let refused = r.change(Writer::Secretary, PARTIES, &other.id, renamed);
        assert!(refused.is_err());
        let same = Patch::titled("田中 太郎").with("Birthday", "2000-01-01");
        r.change(Writer::Secretary, PARTIES, &tanaka.id, same)
            .unwrap();
        let org = Patch::titled("田中太郎").with("Kind", "org");
        let org = add(Writer::Secretary, org).unwrap();
        let person = Patch::default().with("Kind", "person");
        assert!(
            r.change(Writer::Secretary, PARTIES, &org.id, person)
                .is_err()
        );

        add(Writer::Owner, Patch::titled("田中太郎")).unwrap();
    }

    #[test]
    fn what_came_before_the_owner_put_a_party_to_rest_does_not_return_it() {
        let (_tmp, r) = records("records-rested-by-hand");
        let p = add(&r, PARTIES, "田中");
        let rest = Patch::default().with("Status", RESTING);
        r.change(Writer::Owner, PARTIES, &p.id, rest).unwrap();

        let p = seen(&r, Writer::Secretary, &p.id, "2026-09-01").unwrap();
        assert!(is_resting(&p));
        let tomorrow = chrono::Local::now().date_naive() + chrono::Duration::days(1);
        let p = seen(&r, Writer::Secretary, &p.id, &tomorrow.to_string()).unwrap();
        assert!(!is_resting(&p));
        assert_eq!(p.meta.text(RESTED), None);
    }
}
