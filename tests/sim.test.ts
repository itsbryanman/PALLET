import { describe, expect, it } from "vitest";
import * as C from "../src/sim/constants";
import { crateMass, crateSize, formatBytes } from "../src/sim/crate";
import { type DriveInput, newForklift, step } from "../src/sim/forklift";

const open: never[] = [];
const one = { count: 1, mass: 1 };

function lcg(seed: number) {
  let s = seed >>> 0;
  return () => ((s = (Math.imul(s, 1664525) + 1013904223) >>> 0) / 2 ** 32);
}

function drive(inputs: DriveInput[], forksUp = true, load = one) {
  const f = newForklift(0, 0);
  f.fork = forksUp ? 1 : 0;
  f.forkTarget = f.fork;
  let tipAt = -1;
  inputs.forEach((input, i) => {
    for (const e of step(f, input, open, load)) if (e.kind === "tip" && tipAt < 0) tipAt = i;
  });
  return { f, tipAt };
}

const hold = (n: number, input: DriveInput) => Array.from({ length: n }, () => ({ ...input }));

describe("crate size", () => {
  it("follows the capped log curve", () => {
    expect(crateSize(0)).toEqual({ w: 28, h: 24 });
    expect(crateSize(1024 ** 3)).toEqual({ w: 64, h: 48 });
    expect(crateSize(50e9)).toEqual({ w: 64, h: 48 });
    const mid = crateSize(1 << 20);
    expect(mid.w).toBeGreaterThan(28);
    expect(mid.w).toBeLessThan(64);
    expect(crateMass(0)).toBe(1);
    expect(crateMass(50e9)).toBe(4);
    expect(formatBytes(37)).toBe("37 B");
    expect(formatBytes(3 << 20)).toBe("3.00 MiB");
    expect(formatBytes(50e9)).toBe("46.6 GiB");
  });
});

describe("tipping", () => {
  const sharp: DriveInput = { throttle: 1, steer: 1, brake: false, precision: false };

  it("tips on a fast, sharp, forks-up turn after the hold time", () => {
    const { f, tipAt } = drive([...hold(90, { throttle: 1, steer: 0, brake: false, precision: false }), ...hold(60, sharp)]);
    expect(f.tipped?.cause).toBe("turn");
    // Steering ramps past 60% after ~7 frames, then 250 ms (15 frames) of hold.
    expect(tipAt).toBeGreaterThan(90 + 15);
    expect(tipAt).toBeLessThan(90 + 30);
  });

  it("doesn't tip with forks lowered, gentle steering, or at precision speed", () => {
    const straight = hold(90, { throttle: 1, steer: 0, brake: false, precision: false });
    expect(drive([...straight, ...hold(120, sharp)], false).f.tipped).toBeNull();
    expect(drive([...straight, ...hold(120, { ...sharp, steer: 0.5 })]).f.tipped).toBeNull();
    expect(drive(hold(200, { ...sharp, precision: true })).f.tipped).toBeNull();
  });

  it("a brief flick shorter than 250 ms doesn't tip", () => {
    const straight = hold(90, { throttle: 1, steer: 0, brake: false, precision: false });
    const flick = hold(14, sharp);
    const { f } = drive([...straight, ...flick, ...hold(30, { throttle: 1, steer: 0, brake: false, precision: false })]);
    expect(f.tipped).toBeNull();
  });

  it("taller stacks tip sooner", () => {
    const run = (count: number) => {
      const inputs = [...hold(40, { throttle: 1, steer: 0, brake: false, precision: false }), ...hold(80, { ...sharp, steer: 0.55 })];
      return drive(inputs, true, { count, mass: count }).f.tipped !== null;
    };
    expect(run(1)).toBe(false);
    expect(run(6)).toBe(true);
  });

  it("is deterministic: identical recorded inputs give identical incidents", () => {
    for (const seed of [1, 2, 3, 0xc0ffee]) {
      const r = lcg(seed);
      const inputs: DriveInput[] = Array.from({ length: 900 }, () => ({
        throttle: r() < 0.7 ? 1 : r() < 0.5 ? -1 : 0,
        steer: Math.round((r() * 2 - 1) * 4) / 4,
        brake: r() < 0.05,
        precision: r() < 0.1,
      }));
      const a = drive(inputs);
      const b = drive(inputs);
      expect(a).toEqual(b);
    }
  });
});

describe("walls", () => {
  const wall = [{ x: 100, y: -50, w: 20, h: 100 }];
  const ram = hold(120, { throttle: 1, steer: 0, brake: false, precision: false });

  it("a loaded full-speed impact tips; an empty one just bumps", () => {
    const loaded = newForklift(-140, 0);
    const empty = newForklift(-140, 0);
    const kinds = (f: ReturnType<typeof newForklift>, load: { count: number; mass: number }) =>
      ram.flatMap((i) => step(f, i, wall, load).map((e) => e.kind));
    expect(kinds(loaded, one)).toContain("tip");
    expect(loaded.tipped?.cause).toBe("wall");
    const e = kinds(empty, { count: 0, mass: 0 });
    expect(e).toContain("bump");
    expect(e).not.toContain("tip");
    expect(empty.x).toBeLessThan(100 - C.BODY_RADIUS + 1);
  });
});
