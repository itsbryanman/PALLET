// Picks one of eight pre-rendered directions from a continuous heading.
// Convention: heading 0 points +x (east), y grows downward, so +PI/2 is south.
// Frame i covers heading i * PI/4; the order is E, SE, S, SW, W, NW, N, NE.

export const DIRECTIONS = ["e", "se", "s", "sw", "w", "nw", "n", "ne"] as const;
export type Direction = (typeof DIRECTIONS)[number];

const STEP = Math.PI / 4;
/** Extra angle past a sector edge before switching, so a heading wobbling on a boundary doesn't flicker. */
export const HYSTERESIS = 0.07;

function diff(a: number, b: number): number {
  return Math.abs(Math.atan2(Math.sin(a - b), Math.cos(a - b)));
}

export function nearestFrame(heading: number): number {
  return (((Math.round(heading / STEP) % 8) + 8) % 8);
}

export function pickFrame(heading: number, current: number | null): number {
  if (current !== null && diff(heading, current * STEP) <= STEP / 2 + HYSTERESIS) return current;
  return nearestFrame(heading);
}

export function frameAngle(i: number): number {
  return i * STEP;
}
