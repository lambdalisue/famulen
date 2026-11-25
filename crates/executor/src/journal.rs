//! What happened, one JSON line each, in a file only the owner can read.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{BufRead as _, BufReader, Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::watch;

use crate::wire::{Record, What};

pub(crate) fn path(dir: &Path) -> PathBuf {
    dir.join("journal")
}

/// Room on the disk for the last record, whatever else fills it.
const RESERVE: usize = 16 * 1024;

fn reserve(dir: &Path) -> PathBuf {
    dir.join("reserve")
}

pub(crate) struct Journal {
    file: File,
    reserve: PathBuf,
    seq: u64,
    bytes: u64,
    limit: u64,
    /// Keys of lines written to the program, and of marks.
    written: HashSet<String>,
    marked: HashSet<String>,
    closed: bool,
    /// How many records are there to be read.
    kept: watch::Sender<u64>,
}

#[derive(Debug)]
pub(crate) enum NotKept {
    /// More was kept than may be.
    Full,
    /// Its last record is there.
    Closed,
    Failed,
}

impl Journal {
    pub(crate) fn create(dir: &Path, limit: u64) -> std::io::Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        std::fs::write(reserve(dir), vec![b' '; RESERVE])?;
        Ok(Self {
            file: options.open(path(dir))?,
            reserve: reserve(dir),
            seq: 0,
            bytes: 0,
            limit,
            written: HashSet::new(),
            marked: HashSet::new(),
            closed: false,
            kept: watch::Sender::new(0),
        })
    }

    pub(crate) fn watch(&self) -> watch::Receiver<u64> {
        self.kept.subscribe()
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }

    /// Write a line whole or not at all: after half of one nothing could be
    /// read.
    fn write(&mut self, line: &str) -> std::io::Result<()> {
        self.file.write_all(line.as_bytes()).inspect_err(|_| {
            let back = self.file.set_len(self.bytes);
            let back = back.and_then(|()| self.file.seek(SeekFrom::Start(self.bytes)));
            self.closed = back.is_err();
        })
    }

    /// Keep what happened. Whether it is new: a line or mark by a key that is
    /// there already is not kept again.
    pub(crate) fn keep(&mut self, what: What) -> Result<bool, NotKept> {
        if self.closed {
            return Err(NotKept::Closed);
        }
        let last = matches!(what, What::Exit { .. });
        if !last && self.bytes >= self.limit {
            return Err(NotKept::Full);
        }
        let fresh = match &what {
            What::In { key, .. } => !self.written.contains(key),
            What::Mark { key } => !self.marked.contains(key),
            _ => true,
        };
        if !fresh {
            return Ok(false);
        }
        let record = Record {
            seq: self.seq + 1,
            at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
            what,
        };
        let mut line = serde_json::to_string(&record).map_err(|_| NotKept::Failed)?;
        line.push('\n');
        let mut written = self.write(&line);
        if written.is_err() && last {
            std::fs::remove_file(&self.reserve).ok();
            written = self.write(&line);
        }
        if written.is_err() {
            return Err(NotKept::Failed);
        }
        self.seq = record.seq;
        self.bytes += line.len() as u64;
        match record.what {
            What::In { key, .. } => {
                self.written.insert(key);
            }
            What::Mark { key } => {
                self.marked.insert(key);
            }
            _ => {}
        }
        self.closed = last;
        self.kept.send_replace(self.seq);
        Ok(true)
    }
}

/// Reads what is there, one record after another.
pub(crate) struct Kept(BufReader<File>);

impl Kept {
    pub(crate) fn open(dir: &Path) -> std::io::Result<Self> {
        Ok(Self(BufReader::new(File::open(path(dir))?)))
    }

    /// `None` at the end, or at a record that was not kept whole.
    pub(crate) fn next(&mut self) -> std::io::Result<Option<Record>> {
        let mut line = Vec::new();
        self.0.read_until(b'\n', &mut line)?;
        if !line.ends_with(b"\n") {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&line)?))
    }
}

/// The first record.
pub(crate) fn first(dir: &Path) -> std::io::Result<Option<Record>> {
    Kept::open(dir)?.next()
}

/// The last record that was kept whole.
pub(crate) fn last(dir: &Path) -> std::io::Result<Option<Record>> {
    let mut file = File::open(path(dir))?;
    let length = file.metadata()?.len();
    let mut take = 64 * 1024;
    loop {
        let from = length.saturating_sub(take);
        file.seek(SeekFrom::Start(from))?;
        let mut tail = Vec::new();
        (&file).take(length - from).read_to_end(&mut tail)?;
        let Some(end) = tail.iter().rposition(|b| *b == b'\n') else {
            if from == 0 {
                return Ok(None);
            }
            take *= 4;
            continue;
        };
        let begin = tail[..end].iter().rposition(|b| *b == b'\n');
        match begin {
            Some(begin) => return Ok(Some(serde_json::from_slice(&tail[begin + 1..end])?)),
            None if from == 0 => return Ok(Some(serde_json::from_slice(&tail[..end])?)),
            None => take *= 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_whole_record_is_found_however_long_it_and_what_is_left_of_the_next_are() {
        let dir = std::env::temp_dir().join(format!("fx-journal-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let mut journal = Journal::create(&dir, 1 << 30).unwrap();
        assert_eq!(last(&dir).unwrap(), None);
        for length in [1, 70_000, 400_000, 3] {
            let what = What::Out {
                line: "x".repeat(length),
            };
            journal.keep(what.clone()).unwrap();
            assert_eq!(last(&dir).unwrap().unwrap().what, what);
        }
        let mut file = OpenOptions::new().append(true).open(path(&dir)).unwrap();
        file.write_all("y".repeat(100_000).as_bytes()).unwrap();
        let record = last(&dir).unwrap().unwrap();
        assert_eq!(record.seq, 4);
        assert_eq!(first(&dir).unwrap().unwrap().seq, 1);
        let mut kept = Kept::open(&dir).unwrap();
        let mut seqs = Vec::new();
        while let Some(record) = kept.next().unwrap() {
            seqs.push(record.seq);
        }
        assert_eq!(seqs, [1, 2, 3, 4]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
