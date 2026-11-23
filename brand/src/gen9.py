"""Ninth round of Famulen drafts: the man's counterpart of W4.

W4 is a swept-fringe bob with a plain V neck and a notebook. The man keeps
the same build: short hair with the fringe swept to one side, the same V neck
and notebook. Variants differ only at the neck.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen8 as q  # noqa: E402
import gen7 as p  # noqa: E402

PHI, B, BOTTOM = q.PHI, q.B, q.BOTTOM
NAVY, CREAM, BRASS, WHITE = q.NAVY, q.CREAM, q.BRASS, q.WHITE
CX, R, HCY, SH_TOP, SH_W = q.CX, q.R, q.HCY, q.SH_TOP, q.SH_W
LINE = q.LINE
OUT = os.path.dirname(os.path.abspath(__file__))

# Short hair, fringe swept from the left down to the right: the same sweep as W4, cut shorter.
SWEPT_SHORT = q.hl([(-1, -0.3), (-0.25, -0.55), (0.25, -0.3), (0.7, -0.05), (1, -0.1)])


def short_hair(color):
    """Short hair: the head itself, a little fuller on top."""
    return (
        f'<circle cx="{CX:.3f}" cy="{HCY:.3f}" r="{R:.3f}" fill="{color}"/>'
        f'<ellipse cx="{CX + R * 0.08:.3f}" cy="{HCY - R * 0.28:.3f}" rx="{R * 1.04:.3f}" ry="{R * 0.8:.3f}" fill="{color}"/>'
    )


def man_face(uid, hairline, bg):
    """Face cut flush with the bottom of the head, so no ring is left under the chin."""
    fr = R * 0.84
    fcy = HCY + R - fr
    region = f"{hairline} V{HCY + 2 * R:.3f} H{CX - 2 * R:.3f} Z"
    return (
        f'<clipPath id="{uid}"><path d="{region}"/></clipPath>'
        f'<circle cx="{CX:.3f}" cy="{fcy:.3f}" r="{fr:.3f}" fill="{bg}" clip-path="url(#{uid})"/>'
    )


def slim_tie(color):
    k = B / PHI**5
    top = SH_TOP + B / PHI**6
    end = SH_TOP + B / PHI**2.4
    w = B / PHI**4.3
    return (
        f'<path d="M{CX - k / 2:.3f} {top:.3f} H{CX + k / 2:.3f} L{CX + k / 3:.3f} {top + k:.3f} H{CX - k / 3:.3f} Z" fill="{color}"/>'
        f'<path d="M{CX - k / 3:.3f} {top + k + LINE * 0.8:.3f} H{CX + k / 3:.3f} L{CX + w / 2:.3f} {end - w:.3f} '
        f'L{CX:.3f} {end:.3f} L{CX - w / 2:.3f} {end - w:.3f} Z" fill="{color}"/>'
    )


def lapels(bg):
    return "".join(
        f'<path d="M{CX + s * SH_W * 0.2:.3f} {SH_TOP + 0.5:.3f} L{CX + s * B / PHI**4.6:.3f} {SH_TOP + B / PHI**2.2:.3f}" '
        f'stroke="{bg}" stroke-width="{LINE * 1.2:.3f}" stroke-linecap="round"/>'
        for s in (-1, 1)
    )


def draw(fig=NAVY, prop=BRASS, bg=WHITE, detail=NAVY):
    out = {}
    k = bg[1:]
    cid = "c9" + k
    clip = f'<clipPath id="{cid}"><rect x="0" y="0" width="100" height="{BOTTOM:.3f}"/></clipPath>'

    def wrap(inner):
        return clip + f'<g clip-path="url(#{cid})">{inner}</g>'

    v_d, v_w = B / PHI**2.4, B / PHI**2.3
    out["W4-swept-notebook"] = wrap(
        q.shoulders(fig) + q.neck_v(bg, v_d, v_w) + q.bob(fig, bg) + q.face("w4" + k, q.SWEPT, bg) + q.nb(prop, bg, detail)
    )
    out["M4a-swept-notebook"] = wrap(
        q.shoulders(fig) + q.neck_v(bg, v_d, v_w) + short_hair(fig) + man_face("a" + k, SWEPT_SHORT, bg) + q.nb(prop, bg, detail)
    )
    deep_d = B / PHI**1.7
    out["M4b-necktie"] = wrap(
        q.shoulders(fig)
        + q.neck_v(bg, deep_d, v_w)
        + slim_tie(prop)
        + short_hair(fig)
        + man_face("b" + k, SWEPT_SHORT, bg)
        + q.nb(prop, bg, detail, small=True)
    )
    out["M4c-lapels"] = wrap(
        q.shoulders(fig)
        + q.neck_v(bg, deep_d, v_w * PHI**0.3)
        + lapels(bg)
        + short_hair(fig)
        + man_face("c" + k, SWEPT_SHORT, bg)
        + q.nb(prop, bg, detail, small=True)
    )
    return out


light = draw()
dark = draw(fig=CREAM, bg=NAVY, detail=NAVY)

for name, body in light.items():
    with open(os.path.join(OUT, f"famulen9-{name}.svg"), "w") as fh:
        fh.write(
            '<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100" '
            f'role="img" aria-label="Famulen — {name}"><title>Famulen — {name}</title>{body}</svg>\n'
        )

names = list(light)
cells = []
for i, name in enumerate(names):
    x = 20 + i * 265
    cells.append(
        f'<g transform="translate({x} 20) scale(1.7)">{light[name]}</g>'
        f'<rect x="{x}" y="200" width="56" height="56" rx="13" fill="{NAVY}"/>'
        f'<g transform="translate({x} 200) scale(0.56)">{dark[name]}</g>'
        f'<g transform="translate({x + 70} 212) scale(0.32)">{light[name]}</g>'
        f'<g transform="translate({x + 112} 220) scale(0.16)">{light[name]}</g>'
        f'<g transform="translate({x} 270) scale(0.42)">{q.lockup(light[name], NAVY)}</g>'
        f'<text x="{x}" y="332" font-family="Helvetica" font-size="14" fill="{NAVY}">{name}</text>'
    )

sheet = (
    '<svg xmlns="http://www.w3.org/2000/svg" width="1080" height="350" viewBox="0 0 1080 350">'
    f'<rect width="1080" height="350" fill="#ffffff"/>{"".join(cells)}</svg>\n'
)
with open(os.path.join(OUT, "sheet9.svg"), "w") as fh:
    fh.write(sheet)
