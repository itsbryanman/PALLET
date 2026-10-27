#!/usr/bin/env python3
"""Slice the 8-direction forklift sheet into registered frames and an atlas.

Reproducible: `python3 scripts/slice_forklift.py` (needs Pillow, numpy, scipy).

Input   assets/sprites/forklift-sheet.png   (original, never modified)
Output  assets/sprites/cells/forklift-<dir>.png   proportional grid cells, as cut
        assets/sprites/frames/forklift-<dir>.png  cleaned, registered frames
        public/sprites/forklift-atlas.png + .json the runtime atlas

Layout of the sheet, by the direction the FORKS point:
    top row:    N  NE  E  SE
    bottom row: S  SW  W  NW

Notes from inspecting the original (1774 x 887 RGBA):
- Cell boundaries are proportional: x[i] = round(i*W/4), y[j] = round(j*H/2).
- Three vehicles cross their cell: the fork tips of E, SW and W extend 10-20 px
  into the neighbouring cell, so the raw cells clip them and the neighbours pick
  up the fragments. The registered frames therefore take each cell's vehicle as
  the connected component whose centre of mass lies in that cell, from the full
  sheet. Nothing is clipped and nothing from a neighbour is included.
- The background removal left a haze of alpha 1-2 pixels across the sheet,
  some of it touching the vehicles. Alpha <= 4 is set to 0 (invisible either
  way); disconnected specks are dropped (their max alpha is 7). Thin forks and
  cage bars are opaque and unaffected.
- Pivots are the ground-plane centre of the chassis (middle of the four tyre
  contact points, hidden wheels estimated), identified by eye on a 25 px grid.
  Fork points are the middle of the two tines. Both are in sheet pixels.
- The frames are not one consistent projection: N and S are near top-down, E
  and W are side elevations, the diagonals are three-quarter views.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

ROOT = Path(__file__).resolve().parent.parent
SHEET = ROOT / "assets/sprites/forklift-sheet.png"
CELLS = ROOT / "assets/sprites/cells"
FRAMES = ROOT / "assets/sprites/frames"
ATLAS = ROOT / "public/sprites/forklift-atlas.png"

GRID = [["n", "ne", "e", "se"], ["s", "sw", "w", "nw"]]

# Hand-identified, sheet coordinates (see module docstring).
PIVOT = {
    "n": (221, 290), "ne": (650, 328), "e": (1060, 360), "se": (1526, 292),
    "s": (221, 615), "sw": (676, 694), "w": (1158, 776), "nw": (1577, 712),
}
FORK = {
    "n": (221, 52), "ne": (815, 160), "e": (1272, 318), "se": (1700, 355),
    "s": (221, 800), "sw": (517, 775), "w": (945, 735), "nw": (1424, 582),
}
# Whether the forks (and cargo) sit in front of the body, for draw order.
CARGO_IN_FRONT = {"n": False, "ne": False, "e": True, "se": True, "s": True, "sw": True, "w": True, "nw": False}

ALPHA_FLOOR = 4  # alpha at or below this is background haze
PAD = 8  # transparent padding around every frame in the atlas


def main() -> int:
    sheet = Image.open(SHEET)
    if sheet.mode != "RGBA":
        print("expected RGBA", file=sys.stderr)
        return 1
    width, height = sheet.size
    xs = [round(i * width / 4) for i in range(5)]
    ys = [round(j * height / 2) for j in range(3)]
    print(f"sheet {width}x{height}, x bounds {xs}, y bounds {ys}")

    CELLS.mkdir(parents=True, exist_ok=True)
    FRAMES.mkdir(parents=True, exist_ok=True)
    ATLAS.parent.mkdir(parents=True, exist_ok=True)

    # 1. The literal proportional cells, alpha preserved, nothing else done.
    for j, row in enumerate(GRID):
        for i, name in enumerate(row):
            sheet.crop((xs[i], ys[j], xs[i + 1], ys[j + 1])).save(CELLS / f"forklift-{name}.png")

    # 2. Clean haze and find each vehicle as a connected component.
    px = np.array(sheet)
    px[..., 3][px[..., 3] <= ALPHA_FLOOR] = 0
    mask = px[..., 3] > 0
    labels, count = ndimage.label(mask, structure=np.ones((3, 3)))
    sizes = ndimage.sum(mask, labels, range(1, count + 1))
    centres = ndimage.center_of_mass(mask, labels, range(1, count + 1))

    vehicles: dict[str, int] = {}
    for j, row in enumerate(GRID):
        for i, name in enumerate(row):
            best = None
            for k in range(count):
                cy, cx = centres[k]
                if xs[i] <= cx < xs[i + 1] and ys[j] <= cy < ys[j + 1] and (best is None or sizes[k] > sizes[best]):
                    best = k
            assert best is not None and sizes[best] > 20000, name
            vehicles[name] = best + 1

    boxes = {}
    for name, lab in vehicles.items():
        yy, xx = np.where(labels == lab)
        boxes[name] = (int(xx.min()), int(yy.min()), int(xx.max()) + 1, int(yy.max()) + 1)
        j = 0 if name in GRID[0] else 1
        i = GRID[j].index(name)
        b = boxes[name]
        clipped = b[0] < xs[i] or b[2] > xs[i + 1] or b[1] < ys[j] or b[3] > ys[j + 1]
        print(f"{name:>2}: bbox {boxes[name]}, {int(sizes[lab - 1])} px{'  (crosses its cell)' if clipped else ''}")

    # 3. Register: shared pivot on equal canvases, one physical scale (no resizing).
    left = max(PIVOT[n][0] - b[0] for n, b in boxes.items())
    right = max(b[2] - PIVOT[n][0] for n, b in boxes.items())
    up = max(PIVOT[n][1] - b[1] for n, b in boxes.items())
    down = max(b[3] - PIVOT[n][1] for n, b in boxes.items())
    half_w = max(left, right) + PAD
    cw, ch = 2 * half_w, up + down + 2 * PAD
    pivot_canvas = (half_w, up + PAD)
    print(f"canvas {cw}x{ch}, pivot at {pivot_canvas}")

    frames = {}
    order = ["e", "se", "s", "sw", "w", "nw", "n", "ne"]  # heading 0, +45deg, ... (y down)
    atlas = Image.new("RGBA", (cw * 4, ch * 2), (0, 0, 0, 0))
    for idx, name in enumerate(order):
        only = np.where((labels == vehicles[name])[..., None], px, 0).astype(np.uint8)
        b = boxes[name]
        crop = Image.fromarray(only, "RGBA").crop(b)
        canvas = Image.new("RGBA", (cw, ch), (0, 0, 0, 0))
        ox = pivot_canvas[0] - (PIVOT[name][0] - b[0])
        oy = pivot_canvas[1] - (PIVOT[name][1] - b[1])
        canvas.paste(crop, (ox, oy))
        canvas.save(FRAMES / f"forklift-{name}.png")
        ax, ay = (idx % 4) * cw, (idx // 4) * ch
        atlas.paste(canvas, (ax, ay))
        frames[name] = {
            "frame": {"x": ax, "y": ay, "w": cw, "h": ch},
            "pivot": {"x": pivot_canvas[0], "y": pivot_canvas[1]},
            "fork": {"x": FORK[name][0] - PIVOT[name][0], "y": FORK[name][1] - PIVOT[name][1]},
            "cargoInFront": CARGO_IN_FRONT[name],
        }
    atlas.save(ATLAS, optimize=True)
    meta = {
        "image": ATLAS.name,
        "size": {"w": atlas.width, "h": atlas.height},
        "source": "assets/sprites/forklift-sheet.png",
        "headingOrder": order,
        "note": "Frames are indexed by fork direction, heading 0 = +x (east), y down. Pivot is the ground-plane chassis centre; fork is the pickup point relative to the pivot, in source pixels. Forks are lowered in every frame.",
        "frames": frames,
    }
    ATLAS.with_suffix(".json").write_text(json.dumps(meta, indent=2) + "\n")
    print(f"atlas {atlas.width}x{atlas.height} -> {ATLAS.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
