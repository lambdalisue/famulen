"""Famulen marks, icons and lockups for the chosen pair.

The secretary with a centre-parted bob and the one with ear-length hair, both
holding a Moleskine-style notebook, in one ink. Cut-outs (the face, the rings
that part the notebook and hair from the figure) are real holes: the drawing
is rendered into a luminance mask and the ink is a rectangle seen through it,
so the marks sit on any background.

Wordmark: Outfit SemiBold, outlined, with the font's pair kerning, laid out by
the rules in brand/README.md (the mark's height is its drawn extent).
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen8 as q  # noqa: E402
import gen9 as m  # noqa: E402
import gen11 as w  # noqa: E402
import gen12 as h  # noqa: E402

from fontTools.pens.svgPathPen import SVGPathPen  # noqa: E402
from fontTools.pens.transformPen import TransformPen  # noqa: E402
from fontTools.ttLib import TTFont  # noqa: E402

PHI = q.PHI
B = q.B
CX, R, HCY = q.CX, q.R, q.HCY
INK, PAPER = "#111111", "#ffffff"
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "final")
os.makedirs(OUT, exist_ok=True)
FONT = os.path.join(HERE, "fonts", "Outfit-SemiBold.ttf")

# Ear-length hair: a fuller crown, sides that stop at the ear and tuck in,
# the fringe swept to the viewer's right.
EAR_SWEPT = q.hl([(-1, -0.3), (-0.4, -0.85), (0.05, -0.5), (0.6, -0.2), (1, -0.25)])


def ear_hair(ink, bg):
    wdt = R * 1.1
    bottom = HCY + R * 0.5
    rr = R * 0.5
    crown = (
        f'<ellipse cx="{CX + R * 0.06:.3f}" cy="{HCY - R * 0.3:.3f}" rx="{R * 1.06:.3f}" ry="{R * 0.8:.3f}" fill="{ink}"/>'
    )
    sides = h.path_hair(h.rounded_drop(wdt, bottom, rr), ink, bg)
    return sides + crown


def face_groove(uid, hairline, bg):
    """Keep the face solid and part it from the hair by a thin groove: the
    hairline inside the face, and the face's edge below the hairline."""
    fr = R * 0.84
    fcy = HCY + R - fr
    gw = q.GAP * 1.3
    below = f"{hairline} V{HCY + 2 * R:.3f} H{CX - 2 * R:.3f} Z"
    return (
        f'<clipPath id="{uid}-below"><path d="{below}"/></clipPath>'
        f'<clipPath id="{uid}-face"><circle cx="{CX:.3f}" cy="{fcy:.3f}" r="{fr:.3f}"/></clipPath>'
        f'<circle cx="{CX:.3f}" cy="{fcy:.3f}" r="{fr:.3f}" fill="none" stroke="{bg}" stroke-width="{gw:.3f}" clip-path="url(#{uid}-below)"/>'
        f'<path d="{hairline}" fill="none" stroke="{bg}" stroke-width="{gw:.3f}" clip-path="url(#{uid}-face)"/>'
    )


def figure(kind, ink, bg, uid, notebook="ink"):
    if notebook == "only":
        return w.paper_notebook(ink, bg)
    body = q.shoulders(ink) + q.neck_v(bg, B / PHI**2.4, B / PHI**2.3)
    if kind == "bob":
        hair = q.bob(ink, bg) + q.face(uid, h.CURTAIN, bg)
    else:
        hair = h.ear_length(ink, bg) + m.man_face(uid, h.M_EAR, bg)
    cid = "clip-" + uid
    clip = f'<clipPath id="{cid}"><rect x="0" y="0" width="100" height="{q.BOTTOM:.3f}"/></clipPath>'
    return clip + f'<g clip-path="url(#{cid})">{body}{hair}{w.paper_notebook(bg if notebook == "hole" else ink, bg)}</g>'


def mark_group(kind, color, uid, nb=None):
    """The mark in `color` with transparent cut-outs, in the 100x100 box (drawn extent 18..82).
    With `nb`, the notebook is drawn in that colour and the figure is cut around it."""
    mask = f"mask-{uid}"
    if nb is None:
        return (
            f'<defs><mask id="{mask}" maskUnits="userSpaceOnUse" x="0" y="0" width="100" height="100">'
            f'<rect width="100" height="100" fill="#000"/>{figure(kind, "#fff", "#000", uid)}</mask></defs>'
            f'<rect width="100" height="100" fill="{color}" mask="url(#{mask})"/>'
        )
    return (
        f'<defs><mask id="{mask}" maskUnits="userSpaceOnUse" x="0" y="0" width="100" height="100">'
        f'<rect width="100" height="100" fill="#000"/>{figure(kind, "#fff", "#000", uid, notebook="hole")}</mask>'
        f'<mask id="{mask}-nb" maskUnits="userSpaceOnUse" x="0" y="0" width="100" height="100">'
        f'<rect width="100" height="100" fill="#000"/>{figure(kind, "#fff", "#000", uid + "-nb", notebook="only")}</mask></defs>'
        f'<rect width="100" height="100" fill="{color}" mask="url(#{mask})"/>'
        f'<rect width="100" height="100" fill="{nb}" mask="url(#{mask}-nb)"/>'
    )


# --- wordmark -------------------------------------------------------------

font = TTFont(FONT)
glyphset = font.getGlyphSet()
cmap = font.getBestCmap()
upm = font["head"].unitsPerEm
hmtx = font["hmtx"]


def pair_kerning(left, right):
    """Pair adjustment from GPOS PairPos lookups (formats 1 and 2), in font units."""
    if "GPOS" not in font:
        return 0
    gpos = font["GPOS"].table
    total = 0
    for lookup in gpos.LookupList.Lookup:
        subs = lookup.SubTable
        if lookup.LookupType == 9:
            subs = [s.ExtSubTable for s in subs]
        elif lookup.LookupType != 2:
            continue
        for st in subs:
            if getattr(st, "LookupType", 2) != 2 or left not in st.Coverage.glyphs:
                continue
            if st.Format == 1:
                idx = st.Coverage.glyphs.index(left)
                for rec in st.PairSet[idx].PairValueRecord:
                    if rec.SecondGlyph == right and rec.Value1 is not None:
                        total += getattr(rec.Value1, "XAdvance", 0) or 0
            elif st.Format == 2:
                c1 = st.ClassDef1.classDefs.get(left, 0)
                c2 = st.ClassDef2.classDefs.get(right, 0)
                rec = st.Class1Record[c1].Class2Record[c2]
                if rec.Value1 is not None:
                    total += getattr(rec.Value1, "XAdvance", 0) or 0
            break
    return total


def word_path(text):
    """Outline `text` in font units, baseline at y=0, y pointing down. Returns (d, width)."""
    names = [cmap[ord(c)] for c in text]
    pen = SVGPathPen(glyphset)
    x = 0
    for i, name in enumerate(names):
        glyphset[name].draw(TransformPen(pen, (1, 0, 0, -1, x, 0)))
        x += hmtx[name][0]
        if i + 1 < len(names):
            x += pair_kerning(name, names[i + 1])
    return pen.getCommands(), x


WORD, WORD_W = word_path("Famulen")
L_TOP = glyphset[cmap[ord("l")]]
_bp = __import__("fontTools.pens.boundsPen", fromlist=["BoundsPen"]).BoundsPen(glyphset)
L_TOP.draw(_bp)
L_HEIGHT = _bp.bounds[3]  # top of "l" above the baseline, font units

MARK = 100.0  # the mark's drawn height in the lockups
SCALE_IN = MARK / B  # 64 drawn units -> 100


def placed_mark(kind, color, uid, x, y, nb=None):
    """Place the mark so its drawn extent (18..82) lands at (x, y) with height MARK."""
    s = SCALE_IN
    return f'<g transform="translate({x - 18 * s:.3f} {y - 18 * s:.3f}) scale({s:.4f})">{mark_group(kind, color, uid, nb)}</g>'


def horizontal(kind, color, uid, nb=None):
    text_h = MARK / PHI
    s = text_h / L_HEIGHT
    gap = MARK / PHI**2
    baseline = MARK / 2 + text_h / 2
    width = MARK + gap + WORD_W * s
    body = placed_mark(kind, color, uid, 0, 0, nb) + (
        f'<path transform="translate({MARK + gap:.3f} {baseline:.3f}) scale({s:.5f})" d="{WORD}" fill="{color}"/>'
    )
    return width, MARK, body


def vertical(kind, color, uid, nb=None):
    text_w = MARK * PHI
    s = text_w / WORD_W
    gap = MARK / PHI**2
    asc = L_HEIGHT * s
    width = text_w
    body = placed_mark(kind, color, uid, (width - MARK) / 2, 0, nb) + (
        f'<path transform="translate(0 {MARK + gap + asc:.3f}) scale({s:.5f})" d="{WORD}" fill="{color}"/>'
    )
    # Descenders: "Famulen" has none, so the height ends at the baseline.
    return width, MARK + gap + asc, body


def svg(w_, h_, body, title):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w_:.3f}" height="{h_:.3f}" viewBox="0 0 {w_:.3f} {h_:.3f}" '
        f'role="img" aria-label="{title}"><title>{title}</title>{body}</svg>\n'
    )


KIND = "ear"  # ear-length hair, the chosen figure
COLORS = {
    "brass": "#d6a24a",
    "red": "#c8302c",
    "oxblood": "#a8332c",
    "green": "#2f6b4f",
    "ultramarine": "#2d4fa3",
}

# Clear earlier outputs so the folder only holds the chosen figure.
for name in os.listdir(OUT):
    os.remove(os.path.join(OUT, name))


def write_set(prefix, nb, kind=KIND):
    for variant, color in (("", INK), ("-light", PAPER)):
        uid = f"{prefix}{variant}"
        with open(os.path.join(OUT, f"{prefix}-mark{variant}.svg"), "w") as fh:
            fh.write(svg(100, 100, mark_group(kind, color, uid, nb), "Famulen"))
        wdt, hgt, body = horizontal(kind, color, uid + "-h", nb)
        with open(os.path.join(OUT, f"{prefix}-logo-horizontal{variant}.svg"), "w") as fh:
            fh.write(svg(wdt, hgt, body, "Famulen"))
        wdt, hgt, body = vertical(kind, color, uid + "-v", nb)
        with open(os.path.join(OUT, f"{prefix}-logo-vertical{variant}.svg"), "w") as fh:
            fh.write(svg(wdt, hgt, body, "Famulen"))
    icon = f'<rect width="100" height="100" rx="23" fill="{INK}"/>' + mark_group(kind, PAPER, prefix + "-icon", nb)
    with open(os.path.join(OUT, f"{prefix}-icon.svg"), "w") as fh:
        fh.write(svg(100, 100, icon, "Famulen"))


def write_avatar(prefix, nb, kind):
    """Slack app / bot avatar: a full-bleed square (Slack rounds the corners itself),
    the figure a little smaller so the rounding never touches it."""
    s = 0.86
    off = 50 - 50 * s
    body = (
        f'<rect width="100" height="100" fill="{INK}"/>'
        f'<g transform="translate({off:.3f} {off + 3:.3f}) scale({s})">{mark_group(kind, PAPER, prefix + "-avatar", nb)}</g>'
    )
    with open(os.path.join(OUT, f"{prefix}-avatar.svg"), "w") as fh:
        fh.write(svg(100, 100, body, "Famulen"))


RED = COLORS["red"]
# The chosen figure (ear-length hair): black and white, and the red notebook.
write_set("famulen", None)
write_set("famulen-red", RED)
write_avatar("famulen-red", RED, KIND)
write_avatar("famulen", None, KIND)
# The woman variant the owner can choose instead (centre-parted bob).
write_set("famulen-woman", None, "bob")
write_set("famulen-woman-red", RED, "bob")
write_avatar("famulen-woman-red", RED, "bob")
write_avatar("famulen-woman", None, "bob")
# Rejected notebook colours, kept for reference.
for cname, c in COLORS.items():
    if cname != "red":
        write_set(f"famulen-{cname}", c)

# Review sheet: one row per version (black and white, then each notebook colour):
# the mark, the icon at 64/32/16 px, the horizontal lockup on white and on black.
rows = [("mono", None)] + list(COLORS.items())
cells = []
for r, (cname, c) in enumerate(rows):
    y = 15 + r * 130
    cells.append(f'<g transform="translate(0 {y - 12}) scale(1.2)">{mark_group(KIND, INK, f"r{r}m", c)}</g>')
    icon = f'<rect width="100" height="100" rx="23" fill="{INK}"/>' + mark_group(KIND, PAPER, f"r{r}i", c)
    for x, size in ((130, 96), (236, 64), (310, 32), (352, 16)):
        cells.append(f'<g transform="translate({x} {y}) scale({size / 100})">{icon}</g>')
    _, _, body = horizontal(KIND, INK, f"r{r}h", c)
    cells.append(f'<g transform="translate(390 {y + 15}) scale(0.4)">{body}</g>')
    cells.append(f'<rect x="590" y="{y - 10}" width="480" height="120" fill="{INK}"/>')
    _, _, body = horizontal(KIND, PAPER, f"r{r}hl", c)
    cells.append(f'<g transform="translate(615 {y + 15}) scale(0.4)">{body}</g>')
    cells.append(f'<g transform="translate(840 {y}) scale(0.5)">{vertical(KIND, PAPER, f"r{r}v", c)[2]}</g>')
    cells.append(f'<text x="1000" y="{y + 100}" font-family="Helvetica" font-size="13" fill="#888">{cname}</text>')

H = 15 + len(rows) * 130
sheet = (
    f'<svg xmlns="http://www.w3.org/2000/svg" width="1080" height="{H}" viewBox="0 0 1080 {H}">'
    f'<rect width="1080" height="{H}" fill="#ffffff"/>{"".join(cells)}</svg>\n'
)
with open(os.path.join(HERE, "sheet-final.svg"), "w") as fh:
    fh.write(sheet)
