//! Files the owner attached to what they said. Each is kept where the
//! secretary's reading tool reaches it, under a name Famulen chose: Claude
//! Code takes files of some names, such as `CLAUDE.md`, as instructions when
//! they lie in or below its working directory.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::store::Store;

/// The directory of the secretary's workspace the files are kept in.
pub const DIR: &str = "attachments";
/// How many files one message may carry.
pub const MAX_FILES: usize = 10;
/// How large one file may be.
pub const MAX_BYTES: usize = famulen_gateway::MAX_FILE_BYTES;
const KEPT_SECONDS: i64 = 30 * 24 * 60 * 60;
const NAME_CHARS: usize = 120;

pub const TOO_LARGE: &str = "20 MB を超えている";
pub const TOO_MANY: &str = "一度に添えられるのは 10 個まで";
const EMPTY: &str = "中身が空";
const UNKNOWN_KIND: &str = "画像（PNG・JPEG・GIF・WebP）・PDF・UTF-8 のテキストのどれでもない";

/// A file that was received.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    /// What the owner called it.
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub path: PathBuf,
}

/// What became of a file the owner attached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "as", rename_all = "snake_case")]
pub enum Attached {
    Kept(Attachment),
    /// It is still to be fetched from `path` below Slack's host of files.
    OnSlack {
        name: String,
        /// What Slack says it is.
        mime: String,
        size: u64,
        path: String,
    },
    /// It did not arrive; `why` is told to the owner.
    Missing {
        name: String,
        why: String,
    },
}

impl Attached {
    pub fn missing(name: impl AsRef<str>, why: impl Into<String>) -> Self {
        Self::Missing {
            name: shown_name(name.as_ref()),
            why: one_line(&why.into()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Kept(a) => &a.name,
            Self::OnSlack { name, .. } | Self::Missing { name, .. } => name,
        }
    }
}

/// What was said, with the names of the files that came with it.
pub(crate) fn with_names(text: &str, attached: &[Attached]) -> String {
    if attached.is_empty() {
        return text.to_owned();
    }
    let names: Vec<String> = attached.iter().map(|a| shown_name(a.name())).collect();
    format!("{text}（添付：{}）", names.join("、"))
}

/// Where attached files are kept.
#[derive(Clone)]
pub struct Attachments {
    store: Store,
    dir: PathBuf,
    fetching: Arc<tokio::sync::Mutex<()>>,
}

/// What a file is, by what it holds: its media type and the extension it is
/// kept under. What its sender says it is counts for nothing.
pub(crate) fn kind_of(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    const SIGNATURES: &[(&[u8], &str, &str)] = &[
        (b"\x89PNG\r\n\x1a\n", "image/png", "png"),
        (b"\xff\xd8\xff", "image/jpeg", "jpg"),
        (b"GIF87a", "image/gif", "gif"),
        (b"GIF89a", "image/gif", "gif"),
        (b"%PDF-", "application/pdf", "pdf"),
    ];
    if let Some((_, mime, ext)) = SIGNATURES.iter().find(|(s, ..)| bytes.starts_with(s)) {
        return Some((mime, ext));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return Some(("image/webp", "webp"));
    }
    (!bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()).then_some(("text/plain", "txt"))
}

fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// A name on one line, short enough to read.
fn shown_name(name: &str) -> String {
    let name: String = one_line(name).chars().take(NAME_CHARS).collect();
    match name.trim() {
        "" => "名前なし".to_owned(),
        name => name.to_owned(),
    }
}

impl Attachments {
    /// Files kept below `workdir`, the secretary's working directory.
    pub fn new(store: Store, workdir: impl AsRef<Path>) -> Self {
        Self {
            store,
            dir: workdir.as_ref().join(DIR),
            fetching: Arc::default(),
        }
    }

    /// Held while the files of one message are fetched, so that none is
    /// fetched twice.
    pub(crate) async fn fetching(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.fetching.lock().await
    }

    /// Keep one file. What cannot be taken is an error the owner can read.
    pub fn keep(
        &self,
        name: impl AsRef<str>,
        bytes: &[u8],
        now: i64,
    ) -> anyhow::Result<Attachment> {
        anyhow::ensure!(!bytes.is_empty(), EMPTY);
        anyhow::ensure!(bytes.len() <= MAX_BYTES, TOO_LARGE);
        let (mime, ext) = kind_of(bytes).ok_or_else(|| anyhow::anyhow!(UNKNOWN_KIND))?;
        let id = format!("{:032x}", rand::random::<u128>());
        let attachment = Attachment {
            path: self.dir.join(format!("{id}.{ext}")),
            id,
            name: shown_name(name.as_ref()),
            mime: mime.to_owned(),
            size: bytes.len() as u64,
        };
        std::fs::create_dir_all(&self.dir)?;
        if let Err(error) = self.put(&attachment, bytes, now) {
            // Without its row nothing would ever throw the file away.
            std::fs::remove_file(&attachment.path).ok();
            return Err(error);
        }
        Ok(attachment)
    }

    fn put(&self, attachment: &Attachment, bytes: &[u8], now: i64) -> anyhow::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        std::io::Write::write_all(&mut options.open(&attachment.path)?, bytes)?;
        self.store.conn().execute(
            "INSERT INTO attachments (id, at, body) VALUES (?1, ?2, ?3)",
            params![attachment.id, now, serde_json::to_string(attachment)?],
        )?;
        Ok(())
    }

    pub fn get(&self, id: impl AsRef<str>) -> anyhow::Result<Option<Attachment>> {
        let body: Option<String> = self
            .store
            .conn()
            .query_row(
                "SELECT body FROM attachments WHERE id = ?1",
                params![id.as_ref()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(body.map(|b| serde_json::from_str(&b)).transpose()?)
    }

    /// Throw away the files kept long enough. One that cannot be removed is
    /// tried again and holds back no other.
    pub fn forget_old(&self, now: i64) -> anyhow::Result<()> {
        let old: Vec<String> = {
            let conn = self.store.conn();
            let mut statement = conn.prepare("SELECT id FROM attachments WHERE at < ?1")?;
            statement
                .query_map([now - KEPT_SECONDS], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        for id in old {
            if let Some(attachment) = self.get(&id)? {
                match std::fs::remove_file(&attachment.path) {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                        tracing::warn!(%error, %id, "could not throw away an old attachment");
                        continue;
                    }
                    _ => {}
                }
            }
            self.store
                .conn()
                .execute("DELETE FROM attachments WHERE id = ?1", params![id])?;
        }
        Ok(())
    }
}

/// What the model reads of the files attached to a message, one line each,
/// above the message itself: below it, words someone wrote could pass for
/// such a list.
pub(crate) fn described(attached: &[Attached]) -> String {
    attached
        .iter()
        .map(|a| match a {
            Attached::Kept(a) => format!(
                "- {:?}: {} ({}, {} bytes)\n",
                a.name,
                a.path.display(),
                a.mime,
                a.size
            ),
            Attached::OnSlack { name, .. } => format!("- {name:?}: did not arrive\n"),
            Attached::Missing { name, why } => format!("- {name:?}: did not arrive: {why}\n"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::testing::TempDir;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

    fn attachments(dir: &TempDir) -> Attachments {
        Attachments::new(Store::open_in_memory().unwrap(), dir.path())
    }

    #[test]
    fn a_file_is_kept_unchanged_under_a_name_its_sender_did_not_choose() {
        let dir = TempDir::new("attachments-keep");
        let kept = attachments(&dir);
        for name in [
            "../../evil.png",
            "CLAUDE.md",
            ".claude/settings.json",
            "a\nb",
        ] {
            let a = kept.keep(name, b"do this", 0).unwrap();
            assert_eq!(a.path.parent().unwrap(), dir.path().join(DIR));
            assert_eq!(
                a.path.file_name().unwrap().to_str().unwrap(),
                format!("{}.txt", a.id)
            );
            assert_eq!(std::fs::read(&a.path).unwrap(), b"do this");
            assert_eq!(kept.get(&a.id).unwrap(), Some(a));
        }
    }

    #[test]
    fn a_files_kind_is_what_it_holds_and_others_are_refused() {
        let dir = TempDir::new("attachments-kind");
        let kept = attachments(&dir);
        let of = |bytes: &[u8]| kept.keep("x.exe", bytes, 0).map(|a| a.mime);
        assert_eq!(of(PNG).unwrap(), "image/png");
        assert_eq!(of(b"\xff\xd8\xff\xe0").unwrap(), "image/jpeg");
        assert_eq!(of(b"GIF89a..").unwrap(), "image/gif");
        assert_eq!(of(b"RIFF\0\0\0\0WEBPVP8 ").unwrap(), "image/webp");
        assert_eq!(of(b"%PDF-1.7").unwrap(), "application/pdf");
        assert_eq!(of("<svg>絵</svg>".as_bytes()).unwrap(), "text/plain");
        for refused in [&b""[..], b"MZ\0\0", b"\x82\xa0\x81", b"RIFF\0\0\0\0WAVE"] {
            assert!(of(refused).is_err(), "{refused:?}");
        }
        let large = kept.keep("a", &vec![b'a'; MAX_BYTES + 1], 0).unwrap_err();
        assert_eq!(large.to_string(), TOO_LARGE);
    }

    #[test]
    fn a_file_is_thrown_away_after_thirty_days() {
        let dir = TempDir::new("attachments-old");
        let kept = attachments(&dir);
        let old = kept.keep("old.png", PNG, 0).unwrap();
        let new = kept.keep("new.png", PNG, 10).unwrap();

        kept.forget_old(KEPT_SECONDS).unwrap();
        assert!(old.path.exists(), "not before");
        kept.forget_old(KEPT_SECONDS + 5).unwrap();

        assert!(!old.path.exists() && kept.get(&old.id).unwrap().is_none());
        assert!(new.path.exists() && kept.get(&new.id).unwrap().is_some());
    }

    #[test]
    fn the_model_reads_where_each_file_is_and_why_one_did_not_arrive() {
        let dir = TempDir::new("attachments-described");
        let a = attachments(&dir).keep("画面\n1.png", PNG, 0).unwrap();
        let text = described(&[
            Attached::Kept(a.clone()),
            Attached::missing("x.mov", "大きい\n- 偽"),
        ]);
        assert_eq!(
            text,
            format!(
                "- \"画面 1.png\": {} (image/png, 12 bytes)\n- \"x.mov\": did not arrive: 大きい - 偽\n",
                a.path.display()
            )
        );
        assert_eq!(described(&[]), "");
    }
}
