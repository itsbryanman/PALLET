//! Process-termination tests. A child process runs one operation and is killed
//! (SIGABRT at a failpoint, or SIGKILL while paused at one). The parent then
//! reopens the warehouse and checks the exactly-one invariant and recovery.
//!
//! These prove behavior under process crashes only. They are not power-loss tests.
#![cfg(all(target_os = "linux", feature = "failpoints"))]

mod common;

use common::*;
use pallet_core::model::TxStatus;
use std::io::{BufRead, BufReader};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_pallet-crashtest");

fn run(fx: &Fx, failpoint: &str, args: &[&str]) -> (Option<i32>, String) {
    let out = Command::new(BIN)
        .arg(&fx.root)
        .args(args)
        .env("PALLET_FAILPOINT", failpoint)
        .output()
        .unwrap();
    (
        out.status.signal(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Starts the child paused at `failpoint`, then SIGKILLs it.
fn kill_at(fx: &Fx, failpoint: &str, args: &[&str]) {
    let mut child = Command::new(BIN)
        .arg(&fx.root)
        .args(args)
        .env("PALLET_FAILPOINT", format!("{failpoint}:pause"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), format!("FAILPOINT {failpoint}"));
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(9));
}

fn reopen_and_check(
    fx: &Fx,
    before: &Snap,
    tx: &str,
    expect: Option<&str>,
) -> pallet_core::warehouse::Warehouse {
    let w = fx.open();
    let st = w.status();
    assert!(st.frozen.is_none(), "{:?}", st.frozen);
    match expect {
        Some(d) => {
            assert_eq!(st.recovery.len(), 1, "{:?}", st.recovery);
            assert_eq!(st.recovery[0].tx, tx);
            assert_eq!(st.recovery[0].decision, d);
        }
        None => assert!(st.recovery.is_empty(), "{:?}", st.recovery),
    }
    assert_same_objects(before, &fx.snap());
    w
}

#[test]
fn move_killed_at_every_step() {
    let cases = [
        ("move_after_prepared", Some("NOT_MOVED"), false),
        ("move_before_rename", Some("NOT_MOVED"), false),
        ("move_after_rename", Some("RECOVERED_COMMITTED"), true),
        ("move_after_dir_sync", Some("RECOVERED_COMMITTED"), true),
        ("move_after_committed", None, true),
    ];
    for (point, decision, moved) in cases {
        for sigkill in [false, true] {
            let fx = Fx::new();
            let before = fx.snap();
            let args = ["move", "tx-1", "hello.txt", "Documents"];
            if sigkill {
                kill_at(&fx, point, &args);
            } else {
                let (sig, _) = run(&fx, point, &args);
                assert_eq!(sig, Some(6), "{point} should abort");
            }
            let mut w = reopen_and_check(&fx, &before, "tx-1", decision);
            assert_eq!(fx.root.join("hello.txt").exists(), !moved, "{point}");
            assert_eq!(
                fx.root.join("Documents/hello.txt").exists(),
                moved,
                "{point}"
            );

            // The renderer retries with the same transaction ID: status, not a second move.
            let journal = fx.journal();
            let docs = bay_id(&mut w, "Documents");
            let o = w.deliver("tx-1", "irrelevant", &docs, None, None);
            assert!(o.replayed, "{point}: {o:?}");
            let want = if moved {
                TxStatus::Committed
            } else {
                TxStatus::Refused
            };
            assert_eq!(o.status, want, "{point}");
            assert_eq!(fx.journal(), journal);
            drop(w);
            // Idempotent: a second open has nothing left to decide.
            reopen_and_check(&fx, &before, "tx-1", None);
        }
    }
}

#[test]
fn torn_journal_from_a_real_crash_mid_append() {
    // Append #1 at open is OPENED, #2 is PREPARED. Tear PREPARED: nothing moved.
    let fx = Fx::new();
    let before = fx.snap();
    let (sig, _) = run(
        &fx,
        "journal_write:torn@2",
        &["move", "tx-1", "hello.txt", "Documents"],
    );
    assert_eq!(sig, Some(6));
    let mut w = fx.open();
    assert_eq!(
        w.status().frozen.unwrap().kind,
        pallet_core::model::FreezeKind::TornJournalTail
    );
    assert_eq!(fx.snap(), before);
    w.acknowledge().unwrap();
    assert!(w.status().frozen.is_none());
    assert!(fx.root.join("hello.txt").exists());

    // Tear COMMITTED (append #3): the move happened; identity says so.
    let fx = Fx::new();
    let before = fx.snap();
    let (sig, _) = run(
        &fx,
        "journal_write:torn@3",
        &["move", "tx-2", "hello.txt", "Documents"],
    );
    assert_eq!(sig, Some(6));
    let mut w = fx.open();
    assert!(w.status().frozen.is_some());
    let st = w.acknowledge().unwrap();
    assert_eq!(st.recovery[0].decision, "RECOVERED_COMMITTED");
    assert_same_objects(&before, &fx.snap());
    assert!(fx.root.join("Documents/hello.txt").exists());
}

#[test]
fn killing_recovery_itself_is_harmless() {
    let fx = Fx::new();
    let before = fx.snap();
    let (sig, _) = run(
        &fx,
        "move_after_rename",
        &["move", "tx-1", "hello.txt", "Documents"],
    );
    assert_eq!(sig, Some(6));
    for _ in 0..3 {
        let (sig, _) = run(&fx, "recovery_before_record", &["open"]);
        assert_eq!(sig, Some(6));
        assert_same_objects(&before, &fx.snap());
    }
    kill_at(&fx, "recovery_before_record", &["open"]);
    reopen_and_check(&fx, &before, "tx-1", Some("RECOVERED_COMMITTED"));
    reopen_and_check(&fx, &before, "tx-1", None);
}

#[test]
fn undo_killed_at_every_step() {
    for (point, decision, back) in [
        ("move_after_prepared", Some("NOT_MOVED"), false),
        ("move_after_rename", Some("RECOVERED_COMMITTED"), true),
        ("move_after_committed", None, true),
    ] {
        let fx = Fx::new();
        let (_, out) = run(&fx, "", &["move", "tx-1", "hello.txt", "Documents"]);
        assert!(out.contains("\"committed\""), "{out}");
        let before = fx.snap();
        let (sig, _) = run(&fx, point, &["undo", "undo-1", "tx-1"]);
        assert_eq!(sig, Some(6));
        reopen_and_check(&fx, &before, "undo-1", decision);
        assert_eq!(fx.root.join("hello.txt").exists(), back, "{point}");
    }
}

#[test]
fn copy_killed_at_every_step_never_shows_partial_cargo() {
    for (point, decision, published) in [
        ("copy_after_stream", Some("NOT_COPIED"), false),
        ("publish_before_rename", Some("NOT_PUBLISHED"), false),
        ("publish_after_rename", Some("RECOVERED_COMMITTED"), true),
    ] {
        let fx = Fx::new();
        let before = fx.snap();
        let (sig, _) = run(&fx, point, &["copy", "c-1", "hello.txt"]);
        assert_eq!(sig, Some(6), "{point}");
        let mut w = fx.open();
        let st = w.status();
        assert_eq!(st.recovery[0].decision, decision.unwrap(), "{point}");
        let after = fx.snap();
        let copy = fx.root.join("hello (copy).txt");
        assert_eq!(copy.exists(), published, "{point}");
        if published {
            assert_eq!(std::fs::read(&copy).unwrap(), b"hello\n");
            let mut expected = before.clone();
            expected.insert("hello (copy).txt".into(), after["hello (copy).txt"].clone());
            assert_eq!(
                after, expected,
                "{point}: exactly one intentional extra file"
            );
            assert!(fx.tmp_entries().is_empty());
        } else {
            assert_eq!(after, before, "{point}");
            let f = floor(&mut w, None);
            assert!(!f.cargo.iter().any(|c| c.name.contains("copy")));
        }
        drop(w);
        assert!(fx.open().status().recovery.is_empty());
    }
}

#[test]
fn incident_report_killed_mid_publish() {
    let fx = Fx::new();
    let (sig, _) = run(&fx, "publish_after_rename", &["incident", "i-1"]);
    assert_eq!(sig, Some(6));
    let w = fx.open();
    assert_eq!(w.status().recovery[0].decision, "RECOVERED_COMMITTED");
    let reports: Vec<_> = std::fs::read_dir(&fx.root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("incident-report-")
        })
        .collect();
    assert_eq!(reports.len(), 1);
}

#[test]
fn second_process_cannot_open_a_busy_warehouse() {
    let fx = Fx::new();
    let _held = fx.open();
    let (_, out) = run(&fx, "", &["open"]);
    assert_eq!(out.trim(), "OPEN_ERROR BUSY");
}
