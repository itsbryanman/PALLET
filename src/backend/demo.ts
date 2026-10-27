// The demo backend: an in-memory pretend filesystem that follows the same
// rules as the Rust core (no overwrite, no delete, refusal codes, replay,
// conditional undo). It cannot read or write anything on the visitor's computer:
// it has no file APIs at all. State lives only in this page and resets on reload.

import type { Backend, BayInfo, CargoInfo, Floor, HistoryEntry, TxOutcome, WarehouseStatus } from "./types";

interface FileNode {
  kind: "file";
  ino: number;
  name: string;
  bytes: number;
  special?: string;
}
interface DirNode {
  kind: "dir";
  ino: number;
  name: string;
  children: Map<string, Node>;
}
type Node = FileNode | DirNode;

interface Rec {
  tx: string;
  op: TxOutcome["op"];
  status: TxOutcome["status"];
  code: string | null;
  message: string | null;
  from: string | null;
  to: string | null;
  ino: number | null;
  undoOf: string | null;
  batch: string | null;
  at: number;
}

export const DEMO_ROOT = "demo://PALLET-Warehouse";

const ZIP_BYTES = 22;

export class DemoBackend implements Backend {
  readonly mode = "virtual" as const;
  readonly note: string | null;
  private nextIno = 2;
  private root: DirNode = { kind: "dir", ino: 1, name: "", children: new Map() };
  private journal: Rec[] = [];
  private ids = new Map<string, number>(); // opaque id -> ino
  private idOf = new Map<number, string>();
  private counter = 0;
  /** Test hook: fail the n-th delivery with this code. */
  failNext: { code: string; message: string } | null = null;

  constructor(note: string | null = null) {
    this.note = note;
    for (const d of ["Documents", "Photos", "Archive"]) this.mkdir(this.root, d);
    this.put(this.root, "hello.txt", 37);
    this.put(this.root, "inventory.csv", 61);
    this.put(this.root, "forklift-manual.md", 263);
    this.put(this.root, "empty-envelope.txt", 0);
    this.put(this.root, "old-backups.zip", ZIP_BYTES);
    this.put(this.root, "heavy-machinery.bin", 3 << 20);
    this.put(this.dir(["Documents"])!, "notes.txt", 21);
    this.put(this.dir(["Archive"])!, "2019-taxes.zip", ZIP_BYTES);
  }

  private mkdir(parent: DirNode, name: string): DirNode {
    const d: DirNode = { kind: "dir", ino: this.nextIno++, name, children: new Map() };
    parent.children.set(name, d);
    return d;
  }

  private put(parent: DirNode, name: string, bytes: number): FileNode {
    const f: FileNode = { kind: "file", ino: this.nextIno++, name, bytes };
    parent.children.set(name, f);
    return f;
  }

  /** Adds an unavailable entry, for tests and the demo's "link" marker. */
  addSpecial(path: string[], name: string, what: string): void {
    const d = this.dir(path)!;
    d.children.set(name, { kind: "file", ino: this.nextIno++, name, bytes: 0, special: what });
  }

  private dir(path: string[]): DirNode | null {
    let d: Node = this.root;
    for (const p of path) {
      if (d.kind !== "dir") return null;
      const n: Node | undefined = d.children.get(p);
      if (!n || n.kind !== "dir") return null;
      d = n;
    }
    return d.kind === "dir" ? d : null;
  }

  /** Finds a node by identity. Returns its directory path and name. */
  private locate(ino: number, d: DirNode = this.root, path: string[] = []): { path: string[]; node: Node; parent: DirNode } | null {
    for (const n of d.children.values()) {
      if (n.ino === ino) return { path, node: n, parent: d };
      if (n.kind === "dir") {
        const r = this.locate(ino, n, [...path, n.name]);
        if (r) return r;
      }
    }
    return null;
  }

  private id(ino: number): string {
    let id = this.idOf.get(ino);
    if (!id) {
      id = `d${(++this.counter).toString(36)}${Math.random().toString(36).slice(2, 8)}`;
      this.idOf.set(ino, id);
      this.ids.set(id, ino);
    }
    return id;
  }

  private rel(path: string[], name: string): string {
    return [...path, name].join("/");
  }

  async status(): Promise<WarehouseStatus> {
    return { rootPath: DEMO_ROOT, sample: true, frozen: null, recovery: [] };
  }

  async listFloor(bay: string | null): Promise<Floor> {
    const ino = bay === null ? 1 : this.ids.get(bay);
    if (ino === undefined) throw new Error("Unknown bay.");
    const where = ino === 1 ? { path: [] as string[], node: this.root as Node } : this.locate(ino);
    if (!where || where.node.kind !== "dir") throw new Error("That bay is gone.");
    const dirPath = ino === 1 ? [] : [...where.path, where.node.name];
    const d = where.node;
    const entries = [...d.children.values()].sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()) || (a.name < b.name ? -1 : 1));
    const bays: BayInfo[] = [];
    const cargo: CargoInfo[] = [];
    for (const n of entries) {
      if (n.kind === "dir") bays.push({ id: this.id(n.ino), name: n.name, unavailable: null, items: [...n.children.values()].filter((c) => c.kind === "file").length });
      else
        cargo.push({
          id: n.special ? `x-${n.ino}` : this.id(n.ino),
          name: n.name,
          bytes: n.bytes,
          zip: /.\.zip$/i.test(n.name),
          unavailable: n.special ?? null,
        });
    }
    const breadcrumbs: BayInfo[] = [{ id: this.id(1), name: "PALLET-Warehouse", unavailable: null, items: 0 }];
    for (let i = 1; i <= dirPath.length; i++) {
      const sub = this.dir(dirPath.slice(0, i))!;
      breadcrumbs.push({ id: this.id(sub.ino), name: sub.name, unavailable: null, items: 0 });
    }
    return {
      id: breadcrumbs[breadcrumbs.length - 1].id,
      displayPath: [DEMO_ROOT, ...dirPath].join("/"),
      relPath: dirPath.join("/"),
      breadcrumbs,
      parent: breadcrumbs.length > 1 ? breadcrumbs[breadcrumbs.length - 2] : null,
      bays,
      cargo,
      truncated: false,
    };
  }

  private record(r: Omit<Rec, "at">): TxOutcome {
    this.journal.push({ ...r, at: Date.now() });
    return { tx: r.tx, op: r.op, status: r.status, code: r.code, message: r.message, from: r.from, to: r.to, suggestion: null, replayed: false };
  }

  private refuse(tx: string, op: TxOutcome["op"], code: string, message: string, suggestion: string | null = null, from: string | null = null, to: string | null = null): TxOutcome {
    const o = this.record({ tx, op, status: "refused", code, message, from, to, ino: null, undoOf: null, batch: null });
    return { ...o, suggestion };
  }

  private validName(name: string, atRoot: boolean): string | null {
    if (!name || name === "." || name === "..") return "Names `.` and `..` (or empty) are not allowed.";
    if (name.includes("/") || name.includes("\0")) return "Names can't contain `/`.";
    if (atRoot && name === ".pallet") return "`.pallet` is reserved for PALLET's own records.";
    if (new TextEncoder().encode(name).length > 255) return "Name is longer than 255 bytes.";
    return null;
  }

  private numbered(name: string, n: number): string {
    const i = name.lastIndexOf(".");
    return i > 0 ? `${name.slice(0, i)} (${n})${name.slice(i)}` : `${name} (${n})`;
  }

  private copyName(name: string, n: number): string {
    const i = name.lastIndexOf(".");
    const tag = n <= 1 ? "(copy)" : `(copy ${n})`;
    return i > 0 ? `${name.slice(0, i)} ${tag}${name.slice(i)}` : `${name} ${tag}`;
  }

  private free(d: DirNode, name: string, make: (n: string, i: number) => string, start: number): string {
    for (let i = start; i < 1000; i++) {
      const c = make(name, i);
      if (!d.children.has(c)) return c;
    }
    return make(name, 1000);
  }

  private replay(tx: string): TxOutcome | null {
    const r = [...this.journal].reverse().find((x) => x.tx === tx);
    if (!r) return null;
    return { tx, op: r.op, status: r.status, code: r.code, message: r.message, from: r.from, to: r.to, suggestion: null, replayed: true };
  }

  private rename(tx: string, op: "move" | "undo", ino: number, destIno: number, newName: string | null, batch: string | null, undoOf: string | null): TxOutcome {
    const src = this.locate(ino);
    if (!src || src.node.kind !== "file") return this.refuse(tx, op, "SOURCE_MISSING", "That file isn't where it was anymore.");
    const destWhere = destIno === 1 ? { path: [] as string[], node: this.root as Node } : this.locate(destIno);
    if (!destWhere || destWhere.node.kind !== "dir") return this.refuse(tx, op, "DESTINATION_MISSING", "That folder is gone.");
    const dest = destWhere.node;
    const destPath = destIno === 1 ? [] : [...destWhere.path, dest.name];
    const name = newName ?? src.node.name;
    const bad = this.validName(name, destPath.length === 0);
    if (bad) return this.refuse(tx, op, "BAD_NAME", bad);
    const from = this.rel(src.path, src.node.name);
    const to = this.rel(destPath, name);
    if (src.parent === dest && name === src.node.name) return this.refuse(tx, op, "SAME_PLACE", "That crate is already in this bay.");
    if (dest.children.has(name)) {
      return this.refuse(tx, op, "NAME_EXISTS", `Something called \`${name}\` is already there. PALLET never overwrites.`, this.free(dest, name, (n, i) => this.numbered(n, i), 2), from, to);
    }
    if (this.failNext) {
      const f = this.failNext;
      this.failNext = null;
      return this.refuse(tx, op, f.code, f.message, null, from, to);
    }
    src.parent.children.delete(src.node.name);
    src.node.name = name;
    dest.children.set(name, src.node);
    return this.record({ tx, op, status: "committed", code: null, message: null, from, to, ino, undoOf, batch });
  }

  async deliver(tx: string, cargo: string, bay: string, newName: string | null, batch: string | null = null): Promise<TxOutcome> {
    const done = this.replay(tx);
    if (done) return done;
    const ino = this.ids.get(cargo);
    const dest = this.ids.get(bay);
    if (ino === undefined) return this.refuse(tx, "move", "UNKNOWN_CARGO", "That crate isn't on file. Reload the floor.");
    if (dest === undefined) return this.refuse(tx, "move", "UNKNOWN_BAY", "That bay isn't on file.");
    return this.rename(tx, "move", ino, dest, newName, batch, null);
  }

  async deliverBatch(batch: string, items: [string, string][], bay: string): Promise<TxOutcome[]> {
    const out: TxOutcome[] = [];
    for (const [tx, cargo] of items.slice(0, 6)) {
      const o = await this.deliver(tx, cargo, bay, null, batch);
      out.push(o);
      if (o.status !== "committed") break;
    }
    return out;
  }

  async history(): Promise<HistoryEntry[]> {
    const committed = this.journal.filter((r) => r.status === "committed" && r.op !== "incident");
    return committed
      .map((r) => ({
        tx: r.tx,
        op: r.op,
        from: r.from ?? "",
        to: r.to ?? "",
        atMs: r.at,
        batch: r.batch,
        undone: committed.some((u) => u.undoOf === r.tx),
      }))
      .reverse();
  }

  async undo(tx: string, target: string, newName: string | null): Promise<TxOutcome> {
    const done = this.replay(tx);
    if (done) return done;
    const t = this.journal.find((r) => r.tx === target && r.status === "committed");
    if (!t || t.ino === null || (t.op !== "move" && t.op !== "copy")) return this.refuse(tx, "undo", "NOTHING_TO_UNDO", "That delivery isn't in the journal as completed.");
    if (this.journal.some((r) => r.undoOf === target && r.status === "committed")) return this.refuse(tx, "undo", "ALREADY_UNDONE", "That delivery was already undone.");
    const now = this.locate(t.ino);
    if (!now || this.rel(now.path, now.node.name) !== t.to) return this.refuse(tx, "undo", "SOURCE_CHANGED", "The delivered file isn't where the delivery left it.");
    if (t.op === "copy") {
      let bay = this.root.children.get("Recovered Copies");
      if (!bay) bay = this.mkdir(this.root, "Recovered Copies");
      if (bay.kind !== "dir") return this.refuse(tx, "undo", "BAY_UNSAFE", "`Recovered Copies` must be a plain folder.");
      return this.rename(tx, "undo", t.ino, bay.ino, newName, null, target);
    }
    const parts = (t.from ?? "").split("/");
    const origName = parts.pop()!;
    const origDir = this.dir(parts);
    if (!origDir) return this.refuse(tx, "undo", "DESTINATION_MISSING", `\`/${parts.join("/")}\` is gone.`);
    return this.rename(tx, "undo", t.ino, origDir.ino, newName ?? origName, null, target);
  }

  async duplicate(tx: string, cargo: string, newName: string | null): Promise<TxOutcome> {
    const done = this.replay(tx);
    if (done) return done;
    const ino = this.ids.get(cargo);
    const src = ino === undefined ? null : this.locate(ino);
    if (!src || src.node.kind !== "file") return this.refuse(tx, "copy", "UNKNOWN_CARGO", "That crate isn't on file.");
    if (src.node.bytes > 1024 ** 3) return this.refuse(tx, "copy", "TOO_HEAVY", "The duplicator only takes crates up to 1 GiB.");
    const name = newName ?? this.free(src.parent, src.node.name, (n, i) => this.copyName(n, i), 1);
    if (src.parent.children.has(name)) return this.refuse(tx, "copy", "NAME_EXISTS", `\`${name}\` already exists.`, this.free(src.parent, name, (n, i) => this.numbered(n, i), 2));
    const copy = this.put(src.parent, name, src.node.bytes);
    return this.record({ tx, op: "copy", status: "committed", code: null, message: null, from: this.rel(src.path, src.node.name), to: this.rel(src.path, name), ino: copy.ino, undoOf: null, batch: null });
  }

  async saveIncident(tx: string, text: string): Promise<TxOutcome> {
    const done = this.replay(tx);
    if (done) return done;
    const d = new Date();
    const p = (n: number) => String(n).padStart(2, "0");
    const base = `incident-report-${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())}-${p(d.getUTCHours())}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())}.txt`;
    const name = this.root.children.has(base) ? this.free(this.root, base, (n, i) => this.numbered(n, i), 2) : base;
    const f = this.put(this.root, name, new TextEncoder().encode(text).length);
    return this.record({ tx, op: "incident", status: "committed", code: null, message: null, from: null, to: name, ino: f.ino, undoOf: null, batch: null });
  }

  async txStatus(tx: string): Promise<TxOutcome | null> {
    return this.replay(tx);
  }

  async acknowledge(): Promise<WarehouseStatus> {
    return this.status();
  }

  /** Test helper: every file object, by identity, with its path. */
  snapshot(): Map<number, string> {
    const out = new Map<number, string>();
    const walk = (d: DirNode, path: string[]) => {
      for (const n of d.children.values()) {
        if (n.kind === "dir") walk(n, [...path, n.name]);
        else out.set(n.ino, this.rel(path, n.name));
      }
    };
    walk(this.root, []);
    return out;
  }
}
