//! The warehouse service. Owns the root handle, the journal, the cargo/bay ID
//! tables, and every mutation. Callers are expected to serialize access (the
//! app holds it behind one mutex: S3 step 1, "one backend worker").

use crate::failpoint;
use crate::journal::{Damage, Fingerprint, Ident, Journal, Kind, Record};
use crate::model::*;
use crate::names::{self, META_DIR, RECOVERED_COPIES};
use crate::sys;
use rustix::fd::OwnedFd;
use rustix::fs::FileType;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Below this much free space, deliveries are refused before journaling (S6).
const JOURNAL_RESERVE: u64 = 1024 * 1024;
/// The duplicator handles small crates only.
pub const COPY_LIMIT: u64 = 1024 * 1024 * 1024;
const FLOOR_LIMIT: usize = 5000;
const INCIDENT_LIMIT: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenError {
    pub code: &'static str,
    pub message: String,
}

fn open_err(code: &'static str, message: impl Into<String>) -> OpenError {
    OpenError {
        code,
        message: message.into(),
    }
}

#[derive(Debug, Clone)]
struct Refusal {
    code: &'static str,
    message: String,
    suggestion: Option<String>,
}

fn refusal(code: &'static str, message: impl Into<String>) -> Refusal {
    Refusal {
        code,
        message: message.into(),
        suggestion: None,
    }
}

/// `(from, to)` relative paths on success; the refusal and any known paths otherwise.
type TxResult = Result<(String, String), (Refusal, Option<(String, String)>)>;

#[derive(Debug, Clone)]
struct CargoRef {
    dir: Vec<String>,
    dir_id: Ident,
    name: String,
    id: Ident,
    fp: Fingerprint,
}

#[derive(Debug, Clone)]
struct BayRef {
    path: Vec<String>,
    id: Ident,
}

/// A located directory entry: directory components, that directory's identity, and a name.
#[derive(Debug, Clone)]
struct Loc {
    dir: Vec<String>,
    dir_id: Ident,
    name: String,
}

impl Loc {
    fn rel(&self) -> String {
        join_rel(&self.dir, &self.name)
    }
}

fn join_rel(dir: &[String], name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", dir.join("/"), name)
    }
}

/// Splits a journaled relative path back into components. Each component is
/// validated again: the journal is data, not authority.
fn split_rel(rel: &str) -> Option<(Vec<String>, String)> {
    let mut parts: Vec<String> = rel.split('/').map(str::to_string).collect();
    let name = parts.pop()?;
    for (i, p) in parts.iter().chain(std::iter::once(&name)).enumerate() {
        names::validate(p, i == 0).ok()?;
    }
    Some((parts, name))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn random_id() -> String {
    let mut b = [0u8; 12];
    getrandom::fill(&mut b).expect("system randomness");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn valid_tx(tx: &str) -> bool {
    (1..=64).contains(&tx.len())
        && tx
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Civil date from a Unix timestamp, for incident report names. (Howard Hinnant's algorithm.)
fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        (rem / 60 % 60) as u32,
        (rem % 60) as u32,
    )
}

/// Root policy (S1). This filters obviously wrong choices; it is not the
/// boundary. The boundary is the handle-based resolution in `sys`.
pub fn check_root_policy(root: &Path, home: Option<&Path>) -> Result<(), OpenError> {
    if root.parent().is_none() {
        return Err(open_err(
            "ROOT_REFUSED",
            "A filesystem root can't be a warehouse.",
        ));
    }
    if let Some(home) = home {
        if home.starts_with(root) {
            return Err(open_err(
                "ROOT_REFUSED",
                "Your home folder (or a folder containing it) can't be a warehouse. Pick or create a dedicated folder.",
            ));
        }
    }
    // Folders strictly inside the user's home are the normal case.
    if home.is_some_and(|h| root.starts_with(h)) {
        return Ok(());
    }
    const SYSTEM: &[&str] = &[
        "/bin", "/boot", "/dev", "/etc", "/lib", "/lib32", "/lib64", "/libx32", "/opt", "/proc",
        "/root", "/run", "/sbin", "/snap", "/srv", "/sys", "/usr", "/var",
    ];
    if SYSTEM.iter().any(|s| root.starts_with(s)) {
        return Err(open_err(
            "ROOT_REFUSED",
            "System directories can't be a warehouse.",
        ));
    }
    Ok(())
}

pub struct Warehouse {
    root_path: PathBuf,
    root: OwnedFd,
    root_id: Ident,
    meta: OwnedFd,
    tmp: OwnedFd,
    evidence: OwnedFd,
    _lock: File,
    journal: Journal,
    records: Vec<Record>,
    sample: bool,
    frozen: Option<Freeze>,
    torn_valid_len: Option<u64>,
    recovery: Vec<RecoveryDecision>,
    cargo: HashMap<String, CargoRef>,
    cargo_by_ident: HashMap<Ident, String>,
    bays: HashMap<String, BayRef>,
    bay_by_ident: HashMap<Ident, String>,
}

impl Warehouse {
    /// Binds a warehouse root. Runs recovery before returning; if anything is
    /// ambiguous the warehouse opens frozen and refuses mutations.
    pub fn open(path: &Path, sample: bool) -> Result<Warehouse, OpenError> {
        let canonical = std::fs::canonicalize(path)
            .map_err(|e| open_err("ROOT_MISSING", format!("Can't open that folder: {e}.")))?;
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .and_then(|h| std::fs::canonicalize(h).ok());
        check_root_policy(&canonical, home.as_deref())?;

        let root = sys::open_root(&canonical)
            .map_err(|e| open_err("ROOT_MISSING", format!("Can't open that folder: {e}.")))?;
        let st = sys::fstat(&root).map_err(|e| open_err("IO", e.to_string()))?;
        if sys::file_type(&st) != FileType::Directory {
            return Err(open_err("ROOT_REFUSED", "That isn't a folder."));
        }
        let root_id = sys::ident(&st);
        let magic = sys::fs_magic(&root).map_err(|e| open_err("IO", e.to_string()))?;
        if magic != sys::EXT4_MAGIC {
            return Err(open_err(
                "UNSUPPORTED_FS",
                format!("This folder is on a filesystem PALLET hasn't been tested on (type 0x{magic:x}). Real-file mode supports ext4 only."),
            ));
        }
        if sys::is_casefold(&root).unwrap_or(true) {
            return Err(open_err(
                "UNSUPPORTED_FS",
                "Case-insensitive (casefold) folders aren't supported.",
            ));
        }
        // Probe openat2. ENOSYS or a seccomp denial means no confinement: refuse.
        sys::open_dir_beneath(&root, &[]).map_err(|e| {
            open_err(
                "UNSUPPORTED_KERNEL",
                format!(
                    "This system can't confine paths with openat2 ({e}). Real-file mode is off."
                ),
            )
        })?;
        if !sys::dir_writable(&root) {
            return Err(open_err(
                "ROOT_READONLY",
                "PALLET can't write to that folder.",
            ));
        }

        let mkdir = |dir: &OwnedFd, name: &str, rel: &[String]| -> Result<OwnedFd, OpenError> {
            sys::mkdir_at(dir, name, 0o700)
                .map_err(|e| open_err("IO", format!("Can't create {name}: {e}")))?;
            sys::open_dir_beneath(&root, rel).map_err(|e| {
                open_err(
                    "META_UNSAFE",
                    format!("`{}` must be a plain folder ({e}).", rel.join("/")),
                )
            })
        };
        let meta = mkdir(&root, META_DIR, &[META_DIR.into()])?;
        let tmp = mkdir(&meta, "tmp", &[META_DIR.into(), "tmp".into()])?;
        let evidence = mkdir(&meta, "evidence", &[META_DIR.into(), "evidence".into()])?;

        let lock =
            sys::open_meta_file(&meta, "lock", false).map_err(|e| open_err("IO", e.to_string()))?;
        sys::lock_exclusive(&lock).map_err(|_| {
            open_err(
                "BUSY",
                "Another PALLET window already has this warehouse open.",
            )
        })?;

        let jf = sys::open_meta_file(&meta, "journal.log", true)
            .map_err(|e| open_err("IO", e.to_string()))?;
        let _ = sys::fsync(&meta);
        let (journal, loaded) = Journal::load(jf).map_err(|e| {
            open_err(
                "JOURNAL_UNREADABLE",
                format!("The journal can't be read: {e}. Nothing was changed."),
            )
        })?;

        let mut w = Warehouse {
            root_path: canonical,
            root,
            root_id,
            meta,
            tmp,
            evidence,
            _lock: lock,
            journal,
            records: loaded.records,
            sample,
            frozen: None,
            torn_valid_len: None,
            recovery: Vec::new(),
            cargo: HashMap::new(),
            cargo_by_ident: HashMap::new(),
            bays: HashMap::new(),
            bay_by_ident: HashMap::new(),
        };

        match loaded.damage {
            Some(Damage::Corrupt { offset }) => {
                w.frozen = Some(Freeze {
                    kind: FreezeKind::CorruptJournal,
                    explanation: format!(
                        "The journal has an unreadable record at byte {offset}, before its end. PALLET won't guess what happened after it. Nothing was changed. See RECOVERY.md."
                    ),
                    acknowledgeable: false,
                });
                return Ok(w);
            }
            Some(Damage::TornTail { offset }) => {
                // Analyse without writing; appending after torn bytes would hide them.
                w.torn_valid_len = Some(offset);
                w.recovery = w.pending().iter().map(|r| w.decide(r)).collect();
                w.frozen = Some(Freeze {
                    kind: FreezeKind::TornJournalTail,
                    explanation: format!(
                        "The last journal record (from byte {offset}) is incomplete, which happens when PALLET is stopped while writing it. Continuing keeps a copy of the journal in .pallet/evidence/ and then settles any unfinished delivery by checking file identities."
                    ),
                    acknowledgeable: true,
                });
                return Ok(w);
            }
            None => {}
        }

        w.recover();
        if w.frozen.is_none() {
            let rec = Record {
                root_id: Some(root_id),
                ..w.record("open", Kind::Opened)
            };
            let _ = w.journal.append(&rec);
            w.records.push(rec);
        }
        Ok(w)
    }

    pub fn status(&self) -> WarehouseStatus {
        WarehouseStatus {
            root_path: self.root_path.display().to_string(),
            sample: self.sample,
            frozen: self.frozen.clone(),
            recovery: self.recovery.clone(),
        }
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    fn record(&self, tx: &str, kind: Kind) -> Record {
        Record {
            v: 1,
            tx: tx.to_string(),
            ts: now_ms(),
            kind: Some(kind),
            ..Default::default()
        }
    }

    fn append(&mut self, rec: Record) -> io::Result<()> {
        self.journal.append(&rec)?;
        self.records.push(rec);
        Ok(())
    }

    // ---------- recovery (S3) ----------

    /// Transactions whose last word is PREPARED or STAGED.
    fn pending(&self) -> Vec<Record> {
        let mut open: Vec<String> = Vec::new();
        let mut first: HashMap<&str, &Record> = HashMap::new();
        let mut closed: HashMap<&str, bool> = HashMap::new();
        for r in &self.records {
            match r.kind {
                Some(Kind::Prepared) => {
                    if !first.contains_key(r.tx.as_str()) {
                        open.push(r.tx.clone());
                    }
                    first.insert(&r.tx, r);
                }
                Some(Kind::Committed | Kind::Failed | Kind::Recovery | Kind::Acknowledged) => {
                    closed.insert(&r.tx, true);
                }
                _ => {}
            }
        }
        open.into_iter()
            .filter(|t| !closed.contains_key(t.as_str()))
            .map(|t| (*first[t.as_str()]).clone())
            .collect()
    }

    fn staged_for(&self, tx: &str) -> Option<&Record> {
        self.records
            .iter()
            .find(|r| r.tx == tx && r.kind == Some(Kind::Staged))
    }

    /// Where is the object with identity `id` recorded at `rel`? Never follows links.
    fn present(&self, rel: &str, id: Ident) -> Result<bool, String> {
        let Some((dir, name)) = split_rel(rel) else {
            return Err(format!("journal path `{rel}` is not a valid relative path"));
        };
        let dirfd = match sys::open_dir_beneath(&self.root, &dir) {
            Ok(fd) => fd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(format!("can't open the folder of `{rel}`: {e}")),
        };
        match sys::lstat_at(&dirfd, &name) {
            Ok(st) => Ok(sys::ident(&st) == id && sys::file_type(&st) == FileType::RegularFile),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("can't inspect `{rel}`: {e}")),
        }
    }

    /// Temporary outputs live in `.pallet/tmp`, which journal path validation
    /// deliberately rejects, so they are checked through the held handle.
    fn present_tmp(&self, tmp_rel: &str, id: Ident) -> Result<bool, String> {
        let name = tmp_rel
            .strip_prefix(&format!("{META_DIR}/tmp/"))
            .filter(|n| names::validate(n, false).is_ok())
            .ok_or_else(|| format!("journal temp path `{tmp_rel}` is not in .pallet/tmp"))?;
        match sys::lstat_at(&self.tmp, name) {
            Ok(st) => Ok(sys::ident(&st) == id && sys::file_type(&st) == FileType::RegularFile),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("can't inspect `{tmp_rel}`: {e}")),
        }
    }

    fn decide(&self, p: &Record) -> RecoveryDecision {
        let op = p.op.unwrap_or(Op::Move);
        let mk = |decision: &str, evidence: String| RecoveryDecision {
            tx: p.tx.clone(),
            op,
            decision: decision.to_string(),
            evidence,
        };
        let src = p.src.clone().unwrap_or_default();
        let dst = p.dst.clone().unwrap_or_default();
        match op {
            Op::Move | Op::Undo => {
                let Some(id) = p.src_id else {
                    return mk("FROZEN", "PREPARED record has no source identity".into());
                };
                let at_src = self.present(&src, id);
                let at_dst = self.present(&dst, id);
                match (at_src, at_dst) {
                    (Ok(true), Ok(false)) => mk("NOT_MOVED", format!("`{src}` is still at its source; nothing was moved.")),
                    (Ok(false), Ok(true)) => mk("RECOVERED_COMMITTED", format!("The same file is at `{dst}`; the move completed.")),
                    (Ok(true), Ok(true)) => mk("FROZEN", format!("The same file appears at both `{src}` and `{dst}` (hard link?).")),
                    (Ok(false), Ok(false)) => mk("FROZEN", format!("The file is at neither `{src}` nor `{dst}`. Something outside PALLET moved it.")),
                    (Err(e), _) | (_, Err(e)) => mk("FROZEN", e),
                }
            }
            Op::Copy | Op::Incident => {
                let tmp = p.tmp.clone().unwrap_or_default();
                let Some(staged) = self.staged_for(&p.tx) else {
                    return mk("NOT_COPIED", format!("No verified output existed yet. Any partial file stays in `{tmp}` and is not cargo."));
                };
                let Some(id) = staged.tmp_id else {
                    return mk("FROZEN", "STAGED record has no identity".into());
                };
                match (self.present(&dst, id), self.present_tmp(&tmp, id)) {
                    (Ok(true), Ok(false)) => mk(
                        "RECOVERED_COMMITTED",
                        format!("The verified output is at `{dst}`."),
                    ),
                    (Ok(false), Ok(true)) => mk(
                        "NOT_PUBLISHED",
                        format!("The verified output was never published; it stays in `{tmp}`."),
                    ),
                    (Ok(a), Ok(b)) => mk(
                        "FROZEN",
                        format!("Output found at destination: {a}, in temporary storage: {b}."),
                    ),
                    (Err(e), _) | (_, Err(e)) => mk("FROZEN", e),
                }
            }
        }
    }

    /// Settles pending transactions by identity and journals each decision.
    /// Idempotent: a settled transaction is never pending again.
    fn recover(&mut self) {
        let pending = self.pending();
        let mut decisions = Vec::new();
        let mut ambiguous = false;

        let last_root = self
            .records
            .iter()
            .rev()
            .find(|r| r.kind == Some(Kind::Opened))
            .and_then(|r| r.root_id);
        let root_changed = last_root.is_some_and(|id| id != self.root_id);

        for p in pending {
            let mut d = self.decide(&p);
            if root_changed && d.decision != "NOT_MOVED" && d.decision != "NOT_COPIED" {
                d.decision = "FROZEN".into();
                d.evidence = format!(
                    "The warehouse folder is a different folder than when this ran. {}",
                    d.evidence
                );
            }
            if d.decision == "FROZEN" {
                ambiguous = true;
                decisions.push(d);
                continue;
            }
            let dst_id = if d.decision == "RECOVERED_COMMITTED" {
                match p.op {
                    Some(Op::Copy | Op::Incident) => self.staged_for(&p.tx).and_then(|s| s.tmp_id),
                    _ => p.src_id,
                }
            } else {
                None
            };
            let rec = Record {
                op: p.op,
                decision: Some(d.decision.clone()),
                evidence: Some(d.evidence.clone()),
                dst_id,
                ..self.record(&p.tx, Kind::Recovery)
            };
            failpoint::hit("recovery_before_record");
            if self.append(rec).is_err() {
                ambiguous = true;
                d.decision = "FROZEN".into();
                d.evidence = format!("{} (but the decision couldn't be journaled)", d.evidence);
            }
            decisions.push(d);
        }
        self.recovery = decisions;
        if ambiguous {
            self.frozen = Some(Freeze {
                kind: FreezeKind::AmbiguousTransaction,
                explanation: "An unfinished delivery couldn't be settled from file identities. PALLET changed nothing. Check the listed files with your file manager, then acknowledge to continue.".into(),
                acknowledgeable: true,
            });
        }
    }

    /// The user reviewed a frozen state and chose to continue.
    pub fn acknowledge(&mut self) -> Result<WarehouseStatus, String> {
        let Some(freeze) = self.frozen.clone() else {
            return Ok(self.status());
        };
        if !freeze.acknowledgeable {
            return Err("This state can't be cleared from inside PALLET. See RECOVERY.md.".into());
        }
        let tx = random_id();
        if freeze.kind == FreezeKind::TornJournalTail {
            let valid = self.torn_valid_len.ok_or("no torn tail recorded")?;
            let bytes = self.journal.bytes().map_err(|e| e.to_string())?;
            let (y, m, d, hh, mm, ss) = civil(now_ms() / 1000);
            let name = format!(
                "journal-{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}-{}.torn",
                &tx[..6]
            );
            let mut f =
                sys::create_excl(&self.evidence, &name, 0o600).map_err(|e| e.to_string())?;
            f.write_all(&bytes)
                .and_then(|_| f.sync_all())
                .map_err(|e| e.to_string())?;
            sys::fsync(&self.evidence).map_err(|e| e.to_string())?;
            self.journal.truncate_to(valid).map_err(|e| e.to_string())?;
            self.torn_valid_len = None;
            let rec = Record {
                evidence: Some(format!(".pallet/evidence/{name}")),
                ..self.record(&tx, Kind::Acknowledged)
            };
            self.append(rec).map_err(|e| e.to_string())?;
            self.frozen = None;
            self.recover();
            if self.frozen.is_none() {
                let rec = Record {
                    root_id: Some(self.root_id),
                    ..self.record("open", Kind::Opened)
                };
                let _ = self.append(rec);
            }
            return Ok(self.status());
        }
        for d in self
            .recovery
            .clone()
            .into_iter()
            .filter(|d| d.decision == "FROZEN")
        {
            let rec = Record {
                op: Some(d.op),
                evidence: Some(d.evidence.clone()),
                ..self.record(&d.tx, Kind::Acknowledged)
            };
            self.append(rec).map_err(|e| e.to_string())?;
        }
        let rec = Record {
            evidence: Some(format!("{:?}: {}", freeze.kind, freeze.explanation)),
            ..self.record(&tx, Kind::Acknowledged)
        };
        self.append(rec).map_err(|e| e.to_string())?;
        self.frozen = None;
        Ok(self.status())
    }

    // ---------- listing ----------

    fn bay_id_for(&mut self, path: Vec<String>, id: Ident) -> String {
        if let Some(existing) = self.bay_by_ident.get(&id) {
            if let Some(b) = self.bays.get_mut(existing) {
                b.path = path;
            }
            return existing.clone();
        }
        let key = random_id();
        self.bays.insert(key.clone(), BayRef { path, id });
        self.bay_by_ident.insert(id, key.clone());
        key
    }

    fn cargo_id_for(&mut self, c: CargoRef) -> String {
        if let Some(existing) = self.cargo_by_ident.get(&c.id) {
            self.cargo.insert(existing.clone(), c);
            return existing.clone();
        }
        let key = random_id();
        self.cargo_by_ident.insert(c.id, key.clone());
        self.cargo.insert(key.clone(), c);
        key
    }

    pub fn root_bay(&mut self) -> String {
        self.bay_id_for(Vec::new(), self.root_id)
    }

    fn open_checked(
        &self,
        dir: &[String],
        id: Ident,
        missing: &'static str,
    ) -> Result<OwnedFd, Refusal> {
        let fd = sys::open_dir_beneath(&self.root, dir).map_err(|e| match e.raw_os_error() {
            Some(2) | Some(20) => refusal(missing, format!("`/{}` is gone.", dir.join("/"))),
            Some(40) => refusal(
                "LINK_IN_PATH",
                format!("`/{}` now goes through a link. Refused.", dir.join("/")),
            ),
            Some(18) => refusal("CROSS_DEVICE", "That folder is on another device. Refused."),
            _ => refusal("IO", format!("Can't open `/{}`: {e}", dir.join("/"))),
        })?;
        let st = sys::fstat(&fd).map_err(|e| refusal("IO", e.to_string()))?;
        if sys::ident(&st) != id {
            return Err(refusal(
                missing,
                format!("`/{}` was replaced by a different folder.", dir.join("/")),
            ));
        }
        Ok(fd)
    }

    pub fn list_floor(&mut self, bay: Option<&str>) -> Result<Floor, String> {
        let bay_ref = match bay {
            None => BayRef {
                path: Vec::new(),
                id: self.root_id,
            },
            Some(b) => self.bays.get(b).cloned().ok_or("Unknown bay.")?,
        };
        let dir = self
            .open_checked(&bay_ref.path, bay_ref.id, "BAY_MISSING")
            .map_err(|r| r.message)?;
        let at_root = bay_ref.path.is_empty();
        let mut entries = sys::read_dir(&dir).map_err(|e| e.to_string())?;
        entries.sort_by(|a, b| {
            a.0.to_lowercase()
                .cmp(&b.0.to_lowercase())
                .then(a.0.cmp(&b.0))
        });
        let truncated = entries.len() > FLOOR_LIMIT;
        entries.truncate(FLOOR_LIMIT);

        let mut bays = Vec::new();
        let mut cargo = Vec::new();
        for (name, utf8) in entries {
            if at_root && name == META_DIR {
                continue;
            }
            let Ok(st) = sys::lstat_at(&dir, &name) else {
                continue;
            };
            let id = sys::ident(&st);
            let ft = sys::file_type(&st);
            let mut path = bay_ref.path.clone();
            path.push(name.clone());
            let unavailable = |reason: &str| Some(reason.to_string());
            match ft {
                FileType::Directory => {
                    let reason = if !utf8 {
                        unavailable("name isn't UTF-8")
                    } else if id.dev != self.root_id.dev {
                        unavailable("another device")
                    } else {
                        match sys::open_dir_beneath(&self.root, &path) {
                            Ok(fd) if sys::is_casefold(&fd).unwrap_or(true) => {
                                unavailable("case-insensitive folder")
                            }
                            Ok(fd) if !sys::dir_writable(&fd) => unavailable("read-only folder"),
                            Ok(_) => None,
                            Err(_) => unavailable("can't open"),
                        }
                    };
                    let bid = self.bay_id_for(path, id);
                    bays.push(BayInfo {
                        id: bid,
                        name,
                        unavailable: reason,
                    });
                }
                FileType::RegularFile => {
                    let reason = if !utf8 {
                        unavailable("name isn't UTF-8")
                    } else if st.st_nlink > 1 {
                        unavailable("hard-linked")
                    } else if st.st_mode & 0o200 == 0 {
                        unavailable("read-only")
                    } else {
                        None
                    };
                    let c = CargoRef {
                        dir: bay_ref.path.clone(),
                        dir_id: bay_ref.id,
                        name: name.clone(),
                        id,
                        fp: sys::fingerprint(&st),
                    };
                    let bytes = st.st_size as u64;
                    let cid = self.cargo_id_for(c);
                    cargo.push(CargoInfo {
                        id: cid,
                        zip: names::is_zip(&name),
                        name,
                        bytes,
                        unavailable: reason,
                    });
                }
                other => {
                    let what = match other {
                        FileType::Symlink => "link",
                        FileType::Fifo => "pipe",
                        FileType::Socket => "socket",
                        FileType::CharacterDevice | FileType::BlockDevice => "device",
                        _ => "special file",
                    };
                    // Unavailable entries get a throwaway ID; they can't be cargo.
                    cargo.push(CargoInfo {
                        id: format!("x-{}", random_id()),
                        zip: false,
                        name,
                        bytes: 0,
                        unavailable: Some(what.to_string()),
                    });
                }
            }
        }

        let mut breadcrumbs = Vec::new();
        let root_name = self
            .root_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Warehouse".into());
        let root_bay = self.root_bay();
        breadcrumbs.push(BayInfo {
            id: root_bay,
            name: root_name,
            unavailable: None,
        });
        for i in 1..=bay_ref.path.len() {
            let prefix = bay_ref.path[..i].to_vec();
            if let Ok(fd) = sys::open_dir_beneath(&self.root, &prefix) {
                if let Ok(st) = sys::fstat(&fd) {
                    let name = prefix.last().cloned().unwrap_or_default();
                    let id = self.bay_id_for(prefix, sys::ident(&st));
                    breadcrumbs.push(BayInfo {
                        id,
                        name,
                        unavailable: None,
                    });
                }
            }
        }
        let this = breadcrumbs.last().cloned().expect("root crumb");
        let parent = (breadcrumbs.len() > 1).then(|| breadcrumbs[breadcrumbs.len() - 2].clone());
        let mut display = self.root_path.clone();
        for p in &bay_ref.path {
            display.push(p);
        }
        Ok(Floor {
            id: this.id,
            display_path: display.display().to_string(),
            rel_path: bay_ref.path.join("/"),
            breadcrumbs,
            parent,
            bays,
            cargo,
            truncated,
        })
    }

    // ---------- transactions ----------

    fn outcome(&self, tx: &str, op: Op, r: TxResult) -> TxOutcome {
        match r {
            Ok((from, to)) => TxOutcome {
                tx: tx.into(),
                op,
                status: TxStatus::Committed,
                code: None,
                message: None,
                from: Some(from),
                to: Some(to),
                suggestion: None,
                replayed: false,
            },
            Err((f, paths)) => TxOutcome {
                tx: tx.into(),
                op,
                status: if f.code == "UNCERTAIN" {
                    TxStatus::Uncertain
                } else {
                    TxStatus::Refused
                },
                code: Some(f.code.into()),
                message: Some(f.message),
                from: paths.as_ref().map(|p| p.0.clone()),
                to: paths.map(|p| p.1),
                suggestion: f.suggestion,
                replayed: false,
            },
        }
    }

    /// The recorded outcome of a transaction ID, if any (S7 replay).
    pub fn tx_status(&self, tx: &str) -> Option<TxOutcome> {
        let recs: Vec<&Record> = self.records.iter().filter(|r| r.tx == tx).collect();
        let first = recs.first()?;
        let op = recs.iter().find_map(|r| r.op).unwrap_or(Op::Move);
        let from = recs.iter().find_map(|r| r.src.clone());
        let to = recs.iter().find_map(|r| r.dst.clone());
        let last = recs.last()?;
        let (status, code, message) = match last.kind {
            Some(Kind::Committed) => (TxStatus::Committed, None, None),
            Some(Kind::Recovery) if last.decision.as_deref() == Some("RECOVERED_COMMITTED") => {
                (TxStatus::Committed, None, None)
            }
            Some(Kind::Failed) => (TxStatus::Refused, last.code.clone(), last.evidence.clone()),
            Some(Kind::Recovery) => (
                TxStatus::Refused,
                last.decision.clone(),
                last.evidence.clone(),
            ),
            _ => (
                TxStatus::Uncertain,
                Some("UNCERTAIN".into()),
                Some("This delivery is unsettled; recovery will decide.".into()),
            ),
        };
        let _ = first;
        Some(TxOutcome {
            tx: tx.into(),
            op,
            status,
            code,
            message,
            from,
            to,
            suggestion: None,
            replayed: true,
        })
    }

    fn precheck(&mut self, tx: &str) -> Result<(), Refusal> {
        if !valid_tx(tx) {
            return Err(refusal(
                "BAD_TX",
                "Transaction IDs are 1–64 letters, digits, `-` or `_`.",
            ));
        }
        if let Some(f) = &self.frozen {
            return Err(refusal("FROZEN", f.explanation.clone()));
        }
        // S4: never silently follow a new directory at the old pathname.
        match sys::lstat_path(&self.root_path) {
            Ok(st) if sys::ident(&st) == self.root_id => Ok(()),
            _ => {
                self.frozen = Some(Freeze {
                    kind: FreezeKind::RootReplaced,
                    explanation: format!(
                        "`{}` no longer names the warehouse PALLET opened. It was renamed, detached, or replaced. Jobs are suspended; reopen the warehouse to continue.",
                        self.root_path.display()
                    ),
                    acknowledgeable: false,
                });
                Err(refusal(
                    "ROOT_REPLACED",
                    "The warehouse folder was moved or replaced. Jobs are suspended.",
                ))
            }
        }
    }

    /// Records a refusal for an attempted transaction (S6). Best effort.
    fn journal_refusal(&mut self, tx: &str, op: Op, f: &Refusal, paths: &Option<(String, String)>) {
        if self.frozen.is_some() || !valid_tx(tx) {
            return;
        }
        let rec = Record {
            op: Some(op),
            code: Some(f.code.into()),
            evidence: Some(f.message.clone()),
            src: paths.as_ref().map(|p| p.0.clone()),
            dst: paths.as_ref().map(|p| p.1.clone()),
            ..self.record(tx, Kind::Failed)
        };
        let _ = self.append(rec);
    }

    fn free_name(
        &self,
        dir: &OwnedFd,
        base: &str,
        make: impl Fn(&str, u32) -> String,
    ) -> Option<String> {
        // numbered_name starts at "(2)"; copy_name treats 1 as plain "(copy)".
        (1..1000).map(|n| make(base, n)).find(
            |n| matches!(sys::lstat_at(dir, n), Err(e) if e.kind() == io::ErrorKind::NotFound),
        )
    }

    /// Validates the source entry at `src` against what was loaded.
    fn check_source(
        &self,
        dir: &OwnedFd,
        name: &str,
        expect: Ident,
        fp: Option<Fingerprint>,
    ) -> Result<(), Refusal> {
        let st = sys::lstat_at(dir, name).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => refusal(
                "SOURCE_MISSING",
                format!("`{name}` isn't where it was anymore."),
            ),
            _ => refusal("IO", e.to_string()),
        })?;
        if sys::file_type(&st) != FileType::RegularFile {
            return Err(refusal(
                "NOT_REGULAR",
                format!("`{name}` isn't a regular file anymore."),
            ));
        }
        if sys::ident(&st) != expect {
            return Err(refusal(
                "SOURCE_CHANGED",
                format!("`{name}` was replaced by a different file."),
            ));
        }
        if fp.is_some_and(|fp| fp != sys::fingerprint(&st)) {
            return Err(refusal(
                "SOURCE_CHANGED",
                format!("`{name}` changed while you were driving. Reload the floor."),
            ));
        }
        if st.st_nlink > 1 {
            return Err(refusal(
                "HARD_LINKED",
                format!("`{name}` has other hard links."),
            ));
        }
        if st.st_mode & 0o200 == 0 {
            return Err(refusal(
                "READ_ONLY",
                format!("`{name}` is read-only. PALLET doesn't change permissions."),
            ));
        }
        Ok(())
    }

    fn check_dest_dir(&self, dir: &OwnedFd) -> Result<(), Refusal> {
        let st = sys::fstat(dir).map_err(|e| refusal("IO", e.to_string()))?;
        if st.st_dev as u64 != self.root_id.dev {
            return Err(refusal(
                "CROSS_DEVICE",
                "That bay is on another device. Cross-drive moves aren't supported.",
            ));
        }
        if sys::is_casefold(dir).unwrap_or(true) {
            return Err(refusal(
                "UNSUPPORTED_FS",
                "Case-insensitive folders aren't supported.",
            ));
        }
        if !sys::dir_writable(dir) {
            return Err(refusal("PERMISSION", "PALLET can't write to that bay."));
        }
        Ok(())
    }

    fn check_free_name(&self, dir: &OwnedFd, name: &str, at_root: bool) -> Result<(), Refusal> {
        names::validate(name, at_root).map_err(|e| refusal("BAD_NAME", e.message()))?;
        match sys::lstat_at(dir, name) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(refusal("IO", e.to_string())),
            Ok(_) => Err(Refusal {
                code: "NAME_EXISTS",
                message: format!(
                    "Something called `{name}` is already there. PALLET never overwrites."
                ),
                suggestion: self.free_name(dir, name, |b, n| names::numbered_name(b, n + 1)),
            }),
        }
    }

    fn rename_errno(e: &io::Error, name: &str) -> Refusal {
        match e.raw_os_error() {
            Some(17) => refusal(
                "NAME_EXISTS",
                format!("`{name}` appeared at the destination just now. Nothing was replaced."),
            ),
            Some(18) => refusal(
                "CROSS_DEVICE",
                "Cross-device move refused. PALLET doesn't copy-then-delete.",
            ),
            Some(22) => refusal(
                "UNSUPPORTED_FS",
                "This filesystem doesn't support no-replace renames. Refused.",
            ),
            Some(2) => refusal(
                "SOURCE_MISSING",
                "The file or a folder disappeared before the move.",
            ),
            Some(13) | Some(1) => refusal(
                "PERMISSION",
                "Permission denied. PALLET doesn't change permissions.",
            ),
            Some(28) => refusal("DISK_FULL", "The disk is full."),
            Some(30) => refusal("READ_ONLY_FS", "The filesystem is read-only."),
            _ => refusal("IO", format!("The move failed: {e}")),
        }
    }

    /// The S3 commit protocol for one same-volume rename.
    #[allow(clippy::too_many_arguments)]
    fn rename_tx(
        &mut self,
        tx: &str,
        op: Op,
        src: Loc,
        dst: Loc,
        expect: Ident,
        fp: Option<Fingerprint>,
        batch: Option<&str>,
        undo_of: Option<&str>,
    ) -> TxResult {
        let paths = Some((src.rel(), dst.rel()));
        let fail = |f: Refusal| (f, paths.clone());

        // 1. Validate.
        let src_dir = self
            .open_checked(&src.dir, src.dir_id, "SOURCE_MISSING")
            .map_err(fail)?;
        let dst_dir = self
            .open_checked(&dst.dir, dst.dir_id, "DESTINATION_MISSING")
            .map_err(fail)?;
        self.check_source(&src_dir, &src.name, expect, fp)
            .map_err(fail)?;
        if !sys::dir_writable(&src_dir) {
            return Err(fail(refusal(
                "PERMISSION",
                "PALLET can't remove entries from the source folder.",
            )));
        }
        self.check_dest_dir(&dst_dir).map_err(fail)?;
        self.check_free_name(&dst_dir, &dst.name, dst.dir.is_empty())
            .map_err(fail)?;
        if sys::free_bytes(&self.root).unwrap_or(0) < JOURNAL_RESERVE {
            return Err(fail(refusal("DISK_FULL", "Less than 1 MiB free; PALLET won't start a delivery it might not be able to record.")));
        }

        // 2. PREPARED, synced, or nothing happens.
        let prepared = Record {
            op: Some(op),
            src: Some(src.rel()),
            dst: Some(dst.rel()),
            src_id: Some(expect),
            fp,
            src_dir_id: Some(src.dir_id),
            dst_dir_id: Some(dst.dir_id),
            root_id: Some(self.root_id),
            batch: batch.map(str::to_string),
            undo_of: undo_of.map(str::to_string),
            ..self.record(tx, Kind::Prepared)
        };
        if let Err(e) = self.append(prepared) {
            return Err(fail(refusal(
                "JOURNAL_FAILED",
                format!("Couldn't write the journal ({e}). Nothing moved."),
            )));
        }
        failpoint::hit("move_after_prepared");

        // Revalidate right before the rename.
        let revalidated = (|| {
            if !matches!(sys::lstat_path(&self.root_path), Ok(st) if sys::ident(&st) == self.root_id)
            {
                return Err(refusal(
                    "ROOT_REPLACED",
                    "The warehouse folder was moved or replaced.",
                ));
            }
            self.open_checked(&src.dir, src.dir_id, "SOURCE_MISSING")?;
            self.open_checked(&dst.dir, dst.dir_id, "DESTINATION_MISSING")?;
            self.check_source(&src_dir, &src.name, expect, fp)
        })();
        failpoint::hit("move_before_rename");

        // 3. Atomic no-replace rename.
        let renamed = revalidated.and_then(|_| {
            failpoint::error("rename").map_err(|e| Self::rename_errno(&e, &dst.name))?;
            sys::rename_noreplace(&src_dir, &src.name, &dst_dir, &dst.name)
                .map_err(|e| Self::rename_errno(&e, &dst.name))
        });
        if let Err(f) = renamed {
            if f.code == "ROOT_REPLACED" {
                let _ = self.precheck(tx);
            }
            let rec = Record {
                op: Some(op),
                code: Some(f.code.into()),
                evidence: Some(f.message.clone()),
                ..self.record(tx, Kind::Failed)
            };
            // If this append fails, recovery later finds the source in place: NOT_MOVED.
            let _ = self.append(rec);
            return Err(fail(f));
        }
        failpoint::hit("move_after_rename");

        // The rename happened. Confirm the object is where we put it, still beneath the root.
        let escaped = match sys::open_dir_beneath(&self.root, &dst.dir) {
            Ok(fd) => {
                !matches!(sys::fstat(&fd), Ok(st) if sys::ident(&st) == dst.dir_id)
                    || !matches!(sys::lstat_at(&fd, &dst.name), Ok(st) if sys::ident(&st) == expect)
            }
            Err(_) => true,
        };
        if escaped {
            self.frozen = Some(Freeze {
                kind: FreezeKind::BoundaryEscape,
                explanation: format!(
                    "`{}` was moved, but its destination folder no longer resolves inside the warehouse. Another program moved that folder during the delivery. PALLET stopped and changed nothing else.",
                    dst.rel()
                ),
                acknowledgeable: true,
            });
            self.recovery = vec![RecoveryDecision {
                tx: tx.into(),
                op,
                decision: "FROZEN".into(),
                evidence: "destination folder left the warehouse during commit".into(),
            }];
            return Err(fail(refusal("UNCERTAIN", "The destination folder moved during delivery. PALLET froze; see the recovery panel.")));
        }

        // 4. Flush both directories, then COMMITTED.
        let flushed = failpoint::error("dir_fsync")
            .and_then(|_| sys::fsync(&dst_dir))
            .and_then(|_| sys::fsync(&src_dir));
        failpoint::hit("move_after_dir_sync");
        if let Err(e) = flushed {
            self.frozen = Some(Freeze {
                kind: FreezeKind::FlushFailed,
                explanation: format!("The file was renamed to `{}`, but flushing the folders failed ({e}). The kernel reports the move happened; durability is unknown.", dst.rel()),
                acknowledgeable: true,
            });
            self.recovery = vec![RecoveryDecision {
                tx: tx.into(),
                op,
                decision: "FROZEN".into(),
                evidence: format!("directory fsync failed: {e}"),
            }];
            return Err(fail(refusal(
                "UNCERTAIN",
                "The move happened but couldn't be flushed. PALLET froze.",
            )));
        }
        let committed = Record {
            op: Some(op),
            dst_id: Some(expect),
            ..self.record(tx, Kind::Committed)
        };
        if let Err(e) = self.append(committed) {
            return Err(fail(refusal("UNCERTAIN", format!("The file moved, but the journal couldn't record it ({e}). PALLET will settle this from file identities next time it opens."))));
        }
        failpoint::hit("move_after_committed");

        if let Some(key) = self.cargo_by_ident.get(&expect).cloned() {
            if let Some(c) = self.cargo.get_mut(&key) {
                c.dir = dst.dir.clone();
                c.dir_id = dst.dir_id;
                c.name = dst.name.clone();
            }
        }
        Ok((src.rel(), dst.rel()))
    }

    fn replay_or(
        &mut self,
        tx: &str,
        op: Op,
        run: impl FnOnce(&mut Self) -> TxResult,
    ) -> TxOutcome {
        if let Some(done) = self.tx_status(tx) {
            return done;
        }
        if let Err(f) = self.precheck(tx) {
            return self.outcome(tx, op, Err((f, None)));
        }
        let r = run(self);
        if let Err((f, paths)) = &r {
            let journaled = self.records.iter().any(|x| x.tx == tx);
            if !journaled && f.code != "UNCERTAIN" {
                self.journal_refusal(tx, op, f, paths);
            }
        }
        self.outcome(tx, op, r)
    }

    /// Moves one crate into a bay (S3). `new_name` is the user-confirmed
    /// alternate after a collision.
    pub fn deliver(
        &mut self,
        tx: &str,
        cargo: &str,
        bay: &str,
        new_name: Option<&str>,
        batch: Option<&str>,
    ) -> TxOutcome {
        let cargo = self.cargo.get(cargo).cloned();
        let bay = self.bays.get(bay).cloned();
        self.replay_or(tx, Op::Move, |w| {
            let c = cargo.ok_or((
                refusal(
                    "UNKNOWN_CARGO",
                    "That crate isn't on file. Reload the floor.",
                ),
                None,
            ))?;
            let b = bay.ok_or((
                refusal("UNKNOWN_BAY", "That bay isn't on file. Reload the floor."),
                None,
            ))?;
            let name = new_name.unwrap_or(&c.name).to_string();
            if b.id == c.dir_id && name == c.name {
                return Err((
                    refusal("SAME_PLACE", "That crate is already in this bay."),
                    None,
                ));
            }
            let src = Loc {
                dir: c.dir.clone(),
                dir_id: c.dir_id,
                name: c.name.clone(),
            };
            let dst = Loc {
                dir: b.path.clone(),
                dir_id: b.id,
                name,
            };
            w.rename_tx(tx, Op::Move, src, dst, c.id, Some(c.fp), batch, None)
        })
    }

    /// Delivers a stacked pallet: each crate is its own transaction, in order,
    /// stopping at the first one that doesn't commit. Not atomic, by design;
    /// the returned list says exactly which crates moved.
    pub fn deliver_batch(
        &mut self,
        batch: &str,
        items: &[(String, String)],
        bay: &str,
    ) -> Vec<TxOutcome> {
        let mut out = Vec::new();
        for (tx, cargo) in items.iter().take(6) {
            let o = self.deliver(tx, cargo, bay, None, Some(batch));
            let stop = o.status != TxStatus::Committed;
            out.push(o);
            if stop {
                break;
            }
        }
        out
    }

    /// Committed transactions, newest first.
    pub fn history(&self) -> Vec<HistoryEntry> {
        let mut out = Vec::new();
        for r in &self.records {
            let settled = matches!(r.kind, Some(Kind::Committed))
                || (r.kind == Some(Kind::Recovery)
                    && r.decision.as_deref() == Some("RECOVERED_COMMITTED"));
            if !settled {
                continue;
            }
            let Some(p) = self
                .records
                .iter()
                .find(|x| x.tx == r.tx && x.kind == Some(Kind::Prepared))
            else {
                continue;
            };
            let op = p.op.unwrap_or(Op::Move);
            let undone = self.records.iter().any(|u| {
                u.undo_of.as_deref() == Some(&r.tx)
                    && self
                        .tx_status(&u.tx)
                        .is_some_and(|s| s.status == TxStatus::Committed)
            });
            out.push(HistoryEntry {
                tx: r.tx.clone(),
                op,
                from: p.src.clone().unwrap_or_default(),
                to: p.dst.clone().unwrap_or_default(),
                at_ms: r.ts,
                undone,
                batch: p.batch.clone(),
            });
        }
        out.reverse();
        out
    }

    /// Undo Move or Undo Copy (S6). A new, journaled transaction.
    pub fn undo(&mut self, tx: &str, target: &str, new_name: Option<&str>) -> TxOutcome {
        let entry = self.history().into_iter().find(|h| h.tx == target);
        let prepared = self
            .records
            .iter()
            .find(|x| x.tx == target && x.kind == Some(Kind::Prepared))
            .cloned();
        let ident = self
            .records
            .iter()
            .rev()
            .find(|x| x.tx == target && matches!(x.kind, Some(Kind::Committed | Kind::Recovery)))
            .and_then(|x| x.dst_id);
        self.replay_or(tx, Op::Undo, |w| {
            let (Some(entry), Some(p), Some(ident)) = (entry, prepared, ident) else {
                return Err((
                    refusal(
                        "NOTHING_TO_UNDO",
                        "That delivery isn't in the journal as completed.",
                    ),
                    None,
                ));
            };
            if entry.undone {
                return Err((
                    refusal("ALREADY_UNDONE", "That delivery was already undone."),
                    None,
                ));
            }
            let (to_dir, to_name) =
                split_rel(&entry.to).ok_or((refusal("IO", "Bad journal path."), None))?;
            let (from_dir, from_name) =
                split_rel(&entry.from).ok_or((refusal("IO", "Bad journal path."), None))?;
            let current = Loc {
                dir: to_dir,
                dir_id: p.dst_dir_id.unwrap_or(w.root_id),
                name: to_name.clone(),
            };
            match entry.op {
                Op::Move => {
                    let back = Loc {
                        dir: from_dir,
                        dir_id: p.src_dir_id.unwrap_or(w.root_id),
                        name: new_name.unwrap_or(&from_name).to_string(),
                    };
                    w.rename_tx(tx, Op::Undo, current, back, ident, None, None, Some(target))
                }
                Op::Copy => {
                    // Undo Copy never deletes: the copy goes to a visible bay.
                    sys::mkdir_at(&w.root, RECOVERED_COPIES, 0o755)
                        .map_err(|e| (refusal("IO", e.to_string()), None))?;
                    let bay = sys::open_dir_beneath(&w.root, &[RECOVERED_COPIES.into()]).map_err(
                        |e| {
                            (
                                refusal(
                                    "BAY_UNSAFE",
                                    format!("`{RECOVERED_COPIES}` must be a plain folder: {e}"),
                                ),
                                None,
                            )
                        },
                    )?;
                    let _ = sys::fsync(&w.root);
                    let bay_id = sys::ident(
                        &sys::fstat(&bay).map_err(|e| (refusal("IO", e.to_string()), None))?,
                    );
                    let dest = Loc {
                        dir: vec![RECOVERED_COPIES.into()],
                        dir_id: bay_id,
                        name: new_name.unwrap_or(&to_name).to_string(),
                    };
                    w.rename_tx(tx, Op::Undo, current, dest, ident, None, None, Some(target))
                }
                _ => Err((
                    refusal("NOT_UNDOABLE", "Only moves and copies can be undone."),
                    None,
                )),
            }
        })
    }

    /// Writes a temporary file in `.pallet/tmp`, verifies it, then publishes it
    /// with a no-replace rename (S6: Copy; also used for incident reports).
    #[allow(clippy::too_many_arguments)]
    fn stage_and_publish(
        &mut self,
        tx: &str,
        op: Op,
        dst: Loc,
        mut prepared: Record,
        fill: &mut dyn FnMut(&mut File) -> Result<String, Refusal>,
    ) -> TxResult {
        let tmp_name = format!("{tx}.part");
        let tmp_rel = format!("{META_DIR}/tmp/{tmp_name}");
        let paths = Some((prepared.src.clone().unwrap_or_default(), dst.rel()));
        let fail = |f: Refusal| (f, paths.clone());

        let dst_dir = self
            .open_checked(&dst.dir, dst.dir_id, "DESTINATION_MISSING")
            .map_err(fail)?;
        self.check_dest_dir(&dst_dir).map_err(fail)?;
        self.check_free_name(&dst_dir, &dst.name, dst.dir.is_empty())
            .map_err(fail)?;

        prepared.tmp = Some(tmp_rel.clone());
        prepared.dst = Some(dst.rel());
        prepared.dst_dir_id = Some(dst.dir_id);
        prepared.root_id = Some(self.root_id);
        if let Err(e) = self.append(prepared) {
            return Err(fail(refusal(
                "JOURNAL_FAILED",
                format!("Couldn't write the journal ({e}). Nothing was created."),
            )));
        }

        let failed = |w: &mut Self, f: Refusal| {
            let rec = Record {
                op: Some(op),
                code: Some(f.code.into()),
                evidence: Some(f.message.clone()),
                ..w.record(tx, Kind::Failed)
            };
            let _ = w.append(rec);
            Err((f, paths.clone()))
        };

        let mut file = match sys::create_excl(&self.tmp, &tmp_name, 0o600) {
            Ok(f) => f,
            Err(e) => {
                return failed(
                    self,
                    refusal("IO", format!("Can't create a temporary file: {e}")),
                )
            }
        };
        let hash = match fill(&mut file) {
            Ok(h) => h,
            Err(f) => return failed(self, f),
        };
        if let Err(e) = file.sync_all().and_then(|_| sys::fsync(&self.tmp)) {
            return failed(
                self,
                refusal("IO", format!("Couldn't flush the output: {e}")),
            );
        }
        let tmp_id = match file.metadata() {
            Ok(m) => {
                use std::os::unix::fs::MetadataExt;
                Ident {
                    dev: m.dev(),
                    ino: m.ino(),
                }
            }
            Err(e) => return failed(self, refusal("IO", e.to_string())),
        };
        let staged = Record {
            op: Some(op),
            tmp_id: Some(tmp_id),
            hash: Some(hash),
            ..self.record(tx, Kind::Staged)
        };
        if let Err(e) = self.append(staged) {
            return failed(
                self,
                refusal(
                    "JOURNAL_FAILED",
                    format!("Couldn't record the verified output ({e}). Nothing was published."),
                ),
            );
        }
        failpoint::hit("publish_before_rename");

        if let Err(e) = sys::rename_noreplace(&self.tmp, &tmp_name, &dst_dir, &dst.name) {
            return failed(self, Self::rename_errno(&e, &dst.name));
        }
        failpoint::hit("publish_after_rename");
        if let Err(e) = sys::fsync(&dst_dir).and_then(|_| sys::fsync(&self.tmp)) {
            self.frozen = Some(Freeze {
                kind: FreezeKind::FlushFailed,
                explanation: format!("`{}` was published but flushing failed ({e}).", dst.rel()),
                acknowledgeable: true,
            });
            return Err(fail(refusal(
                "UNCERTAIN",
                "Published but not flushed. PALLET froze.",
            )));
        }
        let committed = Record {
            op: Some(op),
            dst_id: Some(tmp_id),
            ..self.record(tx, Kind::Committed)
        };
        if let Err(e) = self.append(committed) {
            return Err(fail(refusal(
                "UNCERTAIN",
                format!(
                    "Published, but the journal couldn't record it ({e}). Recovery will settle it."
                ),
            )));
        }
        Ok((paths.as_ref().unwrap().0.clone(), dst.rel()))
    }

    /// The duplicator (S6 Copy). The original is never modified.
    pub fn duplicate(&mut self, tx: &str, cargo: &str, new_name: Option<&str>) -> TxOutcome {
        let cargo = self.cargo.get(cargo).cloned();
        self.replay_or(tx, Op::Copy, |w| {
            let c = cargo.ok_or((
                refusal(
                    "UNKNOWN_CARGO",
                    "That crate isn't on file. Reload the floor.",
                ),
                None,
            ))?;
            let src_dir = w
                .open_checked(&c.dir, c.dir_id, "SOURCE_MISSING")
                .map_err(|f| (f, None))?;
            w.check_source(&src_dir, &c.name, c.id, Some(c.fp))
                .map_err(|f| (f, None))?;
            if c.fp.size > COPY_LIMIT {
                return Err((
                    refusal("TOO_HEAVY", "The duplicator only takes crates up to 1 GiB."),
                    None,
                ));
            }
            let free = sys::free_bytes(&w.root).unwrap_or(0);
            if free < c.fp.size.saturating_add(4 * JOURNAL_RESERVE) {
                return Err((
                    refusal("DISK_FULL", "Not enough free space for a verified copy."),
                    None,
                ));
            }
            let name = match new_name {
                Some(n) => n.to_string(),
                None => w
                    .free_name(&src_dir, &c.name, names::copy_name)
                    .ok_or((refusal("NAME_EXISTS", "Too many copies already."), None))?,
            };
            let mut source = sys::open_file_ro(&src_dir, &c.name)
                .map_err(|e| (refusal("IO", e.to_string()), None))?;
            let st = source
                .metadata()
                .map_err(|e| (refusal("IO", e.to_string()), None))?;
            let mode = {
                use std::os::unix::fs::MetadataExt;
                if (Ident {
                    dev: st.dev(),
                    ino: st.ino(),
                }) != c.id
                {
                    return Err((refusal("SOURCE_CHANGED", "The file was replaced."), None));
                }
                st.mode() & 0o777
            };
            let dst = Loc {
                dir: c.dir.clone(),
                dir_id: c.dir_id,
                name,
            };
            let prepared = Record {
                op: Some(Op::Copy),
                src: Some(join_rel(&c.dir, &c.name)),
                src_id: Some(c.id),
                src_dir_id: Some(c.dir_id),
                fp: Some(c.fp),
                ..w.record(tx, Kind::Prepared)
            };
            let fp = c.fp;
            let mut fill = |out: &mut File| -> Result<String, Refusal> {
                let io_err = |e: io::Error| match e.raw_os_error() {
                    Some(28) => refusal(
                        "DISK_FULL",
                        "The disk filled up during the copy. The original is untouched.",
                    ),
                    _ => refusal(
                        "IO",
                        format!("Copy failed: {e}. The original is untouched."),
                    ),
                };
                let mut h1 = blake3::Hasher::new();
                let mut buf = vec![0u8; 1 << 20];
                source.seek(SeekFrom::Start(0)).map_err(io_err)?;
                loop {
                    let n = source.read(&mut buf).map_err(io_err)?;
                    if n == 0 {
                        break;
                    }
                    h1.update(&buf[..n]);
                    failpoint::error("copy_write").map_err(io_err)?;
                    out.write_all(&buf[..n]).map_err(io_err)?;
                }
                failpoint::hit("copy_after_stream");
                out.sync_all().map_err(io_err)?;
                // Stable snapshot check: same identity, size, and mtime; same bytes on a second read.
                let after = source.metadata().map_err(io_err)?;
                {
                    use std::os::unix::fs::MetadataExt;
                    let now = Fingerprint {
                        size: after.size(),
                        mtime_ns: after.mtime() as i128 * 1_000_000_000
                            + after.mtime_nsec() as i128,
                    };
                    if now != fp {
                        return Err(refusal(
                            "SOURCE_CHANGED",
                            "The file changed while it was being copied. No copy was published.",
                        ));
                    }
                }
                let reread = |f: &File, len: u64| -> io::Result<blake3::Hash> {
                    let mut h = blake3::Hasher::new();
                    let mut buf = vec![0u8; 1 << 20];
                    let mut off = 0u64;
                    while off < len {
                        let n = f.read_at(&mut buf, off)?;
                        if n == 0 {
                            break;
                        }
                        h.update(&buf[..n]);
                        off += n as u64;
                    }
                    Ok(h.finalize())
                };
                let h1 = h1.finalize();
                let h2 = reread(&source, fp.size).map_err(io_err)?;
                let h3 = reread(out, u64::MAX).map_err(io_err)?;
                if h1 != h2 {
                    return Err(refusal(
                        "SOURCE_CHANGED",
                        "The file's contents changed during the copy. No copy was published.",
                    ));
                }
                if h1 != h3 {
                    return Err(refusal(
                        "VERIFY_FAILED",
                        "The copy didn't read back identically. It was not published.",
                    ));
                }
                sys::fchmod(out, mode).map_err(io_err)?;
                Ok(h1.to_hex().to_string())
            };
            w.stage_and_publish(tx, Op::Copy, dst, prepared, &mut fill)
        })
    }

    /// Saves a fictional incident report as a new file at the warehouse root.
    pub fn save_incident(&mut self, tx: &str, text: &str) -> TxOutcome {
        self.replay_or(tx, Op::Incident, |w| {
            if text.len() > INCIDENT_LIMIT {
                return Err((refusal("TOO_LONG", "Incident reports are capped at 16 KiB."), None));
            }
            let (y, m, d, hh, mm, ss) = civil(now_ms() / 1000);
            let base = format!("incident-report-{y:04}-{m:02}-{d:02}-{hh:02}{mm:02}{ss:02}.txt");
            let root = sys::open_dir_beneath(&w.root, &[]).map_err(|e| (refusal("IO", e.to_string()), None))?;
            let name = if matches!(sys::lstat_at(&root, &base), Err(ref e) if e.kind() == io::ErrorKind::NotFound) {
                base
            } else {
                w.free_name(&root, &base, |b, n| names::numbered_name(b, n + 1)).ok_or((refusal("NAME_EXISTS", "Too many reports."), None))?
            };
            let dst = Loc { dir: Vec::new(), dir_id: w.root_id, name };
            let prepared = Record { op: Some(Op::Incident), ..w.record(tx, Kind::Prepared) };
            let body = text.as_bytes().to_vec();
            let mut fill = |out: &mut File| -> Result<String, Refusal> {
                out.write_all(&body).map_err(|e| refusal("IO", e.to_string()))?;
                Ok(blake3::hash(&body).to_hex().to_string())
            };
            w.stage_and_publish(tx, Op::Incident, dst, prepared, &mut fill)
        })
    }

    #[doc(hidden)]
    pub fn meta_fd(&self) -> &OwnedFd {
        &self.meta
    }
}
