use super::*;

/// The detail that says how many of one thing there are, written only when
/// there are several.
pub(crate) const QUANTITY: &str = "数量";

/// The count a value of the detail says, when it says nothing else.
pub(crate) fn counted(value: &str) -> Option<u32> {
    let value = value.trim();
    let value = ["点", "個", "冊"]
        .iter()
        .find_map(|unit| value.strip_suffix(unit))
        .unwrap_or(value);
    value.parse().ok().filter(|n| *n > 0)
}

/// How many the record says there are; one when it says nothing a count can be
/// read from.
pub(crate) fn quantity(record: &Record) -> u32 {
    details(record)
        .iter()
        .filter(|d| d.name == QUANTITY)
        .filter_map(|d| counted(&d.value))
        .max()
        .unwrap_or(1)
}

/// The details with how many set, keeping what was written there that says
/// something else than a count.
pub(crate) fn with_quantity(mut details: Vec<Field>, many: u32) -> Vec<Field> {
    let mut said = Vec::new();
    details.retain(|d| {
        let quantity = d.name == QUANTITY;
        if quantity && counted(&d.value).is_none() {
            said.push(d.value.clone());
        }
        !quantity
    });
    if many > 1 {
        details.push(Field::new(QUANTITY, many.to_string()));
    }
    if !said.is_empty() {
        details.push(Field::new("数量の記載", said.join("、")));
    }
    details
}

impl Held<'_> {
    /// Lead every link to one record to another, and every record moved into
    /// it as well.
    pub(crate) fn relink(
        &self,
        writer: Writer,
        kind: &str,
        from: &str,
        to: &str,
    ) -> anyhow::Result<()> {
        writer.may_write()?;
        let r = self.records();
        let subject = format!("{from} へのリンクを {to} に付け替えた");
        r.notebook
            .relink(&writer.signature(), kind, from, to, &subject)?;
        let moved_from = Some((kind.to_owned(), from.to_owned()));
        let target = format!("{}/{kind}/{to}", famulen_notebook::RECORDS);
        for name in r.notebook.kind_names()? {
            for mut record in r.list(&name)? {
                if record.moved() == moved_from {
                    record.meta.set("moved", target.clone());
                    r.notebook.write(&writer.signature(), &record, &subject)?;
                }
            }
        }
        Ok(())
    }
}
