//! The kernel-enforced boundary: every test here goes through `Worker`, the
//! same confined thread the app uses. The "other program" that interferes runs
//! on a separate, unconfined thread, as it would in real life.
#![cfg(all(target_os = "linux", feature = "failpoints"))]

mod common;

use common::*;
use pallet_core::failpoint;
use pallet_core::model::TxStatus;
use pallet_core::worker::Worker;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;

fn open(fx: &Fx) -> Worker {
    Worker::open(&fx.root, false).expect("open confined warehouse")
}

fn ids(w: &Worker, file: &str, bay: &str) -> (String, String) {
    let (file, bay) = (file.to_string(), bay.to_string());
    w.run(move |w| (cargo(w, &file), bay_id(w, &bay))).unwrap()
}

/// Runs `act` on another (unconfined) thread when the worker reaches `point`,
/// and waits for it, so the interference lands exactly there.
fn interfere_at(w: &Worker, point: &'static str, act: impl FnOnce() + Send + 'static) {
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        if go_rx.recv().is_ok() {
            act();
            let _ = done_tx.send(());
        }
    });
    w.run(move |_| {
        let mut armed = Some((go_tx, done_rx));
        failpoint::set_hook(Some(Box::new(move |p| {
            if p == point {
                if let Some((go, done)) = armed.take() {
                    go.send(()).unwrap();
                    done.recv().unwrap();
                }
            }
        })));
    })
    .unwrap();
}

#[test]
fn the_worker_thread_cannot_touch_anything_outside_the_root() {
    let fx = Fx::new();
    let w = open(&fx);
    let outside = fx.outside.clone();
    let root = fx.root.clone();
    let (write_out, read_out, write_in) = w
        .run(move |_| {
            (
                fs::write(outside.join("escape.txt"), b"x").is_err(),
                fs::read(outside.join("sentinel.txt")).is_err(),
                fs::write(root.join("inside.txt"), b"x").is_ok(),
            )
        })
        .unwrap();
    assert!(write_out, "write outside the root must be denied");
    assert!(read_out, "read outside the root must be denied");
    assert!(write_in, "the root itself stays usable");
    assert!(!fx.outside.join("escape.txt").exists());
    // Other threads of the process are not confined.
    assert!(fs::read(fx.outside.join("sentinel.txt")).is_ok());
}

#[test]
fn a_bay_moved_out_right_before_the_rename_is_refused_by_the_kernel() {
    let fx = Fx::new();
    let before = fx.snap();
    let w = open(&fx);
    let (c, docs) = ids(&w, "hello.txt", "Documents");
    let (root, out): (PathBuf, PathBuf) = (fx.root.clone(), fx.outside.clone());
    // After PALLET's own revalidation, immediately before renameat2.
    interfere_at(&w, "move_before_rename", move || fs::rename(root.join("Documents"), out.join("Documents")).unwrap());
    let o = w.run(move |w| w.deliver("t1", &c, &docs, None, None)).unwrap();
    assert_eq!(o.status, TxStatus::Refused, "{o:?}");
    assert_eq!(o.code.as_deref(), Some("BOUNDARY_REFUSED"), "{o:?}");
    assert_eq!(fs::read(fx.root.join("hello.txt")).unwrap(), b"hello\n", "file never left");
    assert!(!fx.outside.join("Documents/hello.txt").exists(), "nothing landed outside");
    assert!(w.run(|w| w.status().frozen.is_none()).unwrap(), "a refusal, not an ambiguous state");
    // Put the bay back: everything inside is exactly as before.
    fs::rename(fx.outside.join("Documents"), fx.root.join("Documents")).unwrap();
    assert_eq!(fx.snap(), before);
}

#[test]
fn a_symlink_swapped_in_right_before_the_rename_cannot_redirect_it() {
    let fx = Fx::new();
    let outside = fx.outside_snap();
    let w = open(&fx);
    let (c, docs) = ids(&w, "hello.txt", "Documents");
    let (root, out) = (fx.root.clone(), fx.outside.clone());
    interfere_at(&w, "move_before_rename", move || {
        fs::rename(root.join("Documents"), root.join("Documents.real")).unwrap();
        std::os::unix::fs::symlink(&out, root.join("Documents")).unwrap();
    });
    let o = w.run(move |w| w.deliver("t1", &c, &docs, None, None)).unwrap();
    // The held directory handle still names the real folder (now Documents.real),
    // which is inside the warehouse; the symlink is never followed. The recorded
    // path no longer matches, so PALLET freezes rather than guess.
    assert_eq!(o.status, TxStatus::Uncertain, "{o:?}");
    assert_eq!(fx.outside_snap(), outside, "nothing reached the symlink target");
    assert_eq!(fs::read(fx.root.join("Documents.real/hello.txt")).unwrap(), b"hello\n");
    let kind = w.run(|w| w.status().frozen.map(|f| f.kind)).unwrap();
    assert_eq!(kind, Some(pallet_core::model::FreezeKind::DestinationMoved));
}

#[test]
fn normal_work_is_unaffected_by_confinement() {
    let fx = Fx::new();
    let before = fx.snap();
    let w = open(&fx);
    let (c, docs) = ids(&w, "hello.txt", "Documents");
    let o = w.run(move |w| w.deliver("t1", &c, &docs, None, None)).unwrap();
    committed(&o);
    committed(&w.run(|w| w.undo("u1", "t1", None)).unwrap());
    assert_eq!(fx.snap(), before);
    let (b, _) = ids(&w, "b.txt", "Documents");
    committed(&w.run(move |w| w.duplicate("c1", &b, None)).unwrap());
    committed(&w.run(|w| w.save_incident("i1", "Files damaged: 0\n")).unwrap());
    committed(&w.run(|w| w.undo("u2", "c1", None)).unwrap());
    assert!(fx.root.join("Recovered Copies/b (copy).txt").exists());
}

#[test]
fn a_second_worker_on_the_same_warehouse_is_refused_and_the_lock_is_released_on_drop() {
    let fx = Fx::new();
    let w = open(&fx);
    assert_eq!(Worker::open(&fx.root, false).err().unwrap().code, "BUSY");
    drop(w);
    assert!(Worker::open(&fx.root, false).is_ok());
}
