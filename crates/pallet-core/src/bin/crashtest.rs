//! Test helper: runs one warehouse operation in a separate process, through the
//! same confined worker the app uses, so the crash tests can kill it at a
//! failpoint. Not shipped.

use pallet_core::worker::Worker;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let w = match Worker::open(Path::new(&args[1]), false) {
        Ok(w) => w,
        Err(e) => {
            println!("OPEN_ERROR {}", e.code);
            return;
        }
    };
    if let Some(f) = w.run(|w| w.status().frozen).unwrap() {
        println!("FROZEN {:?}", f.kind);
        return;
    }
    let cmd = args.get(2).cloned().unwrap_or_else(|| "open".into());
    if cmd == "open" {
        println!("OPENED");
        return;
    }
    let rest: Vec<String> = args[3..].to_vec();
    let out = w
        .run(move |w| {
            let floor = w.list_floor(None).expect("list root");
            let find_cargo = |name: &str| {
                floor
                    .cargo
                    .iter()
                    .find(|c| c.name == name)
                    .map(|c| c.id.clone())
                    .expect("cargo")
            };
            let find_bay = |name: &str| {
                floor
                    .bays
                    .iter()
                    .find(|b| b.name == name)
                    .map(|b| b.id.clone())
                    .expect("bay")
            };
            match cmd.as_str() {
                "move" => w.deliver(
                    &rest[0],
                    &find_cargo(&rest[1]),
                    &find_bay(&rest[2]),
                    None,
                    None,
                ),
                "copy" => w.duplicate(&rest[0], &find_cargo(&rest[1]), None),
                "incident" => w.save_incident(&rest[0], "Cargo: hello.txt\nFiles damaged: 0\n"),
                "undo" => w.undo(&rest[0], &rest[1], None),
                other => panic!("unknown command {other}"),
            }
        })
        .unwrap();
    println!("OUTCOME {}", serde_json::to_string(&out).unwrap());
}
