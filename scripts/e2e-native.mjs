// Runs the end-to-end loop against the real Tauri app on real files, then
// checks the sample warehouse on disk. Needs tauri-driver on port 4444 (see
// docs/DEVELOPMENT.md) and a built app binary.
//
// usage: node scripts/e2e-native.mjs <app-binary> <sample-warehouse-dir> [shots-dir]
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import run from "./e2e-loop.mjs";
import { openApp } from "./wd.mjs";

const [app, warehouse, shotsDir] = process.argv.slice(2);
const page = await openApp(app);

function disk() {
  const out = {};
  const walk = (dir, rel) => {
    for (const name of readdirSync(dir).sort()) {
      if (rel === "" && name === ".pallet") continue;
      const p = join(dir, name);
      const st = statSync(p);
      const r = rel ? `${rel}/${name}` : name;
      if (st.isDirectory()) walk(p, r);
      else out[r] = `${st.ino}:${readFileSync(p).toString("base64").slice(0, 64)}`;
    }
  };
  walk(warehouse, "");
  return out;
}

const mode = await page.textContent("#mode");
console.log("[native] mode chip:", mode);
if (!/Real files/.test(mode)) throw new Error("app is not in real-file mode");
const before = disk();
if (!before["hello.txt"]) throw new Error("sample warehouse has no hello.txt");

let failed = null;
try {
  await run(page, {
    fsSnap: async () => JSON.stringify(disk()),
    afterDelivery: () => {
      if (existsSync(join(warehouse, "hello.txt")) || !existsSync(join(warehouse, "Documents/hello.txt"))) throw new Error("real file did not move");
      if (disk()["Documents/hello.txt"].split(":")[0] !== before["hello.txt"].split(":")[0]) throw new Error("inode changed: not a rename");
      console.log("[native] on disk: Documents/hello.txt, same inode");
    },
    shots: shotsDir ? (name) => page.screenshot({ path: `${shotsDir}/native-${name}.png` }) : undefined,
  });
} catch (err) {
  failed = err;
}
const after = disk();
const sameSet = JSON.stringify(Object.keys(after).filter((k) => !k.startsWith("incident-report-")).sort()) === JSON.stringify(Object.keys(before).sort());
console.log("[native] files after run match files before:", sameSet);
const journal = readFileSync(join(warehouse, ".pallet/journal.log"));
console.log("[native] journal bytes:", journal.length);
await page.close();
if (failed) throw failed;
if (!sameSet) throw new Error("warehouse contents differ after the round trip");
console.log("[native] all good");
