// Dev helper: screenshot a running PALLET page with system Chromium.
// usage: node scripts/shot.mjs <url> <out.png> [width] [height] [script.js]
import { chromium } from "playwright-core";
const [url, out, w = "1280", h = "800", extra] = process.argv.slice(2);
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/usr/bin/chromium", args: ["--no-sandbox", "--use-gl=swiftshader", "--enable-unsafe-swiftshader"] });
const page = await browser.newPage({ viewport: { width: +w, height: +h } });
const logs = [];
page.on("console", (m) => logs.push(`${m.type()}: ${m.text()}`));
page.on("pageerror", (e) => logs.push(`pageerror: ${e.message}`));
await page.goto(url);
await page.waitForTimeout(1500);
if (extra) {
  const { default: run } = await import(new URL(extra, `file://${process.cwd()}/`).href);
  try {
    const dir = out.replace(/\.png$/, "");
    await run(page, { shots: async (name) => page.screenshot({ path: `${dir}-${name}.png` }) });
  } catch (err) {
    await page.screenshot({ path: out });
    console.log(logs.join("\n"));
    console.log(String(err));
    console.log(JSON.stringify(await page.evaluate(() => { const g = window.__pallet.app.game; return { f: g.fork, load: g.load.length, bay: g.bayUnderForks()?.bay.name ?? null, stopped: g.stopped() }; })));
    await browser.close();
    process.exit(1);
  }
}
await page.screenshot({ path: out });
console.log(logs.join("\n"));
await browser.close();
