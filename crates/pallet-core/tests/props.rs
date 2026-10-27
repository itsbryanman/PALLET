//! Property-based sequences of moves, undos, injected failures, and reopen
//! ("restart") against the real adapter. After every settled step the warehouse
//! must hold exactly the same file objects with the same bytes, and nothing
//! outside it may change. Seeds: CI runs fixed PROPTEST_RNG_SEED values plus a
//! rotating one that it prints.
#![cfg(all(target_os = "linux", feature = "failpoints"))]

mod common;

use common::*;
use pallet_core::failpoint;
use pallet_core::model::TxStatus;
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Step {
    Deliver {
        file: usize,
        bay: usize,
        fault: usize,
    },
    Undo {
        back: usize,
        fault: usize,
    },
    Copy {
        file: usize,
    },
    Reopen,
}

const FAULTS: [&str; 6] = [
    "",
    "journal_append:enospc@1",
    "journal_append:enospc@2",
    "journal_sync:eio@2",
    "rename:eio",
    "dir_fsync:eio",
];
const FILES: [&str; 4] = ["hello.txt", "b.txt", "c.txt", "d.txt"];
const BAYS: [&str; 3] = ["Documents", "Photos", "Archive"];

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        5 => (0..4usize, 0..4usize, 0..FAULTS.len()).prop_map(|(file, bay, fault)| Step::Deliver { file, bay, fault }),
        3 => (0..4usize, 0..FAULTS.len()).prop_map(|(back, fault)| Step::Undo { back, fault }),
        1 => (0..4usize).prop_map(|file| Step::Copy { file }),
        1 => Just(Step::Reopen),
    ]
}

/// Finds a crate by name on any floor (root or one level down).
fn find(w: &mut pallet_core::warehouse::Warehouse, name: &str) -> Option<String> {
    let root = floor(w, None);
    if let Some(c) = root.cargo.iter().find(|c| c.name == name) {
        return Some(c.id.clone());
    }
    for b in root.bays {
        let f = floor(w, Some(&b.id));
        if let Some(c) = f.cargo.iter().find(|c| c.name == name) {
            return Some(c.id.clone());
        }
    }
    None
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn random_sequences_keep_every_file_exactly_once(steps in prop::collection::vec(step(), 1..24)) {
        let fx = Fx::new();
        std::fs::create_dir(fx.root.join("Archive")).unwrap();
        fx.write("c.txt", b"sea\n");
        fx.write("d.txt", b"");
        let outside = fx.outside_snap();
        let mut originals = fx.snap();
        let mut w = fx.open();
        let mut n = 0;
        for s in steps {
            n += 1;
            let tx = format!("tx{n}");
            let outcome = match s {
                Step::Deliver { file, bay, fault } => {
                    let Some(c) = find(&mut w, FILES[file]) else { continue };
                    let b = if bay == 3 { w.root_bay() } else { bay_id(&mut w, BAYS[bay]) };
                    Some(failpoint::scoped(FAULTS[fault], || w.deliver(&tx, &c, &b, None, None)))
                }
                Step::Undo { back, fault } => {
                    let h = w.history();
                    let Some(e) = h.iter().filter(|e| !e.undone && e.op != pallet_core::model::Op::Undo).nth(back) else { continue };
                    let target = e.tx.clone();
                    Some(failpoint::scoped(FAULTS[fault], || w.undo(&tx, &target, None)))
                }
                Step::Copy { file } => {
                    let Some(c) = find(&mut w, FILES[file]) else { continue };
                    let o = w.duplicate(&tx, &c, None);
                    if o.status == TxStatus::Committed {
                        // Intentional duplicate: account for it separately.
                        originals = fx.snap();
                    }
                    Some(o)
                }
                Step::Reopen => {
                    drop(w);
                    w = fx.open();
                    None
                }
            };
            if let Some(o) = outcome {
                if o.status == TxStatus::Uncertain {
                    drop(w);
                    w = fx.open();
                }
            }
            if w.status().frozen.is_some() {
                w.acknowledge().expect("injected faults are acknowledgeable");
            }
            assert_same_objects(&originals, &fx.snap());
            prop_assert_eq!(&fx.outside_snap(), &outside);
            prop_assert_eq!(pallet_core::journal::parse(&fx.journal()).damage, None);
        }
        drop(w);
        let w = fx.open();
        prop_assert!(w.status().frozen.is_none());
        prop_assert!(w.status().recovery.is_empty(), "nothing left unsettled");
    }
}
