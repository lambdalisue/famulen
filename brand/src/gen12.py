"""Twelfth round of Famulen drafts: W4 and M4a with other haircuts.

Black and white, the Moleskine-style notebook, the plain V neck. Only the
hair changes: its outline and the hairline the face is cut below.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen8 as q  # noqa: E402
import gen9 as m  # noqa: E402
import gen11 as w  # noqa: E402

PHI, B, BOTTOM = q.PHI, q.B, q.BOTTOM
CX, R, HCY, SH_TOP, SH_W = q.CX, q.R, q.HCY, q.SH_TOP, q.SH_W
INK, PAPER = w.INK, w.PAPER
OUT = os.path.dirname(os.path.abspath(__file__))


def path_hair(d, ink, bg):
    return q.p.ringed(f'path d="{d}"', ink, bg)


def rounded_drop(w_, bottom, rr):
    """Head-topped hair that falls straight to `bottom` with rounded corners."""
    return (
        f"M{CX - w_:.3f} {HCY:.3f} A{w_:.3f} {w_:.3f} 0 0 1 {CX + w_:.3f} {HCY:.3f} V{bottom - rr:.3f} "
        f"A{rr:.3f} {rr:.3f} 0 0 1 {CX + w_ - rr:.3f} {bottom:.3f} H{CX - w_ + rr:.3f} "
        f"A{rr:.3f} {rr:.3f} 0 0 1 {CX - w_:.3f} {bottom - rr:.3f} Z"
    )


def asym_bob(ink, bg):
    """Shorter on the viewer's left, longer on the right, the hem slanting between."""
    w_ = R * 1.2
    lb, rb = HCY + R * 0.6, HCY + R * 1.25
    rr = R * 0.3
    d = (
        f"M{CX - w_:.3f} {HCY:.3f} A{w_:.3f} {w_:.3f} 0 0 1 {CX + w_:.3f} {HCY:.3f} V{rb - rr:.3f} "
        f"A{rr:.3f} {rr:.3f} 0 0 1 {CX + w_ - rr:.3f} {rb:.3f} L{CX - w_ + rr:.3f} {lb:.3f} "
        f"A{rr:.3f} {rr:.3f} 0 0 1 {CX - w_:.3f} {lb - rr:.3f} Z"
    )
    return path_hair(d, ink, bg)


def quiff(ink):
    """Short hair with the front lifted: the head plus a raised crown."""
    return (
        f'<circle cx="{CX:.3f}" cy="{HCY:.3f}" r="{R:.3f}" fill="{ink}"/>'
        f'<ellipse cx="{CX + R * 0.12:.3f}" cy="{HCY - R * 0.42:.3f}" rx="{R * 1.02:.3f}" ry="{R * 0.82:.3f}" fill="{ink}"/>'
    )


def ear_length(ink, bg):
    return path_hair(rounded_drop(R * 1.1, HCY + R * 0.55, R * 0.45), ink, bg)


# Hairlines (points in head radii from the head's centre).
BANGS = q.BANGS
CURTAIN = q.CURTAIN
SWEPT = q.SWEPT
PIXIE = q.hl([(-1, -0.2), (-0.35, -0.3), (0.2, 0.0), (0.65, 0.3), (1, 0.25)])
ASYM = q.hl([(-1, -0.15), (-0.3, -0.35), (0.3, -0.1), (0.7, 0.15), (1, 0.1)])
M_SIDE = q.hl([(-1, -0.05), (-0.62, -0.95), (-0.38, -0.5), (0.3, -0.35), (1, 0.0)])
M_SLICK = q.hl([(-1, -0.1), (0, -1.05), (1, -0.1)])
M_CENTER = q.hl([(-1, -0.05), (-0.45, -0.25), (0, -0.6), (0.45, -0.25), (1, -0.05)])
M_QUIFF = q.hl([(-1, -0.2), (0, -0.75), (1, -0.2)])
M_EAR = q.hl([(-1, -0.05), (-0.3, -0.35), (0.3, -0.1), (0.7, 0.1), (1, 0.05)])


def body(ink, bg):
    return q.shoulders(ink) + q.neck_v(bg, B / PHI**2.4, B / PHI**2.3)


def draw(ink=INK, bg=PAPER):
    k = bg[1:]
    cid = "c12" + k
    clip = f'<clipPath id="{cid}"><rect x="0" y="0" width="100" height="{BOTTOM:.3f}"/></clipPath>'

    def wrap(inner):
        return clip + f'<g clip-path="url(#{cid})">{inner}</g>' + ""

    nb = w.paper_notebook(ink, bg)
    women = {
        "W4-now": q.bob(ink, bg) + q.face("w0" + k, SWEPT, bg),
        "W-bangs-bob": q.bob(ink, bg) + q.face("w1" + k, BANGS, bg),
        "W-center-bob": q.bob(ink, bg) + q.face("w2" + k, CURTAIN, bg),
        "W-shoulder": path_hair(rounded_drop(R * 1.16, SH_TOP + R * 0.55, R * 0.45), ink, bg) + q.face("w3" + k, SWEPT, bg),
        "W-asym-bob": asym_bob(ink, bg) + q.face("w4" + k, ASYM, bg),
        "W-pixie": m.short_hair(ink) + q.face("w5" + k, PIXIE, bg),
    }
    men = {
        "M4a-now": m.short_hair(ink) + m.man_face("m0" + k, m.SWEPT_SHORT, bg),
        "M-side-part": m.short_hair(ink) + m.man_face("m1" + k, M_SIDE, bg),
        "M-slicked": m.short_hair(ink) + m.man_face("m2" + k, M_SLICK, bg),
        "M-center": m.short_hair(ink) + m.man_face("m3" + k, M_CENTER, bg),
        "M-quiff": quiff(ink) + m.man_face("m4" + k, M_QUIFF, bg),
        "M-ear-length": ear_length(ink, bg) + m.man_face("m5" + k, M_EAR, bg),
    }
    out = {}
    for name, hair in {**women, **men}.items():
        out[name] = wrap(body(ink, bg) + hair + nb)
    return out


light = draw()
dark = draw(ink=PAPER, bg=INK)

for name, body_ in light.items():
    with open(os.path.join(OUT, f"famulen12-{name}.svg"), "w") as fh:
        fh.write(
            '<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100" '
            f'role="img" aria-label="Famulen — {name}"><title>Famulen — {name}</title>{body_}</svg>\n'
        )

names = list(light)
cells = []
for i, name in enumerate(names):
    col, row = i % 6, i // 6
    x = 15 + col * 178
    y = 15 + row * 290
    cells.append(
        f'<g transform="translate({x} {y}) scale(1.5)">{light[name]}</g>'
        f'<rect x="{x}" y="{y + 165}" width="56" height="56" rx="13" fill="{INK}"/>'
        f'<g transform="translate({x} {y + 165}) scale(0.56)">{dark[name]}</g>'
        f'<g transform="translate({x + 68} {y + 177}) scale(0.32)">{light[name]}</g>'
        f'<g transform="translate({x + 110} {y + 185}) scale(0.16)">{light[name]}</g>'
        f'<g transform="translate({x} {y + 230}) scale(0.3)">{w.lockup(light[name], INK)}</g>'
        f'<text x="{x}" y="{y + 275}" font-family="Helvetica" font-size="13" fill="{INK}">{name}</text>'
    )

sheet = (
    '<svg xmlns="http://www.w3.org/2000/svg" width="1080" height="600" viewBox="0 0 1080 600">'
    f'<rect width="1080" height="600" fill="#ffffff"/>{"".join(cells)}</svg>\n'
)
with open(os.path.join(OUT, "sheet12.svg"), "w") as fh:
    fh.write(sheet)
