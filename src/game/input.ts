// Keyboard (remappable) and on-screen pedals. Preferences live in the WebView's
// localStorage: they are per-viewer conveniences, never file data.

import type { DriveInput } from "../sim/forklift";
import type { Action } from "../sim/game";

export const ACTIONS = {
  forward: "Forward",
  reverse: "Reverse",
  left: "Steer left",
  right: "Steer right",
  brake: "Brake",
  precision: "Precision speed",
  lift: "Lift / lower forks",
  stack: "Add / remove pallet crate",
  enter: "Go into a bay's floor",
  pause: "Pause",
  mute: "Mute",
} as const;

export type Control = keyof typeof ACTIONS;
export type Bindings = Record<Control, string[]>;

export const DEFAULT_BINDINGS: Bindings = {
  forward: ["KeyW", "ArrowUp"],
  reverse: ["KeyS", "ArrowDown"],
  left: ["KeyA", "ArrowLeft"],
  right: ["KeyD", "ArrowRight"],
  brake: ["Space"],
  precision: ["ShiftLeft", "ShiftRight"],
  lift: ["KeyE"],
  stack: ["KeyQ"],
  enter: ["Enter"],
  pause: ["Escape"],
  mute: ["KeyM"],
};

export function keyLabel(code: string): string {
  return code
    .replace(/^Key/, "")
    .replace(/^Digit/, "")
    .replace(/^Arrow(.*)/, "$1 arrow")
    .replace(/(Left|Right)$/, " $1")
    .trim();
}

export class Input {
  bindings: Bindings;
  private down = new Set<string>();
  private held = new Set<Control>();
  private taps: Action[] = [];
  onCommand: (c: "pause" | "mute") => void = () => {};
  /** While set, the next key press is captured for remapping instead. */
  capture: ((code: string) => void) | null = null;

  constructor(bindings: Bindings) {
    this.bindings = bindings;
    window.addEventListener("keydown", (e) => this.key(e, true));
    window.addEventListener("keyup", (e) => this.key(e, false));
    window.addEventListener("blur", () => {
      this.down.clear();
      this.held.clear();
    });
  }

  private controlFor(code: string): Control | null {
    for (const [c, codes] of Object.entries(this.bindings)) if (codes.includes(code)) return c as Control;
    return null;
  }

  private key(e: KeyboardEvent, isDown: boolean): void {
    if (this.capture && isDown) {
      e.preventDefault();
      const cb = this.capture;
      this.capture = null;
      cb(e.code);
      return;
    }
    // Releases always count, or a key let go while a document was open would
    // stay "held" and the forklift would drive off when it closes.
    if (!isDown) {
      this.down.delete(e.code);
      return;
    }
    // Typing in a form or using an open document must not drive the forklift.
    const t = e.target instanceof HTMLElement ? e.target : null;
    if (t && (t.closest("dialog[open]") || /^(INPUT|SELECT|TEXTAREA)$/.test(t.tagName))) return;
    if (document.querySelector("dialog[open]")) return;
    const c = this.controlFor(e.code);
    if (!c) return;
    e.preventDefault();
    if (this.down.has(e.code)) return;
    this.down.add(e.code);
    if (c === "lift" || c === "stack" || c === "enter") this.taps.push(c);
    if (c === "pause" || c === "mute") this.onCommand(c);
  }

  private active(c: Control): boolean {
    return this.held.has(c) || this.bindings[c].some((k) => this.down.has(k));
  }

  hold(c: Control, on: boolean): void {
    if (on) this.held.add(c);
    else this.held.delete(c);
  }

  tap(a: Action): void {
    this.taps.push(a);
  }

  drive(): DriveInput {
    return {
      throttle: (this.active("forward") ? 1 : 0) - (this.active("reverse") ? 1 : 0),
      steer: (this.active("right") ? 1 : 0) - (this.active("left") ? 1 : 0),
      brake: this.active("brake"),
      precision: this.active("precision"),
    };
  }

  takeTaps(): Action[] {
    const t = this.taps;
    this.taps = [];
    return t;
  }

  clear(): void {
    this.down.clear();
    this.held.clear();
    this.taps = [];
  }
}

export interface Prefs {
  muted: boolean;
  reducedMotion: boolean;
  pedals: boolean;
  bindings: Bindings;
}

const KEY = "pallet.prefs.v1";

export function loadPrefs(): Prefs {
  const base: Prefs = {
    muted: false,
    reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches,
    pedals: matchMedia("(pointer: coarse)").matches,
    bindings: structuredClone(DEFAULT_BINDINGS),
  };
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return base;
    const p = JSON.parse(raw) as Partial<Prefs>;
    return { ...base, ...p, bindings: { ...base.bindings, ...(p.bindings ?? {}) } };
  } catch {
    return base;
  }
}

export function savePrefs(p: Prefs): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(p));
  } catch {
    // Private windows and locked-down WebViews: preferences just won't persist.
  }
}
