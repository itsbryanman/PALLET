// Records a run of the game to a WebM with system Chromium.
// usage: node scripts/record.mjs <url> <out-dir> <scenario.mjs> [width] [height]
import { chromium } from "playwright-core";
const [url, outDir, scenario, w = "1280", h = "800"] = process.argv.slice(2);
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/usr/bin/chromium", args: ["--no-sandbox", "--use-gl=swiftshader", "--enable-unsafe-swiftshader", "--autoplay-policy=no-user-gesture-required"] });
const context = await browser.newContext({ viewport: { width: +w, height: +h }, recordVideo: { dir: outDir, size: { width: +w, height: +h } } });
const page = await context.newPage();
page.on("pageerror", (e) => console.log("pageerror:", e.message));
await page.goto(url);
await page.waitForFunction(() => window.__pallet, null, { timeout: 20000 });
await page.waitForTimeout(800);
const { default: run } = await import(new URL(scenario, `file://${process.cwd()}/`).href);
let failed = null;
try {
  await run(page, {});
} catch (err) {
  failed = err;
}
const video = page.video();
await context.close();
console.log("video:", await video.path());
await browser.close();
if (failed) {
  console.log(String(failed));
  process.exit(1);
}
