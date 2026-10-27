// Mirrors crates/pallet-core/src/model.rs. The renderer only ever holds opaque
// IDs and display strings; it never sends a path to the backend (SAFETY.md S7).

export interface CargoInfo {
  id: string;
  name: string;
  bytes: number;
  zip: boolean;
  unavailable: string | null;
}

export interface BayInfo {
  id: string;
  name: string;
  unavailable: string | null;
  /** Regular files directly inside (display only). */
  items: number;
}

export interface Floor {
  id: string;
  displayPath: string;
  relPath: string;
  breadcrumbs: BayInfo[];
  parent: BayInfo | null;
  bays: BayInfo[];
  cargo: CargoInfo[];
  truncated: boolean;
}

export type TxStatus = "committed" | "refused" | "uncertain";
export type Op = "move" | "undo" | "copy" | "incident";

export interface TxOutcome {
  tx: string;
  op: Op;
  status: TxStatus;
  code: string | null;
  message: string | null;
  from: string | null;
  to: string | null;
  suggestion: string | null;
  replayed: boolean;
}

export interface HistoryEntry {
  tx: string;
  op: Op;
  from: string;
  to: string;
  atMs: number;
  undone: boolean;
  batch: string | null;
}

export interface RecoveryDecision {
  tx: string;
  op: Op;
  decision: string;
  evidence: string;
}

export interface Freeze {
  kind: string;
  explanation: string;
  acknowledgeable: boolean;
}

export interface WarehouseStatus {
  rootPath: string;
  sample: boolean;
  frozen: Freeze | null;
  recovery: RecoveryDecision[];
}

/** What kind of files this backend touches. Shown on screen at all times. */
export type Mode = "real" | "virtual";

export interface Backend {
  readonly mode: Mode;
  /** Why real-file mode is off, when it is. */
  readonly note: string | null;
  status(): Promise<WarehouseStatus>;
  listFloor(bay: string | null): Promise<Floor>;
  deliver(tx: string, cargo: string, bay: string, newName: string | null): Promise<TxOutcome>;
  deliverBatch(batch: string, items: [string, string][], bay: string): Promise<TxOutcome[]>;
  undo(tx: string, target: string, newName: string | null): Promise<TxOutcome>;
  duplicate(tx: string, cargo: string, newName: string | null): Promise<TxOutcome>;
  saveIncident(tx: string, text: string): Promise<TxOutcome>;
  txStatus(tx: string): Promise<TxOutcome | null>;
  history(): Promise<HistoryEntry[]>;
  acknowledge(): Promise<WarehouseStatus>;
  /** Native only: ask the Rust side to show a folder picker. */
  chooseWarehouse?(): Promise<WarehouseStatus | null>;
  createWarehouse?(): Promise<WarehouseStatus>;
  openSample?(): Promise<WarehouseStatus>;
}

export function newTxId(): string {
  const b = new Uint8Array(12);
  crypto.getRandomValues(b);
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
}
