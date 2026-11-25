//! What an executor and Famulen write to each other, and what is kept.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The protocols this version speaks. One is spoken on for as long as work
/// started by a version that knows no newer one may still be going on.
pub const PROTOCOLS: RangeInclusive<u32> = 1..=1;
/// What stands for a protocol where none is shared: work can be stopped, and
/// nothing else.
pub(crate) const NONE: u32 = 0;

/// Work as an executor is started on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    /// What whoever asks shows to have, or is refused.
    pub key: String,
    /// The directory it makes to keep what it keeps, which must not be there.
    pub dir: PathBuf,
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    /// The lines the program's stdin begins with.
    pub input: Vec<String>,
    /// How long the program may run.
    pub limit_secs: u64,
    /// How much may be kept of it.
    pub limit_bytes: u64,
    /// What Famulen wants to read back of it.
    pub meta: Value,
}

/// One thing that happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Counted from 1, with none left out.
    pub seq: u64,
    /// Unix milliseconds.
    pub at: i64,
    #[serde(flatten)]
    pub what: What,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum What {
    /// The first.
    Start {
        version: String,
        pid: u32,
        /// What tells the process from a later one of the same number.
        born: Option<String>,
        /// What tells the time the system has been up from another.
        boot: Option<String>,
        meta: Value,
    },
    /// A line the program wrote.
    Out { line: String },
    /// A line it wrote to its stderr.
    Err { line: String },
    /// A line it was written.
    In { key: String, line: String },
    /// Something Famulen wants to know it did.
    Mark { key: String },
    /// The last.
    Exit { why: Why, code: Option<i32> },
    /// Of a version to come.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// By itself.
    Ended,
    Stopped,
    TookTooLong,
    WroteTooMuch,
    /// What it wrote could not be kept.
    NotKept,
    /// The executor was told to end by the system.
    Signalled,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Request {
    /// The first; every protocol keeps it, `Prove` and `Stop` as they are.
    Hello {
        from: u32,
        to: u32,
        nonce: String,
    },
    /// The second.
    Prove {
        proof: String,
    },
    Stop {
        grace_ms: u64,
    },
    /// Every record after `after`, as they come.
    Attach {
        after: u64,
    },
    /// Write a line to the program, unless one was written by this key.
    Write {
        key: String,
        line: String,
    },
    /// Keep a mark, unless it is there.
    Mark {
        key: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Reply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) protocol: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) proof: Option<String>,
    /// Whether the key was not there before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) fresh: Option<bool>,
    /// How many records there are now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) kept: Option<u64>,
}

impl Reply {
    pub(crate) fn error(error: impl Into<String>) -> Self {
        Self {
            error: Some(error.into()),
            ..Self::default()
        }
    }
}

/// The newest protocol in both ranges.
pub(crate) fn shared(ours: RangeInclusive<u32>, from: u32, to: u32) -> u32 {
    let newest = (*ours.end()).min(to);
    if newest >= *ours.start() && newest >= from {
        newest
    } else {
        NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_protocol_both_speak_is_spoken() {
        for (ours, theirs, expected) in [
            (1..=1, (1, 1), 1),
            (1..=3, (2, 5), 3),
            (2..=4, (1, 2), 2),
            (3..=4, (1, 2), NONE),
            (1..=2, (3, 4), NONE),
        ] {
            assert_eq!(shared(ours.clone(), theirs.0, theirs.1), expected);
        }
    }

    #[test]
    fn what_a_later_version_keeps_and_asks_is_read_as_unknown() {
        let record: Record =
            serde_json::from_str(r#"{"seq":3,"at":1,"kind":"of_tomorrow","x":1}"#).unwrap();
        assert_eq!((record.seq, record.what), (3, What::Unknown));
        let request: Request = serde_json::from_str(r#"{"op":"of_tomorrow","x":1}"#).unwrap();
        assert!(matches!(request, Request::Unknown));
        let exit: Record =
            serde_json::from_str(r#"{"seq":3,"at":1,"kind":"exit","why":"rain","code":null}"#)
                .unwrap();
        assert!(matches!(
            exit.what,
            What::Exit {
                why: Why::Unknown,
                ..
            }
        ));
    }
}
