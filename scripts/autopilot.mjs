// Keyboard autopilot for end-to-end checks and the demo recording. It runs in
// the page every animation frame and drives only by dispatching keyboard
// events, the same input path a person uses. It reads game state to decide
// which keys to hold; it never edits game state.

function install() {
  if (window.__autopilot) return;
  const game = () => window.__pallet.app.game;
  const held = new Set();
  const key = (code, down) => {
    if (down === held.has(code)) return;
    if (down) held.add(code);
    else held.delete(code);
    window.dispatchEvent(new KeyboardEvent(down ? "keydown" : "keyup", { code, key: code, bubbles: true }));
  };
  const holdOnly = (codes) => {
    for (const c of [...held]) if (!codes.includes(c)) key(c, false);
    for (const c of codes) key(c, true);
  };
  const wrap = (a) => Math.atan2(Math.sin(a), Math.cos(a));
  const frame = () => new Promise((r) => requestAnimationFrame(r));
  const BRAKE = 620;

  async function stop() {
    holdOnly(["Space"]);
    for (let i = 0; i < 240 && Math.abs(game().fork.speed) > 1; i++) await frame();
    holdOnly([]);
  }

  async function goto(x, y, opts) {
    // Options may arrive as null after JSON transport (WebDriver), so default here.
    const { tol = 8, slow = 80, reverse: rev = null, careful = false, timeout = 15000 } = opts ?? {};
    const t0 = performance.now();
    // Unless told, reverse when the target is behind and close.
    const f0 = game().fork;
    const behind = Math.abs(wrap(Math.atan2(y - f0.y, x - f0.x) - f0.heading)) > 2.0;
    const reverse = rev ?? (behind && Math.hypot(x - f0.x, y - f0.y) < 220);
    while (performance.now() - t0 < timeout) {
      const f = game().fork;
      const d = Math.hypot(x - f.x, y - f.y);
      if (d < tol) break;
      if (d <= (f.speed * f.speed) / (2 * BRAKE) + tol * 0.4 && Math.abs(f.speed) > 5) {
        holdOnly(["Space"]);
        await frame();
        continue;
      }
      const want = Math.atan2(y - f.y, x - f.x) + (reverse ? Math.PI : 0);
      const err = wrap(want - f.heading);
      const steer = reverse ? -err : err;
      const keys = [reverse ? "KeyS" : "KeyW"];
      if (steer > 0.03) keys.push("KeyD");
      else if (steer < -0.03) keys.push("KeyA");
      if (d < slow || careful || Math.abs(err) > 0.8) keys.push("ShiftLeft");
      holdOnly(keys);
      await frame();
    }
    await stop();
  }

  /** Creeps until the fork tip sits on (x, y), steering at the target. */
  async function dock(x, y, opts) {
    const { tol = 6, timeout = 10000 } = opts ?? {};
    const t0 = performance.now();
    while (performance.now() - t0 < timeout) {
      const f = game().fork;
      const fx = f.x + Math.cos(f.heading) * 38;
      const fy = f.y + Math.sin(f.heading) * 38;
      if (Math.hypot(x - fx, y - fy) < tol) break;
      const err = wrap(Math.atan2(y - f.y, x - f.x) - f.heading);
      let keys;
      if (Math.abs(err) > 0.45) {
        // Badly misaligned: back up with opposite lock, which swings the forks toward the target.
        keys = ["KeyS", "ShiftLeft", err > 0 ? "KeyA" : "KeyD"];
      } else {
        keys = ["KeyW", "ShiftLeft"];
        if (err > 0.02) keys.push("KeyD");
        else if (err < -0.02) keys.push("KeyA");
      }
      holdOnly(keys);
      await frame();
    }
    await stop();
  }

  async function hold(codes, ms) {
    holdOnly(codes);
    const t0 = performance.now();
    while (performance.now() - t0 < ms) await frame();
  }

  async function tap(code, ms) {
    ms = ms ?? 250;
    key(code, true);
    await frame();
    key(code, false);
    const t0 = performance.now();
    while (performance.now() - t0 < ms) await frame();
  }

  window.__autopilot = { goto, dock, stop, hold, tap, release: () => holdOnly([]) };
}

export function pilot(page) {
  const call = async (name, ...args) => {
    await page.evaluate(install);
    return page.evaluate(([n, a]) => window.__autopilot[n](...a), [name, args]);
  };
  const state = () =>
    page.evaluate(() => {
      const g = window.__pallet.app.game;
      const f = g.fork;
      return { x: f.x, y: f.y, h: f.heading, v: f.speed, fork: f.fork, tipped: !!f.tipped, load: g.load.map((c) => c.name), pending: g.pending };
    });
  return {
    state,
    goto: (x, y, o) => call("goto", x, y, o),
    dock: (x, y, o) => call("dock", x, y, o),
    stop: () => call("stop"),
    hold: (codes, ms = 0) => call("hold", codes, ms),
    tap: (code, ms) => call("tap", code === "e" ? "KeyE" : code, ms),
    release: () => call("release"),
  };
}

/** Crate and bay positions as the game laid them out. */
export function where(page) {
  return page.evaluate(() => {
    const g = window.__pallet.app.game;
    return {
      crates: Object.fromEntries(g.crates().map((c) => [c.cargo.name, { x: c.x, y: c.y }])),
      bays: Object.fromEntries(g.layout.bays.map((b) => [b.bay.name, { x: b.drop.x + b.drop.w / 2, y: b.drop.y + b.drop.h / 2, side: b.side }])),
    };
  });
}
