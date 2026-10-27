//! Real disk-full behavior on a tiny ext4 volume. Needs a mount, so it only
//! runs when PALLET_TEST_SMALL_EXT4 points at an empty, writable ext4 mount of
//! a few MiB (CI creates one with a loop device). Skipped otherwise.
#![cfg(target_os = "linux")]

mod common;

use common::*;
use pallet_core::warehouse::Warehouse;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn small_volume() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("PALLET_TEST_SMALL_EXT4")?);
    Some(p)
}

/// Fills the volume with a filler file until fewer than `leave` bytes remain.
fn fill(dir: &std::path::Path, leave: u64) {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("filler.bin"))
        .unwrap();
    let chunk = vec![0xAAu8; 64 * 1024];
    loop {
        let free = rustix::fs::statvfs(dir)
            .map(|s| s.f_bavail * s.f_frsize)
            .unwrap();
        if free <= leave {
            break;
        }
        let n = ((free - leave) as usize).min(chunk.len());
        if f.write_all(&chunk[..n.max(4096)]).is_err() {
            break;
        }
        f.sync_all().unwrap();
    }
}

#[test]
fn full_disk_refuses_without_losing_or_faking_anything() {
    let Some(vol) = small_volume() else {
        eprintln!("skipped: set PALLET_TEST_SMALL_EXT4 to a small ext4 mount");
        return;
    };
    let base = tempfile::Builder::new()
        .prefix("full-")
        .tempdir_in(&vol)
        .unwrap();
    let root = base.path().join("warehouse");
    fs::create_dir_all(root.join("Documents")).unwrap();
    fs::write(root.join("hello.txt"), b"hello\n").unwrap();
    let mut big = vec![0u8; 1536 * 1024];
    big.iter_mut().enumerate().for_each(|(i, b)| *b = i as u8);
    fs::write(root.join("big.bin"), &big).unwrap();

    let mut w = Warehouse::open(&root, false).unwrap();
    let docs = bay_id(&mut w, "Documents");
    let hello = cargo(&mut w, "hello.txt");
    let bigc = cargo(&mut w, "big.bin");

    // Enough room for a journal record but not for a verified 1.5 MiB copy.
    fill(base.path(), 3 * 1024 * 1024);
    let before = snap_dir(&root, true);
    let o = w.duplicate("c1", &bigc, None);
    refused(&o, "DISK_FULL");
    assert_eq!(snap_dir(&root, true), before);

    // A rename needs no data blocks: it still works with a few MiB free.
    committed(&w.deliver("m1", &hello, &docs, None, None));

    // Exhaust the disk entirely.
    fill(base.path(), 0);
    let before = snap_dir(&root, true);
    let back = cargo_in(&mut w, "Documents", "hello.txt");
    let root_bay = w.root_bay();
    let o = w.deliver("m2", &back, &root_bay, None, None);
    assert_ne!(
        o.status,
        pallet_core::model::TxStatus::Committed,
        "no success on a full disk: {o:?}"
    );
    assert!(
        matches!(o.code.as_deref(), Some("DISK_FULL" | "JOURNAL_FAILED")),
        "{o:?}"
    );
    assert_eq!(snap_dir(&root, true), before, "nothing moved or lost");
    assert_eq!(fs::read(root.join("big.bin")).unwrap(), big);
    drop(w);

    // Free space again: the journal is intact and history survived.
    fs::remove_file(base.path().join("filler.bin")).unwrap();
    let w = Warehouse::open(&root, false).unwrap();
    assert!(w.status().frozen.is_none());
    assert_eq!(
        pallet_core::journal::parse(&fs::read(root.join(".pallet/journal.log")).unwrap()).damage,
        None
    );
    assert!(w.history().iter().any(|h| h.tx == "m1"));
}
