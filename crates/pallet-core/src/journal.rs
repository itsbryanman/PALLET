//! Append-only journal (S6). Each record is framed as
//! `[len: u32 LE][crc32(payload): u32 LE][payload: JSON]`.
//! File contents never appear in a record.

use crate::failpoint;
use crate::model::Op;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

const MAX_RECORD: u32 = 64 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Ident {
    pub dev: u64,
    pub ino: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fingerprint {
    pub size: u64,
    pub mtime_ns: i128,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Kind {
    /// Warehouse opened; records the root identity.
    Opened,
    /// Intent, synced before any mutation.
    Prepared,
    /// Copy/incident only: temporary file written, synced, and verified.
    Staged,
    Committed,
    /// Refused or failed before any mutation took effect.
    Failed,
    /// A recovery decision about an earlier transaction.
    Recovery,
    /// The user acknowledged a frozen state after reviewing it.
    Acknowledged,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub v: u8,
    pub tx: String,
    pub ts: u64,
    pub kind: Option<Kind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<Op>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_id: Option<Ident>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst_id: Option<Ident>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_dir_id: Option<Ident>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst_dir_id: Option<Ident>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fp: Option<Fingerprint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmp_id: Option<Ident>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_id: Option<Ident>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Damage {
    /// The last record is incomplete. Only an interrupted append produces this.
    TornTail { offset: u64 },
    /// A record before the end is unreadable. Nothing after it can be trusted blindly.
    Corrupt { offset: u64 },
}

#[derive(Debug)]
pub struct Loaded {
    pub records: Vec<Record>,
    /// Length of the prefix made of intact records.
    pub valid_len: u64,
    pub damage: Option<Damage>,
}

pub fn encode(record: &Record) -> Vec<u8> {
    let payload = serde_json::to_vec(record).expect("journal record serializes");
    assert!(
        payload.len() as u32 <= MAX_RECORD,
        "journal record too large"
    );
    let mut frame = Vec::with_capacity(payload.len() + 8);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&crc32fast::hash(&payload).to_le_bytes());
    frame.extend_from_slice(&payload);
    frame
}

pub fn parse(bytes: &[u8]) -> Loaded {
    let len = bytes.len();
    let mut pos = 0usize;
    let mut records = Vec::new();
    let damage = loop {
        if pos == len {
            break None;
        }
        let at = pos as u64;
        if len - pos < 8 {
            break Some(Damage::TornTail { offset: at });
        }
        let l = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
        let crc = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap());
        // A crash can leave a zero-filled tail when the size was extended
        // before the data reached the disk.
        if l == 0 {
            if bytes[pos..].iter().all(|&b| b == 0) {
                break Some(Damage::TornTail { offset: at });
            }
            break Some(Damage::Corrupt { offset: at });
        }
        if l > MAX_RECORD {
            break Some(Damage::Corrupt { offset: at });
        }
        let end = pos + 8 + l as usize;
        if end > len {
            break Some(Damage::TornTail { offset: at });
        }
        let payload = &bytes[pos + 8..end];
        if crc32fast::hash(payload) != crc {
            if end == len {
                break Some(Damage::TornTail { offset: at });
            }
            break Some(Damage::Corrupt { offset: at });
        }
        match serde_json::from_slice::<Record>(payload) {
            Ok(r) => records.push(r),
            Err(_) => break Some(Damage::Corrupt { offset: at }),
        }
        pos = end;
    };
    let valid_len = match damage {
        Some(Damage::TornTail { offset }) | Some(Damage::Corrupt { offset }) => offset,
        None => len as u64,
    };
    Loaded {
        records,
        valid_len,
        damage,
    }
}

pub struct Journal {
    file: File,
    len: u64,
}

impl Journal {
    /// Wraps an already-opened journal file (opened by the platform adapter
    /// inside `.pallet`), and reads what is there.
    pub fn load(mut file: File) -> io::Result<(Journal, Loaded)> {
        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(0))?;
        file.read_to_end(&mut bytes)?;
        let loaded = parse(&bytes);
        let len = bytes.len() as u64;
        Ok((Journal { file, len }, loaded))
    }

    pub fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.file.seek(SeekFrom::Start(0))?;
        self.file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Cuts a known-torn tail. Only called after the user chose to continue and
    /// the damaged bytes were preserved as evidence.
    pub fn truncate_to(&mut self, len: u64) -> io::Result<()> {
        self.file.set_len(len)?;
        self.file.sync_all()?;
        self.len = len;
        Ok(())
    }

    /// Appends and syncs one record. On any failure the partial frame is cut
    /// off again so later records are not stranded behind a torn one.
    pub fn append(&mut self, record: &Record) -> io::Result<()> {
        let frame = encode(record);
        let before = self.len;
        let result = (|| {
            failpoint::error("journal_append")?;
            self.file.seek(SeekFrom::End(0))?;
            if failpoint::torn("journal_write") {
                self.file.write_all(&frame[..frame.len() / 2])?;
                self.file.sync_data()?;
                std::process::abort();
            }
            self.file.write_all(&frame)?;
            failpoint::error("journal_sync")?;
            self.file.sync_data()
        })();
        match result {
            Ok(()) => {
                self.len = before + frame.len() as u64;
                Ok(())
            }
            Err(e) => {
                // Our own bytes, known incomplete. If this also fails, the next
                // open sees a torn tail and freezes, which is the safe outcome.
                let _ = self
                    .file
                    .set_len(before)
                    .and_then(|_| self.file.sync_data());
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(tx: &str) -> Record {
        Record {
            v: 1,
            tx: tx.into(),
            ts: 1,
            kind: Some(Kind::Prepared),
            ..Default::default()
        }
    }

    #[test]
    fn roundtrip() {
        let mut bytes = encode(&rec("a"));
        bytes.extend(encode(&rec("b")));
        let l = parse(&bytes);
        assert_eq!(l.records.len(), 2);
        assert_eq!(l.damage, None);
        assert_eq!(l.valid_len, bytes.len() as u64);
    }

    #[test]
    fn every_truncation_is_a_torn_tail() {
        let first = encode(&rec("a"));
        let second = encode(&rec("b"));
        let mut bytes = first.clone();
        bytes.extend(&second);
        for cut in first.len() + 1..bytes.len() {
            let l = parse(&bytes[..cut]);
            assert_eq!(l.records.len(), 1, "cut at {cut}");
            assert_eq!(
                l.damage,
                Some(Damage::TornTail {
                    offset: first.len() as u64
                })
            );
        }
    }

    #[test]
    fn zero_tail_is_torn_and_middle_flip_is_corrupt() {
        let mut bytes = encode(&rec("a"));
        let n = bytes.len();
        bytes.extend([0u8; 37]);
        assert_eq!(
            parse(&bytes).damage,
            Some(Damage::TornTail { offset: n as u64 })
        );

        let mut bytes = encode(&rec("a"));
        bytes.extend(encode(&rec("b")));
        bytes[10] ^= 0x55;
        let l = parse(&bytes);
        assert_eq!(l.records.len(), 0);
        assert_eq!(l.damage, Some(Damage::Corrupt { offset: 0 }));
    }
}
