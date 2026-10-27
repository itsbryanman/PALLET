// The forklift drawn from eight pre-rendered directions (assets/sprites, sliced
// by scripts/slice_forklift.py). The frame is chosen from the heading; the
// raster itself is never rotated with the heading, because that would rotate
// the baked perspective. Movement and collision come from the simulation, not
// from the sprite. Every frame has lowered forks and still wheels: raised forks
// are shown only by lifting the cargo, until matching art exists.

import { Assets, Container, Graphics, Rectangle, Sprite, Texture } from "pixi.js";
import { DIRECTIONS, type Direction, frameAngle, pickFrame } from "../sim/direction";

/** Source pixels to world pixels. Keeps the pivot-to-fork distance near the sim's fork reach. */
export const SPRITE_SCALE = 0.19;

interface FrameMeta {
  frame: { x: number; y: number; w: number; h: number };
  pivot: { x: number; y: number };
  fork: { x: number; y: number };
  cargoInFront: boolean;
}

export interface Atlas {
  textures: Record<Direction, Texture>;
  meta: Record<Direction, FrameMeta>;
}

export async function loadForkliftAtlas(base: string = import.meta.env.BASE_URL): Promise<Atlas> {
  const json = (await (await fetch(`${base}sprites/forklift-atlas.json`)).json()) as { frames: Record<Direction, FrameMeta> };
  const tex = await Assets.load<Texture>(`${base}sprites/forklift-atlas.png`);
  // Drawn at ~15% of source size: mipmaps keep the forks and cage bars from shimmering.
  tex.source.autoGenerateMipmaps = true;
  tex.source.scaleMode = "linear";
  tex.source.updateMipmaps();
  const textures = {} as Record<Direction, Texture>;
  for (const d of DIRECTIONS) {
    const f = json.frames[d].frame;
    textures[d] = new Texture({ source: tex.source, frame: new Rectangle(f.x, f.y, f.w, f.h) });
  }
  return { textures, meta: json.frames };
}

export class ForkliftSprite {
  readonly root = new Container();
  readonly cargo = new Container();
  readonly beacon = new Graphics();
  private shadow = new Graphics();
  private sprite: Sprite;
  private frame: number | null = null;

  constructor(private atlas: Atlas) {
    // Ground shadow: a rounded footprint on the floor, separate from the art.
    this.shadow.ellipse(0, 0, 27, 16).fill({ color: 0, alpha: 0.22 });
    this.sprite = new Sprite(atlas.textures.e);
    this.sprite.scale.set(SPRITE_SCALE);
    this.beacon.circle(0, 0, 3).fill(0xf0a020);
    this.beacon.alpha = 0;
    this.root.addChild(this.shadow, this.cargo, this.sprite, this.beacon);
  }

  get direction(): Direction {
    return DIRECTIONS[this.frame ?? 0];
  }

  /** Where carried cargo sits, relative to the pivot, for the current frame. */
  forkOffset(): { x: number; y: number } {
    const m = this.atlas.meta[this.direction];
    return { x: m.fork.x * SPRITE_SCALE, y: m.fork.y * SPRITE_SCALE };
  }

  /** Snapped angle of the current frame, for orienting top-down cargo art. */
  frameAngle(): number {
    return frameAngle(this.frame ?? 0);
  }

  update(heading: number, tippedSide: number | null): void {
    const next = pickFrame(heading, this.frame);
    if (next !== this.frame) {
      this.frame = next;
      const d = DIRECTIONS[next];
      const m = this.atlas.meta[d];
      this.sprite.texture = this.atlas.textures[d];
      this.sprite.anchor.set(m.pivot.x / m.frame.w, m.pivot.y / m.frame.h);
      // Cargo goes behind the body when the forks point away from the viewer.
      this.root.setChildIndex(this.cargo, m.cargoInFront ? 2 : 1);
      this.beacon.position.set(-m.fork.x * SPRITE_SCALE * 0.45, -m.fork.y * SPRITE_SCALE * 0.45 - 34);
    }
    // The ground shadow lies on the floor, so it may follow the exact heading.
    this.shadow.rotation = heading;
    // Tipping is a one-off pose, not a heading: keel the art over sideways.
    this.sprite.rotation = tippedSide === null ? 0 : tippedSide * 1.2;
    this.sprite.alpha = tippedSide === null ? 1 : 0.95;
  }
}
