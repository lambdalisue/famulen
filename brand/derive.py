#!/usr/bin/env python3
"""Write the files derived from the art in male/ and female/.

Needs only Python and resvg:  nix shell nixpkgs#resvg -c python3 brand/derive.py

The art itself (avatar, app-icon, favicon) is never written, only read and
copied next to the console's pages, which are built without this directory.
"""

import json
import re
import shutil
import subprocess
from pathlib import Path

BRAND = Path(__file__).resolve().parent
CONSOLE = BRAND.parent / "frontend" / "public" / "icons"
PHI = (1 + 5**0.5) / 2
TILE, DRAWING, PAGE = "#0B0D15", "#FFF8EB", "#F4F3EE"
BOX = 1024
# A launcher may crop a maskable icon down to this part of its half width.
SAFE = 0.8
# The lockups' mark height, and the height of the wordmark's l in its units.
MARK, LETTERS = 100.0, 723.0
NAMED = ' role="img" aria-label="Famulen"'
WORD_LEFT, WORD_WIDTH, WORD_DROP = 72.0, 3775.0, 10.0


def svg(left, top, width, height, body, named=""):
    box = " ".join(f"{n:g}" for n in (left, top, width, height))
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{box}"{named}>{body}</svg>\n'


def png(source, target, size):
    subprocess.run(["resvg", "-w", str(size), "-h", str(size), source, target], check=True)


def points(d):
    numbers = [float(n) for n in re.findall(r"-?[\d.]+", d)]
    return list(zip(numbers[::2], numbers[1::2]))


def lockups(word, tile, drawing, box):
    """The horizontal and vertical lockups for light and for dark grounds.

    On a light ground the mark keeps its tile: the drawing is light on dark,
    and filled with ink it would be its own negative.
    """
    left, top, width, height = box
    scale = MARK / height
    bare = f'<g transform="scale({scale:.5f}) translate({-left:g} {-top:g})">{drawing}</g>'
    tile_scale = MARK / 1008
    tiled = f'<g transform="scale({tile_scale:.5f}) translate(-8 -8)">{tile}</g>'
    for suffix, mark, mark_width, ink in (
        ("", tiled, MARK, TILE),
        ("-light", bare, width * scale, DRAWING),
    ):
        gap = MARK / PHI**2

        def words(x, baseline, size):
            return (
                f'<g transform="translate({x:.3f} {baseline:.3f}) scale({size:.5f}) '
                f'translate({-WORD_LEFT:g} 0)" fill="{ink}">{word}</g>'
            )

        letters = MARK / PHI
        size = letters / LETTERS
        x = mark_width + gap
        yield (
            f"logo-horizontal{suffix}.svg",
            svg(0, 0, round(x + WORD_WIDTH * size, 3), MARK,
                mark + words(x, (MARK + letters) / 2, size), NAMED),
        )

        total = MARK * PHI
        size = total / WORD_WIDTH
        baseline = MARK + gap + LETTERS * size
        centred = f'<g transform="translate({(total - mark_width) / 2:.3f} 0)">{mark}</g>'
        yield (
            f"logo-vertical{suffix}.svg",
            svg(0, 0, round(total, 3), round(baseline + WORD_DROP * size, 3),
                centred + words(0, baseline, size), NAMED),
        )


def derive(variant):
    home = BRAND / variant
    app_icon = (home / "app-icon.svg").read_text()
    d = re.search(r'<path[^>]* d="([^"]+)"', app_icon).group(1)
    path = f'<path fill-rule="evenodd" d="{d}"/>'
    drawing = f'<g fill="{DRAWING}">{path}</g>'
    xs, ys = zip(*points(d))
    left, top, width, height = min(xs), min(ys), max(xs) - min(xs), max(ys) - min(ys)
    cx, cy = left + width / 2, top + height / 2

    side = max(width, height)
    (home / "mark.svg").write_text(
        svg(cx - side / 2, cy - side / 2, side, side, f'<g fill="currentColor">{path}</g>')
    )

    # Square to the edges, for a system that rounds or crops it by itself.
    reach = max(((x - cx) ** 2 + (y - cy) ** 2) ** 0.5 for x, y in zip(xs, ys))
    shrink = min(1, SAFE * BOX / 2 / reach)
    square = home / "app-icon-square.svg"
    square.write_text(
        svg(0, 0, BOX, BOX,
            f'<rect width="{BOX}" height="{BOX}" fill="{TILE}"/>'
            f'<g transform="translate({BOX / 2:g} {BOX / 2:g}) scale({shrink:.4f}) '
            f'translate({-cx:g} {-cy:g})">{drawing}</g>')
    )
    png(square, home / "apple-touch-icon.png", 180)
    png(square, home / "app-icon-maskable-512.png", 512)
    for size in (192, 512):
        png(home / "app-icon.svg", home / f"app-icon-{size}.png", size)

    word = re.search(r"<path[^>]*/>", (BRAND / "wordmark.svg").read_text()).group(0)
    word = word.replace(' fill="currentColor"', "")
    tile = f'<rect x="8" y="8" width="1008" height="1008" rx="220" fill="{TILE}"/>{drawing}'
    for name, text in lockups(word, tile, drawing, (left, top, width, height)):
        (home / name).write_text(text)

    served = CONSOLE / variant
    served.mkdir(parents=True, exist_ok=True)
    for name in (
        "app-icon.svg", "app-icon-192.png", "app-icon-512.png", "app-icon-maskable-512.png",
        "apple-touch-icon.png", "avatar.svg", "favicon-16.png", "favicon-32.png", "favicon.ico",
    ):
        shutil.copyfile(home / name, served / name)
    at = f"/icons/{variant}"
    manifest = {
        "id": "/",
        "name": "Famulen",
        "short_name": "Famulen",
        "lang": "ja",
        "start_url": "/",
        "scope": "/",
        "display": "standalone",
        "background_color": TILE,
        "theme_color": PAGE,
        "icons": [
            {"src": f"{at}/app-icon-192.png", "sizes": "192x192", "type": "image/png"},
            {"src": f"{at}/app-icon-512.png", "sizes": "512x512", "type": "image/png"},
            {"src": f"{at}/app-icon-maskable-512.png", "sizes": "512x512", "type": "image/png",
             "purpose": "maskable"},
            {"src": f"{at}/app-icon.svg", "sizes": "any", "type": "image/svg+xml"},
        ],
    }
    (served / "manifest.webmanifest").write_text(json.dumps(manifest, indent=2) + "\n")


for variant in ("male", "female"):
    derive(variant)
