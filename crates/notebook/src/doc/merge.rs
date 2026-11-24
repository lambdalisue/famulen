//! Making one of a memory file that changed in two copies of a notebook.

use std::collections::BTreeMap;

use super::{Doc, Line};
use crate::Entry;

/// Make one of what two copies made of `base`, item by item by their ids;
/// `None` when both changed the same item, or the rest of the file, in
/// different ways, or an item cannot be told by its id.
pub(crate) fn merge(base: &Doc, ours: &Doc, theirs: &Doc) -> Option<Doc> {
    let by_id = [base, ours, theirs].map(items);
    let [Some(was), Some(our), Some(their)] = by_id else {
        return None;
    };
    let rest = [base, ours, theirs].map(other_lines);
    let (kept, also) = if rest[1] == rest[2] || rest[2] == rest[0] {
        (ours, &their)
    } else if rest[1] == rest[0] {
        (theirs, &our)
    } else {
        return None;
    };
    let settle = |id: &str| {
        one_of(
            was.get(id).copied(),
            our.get(id).copied(),
            their.get(id).copied(),
        )
    };
    let mut out = Doc::default();
    for line in &kept.lines {
        match line {
            Line::Other(_) => out.lines.push(line.clone()),
            Line::Entry(entry) => {
                let settled = settle(entry.id()?)?;
                out.lines.extend(settled.cloned().map(Line::Entry));
            }
        }
    }
    for (id, _) in also.iter().filter(|(id, _)| kept.find(id).is_none()) {
        if let Some(entry) = settle(id)? {
            out.push(entry.clone()).ok()?;
        }
    }
    Some(out)
}

/// What one item becomes: `Some(None)` is gone, `None` cannot be settled.
fn one_of<'a>(
    was: Option<&'a Entry>,
    ours: Option<&'a Entry>,
    theirs: Option<&'a Entry>,
) -> Option<Option<&'a Entry>> {
    if ours == theirs || theirs == was {
        Some(ours)
    } else if ours == was {
        Some(theirs)
    } else {
        None
    }
}

fn items(doc: &Doc) -> Option<BTreeMap<&str, &Entry>> {
    let mut items = BTreeMap::new();
    for entry in doc.entries() {
        if items.insert(entry.id()?, entry).is_some() {
            return None;
        }
    }
    Some(items)
}

fn other_lines(doc: &Doc) -> Vec<&str> {
    doc.lines
        .iter()
        .filter_map(|line| match line {
            Line::Other(text) if !text.trim().is_empty() => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged(base: &str, ours: &str, theirs: &str) -> Option<String> {
        merge(&Doc::parse(base), &Doc::parse(ours), &Doc::parse(theirs)).map(|d| d.render())
    }

    const BASE: &str = "# 心得\n\n- 一 {id: a}\n- 二 {id: b}\n";

    #[test]
    fn items_added_apart_are_both_kept() {
        assert_eq!(
            merged(
                BASE,
                &format!("{BASE}- 三 {{id: c}}\n"),
                &format!("{BASE}- 四 {{id: d}}\n")
            )
            .unwrap(),
            format!("{BASE}- 三 {{id: c}}\n- 四 {{id: d}}\n")
        );
        assert_eq!(
            merged("", "# 日\n\n- 三 {id: c}\n", "# 日\n\n- 四 {id: d}\n").unwrap(),
            "# 日\n\n- 三 {id: c}\n- 四 {id: d}\n"
        );
    }

    #[test]
    fn a_change_or_removal_on_one_side_is_kept() {
        assert_eq!(
            merged(
                BASE,
                "# 心得\n\n- 一 {id: a}\n",
                "# 心得\n\n- 壱 {id: a}\n- 二 {id: b}\n"
            )
            .unwrap(),
            "# 心得\n\n- 壱 {id: a}\n"
        );
        assert_eq!(
            merged(
                BASE,
                &format!("{BASE}- 三 {{id: c}}\n"),
                "# 決まり\n\n- 二 {id: b}\n"
            )
            .unwrap(),
            "# 決まり\n\n- 二 {id: b}\n- 三 {id: c}\n"
        );
    }

    #[test]
    fn an_item_changed_in_two_ways_is_not_settled() {
        let ours = "# 心得\n\n- 壱 {id: a}\n- 二 {id: b}\n";
        assert_eq!(
            merged(BASE, ours, "# 心得\n\n- 甲 {id: a}\n- 二 {id: b}\n"),
            None
        );
        assert_eq!(merged(BASE, ours, "# 心得\n\n- 二 {id: b}\n"), None);
        assert_eq!(merged(BASE, "# 心得\n\n- 二 {id: b}\n", ours), None);
        assert_eq!(merged("", "- 三 {id: c}\n", "- 参 {id: c}\n"), None);
    }

    #[test]
    fn what_cannot_be_told_by_its_id_is_not_settled() {
        assert_eq!(merged(BASE, BASE, &format!("{BASE}- 三\n")), None);
        assert_eq!(merged(BASE, BASE, &format!("{BASE}- 一 {{id: a}}\n")), None);
        assert_eq!(merged("# 一\n", "# 二\n", "# 三\n"), None);
    }
}
