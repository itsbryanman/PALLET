// Game state for one floor: the forklift, crates, the carried pallet, and the
// rules for picking up and setting down. It produces *intents* (deliver these
// crates to that bay, duplicate this crate). It has no backend reference, so a
// tip or a bump cannot cause a file operation (SAFETY.md S2, S7).

import type { CargoInfo, Floor } from "../backend/types";
import * as C from "./constants";
import { crateMass } from "./crate";
import { type DriveInput, type Forklift, type LoadInfo, forkPoint, newForklift, step } from "./forklift";
import { type BaySlot, type Layout, buildLayout, inside } from "./layout";

export const MAX_STACK = 6;

export type Action = "lift" | "stack" | "enter";

export type GameEvent =
  | { kind: "pickup"; cargo: CargoInfo }
  | { kind: "stack"; cargo: CargoInfo; count: number }
  | { kind: "unstack"; cargo: CargoInfo }
  | { kind: "setDown"; count: number }
  | { kind: "refuse"; reason: string }
  | { kind: "forks"; up: boolean }
  | { kind: "bump"; impact: number }
  | { kind: "tip"; cause: "turn" | "wall"; load: CargoInfo[] }
  | { kind: "requestDelivery"; bay: BaySlot; load: CargoInfo[] }
  | { kind: "requestCopy"; cargo: CargoInfo }
  | { kind: "enterBay"; bay: BaySlot };

export interface Placed {
  cargo: CargoInfo;
  x: number;
  y: number;
  rot: number;
}

export class Game {
  floor: Floor;
  layout: Layout;
  fork: Forklift;
  load: CargoInfo[] = [];
  /** Crates set down away from their rack. Visual only; the file hasn't moved. */
  parked = new Map<string, Placed>();
  rackPage = 0;
  bayPage = 0;
  /** A backend transaction is in flight. Driving and forks are locked. */
  pending = false;
  paused = false;
  tick = 0;

  constructor(floor: Floor) {
    this.floor = floor;
    this.layout = buildLayout(floor, 0, 0);
    const s = this.layout.spawn;
    this.fork = newForklift(s.x, s.y, s.heading);
  }

  /** Switches floors. The carried pallet comes along; parked crates don't. */
  setFloor(floor: Floor, sameFloor: boolean): void {
    this.floor = floor;
    if (!sameFloor) {
      this.parked.clear();
      this.rackPage = 0;
      this.bayPage = 0;
    }
    const ids = new Set(floor.cargo.map((c) => c.id));
    for (const id of [...this.parked.keys()]) if (!ids.has(id)) this.parked.delete(id);
    this.layout = buildLayout(floor, this.rackPage, this.bayPage);
    if (!sameFloor) {
      const s = this.layout.spawn;
      this.fork = { ...newForklift(s.x, s.y, s.heading), fork: this.fork.fork, forkTarget: this.fork.forkTarget };
    }
  }

  page(rack: number, bay: number): void {
    this.rackPage = Math.max(0, Math.min(this.layout.rackPages - 1, this.rackPage + rack));
    this.bayPage = Math.max(0, Math.min(this.layout.bayPages - 1, this.bayPage + bay));
    this.layout = buildLayout(this.floor, this.rackPage, this.bayPage);
  }

  loadInfo(): LoadInfo {
    return { count: this.load.length, mass: this.load.reduce((m, c) => m + crateMass(c.bytes), 0) };
  }

  /** Crates on the floor right now (rack slots plus parked), excluding carried ones. */
  crates(): Placed[] {
    const carried = new Set(this.load.map((c) => c.id));
    const out: Placed[] = [];
    for (const s of this.layout.slots) {
      if (carried.has(s.cargo.id) || this.parked.has(s.cargo.id)) continue;
      out.push({ cargo: s.cargo, x: s.x, y: s.y, rot: 0 });
    }
    for (const p of this.parked.values()) if (!carried.has(p.cargo.id)) out.push(p);
    return out;
  }

  /** The crate the forks are lined up with, if any, and how well. */
  target(): { crate: Placed; aligned: boolean } | null {
    const tip = forkPoint(this.fork);
    let best: { crate: Placed; d: number; aligned: boolean } | null = null;
    for (const c of this.crates()) {
      const d = Math.hypot(c.x - tip.x, c.y - tip.y);
      if (d > C.PICK_RADIUS * 1.8) continue;
      const toCrate = Math.atan2(c.y - this.fork.y, c.x - this.fork.x);
      let diff = Math.abs(toCrate - this.fork.heading);
      if (diff > Math.PI) diff = 2 * Math.PI - diff;
      const aligned = d <= C.PICK_RADIUS && diff <= C.PICK_ALIGN;
      if (!best || d < best.d) best = { crate: c, d, aligned };
    }
    return best ? { crate: best.crate, aligned: best.aligned } : null;
  }

  bayUnderForks(): BaySlot | null {
    const tip = forkPoint(this.fork);
    return this.layout.bays.find((b) => inside(b.drop, tip)) ?? null;
  }

  bayAround(): BaySlot | null {
    return this.layout.bays.find((b) => inside(b.area, this.fork)) ?? null;
  }

  atDuplicator(): boolean {
    return inside(this.layout.duplicator.intake, forkPoint(this.fork));
  }

  stopped(): boolean {
    return Math.abs(this.fork.speed) < 6;
  }

  /** One fixed step. `actions` are edge-triggered presses since the last step. */
  update(input: DriveInput, actions: Action[]): GameEvent[] {
    const events: GameEvent[] = [];
    if (this.paused) return events;
    this.tick++;
    const locked = this.pending || this.fork.tipped !== null;
    const drive = locked ? { throttle: 0, steer: 0, brake: true, precision: false } : input;
    for (const e of step(this.fork, drive, this.layout.walls, this.loadInfo())) {
      if (e.kind === "bump") events.push({ kind: "bump", impact: e.impact });
      if (e.kind === "tip") events.push({ kind: "tip", cause: e.cause, load: [...this.load] });
    }
    if (locked) return events;
    for (const a of actions) this.act(a, events);
    return events;
  }

  private act(a: Action, events: GameEvent[]): void {
    const f = this.fork;
    if (a === "enter") {
      const bay = this.bayAround();
      if (bay && this.stopped() && !bay.bay.unavailable) events.push({ kind: "enterBay", bay });
      else if (bay?.bay.unavailable) events.push({ kind: "refuse", reason: `${bay.bay.name}: ${bay.bay.unavailable}` });
      return;
    }
    if (a === "stack") {
      if (!this.stopped()) {
        events.push({ kind: "refuse", reason: "Stop before loading the pallet." });
        return;
      }
      const t = this.target();
      if (t?.aligned && this.load.length > 0 && this.load.length < MAX_STACK) {
        if (t.crate.cargo.unavailable) {
          events.push({ kind: "refuse", reason: `${t.crate.cargo.name}: ${t.crate.cargo.unavailable}` });
          return;
        }
        this.load.push(t.crate.cargo);
        this.parked.delete(t.crate.cargo.id);
        events.push({ kind: "stack", cargo: t.crate.cargo, count: this.load.length });
      } else if (this.load.length >= MAX_STACK) {
        events.push({ kind: "refuse", reason: "Six crates is the limit. The pallet has opinions." });
      } else if (this.load.length > 1 && !t) {
        const top = this.load.pop()!;
        const p = forkPoint(f);
        this.parked.set(top.id, { cargo: top, x: p.x + Math.cos(f.heading) * 22, y: p.y + Math.sin(f.heading) * 22, rot: f.heading });
        events.push({ kind: "unstack", cargo: top });
      } else if (this.load.length === 0) {
        events.push({ kind: "refuse", reason: "Lift a crate first (E), then add more with Q." });
      }
      return;
    }
    // lift / lower
    if (f.forkTarget === 0) {
      const t = this.target();
      if (this.load.length === 0 && t?.aligned) {
        if (t.crate.cargo.unavailable) {
          events.push({ kind: "refuse", reason: `${t.crate.cargo.name}: ${t.crate.cargo.unavailable}` });
          return;
        }
        this.load.push(t.crate.cargo);
        this.parked.delete(t.crate.cargo.id);
        events.push({ kind: "pickup", cargo: t.crate.cargo });
      }
      f.forkTarget = 1;
      events.push({ kind: "forks", up: true });
      return;
    }
    f.forkTarget = 0;
    events.push({ kind: "forks", up: false });
    if (this.load.length === 0) return;
    if (!this.stopped()) {
      events.push({ kind: "refuse", reason: "Stop first. Then lower." });
      f.forkTarget = 1;
      return;
    }
    const bay = this.bayUnderForks();
    if (bay) {
      if (bay.bay.unavailable) {
        events.push({ kind: "refuse", reason: `${bay.bay.name}: ${bay.bay.unavailable}` });
        f.forkTarget = 1;
        return;
      }
      events.push({ kind: "requestDelivery", bay, load: [...this.load] });
      return;
    }
    if (this.atDuplicator()) {
      if (this.load.length !== 1) {
        events.push({ kind: "refuse", reason: "The duplicator takes one crate at a time." });
        f.forkTarget = 1;
        return;
      }
      events.push({ kind: "requestCopy", cargo: this.load[0] });
      return;
    }
    // Set the pallet down where it is. Picture only: the files stay put.
    const p = forkPoint(f);
    this.load.forEach((c, i) => {
      this.parked.set(c.id, { cargo: c, x: p.x + i * 3, y: p.y - i * 3, rot: f.heading });
    });
    events.push({ kind: "setDown", count: this.load.length });
    this.load = [];
  }

  /** Backend committed these crates; they leave the forks. */
  delivered(ids: string[]): void {
    const gone = new Set(ids);
    this.load = this.load.filter((c) => !gone.has(c.id));
    if (this.load.length) this.fork.forkTarget = 1;
  }

  /** Backend refused; the load stays on the raised forks. */
  keepLoad(): void {
    this.fork.forkTarget = this.load.length ? 1 : 0;
  }

  /** "Right the forklift": visual reset only, no backend call. */
  recover(): void {
    const f = this.fork;
    f.tipped = null;
    f.speed = 0;
    f.risk = 0;
    f.steer = 0;
    f.forkTarget = 0;
    // Back away from whatever we hit.
    f.x -= Math.cos(f.heading) * 6;
    f.y -= Math.sin(f.heading) * 6;
  }
}
