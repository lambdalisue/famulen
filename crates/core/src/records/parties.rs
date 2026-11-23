//! The accounts of parties.

use anyhow::Context as _;
use famulen_notebook::body::{Item, NewItem};
use famulen_notebook::records::flag;

use super::{PARTIES, Record, Records};
use crate::notebook::Writer;

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

impl Records {
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
        let _writing = self.writing();
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
        let _writing = self.writing();
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
}
