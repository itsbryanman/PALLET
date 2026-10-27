// Native backend: thin wrappers over the Rust commands in src-tauri. The Rust
// side owns every path; this file only passes opaque IDs and names.

import { invoke } from "@tauri-apps/api/core";
import type { Backend, Floor, HistoryEntry, TxOutcome, WarehouseStatus } from "./types";

export interface BootInfo {
  mode: "real" | "virtual";
  note: string | null;
  status: WarehouseStatus | null;
}

export class TauriBackend implements Backend {
  readonly mode = "real" as const;
  readonly note: string | null;

  constructor(note: string | null = null) {
    this.note = note;
  }

  /**
   * Runs a mutation. If the acknowledgment is lost (IPC error), ask what the
   * journal says about this transaction ID. Never send the mutation again (S7).
   */
  private async mutate(tx: string, cmd: string, args: Record<string, unknown>): Promise<TxOutcome> {
    try {
      return await invoke<TxOutcome>(cmd, args);
    } catch (err) {
      const known = await this.txStatus(tx).catch(() => null);
      return known ?? {
        tx,
        op: "move",
        status: "uncertain",
        code: "NO_ACK",
        message: `No answer from the warehouse service (${String(err)}). Its journal will settle this delivery; nothing is retried.`,
        from: null,
        to: null,
        suggestion: null,
        replayed: true,
      };
    }
  }

  status(): Promise<WarehouseStatus> {
    return invoke("status");
  }
  listFloor(bay: string | null): Promise<Floor> {
    return invoke("list_floor", { bay });
  }
  deliver(tx: string, cargo: string, bay: string, newName: string | null): Promise<TxOutcome> {
    return this.mutate(tx, "deliver", { tx, cargo, bay, newName });
  }
  async deliverBatch(batch: string, items: [string, string][], bay: string): Promise<TxOutcome[]> {
    try {
      return await invoke<TxOutcome[]>("deliver_batch", { batch, items, bay });
    } catch {
      // Lost acknowledgment: report what the journal knows, item by item.
      const out: TxOutcome[] = [];
      for (const [tx] of items) {
        const s = await this.txStatus(tx).catch(() => null);
        if (!s) break;
        out.push(s);
        if (s.status !== "committed") break;
      }
      return out;
    }
  }
  undo(tx: string, target: string, newName: string | null): Promise<TxOutcome> {
    return this.mutate(tx, "undo", { tx, target, newName });
  }
  duplicate(tx: string, cargo: string, newName: string | null): Promise<TxOutcome> {
    return this.mutate(tx, "duplicate", { tx, cargo, newName });
  }
  saveIncident(tx: string, text: string): Promise<TxOutcome> {
    return this.mutate(tx, "save_incident", { tx, text });
  }
  txStatus(tx: string): Promise<TxOutcome | null> {
    return invoke("tx_status", { tx });
  }
  history(): Promise<HistoryEntry[]> {
    return invoke("history");
  }
  acknowledge(): Promise<WarehouseStatus> {
    return invoke("acknowledge");
  }
  chooseWarehouse(): Promise<WarehouseStatus | null> {
    return invoke("choose_warehouse");
  }
  createWarehouse(): Promise<WarehouseStatus> {
    return invoke("create_warehouse");
  }
  openSample(): Promise<WarehouseStatus> {
    return invoke("open_sample");
  }
}

export async function boot(): Promise<BootInfo> {
  return invoke<BootInfo>("boot");
}
