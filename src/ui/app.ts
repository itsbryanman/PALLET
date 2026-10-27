// The controller. Game events are intents; this file turns them into paperwork,
// and only a signed docket (or a confirmed Manifest/Logbook action) calls the
// backend. Tips, bumps, and recoveries never do (SAFETY.md S2, S7).

import type { Backend, BayInfo, CargoInfo, Floor, HistoryEntry, TxOutcome, WarehouseStatus } from "../backend/types";
import { newTxId } from "../backend/types";
import type { Renderer } from "../game/renderer";
import type { Sound } from "../game/audio";
import { ACTIONS, type Control, DEFAULT_BINDINGS, type Input, type Prefs, keyLabel, savePrefs } from "../game/input";
import { DT, MAX_SPEED } from "../sim/constants";
import { formatBytes } from "../sim/crate";
import { instability } from "../sim/forklift";
import { Game, type GameEvent } from "../sim/game";
import { incidentLines, incidentText } from "../sim/incident";
import { $, ask, fill, h } from "./dom";

const KMH = 0.065; // px/s to km/h: top speed is about 12 km/h

interface Target {
  bay: BayInfo;
  display: string;
}

export class App {
  private game!: Game;
  private status!: WarehouseStatus;
  private origin = new Map<string, string>();
  private incidents = 0;
  private acc = 0;
  private flash: { text: string; until: number } | null = null;
  private busy = false;

  constructor(
    private backend: Backend,
    private renderer: Renderer,
    private input: Input,
    private sound: Sound,
    private prefs: Prefs,
  ) {}

  async start(): Promise<void> {
    this.status = await this.backend.status();
    const floor = await this.backend.listFloor(null);
    this.game = new Game(floor);
    this.applyPrefs();
    this.wireChrome();
    this.renderChrome();
    this.renderer.app.ticker.add((t) => this.tick(t.deltaMS));
    if (this.status.frozen || this.status.recovery.length) await this.showRecovery();
  }

  // ---------- loop ----------

  private tick(ms: number): void {
    const dt = Math.min(ms, 100) / 1000;
    this.acc += dt;
    let taps = this.input.takeTaps();
    while (this.acc >= DT) {
      this.acc -= DT;
      const events = this.game.update(this.input.drive(), taps);
      taps = [];
      for (const e of events) void this.onEvent(e);
    }
    this.renderer.frame(this.game, dt);
    const f = this.game.fork;
    this.sound.drive(f.speed / MAX_SPEED, f.speed < -4 && !this.game.paused, ms);
    this.renderDash();
  }

  private say(text: string, seconds = 2.6): void {
    this.flash = { text, until: performance.now() + seconds * 1000 };
  }

  private async onEvent(e: GameEvent): Promise<void> {
    const g = this.game;
    switch (e.kind) {
      case "pickup":
      case "stack":
        this.sound.pickup();
        if (!this.origin.has(e.cargo.id)) this.origin.set(e.cargo.id, this.join(g.floor.displayPath, e.cargo.name));
        return;
      case "unstack":
      case "setDown":
        this.sound.setDown();
        if (e.kind === "setDown") this.say("Set down here. The files are still filed where they were.");
        return;
      case "forks":
        this.sound.forks(e.up);
        return;
      case "refuse":
        this.sound.refuse();
        this.say(e.reason);
        return;
      case "bump":
        this.sound.bump(e.impact);
        this.renderer.shake(e.impact * 9);
        return;
      case "tip":
        return this.onTip(e.cause, e.load);
      case "enterBay":
        return this.navigate(e.bay.bay.id);
      case "requestDelivery": {
        const display = e.bay.kind === "parent" ? this.parentDisplay() : this.join(g.floor.displayPath, e.bay.bay.name);
        return this.deliver({ bay: e.bay.bay, display }, e.load, true);
      }
      case "requestCopy":
        return this.duplicate(e.cargo, true);
    }
  }

  private join(dir: string, name: string): string {
    return dir.endsWith("/") ? dir + name : `${dir}/${name}`;
  }

  private parentDisplay(): string {
    const p = this.game.floor.displayPath;
    return p.slice(0, p.lastIndexOf("/")) || p;
  }

  private abs(rel: string | null): string {
    return rel === null ? "" : this.join(this.status.rootPath, rel);
  }

  // ---------- floors ----------

  private async refresh(): Promise<void> {
    try {
      const floor = await this.backend.listFloor(this.game.floor.id);
      this.game.setFloor(floor, true);
    } catch {
      await this.navigate(null);
    }
    this.renderChrome();
  }

  private async navigate(bay: string | null): Promise<void> {
    let floor: Floor;
    try {
      floor = await this.backend.listFloor(bay);
    } catch (err) {
      this.sound.refuse();
      this.say(`Can't open that floor: ${String(err)}`);
      return;
    }
    this.game.setFloor(floor, floor.id === this.game.floor.id);
    this.renderer.clearMarks();
    this.renderChrome();
  }

  // ---------- deliveries ----------

  private modeLine(): HTMLElement {
    return this.backend.mode === "real"
      ? h("p", { class: "notice" }, "These are real files on this computer. PALLET renames them in place; it never overwrites or deletes.")
      : h("p", { class: "notice ok" }, "Demo: these are virtual files that exist only in this tab.");
  }

  /** Delivers one crate or a pallet of up to six. Each crate is its own transaction. */
  private async deliver(target: Target, load: CargoInfo[], fromForks: boolean): Promise<void> {
    if (this.busy || this.game.paused) return;
    this.busy = true;
    this.game.pending = true;
    this.input.clear();
    try {
      const signed = await ask<boolean>(
        $("docket") as HTMLDialogElement,
        (close) => [
          h("h2", { id: "docket-title" }, load.length === 1 ? "Delivery docket" : `Delivery docket, ${load.length} crates`),
          h("p", { class: "sub" }, "Nothing moves until you sign."),
          h(
            "table",
            {},
            h("thead", {}, h("tr", {}, h("th", {}, "Crate"), h("th", {}, "From"), h("th", {}, "To"))),
            h(
              "tbody",
              {},
              ...load.map((c) =>
                h("tr", {}, h("td", {}, c.name, h("br"), h("span", { class: "meta" }, formatBytes(c.bytes))), h("td", {}, this.origin.get(c.id) ?? c.name), h("td", {}, this.join(target.display, c.name))),
              ),
            ),
          ),
          load.length > 1 ? h("p", { class: "fine" }, "Crates move one at a time. If one can't be delivered, PALLET stops there and tells you exactly which ones moved.") : null,
          this.modeLine(),
          h("div", { class: "actions" }, h("button", { type: "button", onclick: () => close(false) }, "Cancel"), h("button", { type: "button", class: "primary", onclick: () => close(true) }, "Sign and deliver")),
          h("p", { class: "signature" }, "Operator signature"),
        ],
        false,
      );
      if (!signed) {
        this.game.keepLoad();
        return;
      }
      let outcomes: TxOutcome[];
      if (load.length === 1) outcomes = [await this.backend.deliver(newTxId(), load[0].id, target.bay.id, null)];
      else outcomes = await this.backend.deliverBatch(newTxId(), load.map((c) => [newTxId(), c.id]), target.bay.id);
      await this.settle(target, load, outcomes, fromForks);
    } finally {
      this.game.pending = false;
      this.busy = false;
    }
  }

  private async settle(target: Target, load: CargoInfo[], outcomes: TxOutcome[], fromForks: boolean): Promise<void> {
    const moved = outcomes.filter((o) => o.status === "committed");
    const movedIds = load.slice(0, moved.length).map((c) => c.id);
    if (fromForks && moved.length) this.renderer.settle(this.game, target.bay.id, load.slice(0, moved.length));
    if (fromForks) this.game.delivered(movedIds);
    for (const id of movedIds) this.origin.delete(id);
    const failed = outcomes.find((o) => o.status !== "committed");
    if (moved.length) {
      this.sound.delivered();
      this.renderer.flashBay(target.bay.id, true);
      this.slip(
        moved.length === 1 ? "Delivery complete" : `${moved.length} crates delivered`,
        moved.flatMap((o) => [
          ["Before", this.abs(o.from)],
          ["After", this.abs(o.to)],
        ]),
        "ok",
      );
    }
    await this.refresh();
    if (!failed) return;
    this.renderer.flashBay(target.bay.id, false);
    this.sound.refuse();
    if (failed.status === "uncertain") {
      await this.showRecovery();
      return;
    }
    const crate = load[moved.length];
    const notAttempted = load.slice(moved.length + 1);
    if (failed.code === "NAME_EXISTS" && failed.suggestion && load.length === 1) {
      const name = await this.askName(
        "Name taken",
        `Something called ${crate.name} is already in ${target.bay.name}. PALLET never overwrites. Deliver under a different name?`,
        failed.suggestion,
      );
      if (name) {
        this.game.pending = true;
        const o = await this.backend.deliver(newTxId(), crate.id, target.bay.id, name);
        await this.settle(target, [crate], [o], fromForks);
      } else if (fromForks) this.game.keepLoad();
      return;
    }
    if (fromForks) this.game.keepLoad();
    await ask<null>(
      $("docket") as HTMLDialogElement,
      (close) => [
        h("h2", { id: "docket-title" }, "Delivery refused"),
        h("p", { class: "notice" }, failed.message ?? failed.code ?? "Refused."),
        h(
          "table",
          {},
          h(
            "tbody",
            {},
            ...load.map((c, i) =>
              h("tr", {}, h("td", {}, c.name), h("td", {}, i < moved.length ? "Delivered" : i === moved.length ? `Refused (${failed.code})` : "Not attempted, still on the forks")),
            ),
          ),
        ),
        notAttempted.length || moved.length ? h("p", { class: "fine" }, "Deliveries aren't all-or-nothing. The list above is exactly what happened.") : null,
        h("div", { class: "actions" }, h("button", { type: "button", class: "primary", onclick: () => close(null) }, "Back to the floor")),
      ],
      null,
    );
  }

  private askName(title: string, text: string, suggestion: string): Promise<string | null> {
    return ask<string | null>(
      $("docket") as HTMLDialogElement,
      (close) => {
        const input = h("input", { type: "text", value: suggestion, spellcheck: "false", "aria-label": "New name" }) as HTMLInputElement;
        const go = () => close(input.value.trim() || null);
        input.addEventListener("keydown", (e) => {
          if ((e as KeyboardEvent).key === "Enter") go();
        });
        return [
          h("h2", { id: "docket-title" }, title),
          h("p", { class: "sub" }, text),
          h("label", { class: "field" }, "Deliver as", input),
          h("div", { class: "actions" }, h("button", { type: "button", onclick: () => close(null) }, "Cancel"), h("button", { type: "button", class: "primary", onclick: go }, "Use this name")),
        ];
      },
      null,
    );
  }

  private async duplicate(cargo: CargoInfo, fromForks: boolean): Promise<void> {
    if (this.busy || this.game.paused) return;
    this.busy = true;
    this.game.pending = true;
    this.input.clear();
    try {
      const ok = await ask<boolean>(
        $("docket") as HTMLDialogElement,
        (close) => [
          h("h2", { id: "docket-title" }, "Duplicator work order"),
          h("p", { class: "sub" }, `Make a copy of ${cargo.name} next to the original. The original isn't changed.`),
          h("table", {}, h("tbody", {}, h("tr", {}, h("th", {}, "Original"), h("td", {}, this.origin.get(cargo.id) ?? cargo.name)), h("tr", {}, h("th", {}, "Size"), h("td", {}, formatBytes(cargo.bytes))))),
          h("p", { class: "fine" }, "The copy is written to a private temporary file, checked against a second read of the original, then published under a new name such as “name (copy).ext”. Undo moves the copy to a Recovered Copies bay; it never deletes it."),
          this.modeLine(),
          h("div", { class: "actions" }, h("button", { type: "button", onclick: () => close(false) }, "Cancel"), h("button", { type: "button", class: "primary", onclick: () => close(true) }, "Sign and duplicate")),
        ],
        false,
      );
      if (fromForks) this.game.keepLoad();
      if (!ok) return;
      const o = await this.backend.duplicate(newTxId(), cargo.id, null);
      if (o.status === "committed") {
        this.sound.delivered();
        this.slip("Copy made", [
          ["Original", this.abs(o.from)],
          ["Copy", this.abs(o.to)],
        ], "ok");
      } else if (o.status === "uncertain") {
        await this.showRecovery();
      } else {
        this.sound.refuse();
        this.slip("Duplicator refused", [["Why", o.message ?? o.code ?? ""]], "bad");
      }
      await this.refresh();
    } finally {
      this.game.pending = false;
      this.busy = false;
    }
  }

  // ---------- incidents ----------

  private async onTip(cause: "turn" | "wall", load: CargoInfo[]): Promise<void> {
    this.sound.crash();
    this.renderer.scatter(this.game, load);
    this.incidents++;
    const incident = {
      number: this.incidents,
      cause,
      speedKmh: Math.abs(this.game.fork.tipped?.speed ?? 0) * KMH,
      cargo: load.map((c) => c.name),
      forksRaised: this.game.fork.fork > 0.5,
    };
    const where = load.map((c) => this.origin.get(c.id) ?? c.name);
    await new Promise((r) => setTimeout(r, this.prefs.reducedMotion ? 300 : 1100));
    this.sound.stamp();
    const dialog = $("incident") as HTMLDialogElement;
    let filed: HTMLElement | null = null;
    await ask<null>(
      dialog,
      (close) => {
        const fileBtn = h("button", { type: "button" }, "File this report") as HTMLButtonElement;
        filed = h("p", { class: "fine", "aria-live": "polite" });
        fileBtn.addEventListener("click", async () => {
          fileBtn.disabled = true;
          const text = incidentText(incident, new Date(), where);
          const o = await this.backend.saveIncident(newTxId(), text);
          if (o.status === "committed") fill(filed!, `Filed as ${this.abs(o.to)}. It shows up in Receiving as a crate. Of course it does.`);
          else fill(filed!, `Couldn't file it: ${o.message ?? o.code}`);
        });
        return [
          h("div", { class: "form-no" }, h("span", {}, "Form PAL-1 (not a real form)"), h("span", {}, `No. ${String(incident.number).padStart(4, "0")}`)),
          h("h2", { id: "incident-title" }, "Fictional warehouse incident report"),
          h("div", { class: "stamp", "aria-hidden": "true" }, "LOAD INCIDENT"),
          h("dl", {}, ...incidentLines(incident).flatMap(([k, v]) => [h("dt", {}, k), h("dd", {}, v)])),
          h("p", { class: "fine" }, where.length ? `Tipping changes no files. ${where.length === 1 ? `${load[0].name} is still at ${where[0]}.` : "Every crate's file is still where it was."}` : "Tipping changes no files."),
          filed,
          h("div", { class: "actions" }, fileBtn, h("button", { type: "button", class: "primary", onclick: () => close(null) }, "Right the forklift")),
          h("p", { class: "fine" }, "Filing saves a new text file in the warehouse folder. Nothing is overwritten."),
        ];
      },
      null,
    );
    this.game.recover();
    this.renderer.recover();
    if (filed && (filed as HTMLElement).textContent) await this.refresh();
  }

  // ---------- drawers ----------

  private async openManifest(): Promise<void> {
    const floor = this.game.floor;
    const targets: Target[] = [];
    if (floor.parent) targets.push({ bay: floor.parent, display: this.parentDisplay() });
    for (const b of floor.bays) if (!b.unavailable) targets.push({ bay: b, display: this.join(floor.displayPath, b.name) });
    const onForks = new Set(this.game.load.map((c) => c.id));
    const rows = floor.cargo.filter((c) => !onForks.has(c.id));
    const choice = await ask<{ kind: "deliver"; ids: string[]; target: Target } | { kind: "copy"; cargo: CargoInfo } | null>(
      $("manifest") as HTMLDialogElement,
      (close) => {
        const boxes: HTMLInputElement[] = [];
        const select = h("select", { "aria-label": "Destination bay" }, ...targets.map((t, i) => h("option", { value: String(i) }, t.bay.id === floor.parent?.id ? `Up to ${t.bay.name}` : t.bay.name))) as HTMLSelectElement;
        const go = h("button", { type: "button", class: "btn primary", disabled: targets.length === 0 }, "Deliver selected") as HTMLButtonElement;
        go.addEventListener("click", () => {
          const ids = boxes.filter((b) => b.checked).map((b) => b.value);
          if (ids.length === 0 || ids.length > 6) return;
          close({ kind: "deliver", ids, target: targets[Number(select.value)] });
        });
        const count = h("span", { class: "meta" }, "None selected");
        const update = () => {
          const n = boxes.filter((b) => b.checked).length;
          fill(count, n === 0 ? "None selected" : n > 6 ? `${n} selected; a pallet holds six` : `${n} selected`);
          go.disabled = n === 0 || n > 6 || targets.length === 0;
        };
        return [
          h("h2", { id: "manifest-title" }, "Manifest"),
          h("p", { class: "sub" }, "Deliver without driving. Same paperwork, same rules."),
          h(
            "ul",
            { class: "list" },
            ...rows.map((c) => {
              const box = h("input", { type: "checkbox", value: c.id, disabled: !!c.unavailable, "aria-label": `Select ${c.name}` }) as HTMLInputElement;
              box.addEventListener("change", update);
              if (!c.unavailable) boxes.push(box);
              const dup = h("button", { type: "button", disabled: !!c.unavailable, onclick: () => close({ kind: "copy", cargo: c }) }, "Duplicate");
              return h("li", { "data-unavailable": c.unavailable ? "" : undefined }, box, h("span", {}, c.name, h("br"), h("span", { class: "meta" }, c.unavailable ?? formatBytes(c.bytes))), dup);
            }),
          ),
          rows.length === 0 ? h("p", { class: "sub" }, "No crates on this floor.") : null,
          targets.length ? h("label", { class: "field" }, "Deliver to", select) : h("p", { class: "sub" }, "This floor has no bays to deliver into."),
          h("div", { class: "actions" }, count, h("button", { type: "button", class: "btn", onclick: () => close(null) }, "Close"), go),
        ];
      },
      null,
    );
    if (!choice) return;
    if (choice.kind === "copy") return this.duplicate(choice.cargo, false);
    const load = choice.ids.map((id) => floor.cargo.find((c) => c.id === id)!);
    for (const c of load) if (!this.origin.has(c.id)) this.origin.set(c.id, this.join(floor.displayPath, c.name));
    await this.deliver(choice.target, load, false);
  }

  private async openLogbook(): Promise<void> {
    let entries: HistoryEntry[] = [];
    try {
      entries = await this.backend.history();
    } catch (err) {
      this.say(String(err));
    }
    const target = await ask<HistoryEntry | null>(
      $("logbook") as HTMLDialogElement,
      (close) => [
        h("h2", { id: "logbook-title" }, "Logbook"),
        h("p", { class: "sub" }, "Completed deliveries, newest first. Undo is a new move back, and only if the old spot is free. It isn't a backup."),
        entries.length === 0 ? h("p", {}, "Nothing delivered yet.") : null,
        h(
          "ul",
          { class: "list" },
          ...entries.map((e) => {
            const verb = e.op === "copy" ? "Copied" : e.op === "undo" ? "Undid" : "Moved";
            const can = !e.undone && (e.op === "move" || e.op === "copy");
            return h(
              "li",
              {},
              h("span", { class: "meta" }, new Date(e.atMs).toLocaleTimeString()),
              h("span", {}, `${verb} ${e.from || "?"}`, h("br"), h("span", { class: "meta" }, `to ${e.to}`)),
              can ? h("button", { type: "button", onclick: () => close(e) }, "Undo") : h("span", { class: "meta" }, e.undone ? "Undone" : ""),
            );
          }),
        ),
        h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => close(null) }, "Close")),
      ],
      null,
    );
    if (target) await this.undo(target, null);
  }

  private async undo(e: HistoryEntry, newName: string | null): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.game.pending = true;
    try {
      const back = e.op === "copy" ? `Recovered Copies/${e.to.split("/").pop()}` : e.from;
      const ok =
        newName !== null ||
        (await ask<boolean>(
          $("docket") as HTMLDialogElement,
          (close) => [
            h("h2", { id: "docket-title" }, e.op === "copy" ? "Undo copy" : "Undo delivery"),
            h("p", { class: "sub" }, e.op === "copy" ? "The copy moves to a visible Recovered Copies bay. PALLET doesn't delete it." : "This is a new move back to where the file was."),
            h("table", {}, h("tbody", {}, h("tr", {}, h("th", {}, "From"), h("td", {}, this.abs(e.to))), h("tr", {}, h("th", {}, "To"), h("td", {}, this.abs(back))))),
            this.modeLine(),
            h("div", { class: "actions" }, h("button", { type: "button", onclick: () => close(false) }, "Cancel"), h("button", { type: "button", class: "primary", onclick: () => close(true) }, "Sign and undo")),
          ],
          false,
        ));
      if (!ok) return;
      const o = await this.backend.undo(newTxId(), e.tx, newName);
      if (o.status === "committed") {
        this.sound.delivered();
        this.slip("Undone", [
          ["Before", this.abs(o.from)],
          ["After", this.abs(o.to)],
        ], "ok");
        await this.refresh();
        return;
      }
      this.sound.refuse();
      if (o.status === "uncertain") return void (await this.showRecovery());
      if (o.code === "NAME_EXISTS" && o.suggestion) {
        const name = await this.askName("Old spot is taken", `${o.message ?? ""} Put it back under a different name?`, o.suggestion);
        this.busy = false;
        if (name) await this.undo(e, name);
        return;
      }
      this.slip("Undo refused", [["Why", o.message ?? o.code ?? ""]], "bad");
    } finally {
      this.game.pending = false;
      this.busy = false;
    }
  }

  private async openSettings(): Promise<void> {
    const dialog = $("settings") as HTMLDialogElement;
    const render = (close: (v: null) => void): (HTMLElement | null)[] => {
      const toggle = (key: "muted" | "reducedMotion" | "pedals", text: string) => {
        const box = h("input", { type: "checkbox", checked: this.prefs[key] }) as HTMLInputElement;
        box.addEventListener("change", () => {
          this.prefs[key] = box.checked;
          this.applyPrefs();
        });
        return h("label", { class: "toggle" }, box, text);
      };
      const keys = h("div", { class: "keys" });
      const drawKeys = () =>
        fill(
          keys,
          ...(Object.keys(ACTIONS) as Control[]).flatMap((c) => {
            const btn = h("button", { type: "button", class: "btn" }, "Change") as HTMLButtonElement;
            btn.addEventListener("click", () => {
              fill(btn, "Press a key…");
              this.input.capture = (code) => {
                // One control per key: take it away from whoever had it.
                for (const other of Object.keys(this.prefs.bindings) as Control[]) this.prefs.bindings[other] = this.prefs.bindings[other].filter((k) => k !== code);
                this.prefs.bindings[c] = [code, ...this.prefs.bindings[c].filter((k) => k !== code)].slice(0, 2);
                this.applyPrefs();
                drawKeys();
              };
            });
            return [h("span", {}, ACTIONS[c]), h("span", {}, ...this.prefs.bindings[c].map((k) => h("kbd", {}, keyLabel(k)))), btn];
          }),
        );
      drawKeys();
      return [
        h("h2", { id: "settings-title" }, "Settings"),
        toggle("muted", "Mute sound"),
        toggle("reducedMotion", "Reduce motion (no shake, wobble, or tumbling)"),
        toggle("pedals", "Show on-screen pedals"),
        h("h3", {}, "Controls"),
        keys,
        h(
          "div",
          { class: "actions" },
          h(
            "button",
            {
              type: "button",
              class: "btn",
              onclick: () => {
                this.prefs.bindings = structuredClone(DEFAULT_BINDINGS);
                this.applyPrefs();
                drawKeys();
              },
            },
            "Reset controls",
          ),
          h("button", { type: "button", class: "btn primary", onclick: () => close(null) }, "Done"),
        ),
        h("p", { class: "fine" }, "Preferences are kept in this window's local storage, not in your warehouse."),
      ];
    };
    await ask<null>(dialog, render, null);
    this.input.capture = null;
  }

  private async openWarehouse(): Promise<void> {
    const b = this.backend;
    if (!b.chooseWarehouse || !b.createWarehouse || !b.openSample) return;
    const choice = await ask<"create" | "choose" | "sample" | null>(
      $("warehouse") as HTMLDialogElement,
      (close) => [
        h("h2", { id: "warehouse-title" }, "Pick a warehouse"),
        h("p", { class: "sub" }, `Now open: ${this.status.rootPath}`),
        h("p", {}, "PALLET works inside one folder at a time and never reaches outside it. Use a dedicated folder and put files in it with your usual file manager."),
        h(
          "div",
          { class: "actions" },
          h("button", { type: "button", class: "btn", onclick: () => close("sample") }, "Back to the sample"),
          h("button", { type: "button", class: "btn", onclick: () => close("choose") }, "Choose a folder"),
          h("button", { type: "button", class: "btn primary", onclick: () => close("create") }, "Create PALLET-Warehouse in my home folder"),
        ),
        h("p", { class: "fine" }, "Your home folder itself, system folders, and filesystems other than ext4 are refused."),
        h("div", { class: "actions" }, h("button", { type: "button", class: "btn", onclick: () => close(null) }, "Cancel")),
      ],
      null,
    );
    if (!choice) return;
    try {
      const st = choice === "create" ? await b.createWarehouse() : choice === "sample" ? await b.openSample() : await b.chooseWarehouse();
      if (!st) return;
      this.status = st;
      this.origin.clear();
      this.game.load = [];
      await this.navigate(null);
      if (st.frozen || st.recovery.length) await this.showRecovery();
    } catch (err) {
      this.sound.refuse();
      this.slip("Can't use that folder", [["Why", String(err)]], "bad");
    }
  }

  private async showRecovery(): Promise<void> {
    this.status = await this.backend.status();
    const st = this.status;
    if (!st.frozen && st.recovery.length === 0) return;
    const ack = await ask<boolean>(
      $("recovery") as HTMLDialogElement,
      (close) => [
        h("h2", { id: "recovery-title" }, st.frozen ? "Warehouse frozen" : "Recovered unfinished deliveries"),
        st.frozen ? h("p", { class: "notice" }, st.frozen.explanation) : h("p", { class: "sub" }, "PALLET stopped last time with deliveries in progress. It checked each file's identity and settled them before taking new jobs."),
        st.recovery.length
          ? h("table", {}, h("tbody", {}, ...st.recovery.map((d) => h("tr", {}, h("th", {}, d.decision.replace(/_/g, " ").toLowerCase()), h("td", {}, d.evidence)))))
          : null,
        st.frozen && !st.frozen.acknowledgeable ? h("p", { class: "fine" }, "Deliveries stay off until this is resolved outside PALLET. See RECOVERY.md in the PALLET repository. You can still drive around.") : null,
        h(
          "div",
          { class: "actions" },
          h("button", { type: "button", class: st.frozen?.acknowledgeable ? "" : "primary", onclick: () => close(false) }, st.frozen ? "Keep it frozen" : "OK"),
          st.frozen?.acknowledgeable ? h("button", { type: "button", class: "primary", onclick: () => close(true) }, "I've checked. Continue") : null,
        ),
      ],
      false,
    );
    if (ack) {
      try {
        this.status = await this.backend.acknowledge();
        if (this.status.frozen) await this.showRecovery();
      } catch (err) {
        this.slip("Still frozen", [["Why", String(err)]], "bad");
      }
    }
    await this.refresh();
  }

  // ---------- chrome ----------

  private slip(title: string, rows: [string, string][], tone: "ok" | "bad"): void {
    const el = $("slip");
    el.dataset.tone = tone;
    fill(el, h("h2", {}, title), h("dl", {}, ...rows.flatMap(([k, v]) => [h("dt", {}, k), h("dd", {}, v)])));
    el.hidden = false;
    clearTimeout((el as unknown as { t?: number }).t);
    (el as unknown as { t?: number }).t = window.setTimeout(() => (el.hidden = true), tone === "ok" ? 9000 : 14000);
  }

  private applyPrefs(): void {
    this.sound.setMuted(this.prefs.muted);
    this.renderer.reducedMotion = this.prefs.reducedMotion;
    document.documentElement.dataset.reducedMotion = String(this.prefs.reducedMotion);
    $("pedals").hidden = !this.prefs.pedals;
    this.input.bindings = this.prefs.bindings;
    const mute = $("btn-mute");
    fill(mute, this.prefs.muted ? "Sound off" : "Sound on");
    mute.setAttribute("aria-pressed", String(this.prefs.muted));
    savePrefs(this.prefs);
  }

  private wireChrome(): void {
    $("btn-manifest").addEventListener("click", () => void this.openManifest());
    $("btn-logbook").addEventListener("click", () => void this.openLogbook());
    $("btn-settings").addEventListener("click", () => void this.openSettings());
    $("btn-mute").addEventListener("click", () => {
      this.prefs.muted = !this.prefs.muted;
      this.applyPrefs();
    });
    const wh = $("btn-warehouse");
    wh.hidden = !this.backend.chooseWarehouse;
    wh.addEventListener("click", () => void this.openWarehouse());
    $("rack-prev").addEventListener("click", () => this.page(-1, 0));
    $("rack-next").addEventListener("click", () => this.page(1, 0));
    $("bay-prev").addEventListener("click", () => this.page(0, -1));
    $("bay-next").addEventListener("click", () => this.page(0, 1));
    this.input.onCommand = (c) => {
      this.sound.wake();
      if (c === "mute") {
        this.prefs.muted = !this.prefs.muted;
        this.applyPrefs();
      } else {
        this.game.paused = !this.game.paused;
        $("paused").hidden = !this.game.paused;
      }
    };
    window.addEventListener("keydown", () => this.sound.wake());
    window.addEventListener("pointerdown", () => this.sound.wake());
    window.addEventListener("focus", () => {
      if (!this.busy) void this.refresh();
    });
    for (const btn of $("pedals").querySelectorAll<HTMLButtonElement>("button")) {
      const hold = btn.dataset.hold as Control | undefined;
      const tap = btn.dataset.tap as "lift" | "stack" | "enter" | undefined;
      if (hold) {
        btn.addEventListener("pointerdown", (e) => {
          btn.setPointerCapture(e.pointerId);
          this.input.hold(hold, true);
        });
        for (const ev of ["pointerup", "pointercancel", "lostpointercapture"]) btn.addEventListener(ev, () => this.input.hold(hold, false));
      }
      if (tap) btn.addEventListener("click", () => this.input.tap(tap));
    }
  }

  private page(rack: number, bay: number): void {
    this.game.page(rack, bay);
    this.renderChrome();
  }

  private renderChrome(): void {
    const floor = this.game.floor;
    const mode = $("mode");
    mode.dataset.mode = this.backend.mode;
    fill(mode, this.backend.mode === "real" ? (this.status.sample ? "Real files: sample warehouse" : "Real files") : __PALLET_DEMO__ ? "Demo: virtual files" : "Sample only: virtual files");
    if (this.backend.note) mode.title = this.backend.note;
    fill($("path"), h("bdi", {}, floor.displayPath));
    const crumbs = $("crumbs");
    fill(
      crumbs,
      ...floor.breadcrumbs.flatMap((b, i) => {
        const last = i === floor.breadcrumbs.length - 1;
        const btn = h("button", { type: "button", "aria-current": last ? "page" : undefined, onclick: () => void this.navigate(b.id) }, b.name);
        return i === 0 ? [btn] : [h("span", { "aria-hidden": "true" }, "/"), btn];
      }),
    );
    const L = this.game.layout;
    const pager = $("pager");
    pager.hidden = L.rackPages <= 1 && L.bayPages <= 1;
    fill($("pager-rack"), `Rack ${this.game.rackPage + 1} of ${L.rackPages}`);
    fill($("pager-bay"), `Bays ${this.game.bayPage + 1} of ${L.bayPages}`);
    if (floor.truncated) this.say("This floor has more than 5,000 entries. Only the first 5,000 are shown.", 6);
  }

  private key(c: Control): HTMLElement {
    return h("kbd", {}, keyLabel(this.prefs.bindings[c][0] ?? "?"));
  }

  private renderDash(): void {
    const g = this.game;
    const f = g.fork;
    const speed = $("speed");
    const kmh = Math.round(Math.abs(f.speed) * KMH);
    if (speed.textContent !== String(kmh)) speed.textContent = String(kmh);
    const wob = instability(f, g.loadInfo());
    const bubble = $("bubble");
    bubble.style.left = `${50 + Math.max(-1, Math.min(1, wob * Math.sign(f.steer || 1))) * 48}px`;
    const level = $("level");
    level.dataset.warn = String(wob > 0.55);
    level.setAttribute("aria-valuenow", String(Math.round((1 - wob) * 100)));
    const pallet = g.load.length === 0 ? (f.forkTarget ? "Forks up, empty" : "Forks down, empty") : g.load.length === 1 ? `Carrying ${g.load[0].name}` : `Pallet: ${g.load.length} of 6 crates`;
    const pe = $("pallet");
    if (pe.textContent !== pallet) pe.textContent = pallet;

    const hint = $("hint");
    let parts: (HTMLElement | string)[];
    if (this.flash && performance.now() < this.flash.until) parts = [this.flash.text];
    else if (f.tipped) parts = ["Tipped. There's paperwork."];
    else if (g.pending) parts = ["Waiting on the warehouse office."];
    else if (g.paused) parts = ["Paused."];
    else if (g.load.length) {
      const bay = g.bayUnderForks();
      if (bay && g.stopped()) parts = [this.key("lift"), ` lowers the forks into ${bay.bay.name}`];
      else if (bay) parts = ["Stop inside the drop zone"];
      else if (g.atDuplicator() && g.stopped()) parts = [this.key("lift"), " feeds the duplicator"];
      else parts = ["Drive to a bay's drop zone. ", this.key("stack"), " adds another crate."];
    } else {
      const t = g.target();
      const bay = g.bayAround();
      if (t?.aligned && !t.crate.cargo.unavailable) parts = [this.key("lift"), ` lifts ${t.crate.cargo.name}`];
      else if (t) parts = ["Line the forks up with the crate"];
      else if (bay && g.stopped()) parts = [this.key("enter"), ` goes into ${bay.bay.name}`];
      else parts = [this.key("forward"), this.key("left"), this.key("reverse"), this.key("right"), " drive  ", this.key("lift"), " forks  ", this.key("precision"), " slow  ", this.key("pause"), " pause"];
    }
    const sig = parts.map((p) => (typeof p === "string" ? p : `[${p.textContent}]`)).join("");
    if (hint.dataset.sig !== sig) {
      hint.dataset.sig = sig;
      fill(hint, ...parts);
    }
  }
}
