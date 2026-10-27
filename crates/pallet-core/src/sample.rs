//! The sample warehouse: a few small files in PALLET's own data directory, so the
//! first launch has something to drive into. Created once; never overwritten.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// The smallest valid ZIP: an empty archive (end-of-central-directory record only).
const EMPTY_ZIP: [u8; 22] = [
    0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

const MANUAL: &str = "# Forklift operator manual (abridged)

1. Forks low when driving.
2. Do not take corners at speed with a raised load.
3. Every delivery needs a signature. PALLET asks for one.
4. Accidents are paperwork, not data loss. The file stays where it was.
";

/// Creates the sample warehouse at `dir` if `dir` does not exist yet.
/// Returns `false` (and touches nothing) if it already exists.
pub fn seed(dir: &Path) -> io::Result<bool> {
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::create_dir(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e),
    }
    let put = |rel: &str, bytes: &[u8]| -> io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(rel))?;
        f.write_all(bytes)?;
        f.sync_all()
    };
    for bay in ["Documents", "Photos", "Archive"] {
        fs::create_dir(dir.join(bay))?;
    }
    put("hello.txt", b"Hello. You moved me with a forklift.\n")?;
    put(
        "inventory.csv",
        b"item,qty\npallets,12\nforklifts,1\nincident reports,0\n",
    )?;
    put("forklift-manual.md", MANUAL.as_bytes())?;
    put("empty-envelope.txt", b"")?;
    put("old-backups.zip", &EMPTY_ZIP)?;
    let mut big = Vec::with_capacity(3 << 20);
    for i in 0..(3u32 << 18) {
        big.extend_from_slice(&i.to_le_bytes());
    }
    put("heavy-machinery.bin", &big)?;
    put("Documents/notes.txt", b"Remember: forks low.\n")?;
    put("Archive/2019-taxes.zip", &EMPTY_ZIP)?;
    Ok(true)
}
