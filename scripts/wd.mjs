// Minimal WebDriver client for the real Tauri app (via tauri-driver), shaped
// like the subset of Playwright's `page` that the e2e scenarios use.
import { writeFileSync } from "node:fs";

export async function openApp(application, { port = 4444 } = {}) {
  const base = `http://127.0.0.1:${port}`;
  const call = async (method, path, body) => {
    const res = await fetch(base + path, { method, headers: { "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
    const json = await res.json();
    if (json.value && json.value.error) throw new Error(`${json.value.error}: ${json.value.message}`);
    return json.value;
  };
  const session = await call("POST", "/session", { capabilities: { alwaysMatch: { "tauri:options": { application } } } });
  const sid = session.sessionId;
  const s = (p) => `/session/${sid}${p}`;
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  // Runs `fn(arg)` in the page and returns its (awaited) result.
  const evaluate = async (fn, arg) => {
    const script = `const done = arguments[arguments.length - 1];
      Promise.resolve((${fn.toString()})(arguments[0])).then((v) => done({ ok: v === undefined ? null : v }), (e) => done({ err: String(e && e.message ? e.message : e) }));`;
    const r = await call("POST", s("/execute/async"), { script, args: [arg ?? null] });
    if (r && r.err) throw new Error(r.err);
    return r ? r.ok : null;
  };
  await call("POST", s("/timeouts"), { script: 600000 });

  const waitFor = async (pred, timeout = 30000, what = "condition") => {
    const t0 = Date.now();
    while (Date.now() - t0 < timeout) {
      if (await pred()) return;
      await sleep(100);
    }
    throw new Error(`timed out waiting for ${what}`);
  };
  const visible = (sel) =>
    evaluate((q) => {
      const el = document.querySelector(q);
      return !!el && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";
    }, sel);
  const clickButton = (name, index = 0) =>
    evaluate(([n, i]) => {
      const scope = document.querySelector("dialog[open]") ?? document;
      const btns = [...scope.querySelectorAll("button")].filter((b) => b.textContent.trim() === n && !b.disabled);
      if (!btns[i]) throw new Error(`no button ${n}`);
      btns[i].click();
    }, [name, index]);

  const page = {
    evaluate,
    waitForTimeout: sleep,
    waitForSelector: (sel, o = {}) => waitFor(() => visible(sel), o.timeout ?? 30000, sel),
    waitForFunction: (fn, arg, o = {}) => waitFor(() => evaluate(fn, arg), o?.timeout ?? 30000, "function"),
    textContent: (sel) => evaluate((q) => document.querySelector(q)?.textContent ?? null, sel),
    click: (sel) => evaluate((q) => document.querySelector(q).click(), sel),
    getByRole: (_role, { name }) => {
      const loc = (i) => ({
        click: async () => {
          // Dialogs open after async IPC; wait for the button like Playwright does.
          const t0 = Date.now();
          for (;;) {
            try {
              return await clickButton(name, i);
            } catch (err) {
              if (Date.now() - t0 > 15000) throw err;
              await sleep(150);
            }
          }
        },
      });
      return { ...loc(0), first: () => loc(0) };
    },
    screenshot: async ({ path }) => {
      const b64 = await call("GET", s("/screenshot"));
      writeFileSync(path, Buffer.from(b64, "base64"));
    },
    close: () => call("DELETE", s("")),
  };
  await waitFor(() => evaluate(() => !!window.__pallet), 30000, "app start");
  return page;
}
