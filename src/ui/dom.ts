// Tiny DOM builder. Strings become text nodes, never HTML, so a filename like
// `<img onerror=…>.txt` is displayed literally (SAFETY.md S7).

type Child = Node | string | number | null | undefined | false;
type Attrs = Record<string, string | boolean | number | ((e: Event) => void) | undefined>;

export function h(tag: string, attrs: Attrs = {}, ...children: Child[]): HTMLElement {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === false) continue;
    if (typeof v === "function") el.addEventListener(k.replace(/^on/, "").toLowerCase(), v);
    else if (v === true) el.setAttribute(k, "");
    else el.setAttribute(k, String(v));
  }
  for (const c of children) {
    if (c === null || c === undefined || c === false) continue;
    el.append(typeof c === "string" || typeof c === "number" ? document.createTextNode(String(c)) : c);
  }
  return el;
}

export function fill(el: HTMLElement, ...children: Child[]): HTMLElement {
  el.replaceChildren();
  for (const c of children) {
    if (c === null || c === undefined || c === false) continue;
    el.append(typeof c === "string" || typeof c === "number" ? document.createTextNode(String(c)) : c);
  }
  return el;
}

export function $(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`#${id} missing`);
  return el;
}

/** Opens a <dialog> and resolves with the value passed to `close(value)`. */
export function ask<T>(dialog: HTMLDialogElement, build: (close: (v: T) => void) => Child[], cancelValue: T): Promise<T> {
  return new Promise((resolve) => {
    let done = false;
    const close = (v: T) => {
      if (done) return;
      done = true;
      dialog.removeEventListener("cancel", onCancel);
      dialog.close();
      resolve(v);
    };
    const onCancel = (e: Event) => {
      e.preventDefault();
      close(cancelValue);
    };
    dialog.addEventListener("cancel", onCancel);
    fill(dialog, ...build(close));
    // Focus the document, not a button: a key still held from driving (Space
    // to brake, Enter) must not sign a docket or dismiss a report on release.
    dialog.tabIndex = -1;
    dialog.showModal();
    dialog.focus();
  });
}
