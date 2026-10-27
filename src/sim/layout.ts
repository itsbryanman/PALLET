// Turns a backend Floor into a warehouse floor plan. Only the current rack page
// and bay page are laid out; everything else is offscreen and not drawn.

import type { BayInfo, CargoInfo, Floor } from "../backend/types";
import type { Rect } from "./forklift";

export const FLOOR_W = 1120;
export const FLOOR_H = 720;
export const WALL = 14;
export const RACK_PAGE = 12;
export const BAY_PAGE = 6;

const BAY_X0 = 520;
const BAY_W = (FLOOR_W - WALL - BAY_X0) / 3;
const BAY_DEPTH = 182;

export interface CrateSlot {
  cargo: CargoInfo;
  x: number;
  y: number;
}

export interface BaySlot {
  bay: BayInfo;
  kind: "sub" | "parent";
  area: Rect;
  drop: Rect;
  sign: { x: number; y: number };
  side: "top" | "bottom";
}

export interface Layout {
  walls: Rect[];
  slots: CrateSlot[];
  bays: BaySlot[];
  receiving: Rect;
  duplicator: { body: Rect; intake: Rect };
  pillars: Rect[];
  spawn: { x: number; y: number; heading: number };
  rackPages: number;
  bayPages: number;
}

export function inside(r: Rect, p: { x: number; y: number }): boolean {
  return p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h;
}

export function buildLayout(floor: Floor, rackPage: number, bayPage: number): Layout {
  const walls: Rect[] = [
    { x: 0, y: 0, w: FLOOR_W, h: WALL },
    { x: 0, y: FLOOR_H - WALL, w: FLOOR_W, h: WALL },
    { x: 0, y: 0, w: WALL, h: FLOOR_H },
    { x: FLOOR_W - WALL, y: 0, w: WALL, h: FLOOR_H },
  ];

  // Receiving racks fill row by row, starting at the aisle, so the first files are closest.
  const rackPages = Math.max(1, Math.ceil(floor.cargo.length / RACK_PAGE));
  const rp = Math.min(rackPage, rackPages - 1);
  const pageCargo = floor.cargo.slice(rp * RACK_PAGE, (rp + 1) * RACK_PAGE);
  const cols = [335, 215, 95];
  const slots = pageCargo.map((cargo, i) => ({
    cargo,
    x: cols[i % 3],
    y: 118 + Math.floor(i / 3) * 116,
  }));

  // Bays: three along the top wall, three along the bottom, openings facing the aisle.
  const all: { bay: BayInfo; kind: "sub" | "parent" }[] = floor.bays.map((bay) => ({ bay, kind: "sub" as const }));
  const perPage = floor.parent ? BAY_PAGE - 1 : BAY_PAGE;
  const bayPages = Math.max(1, Math.ceil(all.length / perPage));
  const bp = Math.min(bayPage, bayPages - 1);
  const pageBays = all.slice(bp * perPage, (bp + 1) * perPage);
  if (floor.parent) pageBays.unshift({ bay: floor.parent, kind: "parent" });

  const bays: BaySlot[] = [];
  for (let col = 0; col <= 3; col++) {
    const x = BAY_X0 + col * BAY_W - 4;
    walls.push({ x, y: WALL, w: 8, h: BAY_DEPTH - 6 });
    walls.push({ x, y: FLOOR_H - WALL - BAY_DEPTH + 6, w: 8, h: BAY_DEPTH - 6 });
  }
  pageBays.forEach((b, i) => {
    const side = i < 3 ? "top" : "bottom";
    const x0 = BAY_X0 + (i % 3) * BAY_W;
    const area: Rect =
      side === "top"
        ? { x: x0 + 4, y: WALL, w: BAY_W - 8, h: BAY_DEPTH }
        : { x: x0 + 4, y: FLOOR_H - WALL - BAY_DEPTH, w: BAY_W - 8, h: BAY_DEPTH };
    const drop: Rect = { x: area.x + 26, y: area.y + (side === "top" ? 46 : 22), w: area.w - 52, h: area.h - 70 };
    bays.push({
      ...b,
      area,
      drop,
      side,
      sign: { x: area.x + area.w / 2, y: side === "top" ? WALL + 16 : FLOOR_H - WALL - 16 },
    });
  });

  const duplicator = {
    body: { x: 60, y: FLOOR_H - WALL - 74, w: 150, h: 74 },
    intake: { x: 88, y: FLOOR_H - WALL - 74 - 62, w: 94, h: 56 },
  };
  walls.push(duplicator.body);

  const pillars: Rect[] = [
    // In the gaps between rack rows, so every row has a clear approach.
    { x: 455, y: 165, w: 22, h: 22 },
    { x: 455, y: 397, w: 22, h: 22 },
  ];
  walls.push(...pillars);

  return {
    walls,
    slots,
    bays,
    receiving: { x: 34, y: 54, w: 370, h: 470 },
    duplicator,
    pillars,
    spawn: { x: 420, y: 360, heading: Math.PI },
    rackPages,
    bayPages,
  };
}
