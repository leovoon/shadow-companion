#!/usr/bin/env python3
"""
Generate menubar PNG icons for Shadow Meter (Perry app).

macOS menubar is 22pt. System battery icon is ~27×13pt — very wide, short.
At @2x Retina: ~54×26px glyph centered in a 54×44px canvas.

Icons are template images (single-color, macOS adapts to light/dark).
"""

import os
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ICONS_DIR = Path.home() / ".shadow-companion" / "perry-icons"

# ── dimensions ─────────────────────────────────────────────────────
# Canvas = menubar height @2x (22pt × 2 = 44px)
# Glyph: wide & short like the macOS system battery

CANVAS_W = 54
CANVAS_H = 44

# Glyph area — centered vertically in canvas, proportions matching system battery
GLYPH_W = 44         # main body width
GLYPH_H = 22         # main body height (~11pt, short like macOS battery)
GLYPH_X = 2          # left offset
GLYPH_Y = (CANVAS_H - GLYPH_H) // 2  # vertically centered

CORNER_R = 4
GAP = 2
TERMINAL_W = 3
TERMINAL_H = int(GLYPH_H * 0.38)

# Template colors — macOS renders as single-color glyph
COLOR_EMPTY = (0, 0, 0, 25)
COLOR_FILLED = (0, 0, 0, 200)
COLOR_OUTLINE = (0, 0, 0, 180)

SLICES = 5


def draw_battery(filled: int, path: str):
    """Draw a 5-slice battery icon with `filled` slices colored."""
    img = Image.new("RGBA", (CANVAS_W, CANVAS_H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    body_x2 = GLYPH_X + GLYPH_W
    body_y2 = GLYPH_Y + GLYPH_H

    # Terminal nub (right side)
    term_x1 = body_x2
    term_y1 = GLYPH_Y + (GLYPH_H - TERMINAL_H) // 2
    term_y2 = term_y1 + TERMINAL_H
    draw.rounded_rectangle(
        [term_x1, term_y1, term_x1 + TERMINAL_W, term_y2],
        radius=2,
        fill=COLOR_OUTLINE,
    )

    # Outer shell
    draw.rounded_rectangle(
        [GLYPH_X, GLYPH_Y, body_x2, body_y2],
        radius=CORNER_R,
        fill=None,
        outline=COLOR_OUTLINE,
        width=2,
    )

    # Inner slices
    inner_pad = 3
    inner_x1 = GLYPH_X + inner_pad
    inner_y1 = GLYPH_Y + inner_pad
    inner_x2 = body_x2 - inner_pad
    inner_y2 = body_y2 - inner_pad
    inner_w = inner_x2 - inner_x1
    inner_h = inner_y2 - inner_y1

    total_gap = GAP * (SLICES - 1)
    slice_w = (inner_w - total_gap) / SLICES

    for i in range(SLICES):
        sx1 = inner_x1 + i * (slice_w + GAP)
        sx2 = sx1 + slice_w
        color = COLOR_FILLED if i < filled else COLOR_EMPTY
        draw.rounded_rectangle(
            [int(sx1), int(inner_y1), int(sx2), int(inner_y2)],
            radius=1,
            fill=color,
        )

    img.save(path)


def draw_text(actual_min: int, target_min: int, path: str):
    """Draw a minimal text icon like '35/60' for the menubar."""
    img = Image.new("RGBA", (CANVAS_W, CANVAS_H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    label = f"{actual_min}/{target_min}"
    try:
        font = ImageFont.truetype("/System/Library/Fonts/SFCompact.ttf", 22)
    except (OSError, IOError):
        try:
            font = ImageFont.truetype("/System/Library/Fonts/Helvetica.ttc", 22)
        except (OSError, IOError):
            font = ImageFont.load_default()

    bbox = draw.textbbox((0, 0), label, font=font)
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]
    x = (CANVAS_W - tw) // 2 - bbox[0]
    y = (CANVAS_H - th) // 2 - bbox[1]

    draw.text((x, y), label, fill=(0, 0, 0, 200), font=font)
    img.save(path)


def main():
    ICONS_DIR.mkdir(parents=True, exist_ok=True)

    # Battery icons: 0..5 filled slices
    for i in range(SLICES + 1):
        p = ICONS_DIR / f"battery-{i}.png"
        draw_battery(i, str(p))
        print(f"  {p.name}")

    # Text icons: every 5 min from 0 to target, for targets 15..180
    text_count = 0
    for target in range(15, 185, 5):
        for actual in range(0, target + 1, 5):
            p = ICONS_DIR / f"text-{actual}-{target}.png"
            if not p.exists():
                draw_text(actual, target, str(p))
                text_count += 1

    text_total = len(list(ICONS_DIR.glob("text-*.png")))
    print(f"\n  {SLICES + 1} battery icons + {text_total} text icons → {ICONS_DIR}")


if __name__ == "__main__":
    main()
