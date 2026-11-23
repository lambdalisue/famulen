"""Eleventh round of Famulen drafts: black and white, no ties.

The secretary pair (W4 and its man counterpart) in one ink. The monocle sits
on the viewer's left eye and its chain falls to the viewer's left chest, away
from the notebook on the right. Black parts over the black figure are cut
loose by a thin white ring.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen8 as q  # noqa: E402
import gen9 as m  # noqa: E402

PHI, B, BOTTOM = q.PHI, q.B, q.BOTTOM
CX, R, HCY, SH_TOP, SH_W, LINE, GAP = q.CX, q.R, q.HCY, q.SH_TOP, q.SH_W, q.LINE, q.GAP
INK, PAPER = "#111111", "#ffffff"
OUT = os.path.dirname(os.path.abspath(__file__))


def monocle(ink, bg):
    ex, ey = CX - R * 0.3, HCY + R * 0.36
    r = R * 0.23
    w = B / PHI**7.3
    sx, sy = ex - r * 0.5, ey + r * 0.87
    tx, ty = CX - SH_W * 0.22, SH_TOP + B / PHI**3.4
    cx, cy = ex - r * 2.6, ey + R * 1.25
    d = f"M{sx:.3f} {sy:.3f} Q{cx:.3f} {cy:.3f} {tx:.3f} {ty:.3f}"
    cw = LINE * 0.42
    return (
        f'<path d="{d}" fill="none" stroke="{bg}" stroke-width="{cw + GAP * 0.7:.3f}" stroke-linecap="round"/>'
        f'<path d="{d}" fill="none" stroke="{ink}" stroke-width="{cw:.3f}" stroke-linecap="round"/>'
        f'<circle cx="{ex:.3f}" cy="{ey:.3f}" r="{r:.3f}" fill="none" stroke="{ink}" stroke-width="{w:.3f}"/>'
    )


def paper_notebook(ink, bg):
    """A white notebook with an ink outline, band and rules, so it reads as paper on the black figure."""
    w, h = B / PHI**1.65, B / PHI**1.55
    cx = CX + B / PHI**3.2
    top = BOTTOM - h + B / PHI**5.5
    x = cx - w / 2
    sw = LINE * 0.9
    tr = f"rotate(-7 {cx:.3f} {top + h / 2:.3f})"
    body = (
        f'<rect x="{x:.3f}" y="{top:.3f}" width="{w:.3f}" height="{h:.3f}" rx="{B / PHI**5.5:.3f}" '
        f'fill="{ink}"/>'
    )
    band = f'<rect x="{x + w - w / PHI**3:.3f}" y="{top:.3f}" width="{LINE * 1.3:.3f}" height="{h:.3f}" fill="{bg}"/>'
    rules = "".join(
        f'<rect x="{x + w / PHI**4:.3f}" y="{top + h / PHI**1.9 + i * h / PHI**3.4:.3f}" width="{w / PHI**0.6:.3f}" '
        f'height="{LINE:.3f}" rx="{LINE / 2:.3f}" fill="{ink}"/>'
        for i in range(0)
    )
    ring = f'<rect x="{x:.3f}" y="{top:.3f}" width="{w:.3f}" height="{h:.3f}" rx="{B / PHI**5.5:.3f}" fill="{bg}" stroke="{bg}" stroke-width="{2 * GAP:.3f}"/>'
    return f'<g transform="{tr}">{ring}{body}{band}{rules}</g>'


def draw(ink=INK, bg=PAPER):
    out = {}
    k = bg[1:]
    cid = "c11" + k
    clip = f'<clipPath id="{cid}"><rect x="0" y="0" width="100" height="{BOTTOM:.3f}"/></clipPath>'

    def wrap(inner):
        return clip + f'<g clip-path="url(#{cid})">{inner}</g>'

    v_d, v_w = B / PHI**2.4, B / PHI**2.3
    # Notebook in ink with paper-coloured details, ringed off the figure.
    notebook = paper_notebook(ink, bg)

    woman = q.shoulders(ink) + q.neck_v(bg, v_d, v_w) + q.bob(ink, bg) + q.face("w" + k, q.SWEPT, bg)
    man = q.shoulders(ink) + q.neck_v(bg, v_d, v_w) + m.short_hair(ink) + m.man_face("m" + k, m.SWEPT_SHORT, bg)
    woman_m = q.shoulders(ink) + q.neck_v(bg, v_d, v_w) + q.bob(ink, bg) + q.face("wm" + k, q.SWEPT, bg)
    man_m = q.shoulders(ink) + q.neck_v(bg, v_d, v_w) + m.short_hair(ink) + m.man_face("mm" + k, m.SWEPT_SHORT, bg)

    out["W4-mono"] = wrap(woman + notebook)
    out["M4a-mono"] = wrap(man + notebook)
    out["M4a-monocle"] = wrap(man_m + notebook + monocle(ink, bg))
    out["W4-monocle"] = wrap(woman_m + notebook + monocle(ink, bg))
    return out


light = draw()
dark = draw(ink=PAPER, bg=INK)

for name, body in light.items():
    with open(os.path.join(OUT, f"famulen11-{name}.svg"), "w") as fh:
        fh.write(
            '<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100" '
            f'role="img" aria-label="Famulen — {name}"><title>Famulen — {name}</title>{body}</svg>\n'
        )


def lockup(body, color):
    mark = 100.0
    text_h = mark / PHI
    return (
        f"<g>{body}</g>"
        f'<text x="{mark + mark / PHI**2:.2f}" y="{mark / 2 + text_h / 2:.2f}" font-family="Outfit" font-weight="600" '
        f'font-size="{text_h / 0.70:.2f}" fill="{color}">Famulen</text>'
    )


names = list(light)
cells = []
for i, name in enumerate(names):
    x = 20 + i * 265
    cells.append(
        f'<g transform="translate({x} 20) scale(1.7)">{light[name]}</g>'
        f'<rect x="{x}" y="200" width="56" height="56" rx="13" fill="{INK}"/>'
        f'<g transform="translate({x} 200) scale(0.56)">{dark[name]}</g>'
        f'<g transform="translate({x + 70} 212) scale(0.32)">{light[name]}</g>'
        f'<g transform="translate({x + 112} 220) scale(0.16)">{light[name]}</g>'
        f'<g transform="translate({x} 270) scale(0.42)">{lockup(light[name], INK)}</g>'
        f'<text x="{x}" y="332" font-family="Helvetica" font-size="14" fill="{INK}">{name}</text>'
    )

sheet = (
    '<svg xmlns="http://www.w3.org/2000/svg" width="1080" height="350" viewBox="0 0 1080 350">'
    f'<rect width="1080" height="350" fill="#ffffff"/>{"".join(cells)}</svg>\n'
)
with open(os.path.join(OUT, "sheet11.svg"), "w") as fh:
    fh.write(sheet)
