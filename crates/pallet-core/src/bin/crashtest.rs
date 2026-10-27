//! Test helper: runs one warehouse operation in a separate process so the
//! crash tests can kill it at a failpoint. Not shipped.

use pallet_core::warehouse::Warehouse;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = Path::new(&args[1]);
    let mut w = match Warehouse::open(root, false) {
        Ok(w) => w,
        Err(e) => {
            println!("OPEN_ERROR {}", e.code);
            return;
        }
    };
    if let Some(f) = &w.status().frozen {
        println!("FROZEN {:?}", f.kind);
        return;
    }
    let cmd = args.get(2).map(String::as_str).unwrap_or("open");
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
    let out = match cmd {
        "open" => {
            println!("OPENED");
            return;
        }
        "move" => w.deliver(
            &args[3],
            &find_cargo(&args[4]),
            &find_bay(&args[5]),
            None,
            None,
        ),
        "copy" => w.duplicate(&args[3], &find_cargo(&args[4]), None),
        "incident" => w.save_incident(&args[3], "Cargo: hello.txt\nFiles damaged: 0\n"),
        "undo" => w.undo(&args[3], &args[4], None),
        other => panic!("unknown command {other}"),
    };
    println!("OUTCOME {}", serde_json::to_string(&out).unwrap());
}
