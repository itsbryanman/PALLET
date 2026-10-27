//! Types that cross the IPC boundary. The renderer only ever sees opaque IDs,
//! display names, and display paths (S7). It never sends a path back.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CargoInfo {
    pub id: String,
    pub name: String,
    pub bytes: u64,
    /// Cosmetic only. Decided from the extension; archive contents are never read.
    pub zip: bool,
    /// `None` when the file can be moved. Otherwise a short reason for the marker.
    pub unavailable: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BayInfo {
    pub id: String,
    pub name: String,
    pub unavailable: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Floor {
    /// The bay ID of this directory. Deliveries "into this floor" use it.
    pub id: String,
    /// Absolute path for display only.
    pub display_path: String,
    /// Path relative to the warehouse root, `/`-separated, empty at the root.
    pub rel_path: String,
    pub breadcrumbs: Vec<BayInfo>,
    pub parent: Option<BayInfo>,
    pub bays: Vec<BayInfo>,
    pub cargo: Vec<CargoInfo>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TxStatus {
    Committed,
    Refused,
    /// The rename may have happened but could not be recorded. Recovery decides.
    Uncertain,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Op {
    Move,
    Undo,
    Copy,
    Incident,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TxOutcome {
    pub tx: String,
    pub op: Op,
    pub status: TxStatus,
    pub code: Option<String>,
    pub message: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// A free alternate name when the refusal was a collision.
    pub suggestion: Option<String>,
    /// True when this outcome was read from the journal instead of performed now.
    pub replayed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub tx: String,
    pub op: Op,
    pub from: String,
    pub to: String,
    pub at_ms: u64,
    pub undone: bool,
    pub batch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryDecision {
    pub tx: String,
    pub op: Op,
    pub decision: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WarehouseStatus {
    pub root_path: String,
    pub sample: bool,
    pub frozen: Option<Freeze>,
    pub recovery: Vec<RecoveryDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Freeze {
    pub kind: FreezeKind,
    pub explanation: String,
    /// Whether the user may acknowledge this in the app and continue.
    pub acknowledgeable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FreezeKind {
    TornJournalTail,
    CorruptJournal,
    AmbiguousTransaction,
    RootReplaced,
    BoundaryEscape,
    FlushFailed,
}
