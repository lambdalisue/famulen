//! The workgroups that records and memories belong to, in
//! `records/workgroups.md`, in the order they are shown.

use crate::body::{Body, Layout, NewItem};
use crate::{Notebook, Signature};

/// The file, relative to the notebook.
pub const WORKGROUPS: &str = "records/workgroups.md";

pub(crate) const TEMPLATE: &str = include_str!("../templates/workgroups.md");

const LIST: &str = "Workgroups";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workgroup {
    pub id: String,
    pub name: String,
    pub color: String,
}

fn parse(text: &str) -> anyhow::Result<Body> {
    Body::parse(
        text,
        &Layout {
            free: false,
            sections: vec![LIST.into()],
        },
    )
}

fn load(notebook: &Notebook) -> anyhow::Result<Body> {
    let path = notebook.root().join(WORKGROUPS);
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => parse(TEMPLATE),
        Err(e) => Err(e.into()),
    }
}

/// The workgroups a text lists; what cannot be read is an error.
pub(crate) fn read(text: &str) -> anyhow::Result<Vec<Workgroup>> {
    let body = parse(text)?;
    let mut all: Vec<Workgroup> = Vec::new();
    for item in body.items(LIST) {
        let id = item.plain();
        anyhow::ensure!(
            !id.is_empty() && !id.contains([',', '[', ']']) && !id.contains(char::is_whitespace),
            "a workgroup's id is one word, not {id:?}"
        );
        anyhow::ensure!(
            !all.iter().any(|w| w.id == id),
            "the workgroup {id} is there twice"
        );
        let of = |name: &str| item.nested(name).map(|i| i.value()).unwrap_or_default();
        let name = Some(of("Name"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.clone());
        all.push(Workgroup {
            name,
            color: of("Color"),
            id,
        });
    }
    Ok(all)
}

pub fn list(notebook: &Notebook) -> anyhow::Result<Vec<Workgroup>> {
    read(load(notebook)?.text())
}

fn save(
    notebook: &Notebook,
    signature: &Signature,
    body: &Body,
    subject: &str,
) -> anyhow::Result<()> {
    read(body.text())?;
    std::fs::write(notebook.root().join(WORKGROUPS), body.text())?;
    notebook.commit(signature, subject, &[WORKGROUPS])?;
    Ok(())
}

/// Add a workgroup, or change the one of its id.
pub fn set(
    notebook: &Notebook,
    signature: &Signature,
    workgroup: &Workgroup,
) -> anyhow::Result<()> {
    let _hold = notebook.hold()?;
    let mut body = load(notebook)?;
    let at = body
        .items(LIST)
        .iter()
        .position(|i| i.plain() == workgroup.id);
    match at {
        Some(at) => {
            body.set_item_field(LIST, at, "Name", workgroup.name.as_str())?;
            body.set_item_field(LIST, at, "Color", workgroup.color.as_str())?;
        }
        None => body.add_item(
            LIST,
            &NewItem::new(&workgroup.id)
                .with("Name", workgroup.name.as_str())
                .with("Color", workgroup.color.as_str()),
        )?,
    }
    save(
        notebook,
        signature,
        &body,
        &format!("ワークグループ {} を書いた", workgroup.name),
    )
}

pub fn remove(notebook: &Notebook, signature: &Signature, id: &str) -> anyhow::Result<()> {
    let _hold = notebook.hold()?;
    let mut body = load(notebook)?;
    let at = body.items(LIST).iter().position(|i| i.plain() == id);
    if let Some(at) = at {
        body.remove_item(LIST, at)?;
        save(
            notebook,
            signature,
            &body,
            &format!("ワークグループ {id} を消した"),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{me, notebook};

    #[test]
    fn workgroups_are_written_changed_and_removed_in_their_order() {
        let (_tmp, nb) = notebook("workgroups");
        let me = me();
        assert_eq!(list(&nb).unwrap(), []);
        let of = |id: &str, name: &str, color: &str| Workgroup {
            id: id.into(),
            name: name.into(),
            color: color.into(),
        };

        set(&nb, &me, &of("work", "仕事", "blue")).unwrap();
        set(&nb, &me, &of("personal", "個人", "green")).unwrap();
        set(&nb, &me, &of("work", "会社", "red")).unwrap();

        assert_eq!(
            list(&nb).unwrap(),
            [of("work", "会社", "red"), of("personal", "個人", "green")]
        );
        remove(&nb, &me, "work").unwrap();
        assert_eq!(list(&nb).unwrap().len(), 1);
        assert!(read("# x\n\n## Workgroups\n\n- a\n- a\n").is_err());
    }
}
