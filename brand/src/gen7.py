"""Seventh round of Famulen drafts: the notebook bust, made to read as a secretary.

Bigger head than the previous round. Each variant adds one secretary cue to
the bust: a steno pad and pen, a clipboard checklist, a pen behind the ear,
glasses, an office collar and necktie, or the butler's collar and bow tie.
"""

import os

PHI = (1 + 5**0.5) / 2
B = 64.0
L0, T0 = 18.0, 18.0
BOTTOM = T0 + B
NAVY, CREAM, BRASS, WHITE = "#232a3a", "#f2ecdf", "#d6a24a", "#ffffff"
OUT = os.path.dirname(os.path.abspath(__file__))

GAP = B / PHI**5.5
LINE = B / PHI**7
HEAD_R = B / PHI**2.85
CX = 50.0
HCY = T0 + HEAD_R
SH_TOP = HCY + HEAD_R + B / PHI**5.5
SH_W = B / PHI**0.15


def ringed(el, fill, bg, ring=GAP):
    return (
        f'<{el} fill="{bg}" stroke="{bg}" stroke-width="{2 * ring:.3f}" stroke-linejoin="round" stroke-linecap="round"/>'
        f'<{el} fill="{fill}"/>'
    )


def stroked(d, color, width, bg):
    return (
        f'<path d="{d}" fill="none" stroke="{bg}" stroke-width="{width + 2 * GAP:.3f}" stroke-linecap="round" stroke-linejoin="round"/>'
        f'<path d="{d}" fill="none" stroke="{color}" stroke-width="{width:.3f}" stroke-linecap="round" stroke-linejoin="round"/>'
    )


def figure(color):
    r = SH_W / (2 * PHI**0.5)
    x0, x1 = CX - SH_W / 2, CX + SH_W / 2
    return (
        f'<circle cx="{CX:.3f}" cy="{HCY:.3f}" r="{HEAD_R:.3f}" fill="{color}"/>'
        f'<path d="M{x0:.3f} {BOTTOM:.3f} V{SH_TOP + r:.3f} A{r:.3f} {r:.3f} 0 0 1 {x0 + r:.3f} {SH_TOP:.3f} '
        f'H{x1 - r:.3f} A{r:.3f} {r:.3f} 0 0 1 {x1:.3f} {SH_TOP + r:.3f} V{BOTTOM:.3f} Z" fill="{color}"/>'
    )


def bow_tie(cx, cy, w, color):
    h = w / PHI
    k = w / PHI**2.6
    wing = (
        f"M{cx:.3f} {cy:.3f} L{cx - w / 2:.3f} {cy - h / 2:.3f} Q{cx - w / 2 - k / 2:.3f} {cy:.3f} {cx - w / 2:.3f} {cy + h / 2:.3f} Z "
        f"M{cx:.3f} {cy:.3f} L{cx + w / 2:.3f} {cy - h / 2:.3f} Q{cx + w / 2 + k / 2:.3f} {cy:.3f} {cx + w / 2:.3f} {cy + h / 2:.3f} Z"
    )
    return (
        f'<path d="{wing}" fill="{color}" stroke="{color}" stroke-width="{k / 2:.3f}" stroke-linejoin="round"/>'
        f'<rect x="{cx - k / 2:.3f}" y="{cy - k * 0.6:.3f}" width="{k:.3f}" height="{k * 1.2:.3f}" rx="{k / 3:.3f}" fill="{color}"/>'
    )


def collar_v(shirt, depth=B / PHI**1.9, width=B / PHI**2.1):
    return (
        f'<path d="M{CX - width / 2:.3f} {SH_TOP - 0.3:.3f} L{CX:.3f} {SH_TOP + depth:.3f} '
        f'L{CX + width / 2:.3f} {SH_TOP - 0.3:.3f} Z" fill="{shirt}"/>'
    )


def necktie(color):
    k = B / PHI**4.4  # knot
    w = B / PHI**3.6  # blade width
    top = SH_TOP + B / PHI**6
    knot = f'<path d="M{CX - k / 2:.3f} {top:.3f} H{CX + k / 2:.3f} L{CX + k / 3:.3f} {top + k:.3f} H{CX - k / 3:.3f} Z" fill="{color}"/>'
    blade = (
        f'<path d="M{CX - k / 3:.3f} {top + k + LINE:.3f} H{CX + k / 3:.3f} L{CX + w / 2:.3f} {BOTTOM - B / PHI**3.5:.3f} '
        f'L{CX:.3f} {BOTTOM - B / PHI**4.5:.3f} L{CX - w / 2:.3f} {BOTTOM - B / PHI**3.5:.3f} Z" fill="{color}"/>'
    )
    return knot + blade


def notebook(cx, top, w, h, tilt, prop, bg, detail, spiral=False, band=True, lines=2):
    tr = f"rotate({tilt} {cx:.3f} {top + h / 2:.3f})"
    x = cx - w / 2
    out = ringed(f'rect x="{x:.3f}" y="{top:.3f}" width="{w:.3f}" height="{h:.3f}" rx="{B / PHI**5.5:.3f}"', prop, bg)
    if band:
        out += f'<rect x="{x + w - w / PHI**3:.3f}" y="{top:.3f}" width="{LINE * 1.3:.3f}" height="{h:.3f}" fill="{detail}"/>'
    if spiral:  # steno pad: rings along the top edge
        n = 4
        for i in range(n):
            rx_ = x + w * (i + 1) / (n + 1)
            out += f'<rect x="{rx_ - LINE:.3f}" y="{top - B / PHI**5.5:.3f}" width="{2 * LINE:.3f}" height="{B / PHI**4.3:.3f}" rx="{LINE:.3f}" fill="{detail}"/>'
    out += "".join(
        f'<rect x="{x + w / PHI**4:.3f}" y="{top + h / PHI**1.9 + i * h / PHI**3.4:.3f}" width="{w / PHI**(0.6 if band else 0.4):.3f}" '
        f'height="{LINE:.3f}" rx="{LINE / 2:.3f}" fill="{detail}"/>'
        for i in range(lines)
    )
    return f'<g transform="{tr}">{out}</g>'


def clipboard(cx, top, w, h, prop, bg, detail, paper):
    x = cx - w / 2
    board = ringed(f'rect x="{x:.3f}" y="{top:.3f}" width="{w:.3f}" height="{h:.3f}" rx="{B / PHI**5.2:.3f}"', prop, bg)
    m = w / PHI**3.2
    sheet = f'<rect x="{x + m:.3f}" y="{top + m * 1.4:.3f}" width="{w - 2 * m:.3f}" height="{h - m * 1.4:.3f}" rx="{B / PHI**7:.3f}" fill="{paper}"/>'
    cw, ch = w / PHI**1.8, B / PHI**4.8
    clip = ringed(f'rect x="{cx - cw / 2:.3f}" y="{top - ch / 2:.3f}" width="{cw:.3f}" height="{ch:.3f}" rx="{ch / 2.5:.3f}"', detail, bg, ring=GAP * 0.7)
    rows = ""
    for i in range(2):
        ry = top + m * 1.4 + h / PHI**2.6 + i * h / PHI**2.6
        s = B / PHI**5
        bx = x + m + w / PHI**4.2
        rows += (
            f'<path d="M{bx:.3f} {ry:.3f} l{s * 0.35:.3f} {s * 0.35:.3f} l{s * 0.65:.3f} {-s * 0.7:.3f}" fill="none" stroke="{detail}" '
            f'stroke-width="{LINE * 1.2:.3f}" stroke-linecap="round" stroke-linejoin="round"/>'
            f'<rect x="{bx + s * 1.5:.3f}" y="{ry - LINE / 2:.3f}" width="{w / PHI**1.3:.3f}" height="{LINE:.3f}" rx="{LINE / 2:.3f}" fill="{detail}"/>'
        )
    return f'<g transform="rotate(-6 {cx:.3f} {top + h / 2:.3f})">{board}{sheet}{clip}{rows}</g>'


def draw(fig=NAVY, prop=BRASS, bg=WHITE, detail=NAVY):
    out = {}
    cid = "c7" + bg[1:]
    clip = f'<clipPath id="{cid}"><rect x="0" y="0" width="100" height="{BOTTOM:.3f}"/></clipPath>'

    def wrap(inner):
        return clip + f'<g clip-path="url(#{cid})">{inner}</g>'

    nb_w, nb_h = B / PHI**1.65, B / PHI**1.55
    nb_cx = CX + B / PHI**3.3
    nb_top = BOTTOM - nb_h + B / PHI**5.5

    # S1. Steno pad and pen: taking dictation.
    pw = B / PHI**5.6
    pen = stroked(
        f"M{nb_cx + nb_w / 2 + B / PHI**4.6:.3f} {nb_top - B / PHI**3.6 - B / PHI**4.6:.3f} L{nb_cx - nb_w / PHI**3:.3f} {nb_top - B / PHI**3.6 + nb_h / PHI**1.6:.3f}",
        detail,
        pw,
        bg,
    )
    out["S1-steno-pen"] = wrap(
        figure(fig) + notebook(nb_cx, nb_top - B / PHI**3.6, nb_w, nb_h, -6, prop, bg, detail, spiral=True, band=False, lines=3) + pen
    )

    # S2. Clipboard with a checklist: the one who keeps the to-dos.
    out["S2-clipboard"] = wrap(figure(fig) + clipboard(nb_cx, nb_top - B / PHI**3.8, nb_w * 1.05, nb_h, prop, bg, detail, bg))

    # S3. A pen behind the ear.
    ex, ey = CX + HEAD_R * 0.93, HCY - HEAD_R * 0.05
    plen = B / PHI**2.9
    ear_pen = stroked(
        f"M{ex - plen * 0.42:.3f} {ey - plen * 0.55:.3f} L{ex + plen * 0.28:.3f} {ey + plen * 0.35:.3f}", prop, B / PHI**5.4, bg
    )
    out["S3-pen-ear"] = wrap(figure(fig) + ear_pen + notebook(nb_cx, nb_top, nb_w, nb_h, -7, prop, bg, detail))

    # S4. Glasses on the head.
    gr = HEAD_R / PHI**2.2
    gy = HCY + HEAD_R / PHI**3
    gd = gr * 1.3
    glasses = (
        f'<circle cx="{CX - gd:.3f}" cy="{gy:.3f}" r="{gr:.3f}" fill="none" stroke="{bg}" stroke-width="{B / PHI**6.2:.3f}"/>'
        f'<circle cx="{CX + gd:.3f}" cy="{gy:.3f}" r="{gr:.3f}" fill="none" stroke="{bg}" stroke-width="{B / PHI**6.2:.3f}"/>'
        f'<path d="M{CX - gd + gr:.3f} {gy:.3f} H{CX + gd - gr:.3f}" stroke="{bg}" stroke-width="{B / PHI**6.2:.3f}"/>'
    )
    out["S4-glasses"] = wrap(figure(fig) + glasses + notebook(nb_cx, nb_top, nb_w, nb_h, -7, prop, bg, detail))

    # S5. Office collar and necktie.
    out["S5-necktie"] = wrap(
        figure(fig)
        + collar_v(bg, depth=B / PHI**1.3, width=B / PHI**2.0)
        + necktie(prop)
        + notebook(CX + B / PHI**2.35, nb_top + B / PHI**3.8, nb_w / PHI**0.5, nb_h / PHI**0.5, -7, prop, bg, detail)
    )

    # S6. Butler's collar and bow tie (P1d, bigger head).
    out["S6-bow-tie"] = wrap(
        figure(fig)
        + collar_v(bg)
        + bow_tie(CX, SH_TOP + B / PHI**4.6, B / PHI**2.0, prop)
        + notebook(CX + B / PHI**2.4, nb_top + B / PHI**3.6, nb_w / PHI**0.5, nb_h / PHI**0.5, -7, prop, bg, detail)
    )
    return out


light = draw()
dark = draw(fig=CREAM, bg=NAVY, detail=NAVY)

for name, body in light.items():
    with open(os.path.join(OUT, f"famulen7-{name}.svg"), "w") as fh:
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
    col, row = i % 3, i // 3
    x = 20 + col * 355
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
with open(os.path.join(OUT, "sheet7.svg"), "w") as fh:
    fh.write(sheet)
