import { describe, expect, it } from "vitest";
import { DIRECTIONS, HYSTERESIS, nearestFrame, pickFrame } from "../src/sim/direction";

const deg = (d: number) => (d * Math.PI) / 180;

describe("eight-direction frame selection", () => {
  it("maps headings to fork directions with y pointing down", () => {
    const at = (d: number) => DIRECTIONS[nearestFrame(deg(d))];
    expect(at(0)).toBe("e");
    expect(at(45)).toBe("se");
    expect(at(90)).toBe("s");
    expect(at(135)).toBe("sw");
    expect(at(180)).toBe("w");
    expect(at(-180)).toBe("w");
    expect(at(-135)).toBe("nw");
    expect(at(-90)).toBe("n");
    expect(at(-45)).toBe("ne");
    expect(at(359)).toBe("e");
  });

  it("holds the current frame a little past the sector edge", () => {
    const edge = deg(22.5);
    expect(pickFrame(edge + HYSTERESIS * 0.5, 0)).toBe(0);
    expect(pickFrame(edge + HYSTERESIS * 1.5, 0)).toBe(1);
    expect(pickFrame(edge - HYSTERESIS * 0.5, 1)).toBe(1);
    expect(pickFrame(edge - HYSTERESIS * 1.5, 1)).toBe(0);
    // No flicker while a heading jitters across the boundary.
    let f: number | null = null;
    const seen = new Set<number>();
    for (let i = 0; i < 200; i++) {
      f = pickFrame(edge + Math.sin(i) * HYSTERESIS * 0.9, f ?? 0);
      seen.add(f);
    }
    expect(seen.size).toBe(1);
  });
});
