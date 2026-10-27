import "@fontsource/big-shoulders-stencil-display/600";
import "@fontsource/big-shoulders-display/600";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/600.css";
import "./styles.css";

import { DemoBackend } from "./backend/demo";
import { TauriBackend, boot } from "./backend/tauri";
import type { Backend } from "./backend/types";
import { Sound } from "./game/audio";
import { Input, loadPrefs } from "./game/input";
import { Renderer } from "./game/renderer";
import { App } from "./ui/app";

async function pickBackend(): Promise<Backend> {
  const native = !__PALLET_DEMO__ && "__TAURI_INTERNALS__" in window;
  if (!native) return new DemoBackend("Browser demo. Files here are pretend and live only in this tab.");
  const info = await boot();
  return info.mode === "real" ? new TauriBackend(info.note) : new DemoBackend(info.note);
}

async function main(): Promise<void> {
  await Promise.all([
    document.fonts.load('600 20px "Big Shoulders Stencil Display"'),
    document.fonts.load('400 12px "IBM Plex Mono"'),
    document.fonts.load('600 12px "IBM Plex Mono"'),
  ]).catch(() => undefined);
  const backend = await pickBackend();
  const prefs = loadPrefs();
  const renderer = new Renderer();
  await renderer.init(document.getElementById("stage")!);
  const app = new App(backend, renderer, new Input(prefs.bindings), new Sound(), prefs);
  await app.start();
  // Exposed for the automated UI check and demo capture; read-only views.
  (window as unknown as { __pallet: unknown }).__pallet = { app, backend };
}

main().catch((err) => {
  document.body.textContent = `PALLET couldn't start: ${String(err)}`;
});
