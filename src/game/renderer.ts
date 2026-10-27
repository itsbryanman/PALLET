// Draws the warehouse. Reads game state; never changes it and never talks to
// the backend. All art is drawn here as vector shapes; there are no image assets.

import "pixi.js/unsafe-eval"; // lets Pixi run under the app's CSP without 'unsafe-eval'
import { Application, Container, Graphics, Text } from "pixi.js";
import type { CargoInfo } from "../backend/types";
import * as C from "../sim/constants";
import { crateSize, formatBytes } from "../sim/crate";
import { forkPoint, instability } from "../sim/forklift";
import type { Game, Placed } from "../sim/game";
import { FLOOR_H, FLOOR_W, type Layout, WALL } from "../sim/layout";
import { ForkliftSprite, loadForkliftAtlas } from "./forklift-sprite";

const COL = {
  concrete: 0xb8b1a4,
  concreteDark: 0xa59e91,
  joint: 0x948d80,
  paint: 0xe3b12b,
  paintDim: 0xc99b22,
  white: 0xf1ece2,
  wall: 0x3b3936,
  wallTop: 0x57534d,
  enamel: 0x1e3553,
  enamelDark: 0x142439,
  forklift: 0xc63b22,
  forkliftDark: 0x7a2414,
  steel: 0x2b2926,
  wood: 0xc4894b,
  woodDark: 0x7b5127,
  woodLight: 0xd9a46a,
  pallet: 0x9c7446,
  paper: 0xf1ece2,
  kraft: 0xd8c08c,
  ok: 0x3fae6a,
  bad: 0xc0392b,
  amber: 0xf0a020,
  stamp: 0xb3261e,
};

const STENCIL = "Big Shoulders Stencil Display";
const MONO = "IBM Plex Mono";

function rng(seed: number) {
  let s = seed >>> 0;
  return () => ((s = (Math.imul(s, 1664525) + 1013904223) >>> 0) / 2 ** 32);
}

function label(text: string, size: number, color: number, font = MONO, weight: "400" | "600" = "400"): Text {
  const t = new Text({ text, style: { fontFamily: font, fontSize: size, fill: color, fontWeight: weight } });
  t.resolution = 3;
  return t;
}

function fit(text: string, maxChars: number): string {
  if (text.length <= maxChars) return text;
  const keep = Math.max(3, maxChars - 1);
  const dot = text.lastIndexOf(".");
  if (dot > 0 && text.length - dot <= 6 && keep > text.length - dot + 3) {
    const ext = text.slice(dot);
    return `${text.slice(0, keep - ext.length)}…${ext}`;
  }
  return `${text.slice(0, keep)}…`;
}

/** A crate seen from above, centered on (0, 0). */
export function drawCrate(cargo: CargoInfo, withLabel = true): Container {
  const c = new Container();
  const { w, h } = crateSize(cargo.bytes);
  const g = new Graphics();
  // pallet slats peeking out underneath
  g.rect(-w / 2 - 3, -h / 2 + 2, w + 6, 4).fill(COL.pallet);
  g.rect(-w / 2 - 3, h / 2 - 6, w + 6, 4).fill(COL.pallet);
  if (cargo.bytes === 0 && !cargo.unavailable) {
    // empty file: a paper envelope
    g.rect(-w / 2, -h / 2, w, h).fill(COL.paper).stroke({ width: 1.2, color: 0x8a8174 });
    g.moveTo(-w / 2, -h / 2).lineTo(0, 2).lineTo(w / 2, -h / 2).stroke({ width: 1.2, color: 0x8a8174 });
  } else {
    const wood = cargo.unavailable ? 0x9a948a : COL.wood;
    const dark = cargo.unavailable ? 0x605b54 : COL.woodDark;
    g.rect(-w / 2, -h / 2, w, h).fill(wood).stroke({ width: 2, color: dark });
    for (let y = -h / 2 + h / 4; y < h / 2 - 1; y += h / 4) g.moveTo(-w / 2 + 2, y).lineTo(w / 2 - 2, y).stroke({ width: 1, color: dark, alpha: 0.5 });
    g.moveTo(-w / 2 + 2, -h / 2 + 2).lineTo(w / 2 - 2, h / 2 - 2).stroke({ width: 1.6, color: dark, alpha: 0.75 });
    g.rect(-w / 2 + 1, -h / 2 + 1, w - 2, 2).fill({ color: COL.woodLight, alpha: 0.6 });
    if (cargo.zip) {
      // shrink wrap: translucent film with highlights. Cosmetic only.
      g.roundRect(-w / 2 - 2, -h / 2 - 2, w + 4, h + 4, 4).fill({ color: 0xdfeaf2, alpha: 0.45 }).stroke({ width: 1, color: 0xffffff, alpha: 0.8 });
      for (let i = -w; i < w; i += 9) g.moveTo(i, -h / 2).lineTo(i + h * 0.6, h / 2).stroke({ width: 1.5, color: 0xffffff, alpha: 0.35 });
    }
    if (cargo.unavailable) {
      g.moveTo(-w / 2, -h / 2).lineTo(w / 2, h / 2).stroke({ width: 4, color: COL.bad, alpha: 0.85 });
      g.moveTo(w / 2, -h / 2).lineTo(-w / 2, h / 2).stroke({ width: 4, color: COL.bad, alpha: 0.85 });
    }
    // shipping label
    const lw = Math.min(w - 8, 30);
    g.rect(-lw / 2, -5, lw, 10).fill(COL.paper).stroke({ width: 0.8, color: dark });
    g.rect(-lw / 2 + 2, -2, lw * 0.6, 1.2).fill(0x3a342c);
    g.rect(-lw / 2 + 2, 1, lw * 0.4, 1.2).fill(0x3a342c);
  }
  c.addChild(g);
  if (withLabel) {
    const plate = new Container();
    const name = label(fit(cargo.name, 17), 10, 0x23201c, MONO, "600");
    const meta = label(cargo.unavailable ? cargo.unavailable : formatBytes(cargo.bytes), 9, cargo.unavailable ? COL.bad : 0x4c463e);
    const pw = Math.max(name.width, meta.width) + 8;
    const bg = new Graphics().roundRect(-pw / 2, 0, pw, 26, 2).fill({ color: COL.paper, alpha: 0.92 });
    name.anchor.set(0.5, 0);
    meta.anchor.set(0.5, 0);
    name.y = 1;
    meta.y = 14;
    plate.addChild(bg, name, meta);
    plate.y = h / 2 + 6;
    c.addChild(plate);
  }
  return c;
}

function drawFloor(layout: Layout, parentName: string | null): Container {
  const root = new Container();
  const spare: Text[] = [];
  const g = new Graphics();
  const r = rng(7);
  g.rect(0, 0, FLOOR_W, FLOOR_H).fill(COL.concrete);
  // pour variation and speckle
  for (let i = 0; i < 26; i++) {
    g.ellipse(r() * FLOOR_W, r() * FLOOR_H, 60 + r() * 140, 40 + r() * 90).fill({ color: r() < 0.5 ? COL.concreteDark : 0xc4bdb0, alpha: 0.18 });
  }
  for (let i = 0; i < 2600; i++) {
    const s = r() < 0.9 ? 1 : 2;
    g.rect(Math.floor(r() * FLOOR_W), Math.floor(r() * FLOOR_H), s, s).fill({ color: r() < 0.6 ? 0x8d877b : 0xd2ccc0, alpha: 0.5 });
  }
  // oil stains near the work areas
  for (const [x, y, rw] of [
    [430, 300, 26],
    [470, 520, 18],
    [300, 610, 22],
    [760, 330, 30],
  ] as const) {
    g.ellipse(x, y, rw, rw * 0.6).fill({ color: 0x4a443b, alpha: 0.12 });
    g.ellipse(x + 6, y + 3, rw * 0.5, rw * 0.3).fill({ color: 0x4a443b, alpha: 0.1 });
  }
  // saw-cut joints
  for (let x = 160; x < FLOOR_W; x += 160) g.moveTo(x, 0).lineTo(x, FLOOR_H).stroke({ width: 1, color: COL.joint, alpha: 0.7 });
  for (let y = 180; y < FLOOR_H; y += 180) g.moveTo(0, y).lineTo(FLOOR_W, y).stroke({ width: 1, color: COL.joint, alpha: 0.7 });

  // receiving: painted outline, with faded floor stencil
  const rv = layout.receiving;
  g.rect(rv.x, rv.y, rv.w, rv.h).stroke({ width: 4, color: COL.paint, alpha: 0.9 });
  for (const s of layout.slots) {
    g.rect(s.x - 40, s.y - 32, 80, 64).stroke({ width: 1.5, color: COL.white, alpha: 0.35 });
  }
  // aisle lane
  for (let x = 440; x < FLOOR_W - 30; x += 34) g.rect(x, FLOOR_H / 2 - 2, 18, 4).fill({ color: COL.paint, alpha: 0.8 });

  // bays: painted floors, dashed drop zones
  for (const b of layout.bays) {
    const a = b.area;
    g.rect(a.x, a.y, a.w, a.h).fill({ color: b.kind === "parent" ? 0x9fa7b0 : COL.concreteDark, alpha: 0.45 });
    const d = b.drop;
    for (let x = d.x; x < d.x + d.w; x += 12) {
      g.rect(x, d.y, Math.min(7, d.x + d.w - x), 3).fill(COL.paint);
      g.rect(x, d.y + d.h - 3, Math.min(7, d.x + d.w - x), 3).fill(COL.paint);
    }
    for (let y = d.y; y < d.y + d.h; y += 12) {
      g.rect(d.x, y, 3, Math.min(7, d.y + d.h - y)).fill(COL.paint);
      g.rect(d.x + d.w - 3, y, 3, Math.min(7, d.y + d.h - y)).fill(COL.paint);
    }
    // hazard stripes at the mouth of the bay
    const my = b.side === "top" ? a.y + a.h - 8 : a.y;
    for (let x = a.x; x < a.x + a.w; x += 16) {
      g.poly([x, my, x + 8, my, x + 16, my + 8, x + 8, my + 8]).fill({ color: COL.paint, alpha: 0.85 });
    }
    if (b.bay.unavailable) {
      g.moveTo(d.x, d.y).lineTo(d.x + d.w, d.y + d.h).stroke({ width: 3, color: COL.bad, alpha: 0.6 });
    }
  }

  // unused bay slots stay visibly empty rather than looking broken
  for (let i = layout.bays.length; i < 6; i++) {
    const x = 520 + (i % 3) * ((FLOOR_W - WALL - 520) / 3) + 4;
    const top = i < 3;
    const t = label("SPARE BAY", 22, 0x8f897d, STENCIL, "600");
    t.anchor.set(0.5);
    t.position.set(x + (FLOOR_W - WALL - 520) / 6 - 4, top ? WALL + 90 : FLOOR_H - WALL - 90);
    t.alpha = 0.7;
    spare.push(t);
  }

  // duplicator intake: hazard-striped plate
  const it = layout.duplicator.intake;
  g.rect(it.x, it.y, it.w, it.h).fill({ color: 0x3b3936, alpha: 0.25 });
  for (let x = it.x - it.h; x < it.x + it.w; x += 14) {
    g.moveTo(Math.max(it.x, x), it.y + Math.max(0, it.x - x)).lineTo(Math.min(it.x + it.w, x + it.h), it.y + Math.min(it.h, it.x + it.w - x)).stroke({ width: 5, color: COL.paint, alpha: 0.55 });
  }
  root.addChild(g, ...spare);

  const stencil = (text: string, x: number, y: number, size: number, alpha: number, rot = 0) => {
    const t = label(text, size, COL.white, STENCIL, "600");
    t.alpha = alpha;
    t.anchor.set(0.5);
    t.position.set(x, y);
    t.rotation = rot;
    root.addChild(t);
  };
  stencil("RECEIVING", rv.x + rv.w / 2, rv.y + rv.h + 24, 30, 0.55);
  if (layout.slots.length === 0) stencil("NOTHING HERE", rv.x + rv.w / 2, rv.y + rv.h / 2, 26, 0.5);
  for (const b of layout.bays) {
    const d = b.drop;
    stencil("DROP", d.x + d.w / 2, d.y + d.h / 2, 30, 0.35);
    // What's already filed here: a small stack, plus the count.
    if (b.bay.items > 0) {
      const sg = new Graphics();
      const n = Math.min(4, b.bay.items);
      const bx = b.area.x + 10;
      const by = b.side === "top" ? b.area.y + b.area.h - 40 : b.area.y + 24;
      for (let i = 0; i < n; i++) {
        sg.rect(bx + i * 3 + 2, by - i * 4 + 3, 16, 12).fill({ color: 0, alpha: 0.2 });
        sg.rect(bx + i * 3, by - i * 4, 16, 12).fill(COL.wood).stroke({ width: 1, color: COL.woodDark });
      }
      root.addChild(sg);
      const t = label(b.bay.items === 1 ? "1 file" : `${b.bay.items} files`, 10, 0x23201c, MONO, "600");
      t.position.set(bx + 30, by - 2);
      root.addChild(t);
    }
  }

  // walls
  const w = new Graphics();
  for (const r of layout.walls) {
    if (layout.pillars.includes(r) || r === layout.duplicator.body) continue;
    w.rect(r.x, r.y, r.w, r.h).fill(COL.wall);
    w.rect(r.x, r.y, r.w, Math.min(3, r.h)).fill(COL.wallTop);
  }
  for (const p of layout.pillars) {
    w.rect(p.x + 3, p.y + 4, p.w, p.h).fill({ color: 0, alpha: 0.25 });
    w.rect(p.x, p.y, p.w, p.h).fill(COL.paint);
    for (let i = -p.h; i < p.w; i += 8) w.moveTo(p.x + Math.max(0, i), p.y + Math.max(0, -i)).lineTo(p.x + Math.min(p.w, i + p.h), p.y + Math.min(p.h, p.w - i)).stroke({ width: 3, color: 0x23201c });
  }
  // duplicator machine
  const db = layout.duplicator.body;
  w.rect(db.x + 4, db.y + 4, db.w, db.h).fill({ color: 0, alpha: 0.25 });
  w.roundRect(db.x, db.y, db.w, db.h, 4).fill(0x5f6b73).stroke({ width: 2, color: 0x2f363b });
  w.rect(db.x + 26, db.y, db.w - 52, 12).fill(0x2f363b);
  for (let x = db.x + 30; x < db.x + db.w - 26; x += 8) w.rect(x, db.y + 2, 4, 8).fill(0x8b979f);
  w.circle(db.x + db.w - 16, db.y + 26, 6).fill(0x2f363b);
  w.circle(db.x + db.w - 16, db.y + 26, 4).fill(COL.ok);
  root.addChild(w);
  const dl = label("DUPLICATOR", 18, COL.white, STENCIL, "600");
  dl.anchor.set(0.5);
  dl.position.set(db.x + db.w / 2 - 6, db.y + 46);
  root.addChild(dl);

  // enamel bay signs on the back wall
  for (const b of layout.bays) {
    const name = b.kind === "parent" ? `UP: ${parentName ?? b.bay.name}` : b.bay.name;
    const t = label(fit(name.toUpperCase(), 16), 19, COL.white, STENCIL, "600");
    t.anchor.set(0.5);
    const sw = Math.min(b.area.w - 18, Math.max(90, t.width + 34));
    const sign = new Graphics();
    sign.rect(-sw / 2 + 3, -13 + 4, sw, 30).fill({ color: 0, alpha: 0.3 });
    sign.roundRect(-sw / 2, -13, sw, 30, 3).fill(b.kind === "parent" ? 0x3d4a57 : COL.enamel).stroke({ width: 2, color: COL.white });
    sign.circle(-sw / 2 + 7, -7, 1.5).fill(COL.white);
    sign.circle(sw / 2 - 7, -7, 1.5).fill(COL.white);
    const holder = new Container();
    holder.position.set(b.sign.x, b.sign.y + (b.side === "top" ? 6 : -6));
    holder.addChild(sign, t);
    t.y = 2;
    if (b.bay.unavailable) {
      const u = label(b.bay.unavailable, 10, COL.white, MONO);
      u.anchor.set(0.5, 0);
      u.y = 18;
      const ub = new Graphics().rect(-u.width / 2 - 4, 17, u.width + 8, 14).fill(COL.bad);
      holder.addChild(ub, u);
    }
    root.addChild(holder);
  }
  return root;
}

function drawForklift(): { root: Container; body: Container; forks: Graphics; load: Container; beacon: Graphics; shadow: Graphics } {
  const root = new Container();
  const shadow = new Graphics().roundRect(-26, -16, 46, 32, 6).fill({ color: 0, alpha: 0.28 });
  shadow.position.set(4, 5);
  const body = new Container();
  const g = new Graphics();
  // tires
  g.rect(-21, -17, 9, 5).fill(0x1a1917);
  g.rect(-21, 12, 9, 5).fill(0x1a1917);
  g.rect(4, -18, 12, 6).fill(0x1a1917);
  g.rect(4, 12, 12, 6).fill(0x1a1917);
  // counterweight and chassis
  g.roundRect(-25, -13, 12, 26, 4).fill(COL.forkliftDark);
  g.roundRect(-17, -14, 32, 28, 4).fill(COL.forklift).stroke({ width: 1.5, color: COL.forkliftDark });
  // overhead guard
  g.rect(-12, -11, 20, 22).fill(COL.enamel);
  for (let x = -9; x < 8; x += 4.5) g.rect(x, -10, 1.6, 20).fill({ color: COL.white, alpha: 0.35 });
  g.rect(-12, -11, 20, 22).stroke({ width: 1.5, color: COL.enamelDark });
  // mast
  g.rect(14, -13, 5, 26).fill(COL.steel);
  g.rect(15, -11, 3, 22).fill(0x4a4641);
  body.addChild(g);
  const beacon = new Graphics().circle(-15, 0, 3.2).fill(COL.amber);
  beacon.alpha = 0;
  body.addChild(beacon);
  const forks = new Graphics();
  const load = new Container();
  root.addChild(shadow, forks, body, load);
  return { root, body, forks, load, beacon, shadow };
}

interface Debris {
  view: Container;
  vx: number;
  vy: number;
  vr: number;
}

export class Renderer {
  app = new Application();
  private world = new Container();
  private floorLayer = new Container();
  private marks = new Graphics();
  private markCount = 0;
  private crateLayer = new Container();
  private guide = new Graphics();
  private fx = new Container();
  private tag = new Container();
  private lamps = new Map<string, { g: Graphics; until: number; ok: boolean }>();
  private fl = drawForklift();
  /** The eight-direction sprite; the vector forklift is only a fallback if the atlas fails to load. */
  private sprite: ForkliftSprite | null = null;
  private layoutRef: Layout | null = null;
  private crateKey = "";
  private loadKey = "";
  private debris: Debris[] = [];
  private settling: { view: Container; tx: number; ty: number; t: number }[] = [];
  private incidentStamp: Container | null = null;
  private shakeAmt = 0;
  private clock = 0;
  reducedMotion = false;

  async init(el: HTMLElement): Promise<void> {
    await this.app.init({ resizeTo: el, background: 0x6f6a61, antialias: true, autoDensity: true, resolution: Math.min(2, devicePixelRatio || 1) });
    el.appendChild(this.app.canvas);
    this.app.canvas.setAttribute("aria-hidden", "true");
    try {
      this.sprite = new ForkliftSprite(await loadForkliftAtlas());
    } catch (err) {
      console.warn("forklift atlas unavailable, using the vector forklift", err);
    }
    this.world.addChild(this.floorLayer, this.marks, this.guide, this.crateLayer, this.sprite ? this.sprite.root : this.fl.root, this.fx, this.tag);
    this.app.stage.addChild(this.world);
  }

  /** Rebuilds static art when the floor plan changes. */
  private sync(game: Game): void {
    if (game.layout !== this.layoutRef) {
      this.layoutRef = game.layout;
      this.floorLayer.removeChildren().forEach((c) => c.destroy({ children: true }));
      const parent = game.floor.parent ? game.floor.parent.name : null;
      const floor = drawFloor(game.layout, parent);
      this.floorLayer.addChild(floor);
      this.lamps.clear();
      for (const b of game.layout.bays) {
        const lamp = new Graphics();
        lamp.position.set(b.sign.x + Math.min(b.area.w / 2 - 20, 70), b.sign.y + (b.side === "top" ? 6 : -6));
        this.floorLayer.addChild(lamp);
        this.lamps.set(b.bay.id, { g: lamp, until: 0, ok: true });
      }
      this.crateKey = "";
    }
    const crates = game.crates();
    const key = crates.map((c) => `${c.cargo.id}@${c.x | 0},${c.y | 0}`).join("|");
    if (key !== this.crateKey) {
      this.crateKey = key;
      this.crateLayer.removeChildren().forEach((c) => c.destroy({ children: true }));
      for (const p of crates) this.crateLayer.addChild(this.placeCrate(p));
    }
    const lk = game.load.map((c) => c.id).join("|");
    if (lk !== this.loadKey) {
      this.loadKey = lk;
      const holder = this.sprite ? this.sprite.cargo : this.fl.load;
      holder.removeChildren().forEach((c) => c.destroy({ children: true }));
      game.load.forEach((c, i) => {
        const v = drawCrate(c, false);
        v.position.set(C.FORK_REACH - i * 1.5, -i * 2.5);
        holder.addChild(v);
      });
      if (game.load.length > 1) {
        const n = label(`${game.load.length}`, 16, COL.white, STENCIL, "600");
        n.anchor.set(0.5);
        n.position.set(C.FORK_REACH - game.load.length * 1.5, -game.load.length * 2.5);
        const bg = new Graphics().circle(n.x, n.y, 9).fill({ color: COL.steel, alpha: 0.85 });
        holder.addChild(bg, n);
      }
    }
  }

  private placeCrate(p: Placed): Container {
    const v = drawCrate(p.cargo);
    v.position.set(p.x, p.y);
    const shadow = new Graphics();
    const { w, h } = crateSize(p.cargo.bytes);
    shadow.rect(-w / 2 + 3, -h / 2 + 4, w, h).fill({ color: 0, alpha: 0.22 });
    v.addChildAt(shadow, 0);
    if (p.rot) v.children[1].rotation = p.rot;
    return v;
  }

  flashBay(bayId: string, ok: boolean): void {
    const l = this.lamps.get(bayId);
    if (l) {
      l.until = this.clock + 1.6;
      l.ok = ok;
    }
  }

  /** Delivered crates slide into the bay and fade, after the backend committed. */
  settle(game: Game, bayId: string, cargo: CargoInfo[]): void {
    if (this.reducedMotion) return;
    const bay = game.layout.bays.find((b) => b.bay.id === bayId);
    if (!bay) return;
    const p = forkPoint(game.fork);
    cargo.forEach((c, i) => {
      const view = drawCrate(c, false);
      view.position.set(p.x - i * 1.5, p.y - i * 2.5);
      view.rotation = game.fork.heading;
      this.fx.addChild(view);
      this.settling.push({ view, tx: bay.drop.x + bay.drop.w / 2, ty: bay.drop.y + bay.drop.h / 2, t: 0 });
    });
  }

  shake(amount: number): void {
    if (!this.reducedMotion) this.shakeAmt = Math.max(this.shakeAmt, amount);
  }

  /** The load comes off the forks. Purely visual: the files haven't moved. */
  scatter(game: Game, cargo: CargoInfo[]): void {
    const f = game.fork;
    const p = forkPoint(f);
    const side = f.tipped?.side ?? 1;
    cargo.forEach((c, i) => {
      const view = drawCrate(c, false);
      view.position.set(p.x, p.y);
      const a = f.heading + side * (0.9 + i * 0.35);
      const sp = 140 + i * 30;
      const d: Debris = { view, vx: Math.cos(a) * sp + f.speed * Math.cos(f.heading) * 0.6, vy: Math.sin(a) * sp + f.speed * Math.sin(f.heading) * 0.6, vr: side * (4 + i) };
      this.fx.addChild(view);
      this.debris.push(d);
      if (this.reducedMotion) {
        for (let k = 0; k < 90; k++) this.stepDebris(d, 1 / 60);
      }
    });
    this.fl.load.visible = false;
    if (this.sprite) this.sprite.cargo.visible = false;
    const stamp = new Container();
    const t = label("LOAD INCIDENT", 34, COL.stamp, STENCIL, "600");
    t.anchor.set(0.5);
    const box = new Graphics().roundRect(-t.width / 2 - 10, -t.height / 2 - 2, t.width + 20, t.height + 4, 4).stroke({ width: 4, color: COL.stamp });
    stamp.addChild(box, t);
    stamp.position.set(Math.min(FLOOR_W - 140, Math.max(140, f.x)), Math.max(70, f.y - 70));
    stamp.rotation = -0.12;
    stamp.alpha = 0.9;
    stamp.scale.set(this.reducedMotion ? 1 : 2.2);
    this.fx.addChild(stamp);
    this.incidentStamp = stamp;
    this.shake(14);
  }

  /** Visual recovery: debris clears, load returns to the forks. */
  recover(): void {
    for (const d of this.debris) d.view.destroy({ children: true });
    this.debris = [];
    this.incidentStamp?.destroy({ children: true });
    this.incidentStamp = null;
    this.fl.load.visible = true;
    if (this.sprite) this.sprite.cargo.visible = true;
  }

  private stepDebris(d: Debris, dt: number): void {
    d.view.x += d.vx * dt;
    d.view.y += d.vy * dt;
    d.view.rotation += d.vr * dt;
    const k = Math.pow(0.04, dt);
    d.vx *= k;
    d.vy *= k;
    d.vr *= k;
    d.view.x = Math.max(WALL + 20, Math.min(FLOOR_W - WALL - 20, d.view.x));
    d.view.y = Math.max(WALL + 20, Math.min(FLOOR_H - WALL - 20, d.view.y));
  }

  clearMarks(): void {
    this.marks.clear();
    this.markCount = 0;
  }

  frame(game: Game, dt: number): void {
    this.clock += dt;
    this.sync(game);
    const f = game.fork;
    const info = game.loadInfo();
    const wob = instability(f, info);

    // forklift pose
    if (this.sprite) this.poseSprite(game, wob);
    else this.poseVector(game, wob);

    // skid marks on hard turns


    // skid marks on hard turns
    if (!f.tipped && Math.abs(f.steer) > 0.7 && Math.abs(f.speed) > C.MAX_SPEED * 0.55 && this.markCount < 900) {
      for (const sy of [-15, 15]) {
        const x = f.x + Math.cos(f.heading) * 10 - Math.sin(f.heading) * sy;
        const y = f.y + Math.sin(f.heading) * 10 + Math.cos(f.heading) * sy;
        this.marks.circle(x, y, 2.2).fill({ color: 0x2b2926, alpha: 0.12 });
        this.markCount++;
      }
    }

    // alignment guide and target tag
    this.guide.clear();
    this.tag.removeChildren().forEach((c) => c.destroy({ children: true }));
    if (!f.tipped && !game.pending) {
      const t = game.load.length < 6 ? game.target() : null;
      if (t && (f.forkTarget === 0 || game.load.length > 0)) {
        const col = t.crate.cargo.unavailable ? COL.bad : t.aligned ? COL.ok : COL.amber;
        const { w, h: ch } = crateSize(t.crate.cargo.bytes);
        this.guide.rect(t.crate.x - w / 2 - 4, t.crate.y - ch / 2 - 4, w + 8, ch + 8).stroke({ width: 2.5, color: col });
        for (const sy of [-8, 8]) {
          const x0 = f.x + Math.cos(f.heading) * 44 - Math.sin(f.heading) * sy;
          const y0 = f.y + Math.sin(f.heading) * 44 + Math.cos(f.heading) * sy;
          this.guide.moveTo(x0, y0).lineTo(x0 + Math.cos(f.heading) * 24, y0 + Math.sin(f.heading) * 24).stroke({ width: 2, color: col, alpha: 0.8 });
        }
        const name = label(t.crate.cargo.name, 12, 0x23201c, MONO, "600");
        const sub = label(t.crate.cargo.unavailable ?? (t.aligned ? (game.load.length ? "Q adds it to the pallet" : "E lifts it") : "line up the forks"), 10, 0x4c463e);
        const pw = Math.max(name.width, sub.width) + 14;
        const bg = new Graphics().roundRect(-pw / 2, 0, pw, 34, 3).fill(COL.paper).stroke({ width: 1.5, color: col });
        name.anchor.set(0.5, 0);
        sub.anchor.set(0.5, 0);
        name.y = 3;
        sub.y = 19;
        this.tag.addChild(bg, name, sub);
        this.tag.position.set(t.crate.x, t.crate.y - ch / 2 - 44);
      }
      if (game.load.length) {
        const bay = game.bayUnderForks();
        const pulse = 0.25 + 0.15 * Math.sin(this.clock * 6);
        if (bay) this.guide.rect(bay.drop.x, bay.drop.y, bay.drop.w, bay.drop.h).fill({ color: bay.bay.unavailable ? COL.bad : COL.paint, alpha: pulse });
        if (game.atDuplicator()) {
          const it = game.layout.duplicator.intake;
          this.guide.rect(it.x, it.y, it.w, it.h).fill({ color: COL.paint, alpha: pulse });
        }
      }
    }

    // bay lamps
    for (const l of this.lamps.values()) {
      l.g.clear();
      const lit = this.clock < l.until;
      const blink = lit && (this.reducedMotion || Math.floor(this.clock * 6) % 2 === 0);
      l.g.circle(0, 0, 6).fill(0x2b2926);
      l.g.circle(0, 0, 4).fill(lit ? (blink ? (l.ok ? COL.ok : COL.bad) : 0x555049) : 0x555049);
      if (lit && blink) l.g.circle(0, 0, 11).fill({ color: l.ok ? COL.ok : COL.bad, alpha: 0.25 });
    }

    for (const d of this.debris) this.stepDebris(d, dt);
    for (const s of this.settling) {
      s.t += dt / 0.7;
      const k = Math.min(1, s.t);
      s.view.x += (s.tx - s.view.x) * Math.min(1, dt * 6);
      s.view.y += (s.ty - s.view.y) * Math.min(1, dt * 6);
      s.view.scale.set(1 - 0.35 * k);
      s.view.alpha = 1 - k * k;
    }
    for (const s of this.settling.filter((x) => x.t >= 1)) s.view.destroy({ children: true });
    this.settling = this.settling.filter((x) => x.t < 1);
    if (this.incidentStamp && this.incidentStamp.scale.x > 1) this.incidentStamp.scale.set(Math.max(1, this.incidentStamp.scale.x - dt * 9));

    this.camera(game, dt);
  }

  private poseVector(game: Game, wob: number): void {
    const f = game.fork;
    const fr = this.fl;
    fr.root.position.set(f.x, f.y);
    let rot = f.heading;
    if (!this.reducedMotion && !f.tipped && wob > 0.45) rot += Math.sin(this.clock * 38) * 0.035 * wob;
    fr.root.rotation = rot;
    if (f.tipped) {
      fr.root.rotation = f.heading + f.tipped.side * 0.5;
      fr.root.scale.set(1, 0.6);
      fr.body.alpha = 0.92;
    } else {
      fr.root.scale.set(1, 1);
      fr.body.alpha = 1;
    }
    // forks: higher reads as lighter, offset toward the viewer, longer shadow
    const h = f.fork;
    fr.forks.clear();
    const forkCol = h > 0.5 ? 0x77716a : 0x3a3734;
    fr.forks.rect(19, -10 - h * 2, 26, 3.5).fill(forkCol);
    fr.forks.rect(19, 6.5 - h * 2, 26, 3.5).fill(forkCol);
    fr.load.position.set(-h * 1.5, -h * 3);
    fr.load.scale.set(1 + h * 0.06);
    fr.shadow.position.set(4 + h * 3, 5 + h * 5);
    if (!this.reducedMotion && !f.tipped && wob > 0.45) fr.load.rotation = Math.sin(this.clock * 31) * 0.06 * wob;
    else fr.load.rotation = 0;
    const reversing = f.speed < -4;
    fr.beacon.alpha = reversing ? (Math.floor(this.clock * 4) % 2 ? 1 : 0.25) : 0;
  }

  private poseSprite(game: Game, wob: number): void {
    const f = game.fork;
    const sp = this.sprite!;
    // Wobble is a small sideways jitter; the art itself is never spun with the heading.
    const jitter = !this.reducedMotion && !f.tipped && wob > 0.45 ? Math.sin(this.clock * 38) * 1.6 * wob : 0;
    sp.root.position.set(f.x + jitter * -Math.sin(f.heading), f.y + jitter * Math.cos(f.heading));
    sp.update(f.heading, f.tipped ? f.tipped.side : null);
    const off = sp.forkOffset();
    const lift = f.fork * 7; // raised forks: the cargo rides higher (the fork art itself is fixed)
    const angle = sp.frameAngle();
    sp.cargo.children.forEach((c, i) => {
      c.position.set(off.x - i * 1.5, off.y - i * 2.5 - lift);
      c.rotation = c instanceof Text ? 0 : angle;
    });
    sp.cargo.rotation = !this.reducedMotion && !f.tipped && wob > 0.45 ? Math.sin(this.clock * 31) * 0.04 * wob : 0;
    sp.beacon.alpha = f.speed < -4 ? (Math.floor(this.clock * 4) % 2 ? 1 : 0.25) : 0;
  }

  private camera(game: Game, dt: number): void {
    const vw = this.app.screen.width;
    const vh = this.app.screen.height;
    const top = 68;
    const bottom = vw > 900 ? 96 : 150;
    const fitScale = Math.min((vw - 24) / FLOOR_W, (vh - top - bottom) / FLOOR_H);
    const scale = Math.max(0.62, Math.min(1.5, fitScale));
    this.world.scale.set(scale);
    const ww = FLOOR_W * scale;
    const wh = FLOOR_H * scale;
    let x = (vw - ww) / 2;
    let y = top + (vh - top - bottom - wh) / 2;
    if (ww > vw) x = Math.min(0, Math.max(vw - ww, vw / 2 - game.fork.x * scale));
    if (wh > vh - top) y = Math.min(top, Math.max(vh - wh, (vh + top) / 2 - game.fork.y * scale));
    if (this.shakeAmt > 0.2) {
      x += (Math.random() - 0.5) * this.shakeAmt;
      y += (Math.random() - 0.5) * this.shakeAmt;
      this.shakeAmt *= Math.pow(0.002, dt);
    }
    this.world.position.set(Math.round(x), Math.round(y));
  }
}
