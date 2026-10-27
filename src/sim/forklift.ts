// Fixed-timestep forklift model. Pure arithmetic: the same inputs from the same
// state always produce the same result, which is what makes incidents
// reproducible and testable. Nothing here can reach the filesystem.

import * as C from "./constants";

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface DriveInput {
  throttle: number; // -1 reverse, 0, 1 forward
  steer: number; // -1 left .. 1 right
  brake: boolean;
  precision: boolean;
}

export interface Forklift {
  x: number;
  y: number;
  heading: number; // radians, 0 faces +x; forks are at the front
  speed: number; // px/s along heading; negative is reverse
  steer: number; // -1..1, smoothed
  fork: number; // 0 lowered .. 1 raised
  forkTarget: number;
  risk: number; // seconds spent in the tipping envelope
  tipped: null | { cause: "turn" | "wall"; side: number; speed: number };
}

export interface LoadInfo {
  count: number;
  mass: number; // summed crate mass units
}

export type DriveEvent =
  | { kind: "bump"; impact: number }
  | { kind: "tip"; cause: "turn" | "wall"; impact: number };

export function newForklift(x: number, y: number, heading = 0): Forklift {
  return { x, y, heading, speed: 0, steer: 0, fork: 0, forkTarget: 0, risk: 0, tipped: null };
}

function approach(v: number, target: number, rate: number): number {
  if (v < target) return Math.min(target, v + rate);
  return Math.max(target, v - rate);
}

export function thresholds(load: LoadInfo): { speed: number; steer: number } {
  const extra = Math.max(0, load.count - 1);
  const heavy = Math.max(0, load.mass - load.count);
  return {
    speed: C.TIP_SPEED - C.STACK_PENALTY_SPEED * extra - C.MASS_PENALTY * heavy,
    steer: C.TIP_STEER - C.STACK_PENALTY_STEER * extra - C.MASS_PENALTY * heavy,
  };
}

/**
 * How close the forklift is to tipping, 0 (planted) to 1 (going over).
 * Drives the wobble animation and the stability gauge.
 */
export function instability(f: Forklift, load: LoadInfo): number {
  if (f.tipped) return 1;
  const t = thresholds(load);
  const speed = Math.abs(f.speed) / C.MAX_SPEED / t.speed;
  const steer = Math.abs(f.steer) / t.steer;
  const forks = f.fork > C.TIP_FORK ? 1 : 0.35;
  const base = Math.min(1, speed * steer * forks * 0.85);
  return Math.min(1, Math.max(base, f.risk / C.TIP_HOLD));
}

function hits(x: number, y: number, r: number, o: Rect): boolean {
  const cx = Math.max(o.x, Math.min(x, o.x + o.w));
  const cy = Math.max(o.y, Math.min(y, o.y + o.h));
  const dx = x - cx;
  const dy = y - cy;
  return dx * dx + dy * dy < r * r;
}

function blocked(x: number, y: number, walls: Rect[]): boolean {
  for (const w of walls) if (hits(x, y, C.BODY_RADIUS, w)) return true;
  return false;
}

/** Advances one fixed step of `C.DT` seconds. Mutates `f`; returns events. */
export function step(f: Forklift, input: DriveInput, walls: Rect[], load: LoadInfo): DriveEvent[] {
  const dt = C.DT;
  const events: DriveEvent[] = [];
  f.fork = approach(f.fork, f.forkTarget, C.FORK_RATE * dt);

  if (f.tipped) {
    f.speed = approach(f.speed, 0, C.BRAKE * dt);
    return events;
  }

  const cap = input.precision ? C.PRECISION : 1;
  if (input.brake) {
    f.speed = approach(f.speed, 0, C.BRAKE * dt);
  } else if (input.throttle > 0) {
    f.speed = f.speed < 0 ? approach(f.speed, 0, C.BRAKE * dt) : Math.min(C.MAX_SPEED * cap, f.speed + C.ACCEL * dt);
  } else if (input.throttle < 0) {
    f.speed = f.speed > 0 ? approach(f.speed, 0, C.BRAKE * dt) : Math.max(-C.MAX_REVERSE * cap, f.speed - C.REVERSE_ACCEL * dt);
  } else {
    f.speed = approach(f.speed, 0, C.COAST * dt);
  }
  // Releasing Shift at speed sheds the excess smoothly rather than snapping.
  if (Math.abs(f.speed) > C.MAX_SPEED * cap) f.speed = approach(f.speed, Math.sign(f.speed) * C.MAX_SPEED * cap, C.BRAKE * dt);

  const steerTarget = Math.max(-1, Math.min(1, input.steer));
  f.steer = approach(f.steer, steerTarget, (steerTarget === 0 ? C.STEER_RETURN : C.STEER_RATE) * dt);

  f.heading += (f.speed / C.WHEELBASE) * Math.tan(f.steer * C.MAX_STEER) * dt;
  if (f.heading > Math.PI) f.heading -= 2 * Math.PI;
  if (f.heading < -Math.PI) f.heading += 2 * Math.PI;

  const nx = f.x + Math.cos(f.heading) * f.speed * dt;
  const ny = f.y + Math.sin(f.heading) * f.speed * dt;
  if (!blocked(nx, ny, walls)) {
    f.x = nx;
    f.y = ny;
  } else {
    const impact = Math.abs(f.speed) / C.MAX_SPEED;
    // Slide along whichever axis is free, at reduced speed.
    if (!blocked(nx, f.y, walls)) f.x = nx;
    else if (!blocked(f.x, ny, walls)) f.y = ny;
    if (impact > C.WALL_TIP_SPEED && load.count > 0) {
      f.tipped = { cause: "wall", side: f.steer >= 0 ? 1 : -1, speed: f.speed };
      f.speed *= 0.2;
      events.push({ kind: "tip", cause: "wall", impact });
      return events;
    }
    f.speed *= -0.15;
    if (impact > C.BUMP_SPEED) events.push({ kind: "bump", impact });
  }

  const t = thresholds(load);
  const risky = Math.abs(f.speed) / C.MAX_SPEED > t.speed && f.fork > C.TIP_FORK && Math.abs(f.steer) > t.steer;
  f.risk = risky ? f.risk + dt : 0;
  if (f.risk >= C.TIP_HOLD - 1e-9) {
    f.tipped = { cause: "turn", side: f.steer >= 0 ? 1 : -1, speed: f.speed };
    events.push({ kind: "tip", cause: "turn", impact: Math.abs(f.speed) / C.MAX_SPEED });
  }
  return events;
}

/** The point just ahead of the forks where a crate sits when carried. */
export function forkPoint(f: Forklift): { x: number; y: number } {
  return { x: f.x + Math.cos(f.heading) * C.FORK_REACH, y: f.y + Math.sin(f.heading) * C.FORK_REACH };
}
