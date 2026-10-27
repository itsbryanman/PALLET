#!/usr/bin/env python3
"""Inspection boards for the registered forklift frames (not shipped).

usage: python3 scripts/inspect_forklift.py <out-dir>
Writes each frame on light grey, dark concrete and a checkerboard with the
pivot (red) and fork pickup point (cyan) marked, plus an onion-skin overlay of
all eight frames on the shared pivot.
"""
import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
meta = json.loads((ROOT / "public/sprites/forklift-atlas.json").read_text())
atlas = Image.open(ROOT / "public/sprites/forklift-atlas.png")
order = meta["headingOrder"]
f0 = meta["frames"][order[0]]["frame"]
cw, ch = f0["w"], f0["h"]


def checker(w, h, s=12):
    im = Image.new("RGBA", (w, h))
    d = ImageDraw.Draw(im)
    for y in range(0, h, s):
        for x in range(0, w, s):
            d.rectangle([x, y, x + s - 1, y + s - 1], fill=(235, 235, 235, 255) if (x // s + y // s) % 2 else (190, 190, 190, 255))
    return im


backgrounds = {"light": lambda: Image.new("RGBA", (cw, ch), (214, 214, 210, 255)), "concrete": lambda: Image.new("RGBA", (cw, ch), (62, 59, 54, 255)), "checker": lambda: checker(cw, ch)}
for bgname, make in backgrounds.items():
    board = Image.new("RGBA", (cw * 4, ch * 2))
    for idx, name in enumerate(order):
        fr = meta["frames"][name]
        f = fr["frame"]
        tile = make()
        tile.alpha_composite(atlas.crop((f["x"], f["y"], f["x"] + f["w"], f["y"] + f["h"])))
        d = ImageDraw.Draw(tile)
        px, py = fr["pivot"]["x"], fr["pivot"]["y"]
        d.line([(px - 14, py), (px + 14, py)], fill=(255, 40, 40, 255), width=3)
        d.line([(px, py - 14), (px, py + 14)], fill=(255, 40, 40, 255), width=3)
        fx, fy = px + fr["fork"]["x"], py + fr["fork"]["y"]
        d.ellipse([fx - 7, fy - 7, fx + 7, fy + 7], outline=(0, 220, 255, 255), width=3)
        d.text((8, 8), name.upper(), fill=(255, 40, 40, 255))
        board.paste(tile, ((idx % 4) * cw, (idx // 4) * ch))
    board.convert("RGB").save(out / f"frames-{bgname}.png")

onion = backgrounds["light"]()
for name in order:
    f = meta["frames"][name]["frame"]
    fr = atlas.crop((f["x"], f["y"], f["x"] + f["w"], f["y"] + f["h"]))
    a = fr.getchannel("A").point(lambda v: v * 0.22)
    fr.putalpha(a)
    onion.alpha_composite(fr)
d = ImageDraw.Draw(onion)
px, py = meta["frames"][order[0]]["pivot"]["x"], meta["frames"][order[0]]["pivot"]["y"]
d.line([(px - 20, py), (px + 20, py)], fill=(255, 0, 0, 255), width=3)
d.line([(px, py - 20), (px, py + 20)], fill=(255, 0, 0, 255), width=3)
onion.convert("RGB").save(out / "onion.png")
print("wrote", out)
