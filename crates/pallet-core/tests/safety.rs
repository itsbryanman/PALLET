//! In-process safety tests against the real Linux adapter on ext4.
//! Each test checks the outside sentinel and, where relevant, the exactly-one invariant.
#![cfg(target_os = "linux")]

mod common;

use common::*;
use pallet_core::failpoint;
use pallet_core::model::{FreezeKind, TxStatus};
use pallet_core::warehouse::{check_root_policy, Warehouse};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;

#[test]
fn move_then_undo_round_trip() {
    let fx = Fx::new();
    let outside = fx.outside_snap();
    let before = fx.snap();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");

    let o = w.deliver("t1", &c, &docs, None, None);
    committed(&o);
    assert_eq!(o.from.as_deref(), Some("hello.txt"));
    assert_eq!(o.to.as_deref(), Some("Documents/hello.txt"));
    assert!(!fx.root.join("hello.txt").exists());
    assert_eq!(
        fs::read(fx.root.join("Documents/hello.txt")).unwrap(),
        b"hello\n"
    );
    assert_same_objects(&before, &fx.snap());

    let h = w.history();
    assert_eq!(h[0].tx, "t1");
    assert!(!h[0].undone);

    committed(&w.undo("u1", "t1", None));
    assert_eq!(
        fx.snap(),
        before,
        "undo restores names, inodes, bytes, modes"
    );
    assert!(w.history().iter().any(|e| e.tx == "t1" && e.undone));
    refused(&w.undo("u2", "t1", None), "ALREADY_UNDONE");
    assert_eq!(fx.outside_snap(), outside);
}

#[test]
fn collision_is_refused_with_suggestion_then_alternate_name_works() {
    let fx = Fx::new();
    fx.write("Documents/hello.txt", b"existing\n");
    let before = fx.snap();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");

    let o = w.deliver("t1", &c, &docs, None, None);
    refused(&o, "NAME_EXISTS");
    assert_eq!(o.suggestion.as_deref(), Some("hello (2).txt"));
    assert_eq!(fx.snap(), before);

    committed(&w.deliver("t2", &c, &docs, Some("hello (2).txt"), None));
    assert_eq!(
        fs::read(fx.root.join("Documents/hello.txt")).unwrap(),
        b"existing\n"
    );
    assert_eq!(
        fs::read(fx.root.join("Documents/hello (2).txt")).unwrap(),
        b"hello\n"
    );
    assert_same_objects(&before, &fx.snap());
}

#[test]
fn collision_created_at_commit_is_caught_by_noreplace() {
    let fx = Fx::new();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");
    let late = fx.root.join("Documents/hello.txt");
    failpoint::set_hook(Some(Box::new(move |p| {
        if p == "move_before_rename" {
            fs::write(&late, b"arrived late\n").unwrap();
        }
    })));
    let o = w.deliver("t1", &c, &docs, None, None);
    failpoint::set_hook(None);
    refused(&o, "NAME_EXISTS");
    assert_eq!(
        fs::read(fx.root.join("Documents/hello.txt")).unwrap(),
        b"arrived late\n"
    );
    assert_eq!(fs::read(fx.root.join("hello.txt")).unwrap(), b"hello\n");
}

#[test]
fn concurrent_creators_never_lose_content() {
    // A second thread races to create the destination name while deliveries run.
    for i in 0..60 {
        let fx = Fx::new();
        let mut w = fx.open();
        let docs = bay_id(&mut w, "Documents");
        let c = cargo(&mut w, "hello.txt");
        let target = fx.root.join("Documents/hello.txt");
        let racer = std::thread::spawn(move || {
            for _ in 0..(i * 7 % 50) {
                std::hint::spin_loop();
            }
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map(|_| ())
                .is_ok()
        });
        let o = w.deliver(&format!("t{i}"), &c, &docs, None, None);
        let racer_won = racer.join().unwrap();
        match o.status {
            TxStatus::Committed => {
                assert!(!racer_won, "both claimed the name");
                assert_eq!(
                    fs::read(fx.root.join("Documents/hello.txt")).unwrap(),
                    b"hello\n"
                );
            }
            TxStatus::Refused => {
                assert_eq!(o.code.as_deref(), Some("NAME_EXISTS"));
                assert!(racer_won);
                assert_eq!(fs::read(fx.root.join("hello.txt")).unwrap(), b"hello\n");
                assert_eq!(fs::read(fx.root.join("Documents/hello.txt")).unwrap(), b"");
            }
            TxStatus::Uncertain => panic!("{o:?}"),
        }
    }
}

#[test]
fn symlink_swapped_into_path_before_revalidation_is_refused() {
    let fx = Fx::new();
    let outside = fx.outside_snap();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");
    let (root, out) = (fx.root.clone(), fx.outside.clone());
    failpoint::set_hook(Some(Box::new(move |p| {
        if p == "move_after_prepared" {
            fs::rename(root.join("Documents"), root.join("Documents.real")).unwrap();
            symlink(&out, root.join("Documents")).unwrap();
        }
    })));
    let o = w.deliver("t1", &c, &docs, None, None);
    failpoint::set_hook(None);
    assert_eq!(o.status, TxStatus::Refused, "{o:?}");
    assert!(
        matches!(
            o.code.as_deref(),
            Some("LINK_IN_PATH" | "DESTINATION_MISSING")
        ),
        "{o:?}"
    );
    assert_eq!(
        fx.outside_snap(),
        outside,
        "nothing reached the outside directory"
    );
    assert_eq!(fs::read(fx.root.join("hello.txt")).unwrap(), b"hello\n");
}

#[test]
fn folder_replaced_by_lookalike_is_refused() {
    let fx = Fx::new();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");
    let root = fx.root.clone();
    failpoint::set_hook(Some(Box::new(move |p| {
        if p == "move_after_prepared" {
            fs::rename(root.join("Documents"), root.join("Old Documents")).unwrap();
            fs::create_dir(root.join("Documents")).unwrap();
        }
    })));
    let o = w.deliver("t1", &c, &docs, None, None);
    failpoint::set_hook(None);
    refused(&o, "DESTINATION_MISSING");
    assert!(fx.root.join("hello.txt").exists());
    assert_eq!(
        fs::read_dir(fx.root.join("Documents")).unwrap().count(),
        0,
        "impostor folder untouched"
    );
}

#[test]
fn bay_moved_outside_inside_the_residual_window_is_detected_and_freezes() {
    // Documented residual race (SAFETY.md S1): after revalidation, before rename.
    let fx = Fx::new();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");
    let (root, out) = (fx.root.clone(), fx.outside.clone());
    failpoint::set_hook(Some(Box::new(move |p| {
        if p == "move_before_rename" {
            fs::rename(root.join("Documents"), out.join("Documents")).unwrap();
        }
    })));
    let o = w.deliver("t1", &c, &docs, None, None);
    failpoint::set_hook(None);
    assert_eq!(o.status, TxStatus::Uncertain, "{o:?}");
    assert_eq!(w.status().frozen.unwrap().kind, FreezeKind::BoundaryEscape);
    // The file is intact (in the moved folder), exactly once.
    assert_eq!(
        fs::read(fx.outside.join("Documents/hello.txt")).unwrap(),
        b"hello\n"
    );
    assert!(!fx.root.join("hello.txt").exists());
    let b = cargo(&mut w, "b.txt");
    refused(&w.deliver("t2", &b, &docs, None, None), "FROZEN");
}

#[test]
fn missing_destination_is_not_recreated() {
    let fx = Fx::new();
    let mut w = fx.open();
    let photos = bay_id(&mut w, "Photos");
    let c = cargo(&mut w, "hello.txt");
    let root = fx.root.clone();
    failpoint::set_hook(Some(Box::new(move |p| {
        if p == "move_after_prepared" {
            fs::remove_dir(root.join("Photos")).unwrap();
        }
    })));
    let o = w.deliver("t1", &c, &photos, None, None);
    failpoint::set_hook(None);
    refused(&o, "DESTINATION_MISSING");
    assert!(!fx.root.join("Photos").exists(), "never recreated");
    assert_eq!(fs::read(fx.root.join("hello.txt")).unwrap(), b"hello\n");
}

#[test]
fn root_replaced_suspends_and_never_writes_the_impostor() {
    let fx = Fx::new();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");
    let root = fx.root.clone();
    let moved = fx.root.with_file_name("warehouse.moved");
    let m2 = moved.clone();
    failpoint::set_hook(Some(Box::new(move |p| {
        if p == "move_after_prepared" {
            fs::rename(&root, &m2).unwrap();
            fs::create_dir_all(root.join("Documents")).unwrap();
        }
    })));
    let o = w.deliver("t1", &c, &docs, None, None);
    failpoint::set_hook(None);
    refused(&o, "ROOT_REPLACED");
    assert_eq!(w.status().frozen.unwrap().kind, FreezeKind::RootReplaced);
    assert_eq!(fs::read_dir(fx.root.join("Documents")).unwrap().count(), 0);
    assert!(
        !fx.root.join(".pallet").exists(),
        "impostor gets no metadata"
    );
    assert_eq!(fs::read(moved.join("hello.txt")).unwrap(), b"hello\n");
    refused(&w.deliver("t2", &c, &docs, None, None), "FROZEN");
}

#[test]
fn source_edited_or_replaced_while_driving_is_refused() {
    let fx = Fx::new();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");
    fs::write(fx.root.join("hello.txt"), b"edited, longer\n").unwrap();
    refused(&w.deliver("t1", &c, &docs, None, None), "SOURCE_CHANGED");

    let b = cargo(&mut w, "b.txt");
    fs::rename(fx.root.join("b.txt"), fx.root.join("b.old")).unwrap();
    fs::write(fx.root.join("b.txt"), b"bee\n").unwrap();
    refused(&w.deliver("t2", &b, &docs, None, None), "SOURCE_CHANGED");
    assert_eq!(fs::read(fx.root.join("b.txt")).unwrap(), b"bee\n");
}

#[test]
fn links_special_files_hard_links_and_read_only_are_not_cargo() {
    let fx = Fx::new();
    symlink(
        fx.outside.join("sentinel.txt"),
        fx.root.join("link-to-outside"),
    )
    .unwrap();
    symlink(&fx.outside, fx.root.join("dir-link")).unwrap();
    rustix::fs::mknodat(
        rustix::fs::CWD,
        fx.root.join("pipe"),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o644),
        0,
    )
    .unwrap();
    fx.write("linked.txt", b"two names\n");
    fs::hard_link(
        fx.root.join("linked.txt"),
        fx.root.join("Photos/linked-too.txt"),
    )
    .unwrap();
    fx.write("locked.txt", b"read only\n");
    fs::set_permissions(
        fx.root.join("locked.txt"),
        fs::Permissions::from_mode(0o444),
    )
    .unwrap();
    let outside = fx.outside_snap();

    let mut w = fx.open();
    let f = floor(&mut w, None);
    let reason = |n: &str| {
        f.cargo
            .iter()
            .find(|c| c.name == n)
            .unwrap()
            .unavailable
            .clone()
    };
    assert_eq!(reason("link-to-outside").as_deref(), Some("link"));
    assert_eq!(reason("dir-link").as_deref(), Some("link"));
    assert_eq!(reason("pipe").as_deref(), Some("pipe"));
    assert_eq!(reason("linked.txt").as_deref(), Some("hard-linked"));
    assert_eq!(reason("locked.txt").as_deref(), Some("read-only"));
    assert!(
        !f.bays.iter().any(|b| b.name == "dir-link"),
        "links are never bays"
    );

    let docs = bay_id(&mut w, "Documents");
    for n in ["link-to-outside", "pipe"] {
        let id = f.cargo.iter().find(|c| c.name == n).unwrap().id.clone();
        refused(
            &w.deliver(&format!("x-{n}"), &id, &docs, None, None),
            "UNKNOWN_CARGO",
        );
    }
    let id = f
        .cargo
        .iter()
        .find(|c| c.name == "linked.txt")
        .unwrap()
        .id
        .clone();
    refused(&w.deliver("hl", &id, &docs, None, None), "HARD_LINKED");
    let id = f
        .cargo
        .iter()
        .find(|c| c.name == "locked.txt")
        .unwrap()
        .id
        .clone();
    refused(&w.deliver("ro", &id, &docs, None, None), "READ_ONLY");
    let mode = fs::metadata(fx.root.join("locked.txt"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o444, "permissions never rewritten");
    assert_eq!(fx.outside_snap(), outside);
}

#[test]
fn unwritable_destination_is_refused_without_touching_permissions() {
    if is_root() {
        eprintln!("skipped: root bypasses directory permissions (CI runs this as a normal user)");
        return;
    }
    let fx = Fx::new();
    fs::set_permissions(fx.root.join("Photos"), fs::Permissions::from_mode(0o555)).unwrap();
    let mut w = fx.open();
    let f = floor(&mut w, None);
    assert_eq!(
        f.bays
            .iter()
            .find(|b| b.name == "Photos")
            .unwrap()
            .unavailable
            .as_deref(),
        Some("read-only folder")
    );
    let photos = bay_id(&mut w, "Photos");
    let c = cargo(&mut w, "hello.txt");
    refused(&w.deliver("t1", &c, &photos, None, None), "PERMISSION");
    let mode = fs::metadata(fx.root.join("Photos"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o555);
    fs::set_permissions(fx.root.join("Photos"), fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn open_file_elsewhere_can_be_moved_without_changing_bytes() {
    let fx = Fx::new();
    let held = fs::File::open(fx.root.join("hello.txt")).unwrap();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");
    committed(&w.deliver("t1", &c, &docs, None, None));
    drop(held);
    assert_eq!(
        fs::read(fx.root.join("Documents/hello.txt")).unwrap(),
        b"hello\n"
    );
}

#[test]
fn cross_device_rename_is_refused_with_no_fallback() {
    let fx = Fx::new();
    let before = fx.snap();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");
    let o = failpoint::scoped("rename:exdev", || w.deliver("t1", &c, &docs, None, None));
    refused(&o, "CROSS_DEVICE");
    assert_eq!(fx.snap(), before, "no copy, no delete");
    assert!(fx.tmp_entries().is_empty());
}

#[test]
fn nested_mount_is_unavailable() {
    let fx = Fx::new();
    let mnt = fx.root.join("Mounted");
    fs::create_dir(&mnt).unwrap();
    let ok = std::process::Command::new("mount")
        .args(["-t", "tmpfs", "-o", "size=1m", "tmpfs"])
        .arg(&mnt)
        .status();
    if !matches!(ok, Ok(s) if s.success()) {
        eprintln!(
            "skipped: can't mount tmpfs here (needs root; CI runs it with sudo in a separate job)"
        );
        return;
    }
    let result = std::panic::catch_unwind(|| {
        let mut w = fx.open();
        let f = floor(&mut w, None);
        assert_eq!(
            f.bays
                .iter()
                .find(|b| b.name == "Mounted")
                .unwrap()
                .unavailable
                .as_deref(),
            Some("another device")
        );
        let m = bay_id(&mut w, "Mounted");
        let c = cargo(&mut w, "hello.txt");
        let o = w.deliver("t1", &c, &m, None, None);
        assert_eq!(o.status, TxStatus::Refused);
        assert!(
            matches!(
                o.code.as_deref(),
                Some("CROSS_DEVICE" | "DESTINATION_MISSING")
            ),
            "{o:?}"
        );
        assert!(fx.root.join("hello.txt").exists());
    });
    let _ = std::process::Command::new("umount").arg(&mnt).status();
    result.unwrap();
}

#[test]
fn journal_failures_before_rename_move_nothing() {
    for spec in ["journal_append:enospc@1", "journal_sync:eio@1"] {
        let fx = Fx::new();
        let mut w = fx.open();
        let before = fx.snap();
        let docs = bay_id(&mut w, "Documents");
        let c = cargo(&mut w, "hello.txt");
        let o = failpoint::scoped(spec, || w.deliver("t1", &c, &docs, None, None));
        refused(&o, "JOURNAL_FAILED");
        assert_eq!(fx.snap(), before, "{spec}");
        let parsed = pallet_core::journal::parse(&fx.journal());
        assert_eq!(
            parsed.damage, None,
            "failed append was rolled back ({spec})"
        );
    }
}

#[test]
fn committed_record_failure_is_uncertain_then_recovered_by_identity() {
    let fx = Fx::new();
    let before = fx.snap();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");
    // Append #1 is PREPARED, #2 is COMMITTED.
    let o = failpoint::scoped("journal_append:enospc@2", || {
        w.deliver("t1", &c, &docs, None, None)
    });
    assert_eq!(
        o.status,
        TxStatus::Uncertain,
        "no success without a COMMITTED record"
    );
    drop(w);
    let w = fx.open();
    let st = w.status();
    assert_eq!(st.recovery.len(), 1);
    assert_eq!(st.recovery[0].decision, "RECOVERED_COMMITTED");
    assert_eq!(w.tx_status("t1").unwrap().status, TxStatus::Committed);
    assert_same_objects(&before, &fx.snap());
    drop(w);
    assert!(
        fx.open().status().recovery.is_empty(),
        "recovery is idempotent"
    );
}

#[test]
fn directory_flush_failure_freezes() {
    let fx = Fx::new();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");
    let o = failpoint::scoped("dir_fsync:eio", || w.deliver("t1", &c, &docs, None, None));
    assert_eq!(o.status, TxStatus::Uncertain);
    assert_eq!(w.status().frozen.unwrap().kind, FreezeKind::FlushFailed);
    w.acknowledge().unwrap();
    assert!(w.status().frozen.is_none());
}

#[test]
fn torn_tail_freezes_then_continues_with_evidence() {
    let fx = Fx::new();
    {
        let mut w = fx.open();
        let docs = bay_id(&mut w, "Documents");
        let c = cargo(&mut w, "hello.txt");
        committed(&w.deliver("t1", &c, &docs, None, None));
    }
    let mut j = fx.journal();
    let good = j.len();
    j.extend_from_slice(&[0x40, 0, 0, 0, 0xde, 0xad]);
    fs::write(fx.root.join(".pallet/journal.log"), &j).unwrap();

    let mut w = fx.open();
    let f = w.status().frozen.unwrap();
    assert_eq!(f.kind, FreezeKind::TornJournalTail);
    assert!(f.acknowledgeable);
    let docs = bay_id(&mut w, "Documents");
    let b = cargo(&mut w, "b.txt");
    refused(&w.deliver("t2", &b, &docs, None, None), "FROZEN");
    assert_eq!(fx.journal(), j, "nothing appended behind torn bytes");

    w.acknowledge().unwrap();
    assert!(w.status().frozen.is_none());
    let ev: Vec<_> = fs::read_dir(fx.root.join(".pallet/evidence"))
        .unwrap()
        .collect();
    assert_eq!(ev.len(), 1);
    assert_eq!(
        fs::read(ev[0].as_ref().unwrap().path()).unwrap(),
        j,
        "damaged journal preserved byte-for-byte"
    );
    let parsed = pallet_core::journal::parse(&fx.journal());
    assert_eq!(parsed.damage, None);
    assert!(fx.journal().len() > good);
    committed(&w.deliver("t2", &b, &docs, None, None));
    assert_eq!(
        w.tx_status("t1").unwrap().status,
        TxStatus::Committed,
        "history survives"
    );
}

#[test]
fn corrupt_journal_freezes_without_a_way_around_it() {
    let fx = Fx::new();
    {
        let mut w = fx.open();
        let docs = bay_id(&mut w, "Documents");
        let c = cargo(&mut w, "hello.txt");
        committed(&w.deliver("t1", &c, &docs, None, None));
    }
    let mut j = fx.journal();
    j[12] ^= 0xff;
    fs::write(fx.root.join(".pallet/journal.log"), &j).unwrap();
    let before = fx.snap();
    let mut w = fx.open();
    let f = w.status().frozen.unwrap();
    assert_eq!(f.kind, FreezeKind::CorruptJournal);
    assert!(!f.acknowledgeable);
    assert!(w.acknowledge().is_err());
    let docs = bay_id(&mut w, "Documents");
    let b = cargo(&mut w, "b.txt");
    refused(&w.deliver("t2", &b, &docs, None, None), "FROZEN");
    assert_eq!(fx.journal(), j, "evidence preserved");
    assert_eq!(fx.snap(), before);
}

#[test]
fn unreadable_journal_refuses_to_open() {
    let fx = Fx::new();
    drop(fx.open());
    fs::rename(
        fx.root.join(".pallet/journal.log"),
        fx.root.join(".pallet/journal.moved"),
    )
    .unwrap();
    fs::create_dir(fx.root.join(".pallet/journal.log")).unwrap();
    let before = fx.snap();
    assert!(Warehouse::open(&fx.root, false).is_err());
    assert_eq!(fx.snap(), before);
}

#[test]
fn second_instance_is_refused_and_replay_is_idempotent() {
    let fx = Fx::new();
    let mut w = fx.open();
    let err = Warehouse::open(&fx.root, false).err().unwrap();
    assert_eq!(err.code, "BUSY");

    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");
    committed(&w.deliver("same-tx", &c, &docs, None, None));
    let snap = fx.snap();
    let journal = fx.journal();
    let again = w.deliver("same-tx", &c, &docs, None, None);
    assert!(again.replayed);
    assert_eq!(again.status, TxStatus::Committed);
    assert_eq!(fx.snap(), snap);
    assert_eq!(fx.journal(), journal, "replay writes nothing");
    drop(w);
    let mut w = fx.open();
    let root_floor = floor(&mut w, None);
    let docs = bay_id(&mut w, "Documents");
    assert!(
        w.deliver("same-tx", &root_floor.cargo[0].id, &docs, None, None)
            .replayed
    );
}

#[test]
fn pallet_partial_failure_reports_exactly_what_moved() {
    let fx = Fx::new();
    for i in 1..=6 {
        fx.write(&format!("crate{i}.txt"), format!("crate {i}\n").as_bytes());
    }
    fx.write("Photos/crate3.txt", b"already here\n");
    let before = fx.snap();
    let mut w = fx.open();
    let photos = bay_id(&mut w, "Photos");
    let items: Vec<(String, String)> = (1..=6)
        .map(|i| (format!("p{i}"), cargo(&mut w, &format!("crate{i}.txt"))))
        .collect();
    let out = w.deliver_batch("pallet-1", &items, &photos);
    assert_eq!(out.len(), 3, "stops at the first failure");
    committed(&out[0]);
    committed(&out[1]);
    refused(&out[2], "NAME_EXISTS");
    for i in 3..=6 {
        assert!(
            fx.root.join(format!("crate{i}.txt")).exists(),
            "crate{i} stayed put"
        );
    }
    assert!(
        fx.root.join("Photos/crate1.txt").exists() && fx.root.join("Photos/crate2.txt").exists()
    );
    assert_eq!(
        fs::read(fx.root.join("Photos/crate3.txt")).unwrap(),
        b"already here\n"
    );
    assert_same_objects(&before, &fx.snap());
    let h = w.history();
    assert_eq!(
        h.iter()
            .filter(|e| e.batch.as_deref() == Some("pallet-1"))
            .count(),
        2
    );
}

#[test]
fn undo_refuses_occupied_or_missing_origin_and_accepts_confirmed_alternate() {
    let fx = Fx::new();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let photos = bay_id(&mut w, "Photos");
    let c = cargo(&mut w, "hello.txt");
    committed(&w.deliver("t1", &c, &docs, None, None));
    fx.write("hello.txt", b"squatter\n");
    let o = w.undo("u1", "t1", None);
    refused(&o, "NAME_EXISTS");
    assert_eq!(o.suggestion.as_deref(), Some("hello (2).txt"));
    assert_eq!(fs::read(fx.root.join("hello.txt")).unwrap(), b"squatter\n");
    committed(&w.undo("u2", "t1", Some("hello (2).txt")));
    assert_eq!(fs::read(fx.root.join("hello (2).txt")).unwrap(), b"hello\n");

    // Origin folder removed: refuse, never recreate.
    fx.write("Photos/pic.txt", b"pic\n");
    let pic = cargo_in(&mut w, "Photos", "pic.txt");
    committed(&w.deliver("t2", &pic, &docs, None, None));
    fs::remove_dir(fx.root.join("Photos")).unwrap();
    refused(&w.undo("u3", "t2", None), "DESTINATION_MISSING");
    assert!(!fx.root.join("Photos").exists());
    let _ = photos;

    // The moved file was replaced by a different object: refuse.
    let b = cargo(&mut w, "b.txt");
    committed(&w.deliver("t3", &b, &docs, None, None));
    fs::remove_file(fx.root.join("Documents/b.txt")).unwrap();
    fx.write("Documents/b.txt", b"bee\n");
    refused(&w.undo("u4", "t3", None), "SOURCE_CHANGED");
}

#[test]
fn copy_publishes_a_verified_duplicate_and_undo_copy_never_deletes() {
    let fx = Fx::new();
    let mut w = fx.open();
    let c = cargo(&mut w, "hello.txt");
    let o = w.duplicate("c1", &c, None);
    committed(&o);
    assert_eq!(o.to.as_deref(), Some("hello (copy).txt"));
    assert_eq!(
        fs::read(fx.root.join("hello (copy).txt")).unwrap(),
        b"hello\n"
    );
    assert_eq!(fs::read(fx.root.join("hello.txt")).unwrap(), b"hello\n");
    assert!(fx.tmp_entries().is_empty());
    committed(&w.duplicate("c2", &c, None));
    assert!(fx.root.join("hello (copy 2).txt").exists());

    committed(&w.undo("u1", "c1", None));
    assert!(!fx.root.join("hello (copy).txt").exists());
    assert_eq!(
        fs::read(fx.root.join("Recovered Copies/hello (copy).txt")).unwrap(),
        b"hello\n"
    );
}

#[test]
fn copy_of_a_source_changing_mid_stream_is_not_published() {
    let fx = Fx::new();
    let mut w = fx.open();
    let c = cargo(&mut w, "hello.txt");
    let src = fx.root.join("hello.txt");
    failpoint::set_hook(Some(Box::new(move |p| {
        if p == "copy_after_stream" {
            use std::io::Write;
            fs::OpenOptions::new()
                .append(true)
                .open(&src)
                .unwrap()
                .write_all(b"more")
                .unwrap();
        }
    })));
    let o = w.duplicate("c1", &c, None);
    failpoint::set_hook(None);
    refused(&o, "SOURCE_CHANGED");
    assert!(!fx.root.join("hello (copy).txt").exists());
    let f = floor(&mut w, None);
    assert!(
        !f.cargo.iter().any(|x| x.name.contains("copy")),
        "partial output is never cargo"
    );
}

#[test]
fn copy_disk_full_mid_stream_leaves_original_and_no_cargo() {
    let fx = Fx::new();
    let mut w = fx.open();
    let c = cargo(&mut w, "hello.txt");
    let before = fx.snap();
    let o = failpoint::scoped("copy_write:enospc", || w.duplicate("c1", &c, None));
    refused(&o, "DISK_FULL");
    assert_eq!(fx.snap(), before);
}

#[test]
fn incident_reports_never_overwrite() {
    let fx = Fx::new();
    let mut w = fx.open();
    let a = w.save_incident("i1", "Cargo: hello.txt\nFiles damaged: 0\n");
    committed(&a);
    let b = w.save_incident("i2", "Second incident.\n");
    committed(&b);
    assert_ne!(a.to, b.to);
    let first = fs::read_to_string(fx.root.join(a.to.unwrap())).unwrap();
    assert!(first.contains("Files damaged: 0"));
}

#[test]
fn boundary_attacks_are_refused() {
    let fx = Fx::new();
    let outside = fx.outside_snap();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let c = cargo(&mut w, "hello.txt");
    for (i, bad) in ["../escape.txt", "/etc/passwd", "a/b", "..", ".", "", "x\0y"]
        .iter()
        .enumerate()
    {
        refused(
            &w.deliver(&format!("n{i}"), &c, &docs, Some(bad), None),
            "BAD_NAME",
        );
    }
    let root_bay = w.root_bay();
    let d = cargo_in(&mut w, "Documents", "notes.txt");
    refused(
        &w.deliver("res", &d, &root_bay, Some(".pallet"), None),
        "BAD_NAME",
    );
    refused(&w.deliver("../../x", &c, &docs, None, None), "BAD_TX");
    refused(
        &w.deliver("forged", "deadbeef", &docs, None, None),
        "UNKNOWN_CARGO",
    );
    refused(&w.deliver("forged2", &c, "/etc", None, None), "UNKNOWN_BAY");
    assert!(w.list_floor(Some("../..")).is_err());
    assert_eq!(fx.outside_snap(), outside);
    assert!(fx.root.join("hello.txt").exists());
}

#[test]
fn meta_dir_as_symlink_is_refused() {
    let fx = Fx::new();
    symlink(&fx.outside, fx.root.join(".pallet")).unwrap();
    let outside = fx.outside_snap();
    let err = Warehouse::open(&fx.root, false).err().unwrap();
    assert_eq!(err.code, "META_UNSAFE");
    assert_eq!(fx.outside_snap(), outside);
}

#[test]
fn root_policy_refuses_home_roots_and_system_dirs() {
    let home = Path::new("/home/alice");
    for bad in [
        "/",
        "/home",
        "/home/alice",
        "/etc",
        "/usr/share/x",
        "/var/lib",
        "/proc/self",
    ] {
        assert!(
            check_root_policy(Path::new(bad), Some(home)).is_err(),
            "{bad}"
        );
    }
    assert!(check_root_policy(Path::new("/home/alice/PALLET-Warehouse"), Some(home)).is_ok());
    assert!(check_root_policy(Path::new("/srv/data/w"), Some(home)).is_err());
    assert!(check_root_policy(Path::new("/mnt/data/w"), Some(home)).is_ok());
}

#[test]
fn tmpfs_and_other_filesystems_are_refused() {
    // Needs a non-ext4 directory outside system paths. /tmp is tmpfs on many
    // machines; CI's privileged job mounts one and sets PALLET_TEST_TMPFS.
    let base = std::env::var("PALLET_TEST_TMPFS").unwrap_or_else(|_| "/tmp".into());
    let magic = rustix::fs::statfs(base.as_str())
        .map(|s| s.f_type as u64)
        .unwrap_or(0);
    if magic == 0 || magic == pallet_core::sys::EXT4_MAGIC {
        eprintln!("skipped: {base} is ext4; set PALLET_TEST_TMPFS to a tmpfs mount");
        return;
    }
    let dir = tempfile::Builder::new().tempdir_in(&base).unwrap();
    let err = Warehouse::open(dir.path(), false).err().unwrap();
    assert_eq!(err.code, "UNSUPPORTED_FS");
    assert!(
        !dir.path().join(".pallet").exists(),
        "nothing written to a refused root"
    );
}

#[test]
fn odd_names_empty_files_and_huge_sparse_files() {
    let fx = Fx::new();
    let names = [
        "Ünïcødé 名前.txt",
        "CON",
        "aux.txt",
        "trailing space ",
        "-dash",
        &"L".repeat(250),
    ];
    for n in names {
        fx.write(n, n.as_bytes());
    }
    fx.write("empty.txt", b"");
    let big = fs::File::create(fx.root.join("fifty-gb.bin")).unwrap();
    big.set_len(50 * 1000 * 1000 * 1000).unwrap();
    drop(big);
    let before = fx.snap();
    let mut w = fx.open();
    let docs = bay_id(&mut w, "Documents");
    let f = floor(&mut w, None);
    assert_eq!(
        f.cargo
            .iter()
            .find(|c| c.name == "fifty-gb.bin")
            .unwrap()
            .bytes,
        50_000_000_000
    );
    for (i, n) in names
        .iter()
        .chain(["empty.txt", "fifty-gb.bin"].iter())
        .enumerate()
    {
        let id = cargo(&mut w, n);
        committed(&w.deliver(&format!("t{i}"), &id, &docs, None, None));
    }
    assert_same_objects(&before, &fx.snap());
    let big = cargo_in(&mut w, "Documents", "fifty-gb.bin");
    refused(&w.duplicate("c", &big, None), "TOO_HEAVY");
}
