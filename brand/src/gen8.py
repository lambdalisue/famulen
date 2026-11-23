"""Eighth round of Famulen drafts: a woman secretary and a man butler.

Hair is told by the hairline: the face is cut out of the head in the
background colour, below a hairline whose shape gives the cut. No features.
Props and dress reuse the previous round's parts.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen7 as p  # noqa: E402  (shared parts: notebook, clipboard, bow tie, rings)

PHI, B, T0, BOTTOM = p.PHI, p.B, p.T0, p.BOTTOM
NAVY, CREAM, BRASS, WHITE = p.NAVY, p.CREAM, p.BRASS, p.WHITE
GAP, LINE = p.GAP, p.LINE
OUT = os.path.dirname(os.path.abspath(__file__))

R = B / PHI**2.95  # head radius
CX = 50.0
HCY = T0 + R * 1.05
SH_TOP = HCY + R + B / PHI**5.2
SH_W = B / PHI**0.12


def shoulders(color):
    r = SH_W / (2 * PHI**0.5)
    x0, x1 = CX - SH_W / 2, CX + SH_W / 2
    return (
        f'<path d="M{x0:.3f} {BOTTOM:.3f} V{SH_TOP + r:.3f} A{r:.3f} {r:.3f} 0 0 1 {x0 + r:.3f} {SH_TOP:.3f} '
        f'H{x1 - r:.3f} A{r:.3f} {r:.3f} 0 0 1 {x1:.3f} {SH_TOP + r:.3f} V{BOTTOM:.3f} Z" fill="{color}"/>'
    )


def face(uid, hairline, bg):
    """Cut the face out below `hairline` (a path from the left edge to the right edge of the head)."""
    fr = R * 0.8
    fcy = HCY + R * 0.14
    region = f"{hairline} V{HCY + 2 * R:.3f} H{CX - 2 * R:.3f} Z"
    return (
        f'<clipPath id="{uid}"><path d="{region}"/></clipPath>'
        f'<circle cx="{CX:.3f}" cy="{fcy:.3f}" r="{fr:.3f}" fill="{bg}" clip-path="url(#{uid})"/>'
    )


def hl(points):
    """Hairline through points given as (dx, dy) in head radii, smoothed with quadratic curves."""
    pts = [(CX + dx * R, HCY + dy * R) for dx, dy in points]
    d = f"M{CX - 2 * R:.3f} {pts[0][1]:.3f} L{pts[0][0]:.3f} {pts[0][1]:.3f}"
    for i in range(1, len(pts) - 1, 2):
        (qx, qy), (x, y) = pts[i], pts[i + 1]
        d += f" Q{qx:.3f} {qy:.3f} {x:.3f} {y:.3f}"
    d += f" L{CX + 2 * R:.3f} {pts[-1][1]:.3f}"
    return d


# Hairlines.
SIDE_PART = hl([(-1, 0.0), (-0.6, -0.42), (-0.32, -0.3), (0.3, -0.12), (1, 0.05)])
SLICKED = hl([(-1, -0.05), (0, -0.42), (1, -0.05)])
BANGS = hl([(-1, 0.05), (0, -0.25), (1, 0.05)])
CURTAIN = hl([(-1, 0.1), (-0.45, -0.1), (0, -0.55), (0.45, -0.1), (1, 0.1)])
SWEPT = hl([(-1, -0.45), (-0.2, -0.45), (0.3, -0.05), (0.7, 0.25), (1, 0.15)])


def head(color):
    return f'<circle cx="{CX:.3f}" cy="{HCY:.3f}" r="{R:.3f}" fill="{color}"/>'


def bob(color, bg):
    w = R * 1.2
    bottom = HCY + R * 1.05
    rr = R * 0.35
    d = (
        f"M{CX - w:.3f} {HCY:.3f} A{w:.3f} {w:.3f} 0 0 1 {CX + w:.3f} {HCY:.3f} V{bottom - rr:.3f} "
        f"A{rr:.3f} {rr:.3f} 0 0 1 {CX + w - rr:.3f} {bottom:.3f} H{CX - w + rr:.3f} "
        f"A{rr:.3f} {rr:.3f} 0 0 1 {CX - w:.3f} {bottom - rr:.3f} Z"
    )
    return p.ringed(f'path d="{d}"', color, bg)


def long_hair(color, bg):
    w = R * 1.18
    bottom = SH_TOP + R * 1.0
    rr = R * 0.5
    d = (
        f"M{CX - w:.3f} {HCY:.3f} A{w:.3f} {w:.3f} 0 0 1 {CX + w:.3f} {HCY:.3f} V{bottom - rr:.3f} "
        f"A{rr:.3f} {rr:.3f} 0 0 1 {CX + w - rr:.3f} {bottom:.3f} H{CX - w + rr:.3f} "
        f"A{rr:.3f} {rr:.3f} 0 0 1 {CX - w:.3f} {bottom - rr:.3f} Z"
    )
    return p.ringed(f'path d="{d}"', color, bg)


def neck_v(bg, depth, width):
    return (
        f'<path d="M{CX - width / 2:.3f} {SH_TOP - 0.3:.3f} L{CX:.3f} {SH_TOP + depth:.3f} '
        f'L{CX + width / 2:.3f} {SH_TOP - 0.3:.3f} Z" fill="{bg}"/>'
    )


def ribbon(color):
    """A blouse ribbon: a small bow with two tails."""
    cy = SH_TOP + B / PHI**5
    w = B / PHI**2.7
    tails = (
        f'<path d="M{CX:.3f} {cy:.3f} L{CX - w * 0.42:.3f} {cy + w * 0.95:.3f} L{CX - w * 0.18:.3f} {cy + w * 0.9:.3f} Z" fill="{color}"/>'
        f'<path d="M{CX:.3f} {cy:.3f} L{CX + w * 0.42:.3f} {cy + w * 0.95:.3f} L{CX + w * 0.18:.3f} {cy + w * 0.9:.3f} Z" fill="{color}"/>'
    )
    return tails + p.bow_tie(CX, cy, w, color)


def tailcoat(bg, tie):
    v = neck_v(bg, depth=B / PHI**1.55, width=B / PHI**1.9)
    lapels = "".join(
        f'<path d="M{CX + s * SH_W * 0.2:.3f} {SH_TOP + 0.5:.3f} L{CX + s * B / PHI**4.2:.3f} {SH_TOP + B / PHI**1.55:.3f}" '
        f'stroke="{bg}" stroke-width="{LINE * 1.2:.3f}" stroke-linecap="round"/>'
        for s in (-1, 1)
    )
    return v + lapels + p.bow_tie(CX, SH_TOP + B / PHI**5, B / PHI**2.45, tie)


def pocket_square(color, bg):
    x, y = CX + SH_W * 0.26, SH_TOP + B / PHI**2.3
    s = B / PHI**4.6
    return p.ringed(
        f'path d="M{x - s:.3f} {y:.3f} L{x - s * 0.35:.3f} {y - s:.3f} L{x + s * 0.35:.3f} {y - s * 0.55:.3f} L{x + s:.3f} {y:.3f} Z"',
        color,
        bg,
        ring=GAP * 0.6,
    )


def nb(prop, bg, detail, small=False):
    w, h = B / PHI**1.65, B / PHI**1.55
    if small:
        w, h = w / PHI**0.45, h / PHI**0.45
    cx = CX + (B / PHI**2.45 if small else B / PHI**3.2)
    top = BOTTOM - h + B / PHI**(4.4 if small else 5.5)
    return p.notebook(cx, top, w, h, -7, prop, bg, detail)


def cb(prop, bg, detail):
    w, h = B / PHI**1.6, B / PHI**1.55
    return p.clipboard(CX + B / PHI**3.2, BOTTOM - h + B / PHI**4.6, w, h, prop, bg, detail, bg)


def draw(fig=NAVY, prop=BRASS, bg=WHITE, detail=NAVY):
    out = {}
    k = bg[1:]
    cid = "c8" + k
    clip = f'<clipPath id="{cid}"><rect x="0" y="0" width="100" height="{BOTTOM:.3f}"/></clipPath>'

    def wrap(inner):
        return clip + f'<g clip-path="url(#{cid})">{inner}</g>'

    # Secretary (woman).
    out["W1-bob-ribbon"] = wrap(shoulders(fig) + neck_v(bg, B / PHI**2.4, B / PHI**2.3) + ribbon(prop) + bob(fig, bg) + face("w1" + k, BANGS, bg))
    out["W2-bob-notebook"] = wrap(
        shoulders(fig) + neck_v(bg, B / PHI**2.4, B / PHI**2.3) + ribbon(prop) + bob(fig, bg) + face("w2" + k, BANGS, bg) + nb(prop, bg, detail, small=True)
    )
    out["W3-long-clipboard"] = wrap(shoulders(fig) + long_hair(fig, bg) + face("w3" + k, CURTAIN, bg) + cb(prop, bg, detail))
    out["W4-swept-notebook"] = wrap(
        shoulders(fig) + neck_v(bg, B / PHI**2.4, B / PHI**2.3) + bob(fig, bg) + face("w4" + k, SWEPT, bg) + nb(prop, bg, detail)
    )

    # Butler (man).
    out["M1-part-tailcoat"] = wrap(shoulders(fig) + tailcoat(bg, prop) + head(fig) + face("m1" + k, SIDE_PART, bg))
    out["M2-part-notebook"] = wrap(
        shoulders(fig) + tailcoat(bg, prop) + head(fig) + face("m2" + k, SIDE_PART, bg) + nb(prop, bg, detail, small=True)
    )
    out["M3-slick-pocket"] = wrap(shoulders(fig) + tailcoat(bg, prop) + pocket_square(prop, fig) + head(fig) + face("m3" + k, SLICKED, bg))
    return out


light = draw()
dark = draw(fig=CREAM, bg=NAVY, detail=NAVY)

for name, body in light.items():
    with open(os.path.join(OUT, f"famulen8-{name}.svg"), "w") as fh:
        fh.write(
            '<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100" '
            f'role="img" aria-label="Famulen — {name}"><title>Famulen — {name}</title>{body}</svg>\n'
        )

mark = 100.0
text_h = mark / PHI
font_size = text_h / 0.70
gap = mark / PHI**2
baseline = mark / 2 + text_h / 2


def lockup(body, color):
    return (
        f"<g>{body}</g>"
        f'<text x="{mark + gap:.2f}" y="{baseline:.2f}" font-family="Outfit" font-weight="600" '
        f'font-size="{font_size:.2f}" fill="{color}">Famulen</text>'
    )


names = list(light)
cells = []
for i, name in enumerate(names):
    col, row = (i % 4, 0) if name.startswith("W") else ((i - 4) % 4, 1)
    x = 20 + col * 265
    y = 20 + row * 330
    cells.append(
        f'<g transform="translate({x} {y}) scale(1.7)">{light[name]}</g>'
        f'<rect x="{x}" y="{y + 180}" width="56" height="56" rx="13" fill="{NAVY}"/>'
        f'<g transform="translate({x} {y + 180}) scale(0.56)">{dark[name]}</g>'
        f'<g transform="translate({x + 70} {y + 192}) scale(0.32)">{light[name]}</g>'
        f'<g transform="translate({x + 112} {y + 200}) scale(0.16)">{light[name]}</g>'
        f'<g transform="translate({x} {y + 250}) scale(0.42)">{lockup(light[name], NAVY)}</g>'
        f'<text x="{x}" y="{y + 312}" font-family="Helvetica" font-size="14" fill="{NAVY}">{name}</text>'
    )

sheet = (
    '<svg xmlns="http://www.w3.org/2000/svg" width="1080" height="680" viewBox="0 0 1080 680">'
    f'<rect width="1080" height="680" fill="#ffffff"/>{"".join(cells)}</svg>\n'
)
with open(os.path.join(OUT, "sheet8.svg"), "w") as fh:
    fh.write(sheet)
