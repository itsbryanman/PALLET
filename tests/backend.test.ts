// The renderer can't bypass validation: every mutation goes through a backend
// that checks IDs and names itself, and a lost acknowledgment never retries.
import { afterEach, describe, expect, it } from "vitest";
import { DemoBackend } from "../src/backend/demo";

async function ids(b: DemoBackend) {
  const f = await b.listFloor(null);
  const c = (n: string) => f.cargo.find((x) => x.name === n)!.id;
  const bay = (n: string) => f.bays.find((x) => x.name === n)!.id;
  return { f, c, bay };
}

describe("demo backend follows the core's rules", () => {
  it("moves, refuses collisions with a suggestion, and undoes", async () => {
    const b = new DemoBackend();
    const { c, bay } = await ids(b);
    const before = b.snapshot();
    const o = await b.deliver("t1", c("hello.txt"), bay("Documents"), null);
    expect(o.status).toBe("committed");
    expect(o.to).toBe("Documents/hello.txt");
    expect([...b.snapshot().keys()].sort()).toEqual([...before.keys()].sort());

    const u = await b.undo("u1", "t1", null);
    expect(u.status).toBe("committed");
    expect(b.snapshot()).toEqual(before);
    expect((await b.undo("u2", "t1", null)).code).toBe("ALREADY_UNDONE");

    await b.deliver("t2", c("hello.txt"), bay("Archive"), null);
    const { c: c2, bay: bay2 } = await ids(b);
    const docsFloor = await b.listFloor(bay2("Documents"));
    const notes = docsFloor.cargo.find((x) => x.name === "notes.txt")!.id;
    const archiveFloor = await b.listFloor(bay2("Archive"));
    expect(archiveFloor.cargo.map((x) => x.name)).toContain("hello.txt");
    const x = await b.deliver("t3", c2("inventory.csv"), bay2("Documents"), "notes.txt");
    expect(x.code).toBe("NAME_EXISTS");
    expect(x.suggestion).toBe("notes (2).txt");
    expect(notes).toBeTruthy();
  });

  it("rejects forged IDs and path-like names", async () => {
    const b = new DemoBackend();
    const { c, bay } = await ids(b);
    expect((await b.deliver("a", "forged", bay("Documents"), null)).code).toBe("UNKNOWN_CARGO");
    expect((await b.deliver("b", c("hello.txt"), "/etc", null)).code).toBe("UNKNOWN_BAY");
    for (const [i, bad] of ["../x", "a/b", "..", ""].entries()) {
      expect((await b.deliver(`n${i}`, c("hello.txt"), bay("Documents"), bad)).code).toBe("BAD_NAME");
    }
    const root = (await b.listFloor(null)).id;
    const docs = await b.listFloor(bay("Documents"));
    expect((await b.deliver("r", docs.cargo[0].id, root, ".pallet")).code).toBe("BAD_NAME");
  });

  it("replays a transaction ID instead of performing it twice", async () => {
    const b = new DemoBackend();
    const { c, bay } = await ids(b);
    await b.deliver("same", c("hello.txt"), bay("Documents"), null);
    const snap = b.snapshot();
    const again = await b.deliver("same", c("hello.txt"), bay("Photos"), null);
    expect(again.replayed).toBe(true);
    expect(b.snapshot()).toEqual(snap);
  });

  it("stops a pallet at the first failure and reports exactly what moved", async () => {
    const b = new DemoBackend();
    const { c, bay } = await ids(b);
    const photos = bay("Photos");
    // Something named hello.txt is already in Photos.
    await b.deliver("blocker", c("old-backups.zip"), photos, "hello.txt");
    const names = ["empty-envelope.txt", "forklift-manual.md", "hello.txt", "inventory.csv"];
    const items: [string, string][] = names.map((n, i) => [`p${i}`, c(n)]);
    const out = await b.deliverBatch("pallet", items, photos);
    expect(out.map((o) => o.status)).toEqual(["committed", "committed", "refused"]);
    expect(out[2].code).toBe("NAME_EXISTS");
    const left = (await b.listFloor(null)).cargo.map((x) => x.name);
    expect(left).toContain("hello.txt");
    expect(left).toContain("inventory.csv");
    expect(left).not.toContain("empty-envelope.txt");
  });

  it("copies with (copy) names and undo-copy never deletes", async () => {
    const b = new DemoBackend();
    const { c } = await ids(b);
    const o = await b.duplicate("c1", c("hello.txt"), null);
    expect(o.to).toBe("hello (copy).txt");
    const u = await b.undo("u1", "c1", null);
    expect(u.to).toBe("Recovered Copies/hello (copy).txt");
    expect([...b.snapshot().values()]).toContain("Recovered Copies/hello (copy).txt");
  });
});

describe("native backend wrapper", () => {
  afterEach(() => {
    delete (globalThis as Record<string, unknown>).window;
  });

  it("asks for status after a lost acknowledgment instead of moving again", async () => {
    const calls: string[] = [];
    (globalThis as Record<string, unknown>).window = globalThis;
    const { mockIPC, clearMocks } = await import("@tauri-apps/api/mocks");
    mockIPC((cmd) => {
      calls.push(cmd);
      if (cmd === "deliver") throw new Error("ipc dropped");
      if (cmd === "tx_status") return { tx: "t1", op: "move", status: "committed", code: null, message: null, from: "a", to: "b", suggestion: null, replayed: true };
      return null;
    });
    const { TauriBackend } = await import("../src/backend/tauri");
    const o = await new TauriBackend().deliver("t1", "c", "b", null);
    expect(o.status).toBe("committed");
    expect(calls).toEqual(["deliver", "tx_status"]);
    clearMocks();
  });
});
