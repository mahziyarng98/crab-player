#!/usr/bin/env python3
"""Hand-designed simplified Crab Player icons (16x16, 24x24).

A small crab drawn with geometric primitives chosen for these sizes: two
claws with pincer clefts, a rounded body, eyes, short legs, on the brand
dark rounded square. Drawn entirely by this script at 4x supersampling and
re-quantized to the brand palette so the output is crisp pixel art — no
third-party artwork.

Brand palette (src/ui/theme.rs):
  ACCENT #9146f5  ACCENT_DEEP #7c3aed  ACCENT_LIGHT #c084fc
  THUMB #ddd6fe  PANEL #100a1d  WELL #0c0618
"""

from PIL import Image, ImageDraw
import subprocess

BG = (0x10, 0x0a, 0x1d, 255)
BG_EDGE = (0x0c, 0x06, 0x18, 255)
SHELL = (0x91, 0x46, 0xf5, 255)
DEEP = (0x7c, 0x3a, 0xed, 255)
LIGHT = (0xc0, 0x84, 0xfc, 255)
EYE = (0xdd, 0xd6, 0xfe, 255)
PALETTE = [BG_EDGE, BG, SHELL, DEEP, LIGHT, EYE]

def nearest_palette(rgba):
    best, best_d = None, None
    for p in PALETTE:
        d = sum((a - b) ** 2 for a, b in zip(rgba[:3], p[:3]))
        if best_d is None or d < best_d:
            best, best_d = p, d
    return best[:3] + (255,)

def draw_crab(size):
    """Draw the crab directly at the final pixel size."""
    im = Image.new("RGBA", (size, size), BG)
    d = ImageDraw.Draw(im)

    # Dark rounded square: edge ring, then inner panel.
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=3, fill=BG_EDGE)
    d.rounded_rectangle([1, 1, size - 2, size - 2], radius=2, fill=BG)

    c = (size - 1) / 2.0          # center axis (7.5 / 11.5)
    def claw(side):
        sign = 1 if side == "right" else -1
        cx = c + sign * 4.15
        cy = 4.0
        # main pincer lobe
        d.ellipse([cx - 2.05, cy - 2.05, cx + 2.05, cy + 2.05], fill=SHELL)
        # lower finger
        d.ellipse([cx - 1.5, cy + 0.7, cx + 1.5, cy + 3.4], fill=SHELL)
        # cleft between the two fingers
        d.ellipse([cx - 0.55, cy - 0.55, cx + 0.55, cy + 0.75], fill=BG)
        # wrist toward the body
        d.ellipse([cx - 0.85, cy + 2.4, cx + 0.85, cy + 5.4], fill=SHELL)
    for side in ("left", "right"):
        claw(side)

    # body: rounded, slightly wider than tall
    d.ellipse([c - 4.25, 7.3, c + 4.25, 14.7], fill=SHELL)

    # eyes (one pixel) with a single sheen pixel above each
    for ex in (c - 1.7, c + 1.7):
        d.ellipse([ex - 0.6, 9.4, ex + 0.6, 10.4], fill=EYE)
        d.ellipse([ex - 0.4, 8.8, ex + 0.4, 9.3], fill=LIGHT)

    # legs
    for sign in (-1, 1):
        for ly in (12.4, 13.9, 15.2):
            x0 = c + sign * (4.9 - 0.85)
            x1 = c + sign * (4.9 + 0.85)
            d.ellipse([min(x0, x1), ly, max(x0, x1), ly + 1.3], fill=SHELL)

    out = Image.new("RGBA", (size, size))
    px = out.load()
    for y in range(size):
        for x in range(size):
            px[x, y] = nearest_palette(im.getpixel((x, y)))
    return out

im16 = draw_crab(16)
im24 = draw_crab(24)

out = "assets/icons/hicolor"
im16.save(f"{out}/16x16/apps/io.github.mahziyarng98.crab-player.png")
im24.save(f"{out}/24x24/apps/io.github.mahziyarng98.crab-player.png")
print("wrote 16x16 and 24x24")

for s, im in ((16, im16), (24, im24)):
    for zoom in (8, 10):
        im.resize((s * zoom, s * zoom), Image.NEAREST).save(
            f"/tmp/crab-task2/icon-new-{s}-{zoom}x.png")
    data = subprocess.run(
        ["git", "show",
         f"HEAD:assets/icons/hicolor/{s}x{s}/apps/io.github.mahziyarng98.crab-player.png"],
        capture_output=True).stdout
    with open(f"/tmp/crab-task2/old-{s}.png", "wb") as f:
        f.write(data)
    before = Image.open(f"/tmp/crab-task2/old-{s}.png").convert("RGBA")
    z = 10
    canvas = Image.new("RGBA", (s * z * 2 + 40, s * z), (8, 8, 12, 255))
    canvas.paste(before.resize((s * z, s * z), Image.NEAREST), (0, 0))
    canvas.paste(im.resize((s * z, s * z), Image.NEAREST), (s * z + 40, 0))
    canvas.save(f"/tmp/crab-task2/icon-before-after-{s}.png")
    print("composite saved", s)
