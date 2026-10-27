// The smallest working loop, end to end through the real UI:
// pick up hello.txt, deliver it to Documents, confirm, undo, then tip the
// forklift and check the incident report changed nothing.
import { pilot, where } from "./autopilot.mjs";

export default async function run(page, { shots, fsSnap, afterDelivery } = {}) {
  const p = pilot(page);
  // Demo backend: its virtual inode table. Native: the caller reads the real disk.
  const snap = fsSnap ?? (() => page.evaluate(() => JSON.stringify([...window.__pallet.backend.snapshot().entries()])));
  const listing = async () => page.evaluate(async () => (await window.__pallet.backend.listFloor(null)).cargo.map((c) => c.name));
  const w = await where(page);
  const hello = w.crates["hello.txt"];
  const docs = w.bays["Documents"];
  const log = (...a) => console.log("[e2e]", ...a);
  // Line up and lift a crate in the rack, backing off and retrying like a driver would.
  async function pickup(name) {
    for (let attempt = 0; attempt < 3; attempt++) {
      const c = (await where(page)).crates[name];
      if (attempt === 0) await p.goto(c.x + 220, c.y, { tol: 14 });
      else await p.goto(c.x + 160, c.y, { tol: 12, reverse: true });
      await p.goto(c.x + 100, c.y, { tol: 8, slow: 140 });
      await p.dock(c.x, c.y);
      await p.tap("e", 600);
      const s = await p.state();
      if (s.load[0] === name) return;
      if (s.fork > 0.5) await p.tap("e", 600); // lower the empty forks again
    }
    throw new Error(`pickup of ${name} failed: ${JSON.stringify(await p.state())}`);
  }

  await pickup("hello.txt");
  let s = await p.state();
  log("picked up hello.txt");
  if (shots) await shots("pickup");

  // Back out and drive to the Documents drop zone, forks facing in.
  await p.goto(hello.x + 140, hello.y, { reverse: true, tol: 12, careful: true });
  await p.goto(docs.x, docs.y + 320, { tol: 14, careful: true });
  await p.goto(docs.x, docs.y + 170, { tol: 10, careful: true });
  await p.dock(docs.x, docs.y, { tol: 20 });
  await p.stop();
  await p.tap("e", 400);
  await page.waitForSelector("#docket[open]");
  if (shots) await shots("docket");
  await page.getByRole("button", { name: "Sign and deliver" }).click();
  await page.waitForSelector("#slip:not([hidden])");
  const slip = await page.textContent("#slip");
  log("slip:", slip);
  if (!/Documents\/hello\.txt/.test(slip)) throw new Error("delivery not confirmed");
  if ((await listing()).includes("hello.txt")) throw new Error("hello.txt still in Receiving");
  if (afterDelivery) await afterDelivery();
  if (shots) await shots("delivered");

  // Undo through the Logbook.
  await page.click("#btn-logbook");
  await page.getByRole("button", { name: "Undo" }).first().click();
  await page.getByRole("button", { name: "Sign and undo" }).click();
  await page.waitForFunction(() => /Undone/.test(document.querySelector("#slip")?.textContent ?? ""));
  if (!(await listing()).includes("hello.txt")) throw new Error("undo didn't bring hello.txt back");
  log("undo ok");

  // Tip it: pick up again, then a fast sharp turn with raised forks.
  const before = await snap();
  const w2 = await where(page);
  const h2 = w2.crates["hello.txt"];
  await pickup("hello.txt");
  await p.goto(h2.x + 150, h2.y, { reverse: true, tol: 12 });
  await p.goto(h2.x + 260, h2.y - 20, { tol: 14, careful: true });
  await p.hold(["KeyW"], 900);
  const turning = p.hold(["KeyW", "KeyA"], 2500).catch(() => {});
  await page.waitForSelector("#incident[open]", { timeout: 6000 });
  await turning;
  await p.release();
  if (shots) await shots("incident");
  const report = await page.textContent("#incident");
  if (!/Files damaged\s*0/.test(report)) throw new Error("incident report missing");
  const after = await snap();
  if (before !== after) throw new Error("tip changed files!");
  log("tip ok, files unchanged");
  await page.getByRole("button", { name: "Right the forklift" }).click();
  s = await p.state();
  if (s.tipped || s.load[0] !== "hello.txt") throw new Error("recovery failed");
  log("all good");
}
