#![allow(dead_code)]

use pallet_core::model::{Floor, TxOutcome, TxStatus};
use pallet_core::warehouse::Warehouse;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// One isolated warehouse plus an outside directory holding a sentinel file.
/// Lives under CARGO_TARGET_TMPDIR, which is on ext4 locally and on CI
/// (/tmp is often tmpfs, which PALLET refuses).
pub struct Fx {
    _base: TempDir,
    pub root: PathBuf,
    pub outside: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub ino: u64,
    pub hash: String,
    pub mode: u32,
}

pub type Snap = BTreeMap<String, Entry>;

impl Fx {
    pub fn new() -> Fx {
        let base = tempfile::Builder::new()
            .prefix("pallet-test-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let root = base.path().join("warehouse");
        let outside = base.path().join("outside");
        fs::create_dir_all(root.join("Documents")).unwrap();
        fs::create_dir_all(root.join("Photos")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("hello.txt"), b"hello\n").unwrap();
        fs::write(root.join("b.txt"), b"bee\n").unwrap();
        fs::write(root.join("Documents/notes.txt"), b"notes\n").unwrap();
        fs::write(outside.join("sentinel.txt"), b"do not touch\n").unwrap();
        Fx {
            _base: base,
            root,
            outside,
        }
    }

    pub fn open(&self) -> Warehouse {
        Warehouse::open(&self.root, false).expect("open warehouse")
    }

    pub fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.root.join(rel);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(p, bytes).unwrap();
    }

    /// Every user-visible entry under the root (excluding `.pallet`).
    pub fn snap(&self) -> Snap {
        snap_dir(&self.root, true)
    }

    pub fn outside_snap(&self) -> Snap {
        snap_dir(&self.outside, false)
    }

    pub fn journal(&self) -> Vec<u8> {
        fs::read(self.root.join(".pallet/journal.log")).unwrap_or_default()
    }

    pub fn tmp_entries(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(self.root.join(".pallet/tmp"))
            .map(|r| {
                r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

pub fn snap_dir(dir: &Path, skip_meta: bool) -> Snap {
    let mut out = BTreeMap::new();
    fn walk(base: &Path, dir: &Path, skip_meta: bool, out: &mut Snap) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap()).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
            if skip_meta && rel == ".pallet" {
                continue;
            }
            let md = fs::symlink_metadata(&p).unwrap();
            if md.is_dir() {
                out.insert(
                    format!("{rel}/"),
                    Entry {
                        ino: md.ino(),
                        hash: String::new(),
                        mode: md.mode(),
                    },
                );
                walk(base, &p, skip_meta, out);
            } else if md.is_file() {
                out.insert(
                    rel,
                    Entry {
                        ino: md.ino(),
                        hash: content_hash(&p, md.len()),
                        mode: md.mode(),
                    },
                );
            } else {
                out.insert(
                    rel,
                    Entry {
                        ino: md.ino(),
                        hash: "special".into(),
                        mode: md.mode(),
                    },
                );
            }
        }
    }
    walk(dir, dir, skip_meta, &mut out);
    out
}

/// Full BLAKE3 for normal files. Huge sparse fixtures get size + head + tail,
/// since reading 50 GB of holes proves nothing extra.
fn content_hash(p: &Path, len: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = fs::File::open(p).unwrap();
    if len <= 64 << 20 {
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        return blake3::hash(&b).to_hex().to_string();
    }
    let mut h = blake3::Hasher::new();
    h.update(&len.to_le_bytes());
    let mut b = vec![0u8; 1 << 20];
    f.read_exact(&mut b).unwrap();
    h.update(&b);
    f.seek(SeekFrom::End(-(1 << 20))).unwrap();
    f.read_exact(&mut b).unwrap();
    h.update(&b);
    format!("sampled-{}", h.finalize().to_hex())
}

/// Regular files by inode: (path, hash). Panics if an inode shows up twice.
pub fn by_ino(s: &Snap) -> BTreeMap<u64, (String, String)> {
    let mut m = BTreeMap::new();
    for (k, v) in s {
        if k.ends_with('/') || v.hash == "special" {
            continue;
        }
        assert!(
            m.insert(v.ino, (k.clone(), v.hash.clone())).is_none(),
            "inode {} appears twice",
            v.ino
        );
    }
    m
}

/// The exactly-one invariant: the same set of file objects, same bytes, each once.
pub fn assert_same_objects(before: &Snap, after: &Snap) {
    let a: BTreeMap<u64, String> = by_ino(before)
        .into_iter()
        .map(|(i, (_, h))| (i, h))
        .collect();
    let b: BTreeMap<u64, String> = by_ino(after)
        .into_iter()
        .map(|(i, (_, h))| (i, h))
        .collect();
    assert_eq!(a, b, "file objects or contents changed");
}

pub fn floor(w: &mut Warehouse, bay: Option<&str>) -> Floor {
    w.list_floor(bay).expect("list floor")
}

pub fn cargo(w: &mut Warehouse, name: &str) -> String {
    let f = floor(w, None);
    f.cargo
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no cargo {name}"))
        .id
        .clone()
}

pub fn cargo_in(w: &mut Warehouse, bay: &str, name: &str) -> String {
    let b = bay_id(w, bay);
    let f = floor(w, Some(&b));
    f.cargo
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no cargo {name}"))
        .id
        .clone()
}

pub fn bay_id(w: &mut Warehouse, name: &str) -> String {
    let f = floor(w, None);
    f.bays
        .iter()
        .find(|b| b.name == name)
        .unwrap_or_else(|| panic!("no bay {name}"))
        .id
        .clone()
}

pub fn committed(o: &TxOutcome) {
    assert_eq!(o.status, TxStatus::Committed, "expected commit, got {o:?}");
}

pub fn refused(o: &TxOutcome, code: &str) {
    assert_eq!(
        o.status,
        TxStatus::Refused,
        "expected refusal {code}, got {o:?}"
    );
    assert_eq!(o.code.as_deref(), Some(code), "{o:?}");
}

pub fn is_root() -> bool {
    rustix::process::geteuid().is_root()
}
