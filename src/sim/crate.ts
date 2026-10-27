// Crate size from GAME_DESIGN.md: logarithmic, capped at 1 GiB, so a 50 GB
// file is still a crate you can carry. The label always shows the real size.

const KIB = 1024;
const GIB = 1024 ** 3;

export function sizeFactor(bytes: number): number {
  const s = Math.log2(1 + bytes / KIB) / Math.log2(1 + GIB / KIB);
  return Math.min(1, Math.max(0, s));
}

export function crateSize(bytes: number): { w: number; h: number } {
  const s = sizeFactor(bytes);
  return { w: Math.round(28 + 36 * s), h: Math.round(24 + 24 * s) };
}

/** Gameplay mass units, 1–4. Not kilograms. */
export function crateMass(bytes: number): number {
  return 1 + 3 * sizeFactor(bytes);
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let v = bytes;
  let i = -1;
  do {
    v /= 1024;
    i++;
  } while (v >= 1024 && i < units.length - 1);
  return `${v >= 100 ? v.toFixed(0) : v >= 10 ? v.toFixed(1) : v.toFixed(2)} ${units[i]}`;
}
