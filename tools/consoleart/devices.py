"""Stylised device illustrations. Lodestar's own drawings: generic form factors (a box with a
slot, a handheld, a keyboard computer …) varied per console by proportions, colours and
details. No maker's logo or emblem is drawn.

Every function takes (svg, params) and returns (markup, width, height) in its own units; the
caller scales it into place. Common params: shell (body colour), trim (second colour), acc
(accent: lights, buttons), tint (screen glow), btn (button colour).
"""
import math

from svg import (
    Mat, button, circ_d, circle, disc, dpad, ellipse, group, highlight_rim, led, line, mix, n,
    outline, path, pill, rect, round_poly, rrect_d, screen, shade, slots, star4, stick,
)


def _p(P, k, d):
    return P.get(k, d)


# ====================================================================== home consoles


def console(sv, P):
    W, H, D, r = _p(P, "w", 560), _p(P, "h", 140), _p(P, "d", 44), _p(P, "r", 16)
    c = P["shell"]
    m = Mat(sv, c)
    ins = D * _p(P, "persp", 0.6)
    out = []
    above = 0
    arch = P.get("arch")
    if arch:
        # A loaf-shaped top: the top surface bulges up in a long arc (no flat lid).
        k = -D / 3
        top = "M0 %sC%s %s %s %s %s %sZ" % (n(D), n(W * 0.1), n(k), n(W * 0.9), n(k), n(W), n(D))
        sil = "M0 %sC%s %s %s %s %s %s" % (n(D), n(W * 0.1), n(k), n(W * 0.9), n(k), n(W), n(D)) + round_poly(
            [(W, D), (W, D + H), (0, D + H), (0, D)], [0, r, r, 0], close=False
        ).replace("M", "L", 1) + "Z"
    else:
        sil = round_poly([(ins, 0), (W - ins, 0), (W, D), (W, D + H), (0, D + H), (0, D)], [r * 0.8, r * 0.8, 4, r, r, 4])
    # Cartridge sticking out of the top (drawn first so the slot's lip covers its foot).
    cart = None
    for f in P.get("top", []):
        if f[0] == "cart" and len(f) > 1 and f[1]:
            cart = f
    out.append(path(sil, m.front))
    if "plinth" in P:
        pc = P["plinth"]
        ph = H * _p(P, "plinth_h", 0.34)
        pm = Mat(sv, pc)
        out.append(path(round_poly([(0, D + H - ph), (W, D + H - ph), (W, D + H), (0, D + H)], [0, 0, r, r]), pm.front))
        out.append(line(r * 0.3, D + H - ph, W - r * 0.3, D + H - ph, shade(pc, -0.5), 2.5, stroke_opacity="0.6"))
    if not arch:
        top = round_poly([(ins, 0), (W - ins, 0), (W, D), (0, D)], [r * 0.8, r * 0.8, 3, 3])
    out.append(path(top, m.top))
    if arch:
        # A long glossy reflection along the curve.
        g = sv.lin([(0, "#ffffff", 0), (0.5, "#ffffff", 0.22), (1, "#ffffff", 0)], 0, 0, 1, 0)
        out.append(path("M%s %sC%s %s %s %s %s %s" % (n(W * 0.08), n(D * 0.62), n(W * 0.2), n(D * 0.02), n(W * 0.8), n(D * 0.02), n(W * 0.92), n(D * 0.62)), "none", stroke=g, stroke_width="5", stroke_linecap="round"))
    out.append(line(3, D, W - 3, D, m.edge, 2, stroke_opacity="0.55"))
    for f in P.get("top", []):
        out.append(_top_feature(sv, f, W, D, ins, m, P))
    for f in P.get("front", []):
        out.append(_front_feature(sv, f, W, H, D, m, P))
    out.append(outline(sil, m))
    body = "".join(out)
    skew = P.get("skew")
    if skew:
        # A leaning, parallelogram body (the top edge set back to the right).
        t = math.tan(math.radians(skew))
        tot = D + H
        body = '<g transform="matrix(1 0 %s 1 %s 0)">%s</g>' % (("%.4f" % -t).rstrip("0"), n(t * tot), body)
        return body, W + t * tot, tot
    if cart:
        cw = W * _p(P, "cart_w", 0.3)
        ch = _p(P, "cart_h", 92)
        cc = cart[1]
        cm = Mat(sv, cc)
        x = W / 2 - cw / 2
        y0 = D * 0.5 - ch
        cbody = (
            path(rrect_d(x, y0, cw, ch, [8, 8, 0, 0]), cm.front)
            + rect(x + cw * 0.12, y0 + ch * 0.22, cw * 0.76, ch * 0.5, 4, sv.lin([(0, shade(P["acc"], 0.35)), (1, shade(P["acc"], -0.1))]))
            + slots(x + cw * 0.15, y0 + 6, cw * 0.7, ch * 0.1, 7, cm.dark)
            + path(rrect_d(x, y0, cw, ch, [8, 8, 0, 0]), "none", stroke=cm.outline, stroke_opacity="0.5", stroke_width="2")
        )
        # Slot lip in front of the cartridge.
        lip = rect(W / 2 - cw / 2 - 10, D * 0.5, cw + 20, D * 0.22, 3, shade(c, 0.2))
        body = body + cbody + lip
        above = ch - D * 0.5
        return group(body, 0, above), W, D + H + above
    return body, W, D + H


def _top_feature(sv, f, W, D, ins, m, P):
    kind = f[0]
    c = m.c
    if kind == "cart":
        sw = W * (_p(P, "cart_w", 0.3) + 0.06)
        return rect(W / 2 - sw / 2, D * 0.28, sw, D * 0.42, 4, m.deep) + line(W / 2 - sw / 2 + 4, D * 0.72, W / 2 + sw / 2 - 4, D * 0.72, m.edge, 1.5, stroke_opacity="0.5")
    if kind == "disc":
        cx = W * (f[1] if len(f) > 1 else 0.5)
        rx = W * f[2] if len(f) > 2 else min(W * 0.2, 150)
        ry = D * 0.36
        return (
            ellipse(cx, D * 0.5 + 2, rx, ry, m.dark, fill_opacity="0.6")
            + ellipse(cx, D * 0.5, rx, ry, sv.lin([(0, shade(c, 0.4)), (1, shade(c, 0.05))]))
            + ellipse(cx, D * 0.5, rx * 0.84, ry * 0.8, "none", stroke=m.line, stroke_opacity="0.45", stroke_width="1.5")
            + ellipse(cx, D * 0.5, rx * 0.16, ry * 0.18, m.line, fill_opacity="0.6")
        )
    if kind == "dome":
        cx = W * 0.5
        rx = W * 0.2
        ry = D * 0.4
        return (
            ellipse(cx, D * 0.52, rx, ry, m.deep, fill_opacity="0.8")
            + ellipse(cx, D * 0.5, rx * 0.8, ry * 0.7, sv.lin([(0, shade(c, 0.2)), (1, shade(c, -0.2))]))
            + rect(cx - rx * 0.3, D * 0.38, rx * 0.6, D * 0.18, 3, m.deep)
        )
    if kind == "topbtn":
        # Round buttons set into the top: ("topbtn", [x fractions], y fraction of the depth[, size]).
        k = f[3] if len(f) > 3 else 1
        col = shade(c, -0.08)
        out = []
        for t in f[1]:
            cx, cy = W * t, D * f[2]
            out.append(ellipse(cx, cy + 2, 17 * k, 8 * k, m.dark, fill_opacity="0.7"))
            out.append(ellipse(cx, cy, 15 * k, 7 * k, sv.lin([(0, shade(col, 0.3)), (1, shade(col, -0.1))])))
            out.append(ellipse(cx - 3 * k, cy - 2 * k, 7 * k, 2.5 * k, "#ffffff", fill_opacity="0.35"))
        return "".join(out)
    if kind == "jewel":
        # A round glowing dome set into the middle of the top (no emblem on it).
        cx, cy = W / 2, D * 0.62
        rx, ry = W * 0.075, D * 0.28
        acc = f[1] if len(f) > 1 else P["acc"]
        return (
            ellipse(cx, cy + 3, rx * 1.25, ry * 1.25, m.deep, fill_opacity="0.85")
            + ellipse(cx, cy, rx * 1.12, ry * 1.12, shade(c, -0.2))
            + ellipse(cx, cy, rx, ry, sv.rad([(0, shade(acc, 0.55)), (0.55, acc), (1, shade(acc, -0.55))], cx=0.45, cy=0.35, r=0.6))
            + ellipse(cx - rx * 0.25, cy - ry * 0.4, rx * 0.45, ry * 0.25, "#ffffff", fill_opacity="0.45")
        )
    if kind == "split":
        # Two finishes on the top (a glossy strip on the left) with a light bar between them.
        t = f[1]
        acc = P["acc"]
        xb, xt = W * t, ins + (W - 2 * ins) * t
        gl = path(round_poly([(ins, 0), (xt, 0), (xb, D), (0, D)], [P.get("r", 16) * 0.8, 0, 0, 3]), sv.lin([(0, shade(c, 0.32)), (0.5, shade(c, 0.14)), (1, shade(c, 0.24))], 0, 0, 1, 1))
        halo = path("M%s 0L%s %s" % (n(xt), n(xb), n(D)), "none", stroke=acc, stroke_width="10", stroke_opacity="0.3", stroke_linecap="round")
        bar = path("M%s 2L%s %s" % (n(xt), n(xb), n(D - 1)), "none", stroke=shade(acc, 0.35), stroke_width="3", stroke_linecap="round")
        return gl + halo + bar
    if kind == "vents":
        x0 = ins + (W - 2 * ins) * f[1]
        x1 = ins + (W - 2 * ins) * f[2]
        return slots(x0, D * 0.25, x1 - x0, D * 0.5, f[3] if len(f) > 3 else 10, m.dark, op="0.6")
    if kind == "keypad":
        out = []
        cols, rows = 10, 3
        x0 = W * 0.18
        kw = (W * 0.64) / cols
        for j in range(rows):
            yy = D * (0.2 + 0.24 * j)
            for i in range(cols):
                out.append(rect(x0 + i * kw + 2, yy, kw - 4, D * 0.18, 2, m.light if (i + j) % 7 else P["acc"], fill_opacity="0.9"))
        return "".join(out)
    if kind == "grille":
        cx, cy = W / 2, D * 0.5
        rx, ry = (W - 2 * ins) * 0.36, D * 0.34
        out = [ellipse(cx, cy, rx, ry, m.deep)]
        for k in range(5):
            out.append(ellipse(cx, cy, rx * (0.2 + 0.16 * k), ry * (0.2 + 0.16 * k), "none", stroke=m.dark, stroke_width="2"))
        out.append(ellipse(cx, cy, rx * 0.6, ry * 0.6, sv.rad([(0, P["acc"], 0.55), (1, P["acc"], 0)])))
        return "".join(out)
    return ""


def _front_feature(sv, f, W, H, D, m, P):
    kind = f[0]
    X = lambda t: W * t
    Y = lambda t: D + H * t
    acc = P["acc"]
    if kind in ("tray", "slot"):
        x0, x1, y = X(f[1]), X(f[2]), Y(f[3])
        h = 10 if kind == "tray" else 6
        out = rect(x0, y - h / 2, x1 - x0, h, h / 2, m.deep) + line(x0 + 4, y + h / 2 + 2, x1 - 4, y + h / 2 + 2, m.edge, 1.5, stroke_opacity="0.45")
        if kind == "tray":
            out += rect(x1 + 14, y - 5, 22, 10, 5, m.dark)
        return out
    if kind == "ports":
        k, x0, y = f[1], X(f[2]), Y(f[3])
        z = f[4] if len(f) > 4 else 1
        out = []
        for i in range(k):
            x = x0 + i * 44 * z
            out.append(rect(x, y - 9 * z, 34 * z, 18 * z, 5 * z, m.deep) + rect(x + 6 * z, y - 4 * z, 22 * z, 8 * z, 3 * z, m.dark))
        return "".join(out)
    if kind == "buttons":
        k, x0, y = f[1], X(f[2]), Y(f[3])
        col = f[4] if len(f) > 4 else m.light
        return "".join(button(sv, x0 + i * 32, y, 10, col) for i in range(k))
    if kind == "pills":
        k, x0, y = f[1], X(f[2]), Y(f[3])
        col = f[4] if len(f) > 4 else m.light
        return "".join(pill(sv, x0 + i * 52, y, 40, 13, col) for i in range(k))
    if kind == "led":
        col = f[3] if len(f) > 3 else acc
        return led(sv, X(f[1]), Y(f[2]), 4.5, col)
    if kind == "ledbar":
        x0, x1, y = X(f[1]), X(f[2]), Y(f[3])
        g = sv.lin([(0, acc, 0), (0.2, acc, 0.95), (0.8, acc, 0.95), (1, acc, 0)], 0, 0, 1, 0)
        halo = sv.lin([(0, acc, 0), (0.5, acc, 0.35), (1, acc, 0)], 0, 0, 1, 0)
        return rect(x0, y - 7, x1 - x0, 14, 7, halo) + rect(x0, y - 2, x1 - x0, 4, 2, g)
    if kind == "bands":
        k = f[1]
        out = []
        for i in range(k):
            y = Y(0.18 + 0.64 * (i + 0.5) / k)
            out.append(line(8, y, W - 8, y, m.dark, 3, stroke_opacity="0.7") + line(8, y + 3, W - 8, y + 3, m.edge, 1.2, stroke_opacity="0.35"))
        return "".join(out)
    if kind == "vents":
        x0, x1, y0, y1, k = X(f[1]), X(f[2]), Y(f[3]), Y(f[4]), f[5]
        return slots(x0, y0, x1 - x0, y1 - y0, k, m.dark)
    if kind == "hvents":
        x0, x1, y0, y1, k = X(f[1]), X(f[2]), Y(f[3]), Y(f[4]), f[5]
        return slots(x0, y0, x1 - x0, y1 - y0, k, m.dark, vertical=False)
    if kind == "stripe":
        y, h = Y(f[1]), f[2]
        col = f[3] if len(f) > 3 else acc
        g = sv.lin([(0, shade(col, 0.2)), (1, shade(col, -0.15))])
        return rect(0, y, W, h, 0, g) + line(0, y, W, y, "#ffffff", 1, stroke_opacity="0.25")
    if kind == "flap":
        x0, x1, y0, y1 = X(f[1]), X(f[2]), Y(f[3]), Y(f[4])
        return (
            rect(x0, y0, x1 - x0, y1 - y0, 6, sv.lin([(0, shade(m.c, 0.12)), (1, shade(m.c, -0.08))]))
            + rect(x0, y0, x1 - x0, y1 - y0, 6, "none", stroke=m.line, stroke_width="2.5", stroke_opacity="0.8")
            + line(x0 + 6, y0 + 3, x1 - 6, y0 + 3, m.edge, 1.5, stroke_opacity="0.5")
        )
    if kind == "panel":
        # A second-colour plate across part of the front.
        x0, x1, y0, y1, col = X(f[1]), X(f[2]), Y(f[3]), Y(f[4]), f[5]
        pm = Mat(sv, col)
        return rect(x0, y0, x1 - x0, y1 - y0, 6, pm.front) + line(x0 + 6, y0 + 1, x1 - 6, y0 + 1, pm.edge, 1.5, stroke_opacity="0.5")
    if kind == "switches":
        k, x0, x1, y = f[1], X(f[2]), X(f[3]), Y(f[4])
        step = (x1 - x0) / k
        out = []
        for i in range(k):
            cx = x0 + step * (i + 0.5)
            out.append(rect(cx - 7, y - 18, 14, 36, 4, m.deep) + rect(cx - 5, y - 16 + (i % 2) * 14, 10, 18, 3, sv.lin([(0, "#e4e4e4"), (1, "#9a9a9a")])))
        return "".join(out)
    if kind == "groove":
        y = Y(f[1])
        return line(6, y, W - 6, y, m.dark, 2.5, stroke_opacity="0.7") + line(6, y + 2.5, W - 6, y + 2.5, m.edge, 1, stroke_opacity="0.3")
    return ""


def tower(sv, P):
    W, H, D = _p(P, "w", 210), _p(P, "h", 460), _p(P, "d", 34)
    c = P["shell"]
    m = Mat(sv, c)
    ins = D * 0.5
    r = _p(P, "r", 18)
    out = []
    # Stand.
    sm = Mat(sv, P.get("trim", shade(c, -0.5)))
    out.append(path(rrect_d(W * 0.2, D + H - 4, W * 0.6, 22, [2, 2, 10, 10]), sm.front))
    sil = round_poly([(ins, 0), (W - ins, 0), (W, D), (W, D + H), (0, D + H), (0, D)], [r * 0.8, r * 0.8, 4, r, r, 4])
    out.append(path(sil, m.front))
    if "panels" in P:
        pc = P["panels"]
        pm = Mat(sv, pc)
        pw = W * 0.24
        out.append(path(round_poly([(0, D), (pw, D), (pw * 0.7, D + H), (0, D + H)], [3, 3, 3, r]), pm.front))
        out.append(path(round_poly([(W - pw, D), (W, D), (W, D + H), (W - pw * 0.7, D + H)], [3, 3, r, 3]), pm.front))
    out.append(path(round_poly([(ins, 0), (W - ins, 0), (W, D), (0, D)], [r * 0.8, r * 0.8, 3, 3]), m.top))
    out.append(line(3, D, W - 3, D, m.edge, 2, stroke_opacity="0.55"))
    for f in P.get("top", []):
        out.append(_top_feature(sv, f, W, D, ins, m, P))
    # Vertical disc slot, light, vents.
    if P.get("slot", True):
        out.append(rect(W * 0.5 - 4, D + H * 0.1, 8, H * 0.32, 4, m.deep))
    out.append(led(sv, W * 0.5, D + H * 0.05, 4.5, P["acc"]))
    if P.get("vents", True):
        out.append(slots(W * 0.3, D + H * 0.72, W * 0.4, H * 0.2, 7, m.dark))
    if P.get("ledstrip"):
        g = sv.lin([(0, P["acc"], 0), (0.5, P["acc"], 0.9), (1, P["acc"], 0)])
        out.append(rect(W * 0.5 - 2, D + H * 0.46, 4, H * 0.22, 2, g))
    out.append(outline(sil, m))
    return "".join(out), W, D + H + 18


def cube(sv, P):
    """A compact, nearly cube-shaped console: a carry handle folded over the back, a round lid
    on the top, a light front panel with four round controller ports and two memory-card flaps,
    round power / reset buttons in the top's front corners, and a slightly inset base."""
    W, H, D = _p(P, "w", 380), _p(P, "h", 270), _p(P, "d", 120)
    c = P["shell"]
    m = Mat(sv, c)
    ins = D * 0.3
    r = 14
    up = 34  # the handle rises this far above the top's back edge
    out = []
    # Carry handle behind the back edge.
    hc = P.get("handle", "#1d1e23")
    hx0, hx1 = W * 0.2, W * 0.8
    hd = "M%s %sC%s %s %s %s %s %s" % (n(hx0), n(D * 0.2), n(hx0 + 8), n(-up * 1.3), n(hx1 - 8), n(-up * 1.3), n(hx1), n(D * 0.2))
    out.append(path(hd, "none", stroke=shade(hc, -0.4), stroke_width="30", stroke_linecap="round"))
    out.append(path(hd, "none", stroke=hc, stroke_width="24", stroke_linecap="round"))
    out.append(path(hd, "none", stroke="#ffffff", stroke_width="2", stroke_opacity="0.18", transform="translate(0 -9)"))
    # Base: a slightly narrower, darker band under the body.
    bh = H * 0.13
    bm = Mat(sv, shade(c, -0.22))
    out.append(path(rrect_d(10, D + H - bh - 6, W - 20, bh + 6, [0, 0, 10, 10]), bm.front))
    for x in (W * 0.1, W * 0.9):
        out.append(rect(x - 14, D + H - 4, 28, 8, 3, m.deep))
    sil = round_poly([(ins, 0), (W - ins, 0), (W, D), (W, D + H - bh), (0, D + H - bh), (0, D)], [r, r, 4, 8, 8, 4])
    out.append(path(sil, m.front))
    top = round_poly([(ins, 0), (W - ins, 0), (W, D), (0, D)], [r, r, 3, 3])
    out.append(path(top, m.top))
    out.append(line(3, D, W - 3, D, m.edge, 2, stroke_opacity="0.55"))
    # The lid: an inset plate in the top with a dark round centre.
    li = 0.16
    lid = round_poly([(ins + W * li, D * 0.14), (W - ins - W * li, D * 0.14), (W - W * li * 0.8, D * 0.86), (W * li * 0.8, D * 0.86)], 10)
    out.append(path(lid, "none", stroke=m.line, stroke_width="2", stroke_opacity="0.55"))
    out.append(path(lid, "none", stroke=m.edge, stroke_width="1.2", stroke_opacity="0.35", transform="translate(0 2)"))
    cx, cy = W / 2, D * 0.5
    out.append(ellipse(cx, cy + 2, W * 0.19, D * 0.27, m.deep, fill_opacity="0.6"))
    out.append(ellipse(cx, cy, W * 0.18, D * 0.255, sv.lin([(0, "#34353c"), (1, "#131317")])))
    out.append(ellipse(cx - W * 0.05, cy - D * 0.1, W * 0.07, D * 0.06, "#ffffff", fill_opacity="0.12"))
    # Open button (back right) and the power / reset rings in the front corners of the top.
    out.append(ellipse(W * 0.7, D * 0.14, 15, 6.5, m.dark, fill_opacity="0.6"))
    out.append(ellipse(W * 0.7, D * 0.12, 14, 6, sv.lin([(0, "#e6e6e9"), (1, "#a9aab0")])))
    for x in (W * 0.1, W * 0.9):
        out.append(ellipse(x, D * 0.8, 17, 7, "none", stroke=m.line, stroke_width="2", stroke_opacity="0.6"))
        out.append(ellipse(x, D * 0.8 - 1, 12, 4.5, shade(c, 0.2)))
    # Front: a light, slightly inset panel with four round ports and two flaps.
    pc = P.get("panel", "#c9cad0")
    pm = Mat(sv, pc)
    px0, px1, py0, py1 = W * 0.06, W * 0.94, D + H * 0.1, D + H * 0.76
    out.append(rect(px0 - 3, py0 - 3, px1 - px0 + 6, py1 - py0 + 6, 12, m.dark, fill_opacity="0.55"))
    out.append(rect(px0, py0, px1 - px0, py1 - py0, 10, pm.front))
    out.append(line(px0 + 8, py1 - 1, px1 - 8, py1 - 1, "#ffffff", 1.5, stroke_opacity="0.5"))
    for i in range(4):
        x = W * (0.2 + 0.2 * i)
        y = D + H * 0.32
        out.append(circle(x, y + 2, 21, pm.edge, fill_opacity="0.6"))
        out.append(circle(x, y, 20, sv.lin([(0, "#0c0c0f"), (1, "#2a2b31")])))
        out.append(circle(x, y, 20, "none", stroke=pm.line, stroke_width="2", stroke_opacity="0.7"))
        for k in range(i + 1):
            out.append(circle(x - (i * 6) / 2 + k * 6, y - 30, 1.8, pm.line, fill_opacity="0.8"))
    for x0, x1 in ((0.12, 0.47), (0.53, 0.88)):
        fx0, fx1 = W * x0, W * x1
        fy0, fy1 = D + H * 0.52, D + H * 0.66
        out.append(rect(fx0, fy0, fx1 - fx0, fy1 - fy0, 6, pm.light, fill_opacity="0.75"))
        out.append(rect(fx0, fy0, fx1 - fx0, fy1 - fy0, 6, "none", stroke=pm.line, stroke_width="1.8", stroke_opacity="0.7"))
        out.append(rect(fx0 + 14, fy1 - 9, (fx1 - fx0) * 0.3, 4, 2, pm.line, fill_opacity="0.6"))
    out.append(outline(sil, m))
    return group("".join(out), 0, up), W, D + H + up + 4


def hump(sv, P):
    """A wide, low console with a raised centre that holds a cartridge, flared sides with two
    sliders on top and a row of round controller ports along the front."""
    W = 620
    c = P["shell"]
    m = Mat(sv, c)
    lift = 80
    out = []
    # Top surface of the wide base.
    top = round_poly([(44, 70), (W - 44, 70), (W, 152), (0, 152)], [46, 46, 6, 6])
    front = round_poly([(0, 152), (W, 152), (W - 6, 212), (W - 54, 240), (54, 240), (6, 212)], [4, 4, 26, 30, 30, 26])
    sil = round_poly([(44, 70), (W - 44, 70), (W, 152), (W - 6, 212), (W - 54, 240), (54, 240), (6, 212), (0, 152)], [46, 46, 4, 26, 30, 30, 26, 4])
    out.append(path(front, m.front))
    out.append(path(top, m.top))
    out.append(line(4, 152, W - 4, 152, m.edge, 2, stroke_opacity="0.5"))
    # Sliders in oval wells on the left and right of the top.
    for x in (118, W - 118):
        out.append(ellipse(x, 118, 66, 22, m.deep, fill_opacity="0.75"))
        out.append(ellipse(x, 121, 60, 18, shade(c, -0.15)))
        out.append(pill(sv, x - 10, 117, 46, 14, shade(c, 0.28)))
    # The raised centre.
    hf = round_poly([(166, 64), (W - 166, 64), (W - 154, 146), (154, 146)], [10, 10, 6, 6])
    ht = round_poly([(190, 18), (W - 190, 18), (W - 166, 64), (166, 64)], [14, 14, 4, 4])
    out.append(path(hf, sv.lin([(0, shade(c, 0.12)), (1, shade(c, -0.12))])))
    out.append(path(ht, sv.lin([(0, shade(c, 0.34)), (1, shade(c, 0.18))])))
    out.append(line(168, 64, W - 168, 64, m.edge, 1.6, stroke_opacity="0.5"))
    for i in range(3):
        y = 86 + i * 16
        out.append(line(184 - i * 3, y, W - 184 + i * 3, y, m.dark, 2.2, stroke_opacity="0.55"))
    out.append(path(hf, "none", stroke=m.outline, stroke_width="1.5", stroke_opacity="0.5"))
    # Cartridge standing in the slot.
    cc = P.get("cart", "#8b8d94")
    cm = Mat(sv, cc)
    cw, ch = W * 0.34, 112
    x0, y0 = W / 2 - cw / 2, 34 - ch
    cd = round_poly([(x0 + 6, y0), (x0 + cw - 6, y0), (x0 + cw, y0 + 22), (x0 + cw - 6, 34), (x0 + 6, 34), (x0, y0 + 22)], [14, 14, 6, 0, 0, 6])
    out.append(rect(W / 2 - cw / 2 - 12, 22, cw + 24, 16, 5, m.deep))
    out.append(path(cd, cm.front))
    out.append(rect(x0 + 18, y0 + 30, cw - 36, 58, 5, sv.lin([(0, shade(P["acc"], 0.35)), (1, shade(P["acc"], -0.15))], 0, 0, 1, 1)))
    out.append(slots(x0 + 22, y0 + 8, cw - 44, 10, 9, cm.dark))
    out.append(path(cd, "none", stroke=cm.outline, stroke_width="2", stroke_opacity="0.5"))
    out.append(rect(W / 2 - cw / 2 - 10, 30, cw + 20, 8, 3, shade(c, 0.22)))
    # Controller ports along the front, a power light at the left.
    for i in range(4):
        x = W / 2 + (i - 1.5) * 64
        out.append(circle(x, 196, 23, m.deep))
        out.append(circle(x, 196, 16, sv.lin([(0, "#0b0b0e"), (1, "#26272d")])))
        out.append(path("M%s %sa21 21 0 0 0 42 0" % (n(x - 21), n(201)), "none", stroke=m.edge, stroke_width="1.5", stroke_opacity="0.4"))
    out.append(led(sv, 96, 196, 4, P.get("ledc", "#ff4d5e")))
    out.append(outline(sil, m))
    return group("".join(out), 0, lift), W, 240 + lift


def wedge(sv, P):
    """Front-sloped console (a raked top with switches)."""
    W, H = _p(P, "w", 560), _p(P, "h", 120)
    D = _p(P, "d", 110)
    c = P["shell"]
    m = Mat(sv, c)
    r = 14
    ins = 40
    out = []
    sil = round_poly([(ins, 0), (W - ins, 0), (W, D), (W, D + H), (0, D + H), (0, D)], [r, r, 6, r, r, 6])
    out.append(path(sil, m.front))
    top = round_poly([(ins, 0), (W - ins, 0), (W, D), (0, D)], [r, r, 3, 3])
    out.append(path(top, m.top))
    # Raked ridges on the top.
    for i in range(1, 6):
        y = D * i / 6
        xi = ins * (1 - i / 6)
        out.append(line(xi + 6, y, W - xi - 6, y, m.line, 1.4, stroke_opacity="0.35"))
    out.append(rect(W / 2 - W * 0.17, D * 0.12, W * 0.34, D * 0.2, 4, m.deep))
    if "plinth" in P:
        pm = Mat(sv, P["plinth"])
        out.append(path(round_poly([(0, D), (W, D), (W, D + H), (0, D + H)], [0, 0, r, r]), pm.front))
        for i in range(1, 9):
            out.append(line(10, D + H * i / 9, W - 10, D + H * i / 9, shade(P["plinth"], -0.25), 1.2, stroke_opacity="0.4"))
    out.append(line(3, D, W - 3, D, m.edge, 2, stroke_opacity="0.55"))
    for f in P.get("front", []):
        out.append(_front_feature(sv, f, W, H, D, m, P))
    out.append(outline(sil, m))
    return "".join(out), W, D + H


def disc_console(sv, P):
    """A CD console or add-on: a box with a disc lid, a disc leaning in front."""
    P = dict(P)
    P.setdefault("top", [("disc", 0.42)])
    body, W, H = console(sv, P)
    r = H * 0.5
    d = disc(sv, W * 0.86, H * 0.8, r, P["acc"])
    sh = ellipse(W * 0.86 + r * 0.05, H * 0.8 + r * 0.98, r * 0.85, r * 0.1, "#000000", fill_opacity="0.35")
    return body + sh + d, W + r * 0.5, H * 0.8 + r


def floppy_console(sv, P):
    """A disk drive add-on with a floppy disk."""
    body, W, H = console(sv, P)
    s = H * 0.82
    x, y = W * 0.68, H * 0.42
    fc = P.get("disk", "#e8c34a")
    fm = Mat(sv, fc)
    disk = (
        path(round_poly([(0, 0), (s, 0), (s, s), (s * 0.08, s), (0, s * 0.92)], [6, 6, 6, 2, 6]), fm.front)
        + rect(s * 0.25, 0, s * 0.5, s * 0.3, 2, sv.lin([(0, "#d6d9df"), (1, "#9aa1ad")]))
        + rect(s * 0.52, s * 0.05, s * 0.12, s * 0.2, 1, "#4b5160")
        + rect(s * 0.15, s * 0.45, s * 0.7, s * 0.48, 4, "#f4f1e8")
        + line(s * 0.22, s * 0.58, s * 0.78, s * 0.58, "#b9b3a3", 2)
        + line(s * 0.22, s * 0.7, s * 0.78, s * 0.7, "#b9b3a3", 2)
        + path(round_poly([(0, 0), (s, 0), (s, s), (s * 0.08, s), (0, s * 0.92)], [6, 6, 6, 2, 6]), "none", stroke=fm.outline, stroke_opacity="0.5", stroke_width="2")
    )
    return body + group(disk, x, y, rot=-8), W + s * 0.2, H + s * 0.5


def visor(sv, P):
    """A stereo-visor console on a two-legged stand."""
    W = 520
    c = P["shell"]
    m = Mat(sv, c)
    out = []
    # Legs.
    lm = Mat(sv, shade(c, -0.35))
    for x in (W * 0.32, W * 0.68):
        out.append(rect(x - 9, 160, 18, 150, 6, lm.front))
    out.append(path(round_poly([(W * 0.22, 300), (W * 0.78, 300), (W * 0.84, 330), (W * 0.16, 330)], 8), lm.front))
    sil = round_poly([(30, 30), (W - 30, 30), (W, 90), (W - 20, 175), (20, 175), (0, 90)], [30, 30, 30, 26, 26, 30])
    out.append(path(sil, m.front))
    out.append(path(round_poly([(30, 30), (W - 30, 30), (W - 10, 70), (10, 70)], [24, 24, 6, 6]), m.top))
    # Eye cup with two lenses.
    out.append(path(rrect_d(W * 0.2, 0, W * 0.6, 66, [22, 22, 8, 8]), sv.lin([(0, "#2a2c33"), (1, "#101116")])))
    for x in (W * 0.37, W * 0.63):
        out.append(ellipse(x, 36, 40, 20, sv.rad([(0, P["acc"], 0.9), (0.6, shade(P["acc"], -0.5)), (1, "#050507")])))
        out.append(ellipse(x - 12, 28, 12, 5, "#ffffff", fill_opacity="0.35"))
    out.append(led(sv, W * 0.86, 118, 4.5, P["acc"]))
    out.append(slots(W * 0.1, 110, W * 0.18, 40, 6, m.dark))
    out.append(outline(sil, m))
    return "".join(out), W, 330


# ====================================================================== handhelds


def _body(sv, d, m, depth=10):
    """A flat device body: a darker copy below for thickness, the face, a rim."""
    return (
        path(d, m.deep, transform="translate(0 %s)" % n(depth))
        + path(d, m.front)
        + highlight_rim(d, m, 2.5, 0.3)
        + outline(d, m)
    )


def handheld_tall(sv, P):
    W, H = _p(P, "w", 300), _p(P, "h", 490)
    c = P["shell"]
    m = Mat(sv, c)
    rbr = _p(P, "rbr", 30)
    d = rrect_d(0, 0, W, H, [22, 22, rbr, 22])
    out = [_body(sv, d, m)]
    # Screen bezel and glass.
    bz = P.get("bezel", shade(c, -0.55))
    bx, by, bw, bh = W * 0.1, H * 0.07, W * 0.8, H * 0.4
    out.append(path(rrect_d(bx, by, bw, bh, [12, 12, P.get("rbz", 12), 12]), sv.lin([(0, shade(bz, 0.1)), (1, shade(bz, -0.15))])))
    out.append(led(sv, bx + bw * 0.09, by + bh * 0.4, 3.5, P.get("ledc", "#ff4d5e")))
    out.append(screen(sv, bx + bw * 0.18, by + bh * 0.12, bw * 0.64, bh * 0.76, 4, P["tint"]))
    btn = P.get("btn", P["acc"])
    out.append(dpad(sv, W * 0.26, H * 0.66, W * 0.27, P.get("dpadc", "#2a2c31")))
    out.append(button(sv, W * 0.67, H * 0.675, W * 0.068, btn))
    out.append(button(sv, W * 0.83, H * 0.635, W * 0.068, btn))
    for x in (0.4, 0.58):
        out.append(pill(sv, W * x, H * 0.82, W * 0.12, W * 0.04, shade(c, -0.35), rot=-25))
    # Speaker slits.
    sp = "".join(rect(-3, -H * 0.05, 6, H * 0.1, 3, m.dark) for _ in range(1))
    for i in range(6):
        out.append(group(sp, W * (0.68 + i * 0.045), H * 0.9, rot=-30))
    return "".join(out), W, H + 10


def calculator(sv, P):
    W, H = 280, 520
    c = P["shell"]
    m = Mat(sv, c)
    d = rrect_d(0, 0, W, H, [26, 26, 40, 40])
    out = [_body(sv, d, m)]
    out.append(rect(W * 0.1, H * 0.06, W * 0.8, H * 0.26, 10, shade(c, -0.5)))
    out.append(screen(sv, W * 0.16, H * 0.085, W * 0.68, H * 0.2, 4, P["tint"]))
    # Graph line on the screen.
    pts = []
    for i in range(21):
        t = i / 20
        pts.append("%s %s" % (n(W * (0.2 + 0.6 * t)), n(H * (0.19 - 0.06 * math.sin(t * 6.2)))))
    out.append(path("M" + "L".join(pts), "none", stroke=shade(P["tint"], 0.5), stroke_width="2.5", stroke_opacity="0.85"))
    kc = shade(c, -0.4)
    for j in range(8):
        for i in range(5):
            col = P["acc"] if (j == 0) or (i == 4 and j > 2) else (shade(c, 0.35) if j > 3 and 0 < i < 4 else kc)
            out.append(pill(sv, W * (0.17 + i * 0.165), H * (0.4 + j * 0.068), W * 0.13, H * 0.036, col))
    return "".join(out), W, H + 10


def handheld_wide(sv, P):
    W, H = _p(P, "w", 640), _p(P, "h", 280)
    c = P["shell"]
    m = Mat(sv, c)
    style = P.get("style", "rect")
    out = []
    if style == "pill":
        d = rrect_d(0, 0, W, H, H / 2)
    elif style == "grip":
        d = round_poly([(0, 20), (W, 20), (W, H), (W * 0.78, H + 30), (W * 0.22, H + 30), (0, H)], [60, 60, 90, 40, 40, 90])
    else:
        d = rrect_d(0, 0, W, H, P.get("r", 70))
    # Shoulder buttons behind the body.
    if P.get("shoulders", True):
        sm = Mat(sv, shade(c, -0.3))
        out.append(path(rrect_d(W * 0.06, -12, W * 0.2, 40, [16, 16, 4, 4]), sm.front))
        out.append(path(rrect_d(W * 0.74, -12, W * 0.2, 40, [16, 16, 4, 4]), sm.front))
    out.append(_body(sv, d, m))
    sw = W * _p(P, "screen_w", 0.46)
    sh = min(H * 0.66, sw * _p(P, "screen_aspect", 0.62))
    sx, sy = W / 2 - sw / 2, H * 0.47 - sh / 2
    bz = P.get("bezel", shade(c, -0.55))
    out.append(rect(sx - 16, sy - 14, sw + 32, sh + 28, 14, sv.lin([(0, shade(bz, 0.12)), (1, shade(bz, -0.12))])))
    out.append(screen(sv, sx, sy, sw, sh, 6, P["tint"]))
    lx, rx = W * 0.135, W * 0.865
    btn = P.get("btn", P["acc"])
    sticks = P.get("sticks", 0)
    if sticks == 1:
        out.append(stick(sv, lx, H * 0.72, H * 0.07, shade(c, -0.25)))
        out.append(dpad(sv, lx, H * 0.38, H * 0.27, P.get("dpadc", shade(c, -0.45))))
    elif sticks == 2:
        out.append(dpad(sv, lx, H * 0.36, H * 0.25, P.get("dpadc", shade(c, -0.45))))
        out.append(stick(sv, lx + W * 0.03, H * 0.75, H * 0.065, shade(c, -0.3)))
        out.append(stick(sv, rx - W * 0.03, H * 0.75, H * 0.065, shade(c, -0.3)))
    elif sticks == 3:  # a small joystick instead of a d-pad
        out.append(stick(sv, lx, H * 0.47, H * 0.1, shade(c, -0.3)))
    else:
        out.append(dpad(sv, lx, H * 0.47, H * 0.3, P.get("dpadc", shade(c, -0.45))))
    nb = P.get("buttons", 2)
    by = H * (0.36 if sticks == 2 else 0.47)
    if nb == 4:
        s = H * 0.1
        for dx, dy in ((0, -s), (s, 0), (0, s), (-s, 0)):
            out.append(button(sv, rx + dx, by + dy, H * 0.05, btn))
    else:
        out.append(button(sv, rx - H * 0.08, by + H * 0.05, H * 0.065, btn))
        out.append(button(sv, rx + H * 0.08, by - H * 0.05, H * 0.065, btn))
    out.append(pill(sv, rx, H * 0.83, W * 0.04, H * 0.035, shade(c, -0.4)))
    out.append(led(sv, sx - 30, sy + 10, 3, P.get("ledc", P["acc"])))
    h = H + (30 if style == "grip" else 0) + 10
    return group("".join(out), 0, 12), W, h + 12


def _slim_d(x0, y0, w, h, b):
    """A long body with flat top and bottom and elliptical ends bulging b."""
    k = 0.552
    return (
        "M%s %sH%s" % (n(x0 + b), n(y0), n(x0 + w - b))
        + "C%s %s %s %s %s %s" % (n(x0 + w - b + b * k), n(y0), n(x0 + w), n(y0 + h / 2 - h / 2 * k), n(x0 + w), n(y0 + h / 2))
        + "C%s %s %s %s %s %s" % (n(x0 + w), n(y0 + h / 2 + h / 2 * k), n(x0 + w - b + b * k), n(y0 + h), n(x0 + w - b), n(y0 + h))
        + "H%s" % n(x0 + b)
        + "C%s %s %s %s %s %s" % (n(x0 + b - b * k), n(y0 + h), n(x0), n(y0 + h / 2 + h / 2 * k), n(x0), n(y0 + h / 2))
        + "C%s %s %s %s %s %s" % (n(x0), n(y0 + h / 2 - h / 2 * k), n(x0 + b - b * k), n(y0), n(x0 + b), n(y0))
        + "Z"
    )


def handheld_slim(sv, P):
    """A long, slim widescreen handheld: elliptical ends, a light trim round the edge, a 16:9
    screen filling the middle, a d-pad over a flat analogue nub on the left, four face buttons in
    a ring on the right, thin shoulder buttons flush with the top corners, a dotted grille along
    the top edge and a row of small keys under the screen. Proportions measured from the real
    thing (fractions of the body's width W and height H)."""
    W = _p(P, "w", 680)
    H = W * _p(P, "aspect", 0.435)  # 170 x 74 mm
    c = P["shell"]
    m = Mat(sv, c)
    trim = P.get("trim", "#a9aeb8")
    tm = Mat(sv, trim)
    b = W * _p(P, "bulge", 0.13)  # the ends start curving 13 % in from each side
    d = _slim_d(0, 0, W, H, b)
    out = []
    # Shoulder buttons: thin slivers of the trim colour hugging the top corners, about 2 % of
    # the height proud of the edge, reaching ~24 % of the width along the top and a third of the
    # way down the ends.
    lift = H * 0.022
    sd = _slim_d(-lift * 0.6, -lift, W + lift * 1.2, H * 0.8, b + lift * 0.6)
    sw0 = W * 0.24
    clip = sv.clip(rrect_d(-20, -20, sw0 + 20, H * 0.34 + 20, 0) + rrect_d(W - sw0, -20, sw0 + 20, H * 0.34 + 20, 0))
    out.append(path(sd, sv.lin([(0, shade(trim, 0.35)), (1, shade(trim, -0.2))]), clip_path=clip))
    for x in (sw0, W - sw0):
        out.append(line(x, -lift, x, 3, tm.outline, 1.5, stroke_opacity="0.6"))
    # Edge trim (the body's thickness) in a light metal, then the glossy face.
    out.append(path(d, sv.lin([(0, shade(trim, 0.3)), (0.5, trim), (1, shade(trim, -0.35))]), transform="translate(0 13)"))
    out.append(path(d, "none", stroke=tm.outline, stroke_width="1.5", stroke_opacity="0.5", transform="translate(0 13)"))
    out.append(path(d, m.front))
    out.append(path(_slim_d(10, 6, W - 20, H * 0.46, b * 0.9), sv.lin([(0, "#ffffff", 0.09), (1, "#ffffff", 0)])))
    out.append(highlight_rim(d, m, 2.5, 0.3))
    out.append(outline(d, m))
    # Dotted grille along the top edge between the shoulders.
    for j, y in enumerate((H * 0.022, H * 0.045)):
        x = sw0 + 10 + j * 3
        while x < W - sw0 - 8:
            out.append(circle(x, y, 1.25, m.deep, fill_opacity="0.95"))
            x += 6
    # Screen: 16:9, 56 % of the width (95 x 54 mm), 9 % down, in a thin dark frame.
    sw = W * _p(P, "screen_w", 0.56)
    sh = sw * 9 / 16
    sx, sy = W / 2 - sw / 2, H * 0.085
    out.append(rect(sx - 5, sy - 5, sw + 10, sh + 10, 6, shade(c, -0.55)))
    out.append(screen(sv, sx, sy, sw, sh, 3, P["tint"]))
    # Left: d-pad (13 % of the width) in a round well, centred 12 % in and 31 % down; the
    # analogue nub (6 % across) under it at 60 % down.
    lx, ly = W * 0.12, H * 0.31
    out.append(circle(lx, ly, W * 0.071, shade(c, -0.35)))
    out.append(circle(lx, ly, W * 0.071, "none", stroke=m.edge, stroke_width="1.5", stroke_opacity="0.25"))
    out.append(dpad(sv, lx, ly, W * 0.118, P.get("dpadc", shade(c, 0.12))))
    nx, ny, nr = W * 0.104, H * 0.61, W * 0.03
    out.append(circle(nx, ny, nr * 1.16, shade(c, -0.45)))
    out.append(circle(nx, ny + 2, nr, shade(c, -0.6)))
    out.append(circle(nx, ny, nr * 0.94, sv.rad([(0, shade(c, 0.32)), (1, shade(c, 0.05))], cx=0.45, cy=0.4, r=0.6)))
    out.append(circle(nx, ny, nr * 0.6, "none", stroke=shade(c, -0.2), stroke_width="1.5", stroke_opacity="0.6"))
    # Right: four plain face buttons (5.4 % across) on a thin ring, centred 89 % across, 52 % down.
    rx, ry = W * 0.888, H * 0.52
    s = W * 0.052
    out.append(circle(rx, ry, s, "none", stroke=shade(c, -0.45), stroke_width="2.5", stroke_opacity="0.9"))
    btn = P.get("btn", shade(c, 0.25))
    for dx, dy in ((0, -s), (s, 0), (0, s), (-s, 0)):
        out.append(button(sv, rx + dx, ry + dy, W * 0.026, btn))
    out.append(led(sv, W * 0.9, H * 0.86, 2.4, P.get("ledc", "#6ee7a8")))
    # Small keys under the screen: home at the far left, volume, then two round keys and
    # select / start towards the right.
    ky = (sy + sh + 5 + H) / 2
    kc = shade(c, 0.2)
    kw, kh = W * 0.055, H * 0.045
    out.append(pill(sv, W * 0.2, ky, kw, kh, kc))
    for t in (0.265, 0.31):
        out.append(button(sv, W * t, ky, kh * 0.55, kc))
    for t in (0.585, 0.625):
        out.append(button(sv, W * t, ky, kh * 0.55, kc))
    for t in (0.685, 0.75):
        out.append(pill(sv, W * t, ky, kw, kh, kc))
    return group("".join(out), 0, 8), W, H + 24


def handheld_landscape(sv, P):
    """A wide landscape handheld: rounded grips at both ends, a gently curved waist along the
    bottom, light shoulder bumpers wrapping the top corners, a screen lens in the middle, a
    d-pad with two small keys on the left and two round buttons on the right."""
    W, H = 720, 408
    c = P["shell"]
    m = Mat(sv, c)
    d = (
        "M110 0H610C682 0 720 44 720 132V250C720 340 690 392 622 398"
        "C600 400 580 394 560 392C480 405 240 405 160 392C140 394 120 400 98 398"
        "C30 392 0 340 0 250V132C0 44 38 0 110 0Z"
    )
    out = []
    sc = P.get("shoulder", "#c4c5cc")
    sm = Mat(sv, sc)
    # Bumpers: a thin grey band (about 1.5 % of the height) round the top corners, reaching 30 %
    # of the width along the top and 45 % of the way down the ends.
    for sd in (rrect_d(-5, -6, 222, 186, [116, 24, 0, 0]), rrect_d(W - 217, -6, 222, 186, [24, 116, 0, 0])):
        out.append(path(sd, sm.front))
        out.append(path(sd, "none", stroke=sm.outline, stroke_width="1.5", stroke_opacity="0.5"))
    out.append(_body(sv, d, m, 12))
    # A soft crease where the grips meet the middle.
    for x in (178, W - 178):
        out.append(path("M%s 30Q%s 210 %s 380" % (n(x), n(x + (-14 if x < W / 2 else 14)), n(x)), "none", stroke=m.light, stroke_width="2", stroke_opacity="0.18"))
    # Screen lens: wider at the top corners, rounder at the bottom.
    lx0, lx1, ly0, ly1 = 170, W - 170, 34, 300
    lens = round_poly([(lx0, ly0), (lx1, ly0), (lx1, ly1 - 40), (lx1 - 44, ly1), (lx0 + 44, ly1), (lx0, ly1 - 40)], [22, 22, 40, 26, 26, 40])
    bz = P.get("bezel", "#2b2b33")
    out.append(path(lens, m.deep, transform="translate(0 3)", fill_opacity="0.6"))
    out.append(path(lens, sv.lin([(0, shade(bz, 0.18)), (1, shade(bz, -0.1))])))
    out.append(path(lens, "none", stroke="#ffffff", stroke_width="1.5", stroke_opacity="0.15"))
    sw, sh = 300, 200
    out.append(screen(sv, W / 2 - sw / 2, 52, sw, sh, 3, P["tint"]))
    out.append(rect(W / 2 - 90, 272, 180, 5, 2.5, "#ffffff", fill_opacity="0.12"))
    # Left: d-pad (12 % of the width, 16.5 % in, 30 % down) and two small round keys, each
    # with a blank embossed label plate beside it.
    lc = P.get("btn", "#d6d6dc")
    out.append(dpad(sv, W * 0.165, H * 0.3, W * 0.12, lc))
    for x, y in ((W * 0.175, H * 0.58), (W * 0.157, H * 0.68)):
        plate = rrect_d(x - 64, y - 10, 46, 20, 10)
        out.append(path(plate, "none", stroke=m.light, stroke_width="1.5", stroke_opacity="0.35", transform="translate(0 1)"))
        out.append(path(plate, "none", stroke=m.dark, stroke_width="1.5", stroke_opacity="0.5"))
        out.append(button(sv, x, y, 9, lc))
    # Right: two round buttons, a power light and a speaker.
    out.append(button(sv, 568, 226, 25, lc))
    out.append(button(sv, 636, 196, 25, lc))
    out.append(led(sv, 574, 108, 3.2, P.get("ledc", "#6ee7a8")))
    for i in range(6):
        y = 296 + i * 13
        out.append(group(rect(-36, -2.5, 72, 5, 2.5, m.deep, fill_opacity="0.85"), 604 - i * 3, y, rot=-8))
    return group("".join(out), 0, 10), W, H + 24


def clamshell(sv, P):
    W = _p(P, "w", 440)
    c = P["shell"]
    m = Mat(sv, c)
    top_h, bot_h = 270, 270
    wide = P.get("wide_top", False)
    out = []
    lid = rrect_d(12, 0, W - 24, top_h, [26, 26, 10, 10])
    lm = Mat(sv, shade(c, 0.06))
    out.append(_body(sv, lid, lm, 6))
    sw = (W - 24) * (0.8 if wide else 0.66)
    out.append(rect(W / 2 - sw / 2 - 10, 26, sw + 20, top_h * 0.78 + 4, 12, shade(c, -0.55)))
    out.append(screen(sv, W / 2 - sw / 2, 34, sw, top_h * 0.74 - 6, 6, P["tint"]))
    # Hinge.
    out.append(rect(30, top_h + 2, W - 60, 22, 10, sv.lin([(0, shade(c, -0.15)), (1, shade(c, -0.45))])))
    y0 = top_h + 22
    base = rrect_d(0, y0, W, bot_h, [10, 10, 30, 30])
    out.append(_body(sv, base, m))
    bw = W * 0.5
    out.append(rect(W / 2 - bw / 2 - 8, y0 + 22, bw + 16, bot_h * 0.66 + 4, 10, shade(c, -0.5)))
    out.append(screen(sv, W / 2 - bw / 2, y0 + 30, bw, bot_h * 0.62 - 4, 5, P.get("tint2", P["tint"]), glow=True))
    lx, rx = W * 0.13, W * 0.87
    if P.get("stick"):
        out.append(stick(sv, lx, y0 + bot_h * 0.3, 20, shade(c, -0.3)))
        out.append(dpad(sv, lx, y0 + bot_h * 0.66, 64, shade(c, -0.45)))
    else:
        out.append(dpad(sv, lx, y0 + bot_h * 0.42, 72, shade(c, -0.45)))
    s = 26
    by = y0 + bot_h * 0.42
    for dx, dy in ((0, -s), (s, 0), (0, s), (-s, 0)):
        out.append(button(sv, rx + dx, by + dy, 12, P.get("btn", shade(c, 0.4))))
    out.append(led(sv, W * 0.1, y0 + bot_h * 0.86, 3, P["acc"]))
    return "".join(out), W, y0 + bot_h + 10


def hybrid(sv, P):
    TW, H = 460, 290
    cw = 96
    W = TW + 2 * cw
    c = P["shell"]
    m = Mat(sv, c)
    out = []
    lc, rc = P.get("left", P["acc"]), P.get("right", P["acc"])
    lm, rm = Mat(sv, lc), Mat(sv, rc)
    ld = rrect_d(0, 0, cw + 10, H, [48, 6, 6, 48])
    rd = rrect_d(W - cw - 10, 0, cw + 10, H, [6, 48, 48, 6])
    out.append(_body(sv, ld, lm))
    out.append(_body(sv, rd, rm))
    td = rrect_d(cw, 0, TW, H, 12)
    out.append(_body(sv, td, m))
    out.append(screen(sv, cw + 26, 22, TW - 52, H - 44, 4, P["tint"]))
    jc = shade(lc, -0.6)
    out.append(stick(sv, cw * 0.5, H * 0.3, 22, jc))
    s = 22
    for dx, dy in ((0, -s), (s, 0), (0, s), (-s, 0)):
        out.append(button(sv, cw * 0.5 + dx, H * 0.64 + dy, 10, shade(lc, -0.55)))
        out.append(button(sv, W - cw * 0.5 + dx, H * 0.34 + dy, 10, shade(rc, -0.55)))
    out.append(stick(sv, W - cw * 0.5, H * 0.68, 22, shade(rc, -0.6)))
    return "".join(out), W, H + 10


def tablet(sv, P):
    W, H = 660, 330
    c = P["shell"]
    m = Mat(sv, c)
    d = round_poly(
        [(40, 0), (W - 40, 0), (W, 60), (W - 6, H - 40), (W - 70, H), (70, H), (6, H - 40), (0, 60)],
        [40, 40, 50, 60, 40, 40, 60, 50],
    )
    out = [_body(sv, d, m)]
    sw, sh = 330, 210
    sx, sy = W / 2 - sw / 2, 40
    out.append(rect(sx - 14, sy - 12, sw + 28, sh + 24, 10, shade(c, -0.55) if P.get("dark_bezel", True) else shade(c, -0.1)))
    out.append(screen(sv, sx, sy, sw, sh, 4, P["tint"]))
    out.append(stick(sv, 92, 92, 24, shade(c, -0.3)))
    out.append(stick(sv, W - 92, 92, 24, shade(c, -0.3)))
    out.append(dpad(sv, 92, 200, 74, shade(c, -0.45)))
    s = 25
    for dx, dy in ((0, -s), (s, 0), (0, s), (-s, 0)):
        out.append(button(sv, W - 92 + dx, 200 + dy, 11, P.get("btn", shade(c, 0.3))))
    out.append(led(sv, W / 2, H - 40, 3.5, P["acc"]))
    for x in (W / 2 - 60, W / 2 + 60):
        out.append(pill(sv, x, H - 40, 34, 11, shade(c, -0.3)))
    return "".join(out), W, H + 10


# ====================================================================== computers


def keyboard(sv, P, W=640, D=170, F=44, x0=0, y0=0):
    """A keyboard computer seen from the front and a little above."""
    c = P["shell"]
    m = Mat(sv, c)
    ins = _p(P, "kb_persp", 34)
    r = _p(P, "r", 12)
    out = []
    sil = round_poly([(ins, 0), (W - ins, 0), (W, D), (W, D + F), (0, D + F), (0, D)], [r, r, 4, r, r, 4])
    out.append(path(sil, m.front))
    out.append(path(round_poly([(ins, 0), (W - ins, 0), (W, D), (0, D)], [r, r, 3, 3]), m.top))
    out.append(line(3, D, W - 3, D, m.edge, 2, stroke_opacity="0.5"))
    kc = P.get("keys", shade(c, -0.55))
    fk = P.get("fkeys")
    rows = 5
    flat = P.get("flat_keys", False)
    kx0 = ins + 18
    kx1 = (W - ins - 18) - (90 if fk else 0) - (120 if P.get("floppy_top") else 0)
    km = sv.lin([(0, shade(kc, 0.25 if not flat else 0.1)), (1, shade(kc, -0.12))])
    for j in range(rows):
        t0 = 0.12 + j * 0.16
        y = D * t0
        kh = D * 0.12
        # Perspective: rows nearer the front are wider.
        sh_l = ins * (1 - t0) * 0.9
        a = kx0 - ins + sh_l + 8
        b = kx1 + ins - sh_l - 8
        if j == rows - 1:
            out.append(rect(a + (b - a) * 0.22, y, (b - a) * 0.5, kh, 3, km))
            out.append(rect(a, y, (b - a) * 0.18, kh, 3, km))
            out.append(rect(a + (b - a) * 0.76, y, (b - a) * 0.24, kh, 3, km))
            continue
        if fk and j < 4:
            fm = sv.lin([(0, shade(fk, 0.25)), (1, shade(fk, -0.15))])
            out.append(rect(b + 10, y, 64 - j * 2, kh, 3, fm))
        k = 13
        kw = (b - a) / k
        for i in range(k):
            col = km
            out.append(rect(a + i * kw + 2, y, kw - 4, kh, 3, col))
            if not flat:
                out.append(rect(a + i * kw + 4, y + 1.5, kw - 8, kh * 0.45, 2, "#ffffff", fill_opacity="0.08"))
    if P.get("floppy_top"):
        out.append(rect(W - ins - 130, D * 0.2, 104, D * 0.62, 6, shade(c, -0.12)))
        out.append(rect(W - ins - 120, D * 0.45, 84, 7, 3, m.deep))
    if P.get("floppy"):
        out.append(rect(W * 0.7, D + F * 0.4, W * 0.22, 8, 4, m.deep) + rect(W * 0.94, D + F * 0.35, 14, 10, 3, m.dark))
    if P.get("stripe"):
        out.append(rect(W * 0.04, D + F * 0.3, W * 0.2, F * 0.14, 2, P["acc"]))
    out.append(led(sv, W * 0.06 if not P.get("stripe") else W * 0.28, D + F * 0.45, 3.5, P.get("ledc", P["acc"])))
    out.append(outline(sil, m))
    return group("".join(out), x0, y0), W, D + F


def crt(sv, P, W=430, H=330, x0=0, y0=0, content="glow"):
    c = P.get("monitor", P["shell"])
    m = Mat(sv, c)
    out = []
    d = rrect_d(0, 0, W, H, 26)
    out.append(path(d, m.front))
    out.append(highlight_rim(d, m, 2.5, 0.3))
    out.append(rect(W * 0.07, H * 0.08, W * 0.86, H * 0.74, 22, shade(c, -0.3)))
    sx, sy, sw, sh = W * 0.12, H * 0.13, W * 0.76, H * 0.64
    out.append(screen(sv, sx, sy, sw, sh, 26, P["tint"]))
    out.append(_screen_content(sv, P, sx, sy, sw, sh, content))
    out.append(led(sv, W * 0.88, H * 0.9, 3.5, P["acc"]))
    out.append(slots(W * 0.1, H * 0.86, W * 0.3, H * 0.08, 6, m.dark))
    out.append(outline(d, m))
    return group("".join(out), x0, y0), W, H


def _screen_content(sv, P, x, y, w, h, kind):
    t = shade(P["tint"], 0.45)
    out = []
    if kind == "text":
        for i in range(6):
            ww = w * (0.3 + 0.5 * ((i * 37) % 11) / 11)
            out.append(rect(x + w * 0.1, y + h * (0.16 + i * 0.12), ww, h * 0.045, 2, t, fill_opacity="0.75"))
        out.append(rect(x + w * 0.1, y + h * 0.88, w * 0.05, h * 0.06, 1, t))
    elif kind == "prompt":
        out.append(path("M%s %sl%s %sl%s %s" % (n(x + w * 0.1), n(y + h * 0.18), n(w * 0.05), n(h * 0.06), n(-w * 0.05), n(h * 0.06)), "none", stroke=t, stroke_width="4", stroke_linecap="round", stroke_linejoin="round"))
        out.append(rect(x + w * 0.2, y + h * 0.26, w * 0.06, h * 0.05, 1, t))
        for i in range(4):
            out.append(rect(x + w * 0.1, y + h * (0.45 + i * 0.12), w * (0.25 + 0.12 * (i % 3)), h * 0.04, 2, t, fill_opacity="0.5"))
    elif kind == "tiles":
        cols = [shade(P["acc"], 0.1), shade(P["tint"], 0.3), mix(P["acc"], "#ffffff", 0.5), shade(P["acc"], -0.2)]
        out.append(rect(x, y + h - h * 0.1, w, h * 0.1, 0, "#000000", fill_opacity="0.35"))
        for i in range(3):
            for j in range(2):
                out.append(rect(x + w * (0.08 + i * 0.16), y + h * (0.12 + j * 0.26), w * 0.13, h * 0.2, 4, cols[(i + j) % 4], fill_opacity="0.8"))
        out.append(rect(x + w * 0.58, y + h * 0.14, w * 0.34, h * 0.48, 6, "#ffffff", fill_opacity="0.14"))
        out.append(rect(x + w * 0.58, y + h * 0.14, w * 0.34, h * 0.07, 6, "#ffffff", fill_opacity="0.2"))
    elif kind == "browser":
        out.append(rect(x, y, w, h * 0.12, 0, "#ffffff", fill_opacity="0.16"))
        for i in range(3):
            out.append(circle(x + w * (0.04 + i * 0.035), y + h * 0.06, h * 0.022, "#ffffff", fill_opacity="0.45"))
        out.append(rect(x + w * 0.18, y + h * 0.03, w * 0.6, h * 0.06, h * 0.03, "#ffffff", fill_opacity="0.18"))
        cx, cy, r = x + w / 2, y + h * 0.56, h * 0.22
        out.append(circle(cx, cy, r, P["acc"], fill_opacity="0.9"))
        out.append(path("M%s %sL%s %sL%s %sZ" % (n(cx - r * 0.3), n(cy - r * 0.45), n(cx + r * 0.5), n(cy), n(cx - r * 0.3), n(cy + r * 0.45)), "#ffffff"))
    elif kind == "pixels":
        sprite = ["..x.....x..", "...x...x...", "..xxxxxxx..", ".xx.xxx.xx.", "xxxxxxxxxxx", "x.xxxxxxx.x", "x.x.....x.x", "...xx.xx..."]
        ps = min(w / 16, h / 12)
        ox = x + w / 2 - ps * 5.5
        oy = y + h / 2 - ps * 4
        for j, row in enumerate(sprite):
            for i, ch in enumerate(row):
                if ch == "x":
                    out.append(rect(ox + i * ps, oy + j * ps, ps - 1, ps - 1, 0, t))
    elif kind == "vector":
        g = shade(P["tint"], 0.6)
        pts = [(0.2, 0.7), (0.35, 0.35), (0.5, 0.55), (0.65, 0.25), (0.8, 0.6)]
        d = "M" + "L".join("%s %s" % (n(x + w * a), n(y + h * b)) for a, b in pts)
        out.append(path(d, "none", stroke=g, stroke_width="9", stroke_opacity="0.25", stroke_linejoin="round", stroke_linecap="round"))
        out.append(path(d, "none", stroke=g, stroke_width="2.5", stroke_linejoin="round", stroke_linecap="round"))
        out.append(circle(x + w * 0.5, y + h * 0.82, w * 0.08, "none", stroke=g, stroke_width="2"))
    return "".join(out)


def computer(sv, P):
    style = P.get("style", "keyboard")
    if style == "keyboard":
        return keyboard(sv, P)
    if style == "keyboard_monitor":
        mw, mh = 420, 330
        kb, kw, kh = keyboard(sv, P, W=600, D=150, F=40)
        crt_, _, _ = crt(sv, P, mw, mh, x0=(600 - mw) / 2, y0=0, content=P.get("screen", "text"))
        return crt_ + group(kb, 0, mh - 40), 600, mh - 40 + kh
    if style == "desktop":
        mw, mh = 400, 310
        c = P["shell"]
        m = Mat(sv, c)
        box = path(rrect_d(40, mh - 10, 520, 90, 10), m.front) + line(46, mh + 30, 554, mh + 30, m.line, 2, stroke_opacity="0.5")
        box += rect(330, mh + 40, 140, 8, 4, m.deep) + rect(330, mh + 58, 140, 8, 4, m.deep) + led(sv, 80, mh + 50, 3.5, P["acc"])
        box += outline(rrect_d(40, mh - 10, 520, 90, 10), m)
        crt_, _, _ = crt(sv, P, mw, mh, x0=100, y0=0, content=P.get("screen", "text"))
        kb, kw, kh = keyboard(sv, dict(P, kb_persp=24), W=600, D=110, F=30)
        return crt_ + box + group(kb, 0, mh + 66), 600, mh + 66 + kh
    if style == "allinone":
        c = P["shell"]
        m = Mat(sv, c)
        W, H = 520, 470
        d = round_poly([(60, 0), (W - 60, 0), (W - 30, 260), (W, H), (0, H), (30, 260)], [40, 40, 10, 16, 16, 10])
        out = [path(d, m.front), highlight_rim(d, m, 2.5, 0.3)]
        out.append(rect(110, 30, W - 220, 200, 22, shade(c, -0.5)))
        out.append(screen(sv, 124, 42, W - 248, 176, 18, P["tint"]))
        out.append(_screen_content(sv, P, 124, 42, W - 248, 176, "text"))
        kb, kw, kh = keyboard(sv, dict(P, kb_persp=30), W=W - 40, D=150, F=24)
        out.append(group(kb, 20, 280))
        out.append(outline(d, m))
        return "".join(out), W, H
    if style == "compact":
        c = P["shell"]
        m = Mat(sv, c)
        W, H = 330, 440
        d = rrect_d(0, 0, W, H, 22)
        out = [path(rrect_d(10, H - 8, W - 20, 22, [0, 0, 10, 10]), m.dark), path(d, m.front), highlight_rim(d, m, 2.5, 0.3)]
        out.append(rect(36, 34, W - 72, 210, 16, shade(c, -0.25)))
        out.append(screen(sv, 56, 52, W - 112, 172, 12, P["tint"]))
        out.append(_screen_content(sv, P, 56, 52, W - 112, 172, "tiles"))
        out.append(rect(W * 0.56, 310, W * 0.32, 9, 4, m.deep))
        for i in range(5):
            out.append(line(30, 380 + i * 9, 120, 380 + i * 9, m.line, 2, stroke_opacity="0.4"))
        out.append(led(sv, 44, 312, 3, P["acc"]))
        out.append(outline(d, m))
        return "".join(out), W, H + 14
    if style == "twin":
        c = P["shell"]
        m = Mat(sv, c)
        out = []
        for x in (0, 110):
            d = rrect_d(x, 0, 96, 380, [18, 18, 8, 8])
            out.append(path(d, m.front) + highlight_rim(d, m, 2, 0.3) + outline(d, m))
            out.append(rect(x + 18, 40, 60, 8, 4, m.deep) + rect(x + 18, 64, 60, 8, 4, m.deep))
            out.append(slots(x + 14, 250, 68, 90, 5, m.dark, vertical=False))
        out.append(path(rrect_d(96, 6, 14, 30, 4), m.dark))
        out.append(led(sv, 48, 110, 4, P["acc"]))
        crt_, _, _ = crt(sv, P, 360, 290, x0=250, y0=40, content=P.get("screen", "tiles"))
        kb, kw, kh = keyboard(sv, dict(P, kb_persp=26), W=520, D=110, F=30)
        return "".join(out) + crt_ + group(kb, 80, 330), 610, 330 + kh
    return keyboard(sv, P)


def monitor(sv, P):
    """A flat-panel PC monitor (or a CRT for DOS) with a keyboard in front."""
    content = P.get("screen", "tiles")
    c = P.get("monitor", "#23262d")
    m = Mat(sv, c)
    out = []
    if P.get("crt"):
        body, w, h = crt(sv, P, 470, 360, x0=45, y0=0, content=content)
        out.append(body)
        top_h = 360
    else:
        W, H = 560, 340
        # Stand.
        sm = Mat(sv, shade(c, 0.12))
        out.append(path(round_poly([(W * 0.44, H - 10), (W * 0.56, H - 10), (W * 0.6, H + 60), (W * 0.4, H + 60)], 6), sm.front))
        out.append(path(rrect_d(W * 0.28, H + 54, W * 0.44, 16, [6, 6, 8, 8]), sm.front))
        d = rrect_d(0, 0, W, H, 14)
        out.append(path(d, m.front) + highlight_rim(d, m, 2, 0.35))
        out.append(screen(sv, 14, 14, W - 28, H - 40, 4, P["tint"]))
        out.append(_screen_content(sv, P, 14, 14, W - 28, H - 40, content))
        out.append(led(sv, W / 2, H - 13, 2.5, P["acc"]))
        out.append(outline(d, m))
        top_h = H + 70
    kb_c = P.get("keys_shell", shade(c, 0.1))
    kb, kw, kh = keyboard(sv, dict(P, shell=kb_c, keys=shade(kb_c, -0.5), kb_persp=22, r=8, stripe=False), W=520, D=90, F=20)
    out.append(group(kb, 20, top_h - 30))
    return "".join(out), 560, top_h - 30 + kh


# ====================================================================== other machines


def arcade(sv, P):
    W, H = 330, 600
    c = P["shell"]
    m = Mat(sv, c)
    out = []
    sil = round_poly(
        [(10, 0), (W - 10, 0), (W - 10, 90), (W - 40, 140), (W - 40, 330), (W, 360), (W, 400), (W - 20, 410), (W - 20, H), (20, H), (20, 410), (0, 400), (0, 360), (40, 330), (40, 140), (10, 90)],
        [14, 14, 6, 6, 6, 8, 8, 6, 6, 6, 6, 8, 8, 6, 6, 6],
    )
    out.append(path(sil, m.front))
    # Marquee.
    mq = sv.lin([(0, shade(P["acc"], 0.45)), (1, shade(P["acc"], -0.1))])
    out.append(rect(22, 14, W - 44, 64, 8, mq))
    out.append(rect(22, 14, W - 44, 64, 8, sv.rad([(0, "#ffffff", 0.5), (1, "#ffffff", 0)], cx=0.5, cy=0.3, r=0.6)))
    out.append(star4(W / 2, 46, 20, "#ffffff", "0.85"))
    # Screen.
    out.append(rect(52, 150, W - 104, 170, 10, shade(c, -0.6)))
    out.append(screen(sv, 62, 160, W - 124, 150, 8, P["tint"]))
    out.append(_screen_content(sv, P, 62, 160, W - 124, 150, "pixels"))
    # Control panel.
    out.append(path(round_poly([(4, 360), (W - 4, 360), (W - 4, 398), (4, 398)], 6), sv.lin([(0, shade(c, 0.3)), (1, shade(c, -0.1))])))
    out.append(line(W * 0.22, 372, W * 0.22, 348, "#c9ccd3", 6, stroke_linecap="round"))
    out.append(circle(W * 0.22, 344, 11, sv.rad([(0, "#ff8b8b"), (1, "#b3262f")], cx=0.4, cy=0.35)))
    cols = [P["btn"] if "btn" in P else P["acc"], shade(P["acc"], 0.4)]
    for i in range(3):
        for j in range(2):
            out.append(ellipse(W * (0.48 + i * 0.12) + j * 8, 368 + j * 18, 11, 6, cols[j]))
    # Coin door.
    out.append(rect(W * 0.32, 450, W * 0.36, 110, 8, shade(c, -0.35)))
    for x in (0.42, 0.58):
        out.append(rect(W * x - 4, 470, 8, 24, 3, sv.lin([(0, "#ffb060"), (1, "#d8662a")])))
    out.append(outline(sil, m))
    return "".join(out), W, H


def vector(sv, P):
    W, H = 330, 470
    c = P["shell"]
    m = Mat(sv, c)
    d = rrect_d(0, 0, W, H, [30, 30, 16, 16])
    out = [path(rrect_d(20, H - 10, W - 40, 24, [0, 0, 10, 10]), m.dark), path(d, m.front), highlight_rim(d, m, 2.5, 0.3)]
    out.append(rect(30, 30, W - 60, H - 100, 18, shade(c, -0.6)))
    out.append(screen(sv, 42, 42, W - 84, H - 124, 14, P["tint"]))
    out.append(_screen_content(sv, P, 42, 42, W - 84, H - 124, "vector"))
    out.append(slots(60, H - 56, 120, 26, 8, m.dark))
    out.append(led(sv, W - 60, H - 44, 4, P["acc"]))
    out.append(outline(d, m))
    # Controller in front.
    cm = Mat(sv, shade(c, 0.05))
    cd = rrect_d(-40, H - 30, 220, 80, 12)
    ctl = path(cd, cm.front) + outline(cd, cm) + stick(sv, 0, H + 10, 14, shade(c, -0.3))
    for i in range(4):
        ctl += button(sv, 60 + i * 30, H + 10, 9, P.get("btn", P["acc"]))
    return group("".join(out) + ctl, 40, 0), W + 40, H + 60


def gamepad(sv, P, W=520):
    c = P["shell"]
    m = Mat(sv, c)
    H = W * 0.6
    d = (
        "M%s %s" % (n(W * 0.22), n(H * 0.05))
        + "C%s %s %s %s %s %s" % (n(W * 0.36), n(H * 0.0), n(W * 0.64), n(H * 0.0), n(W * 0.78), n(H * 0.05))
        + "C%s %s %s %s %s %s" % (n(W * 0.95), n(H * 0.1), n(W * 1.02), n(H * 0.7), n(W * 0.97), n(H * 0.92))
        + "C%s %s %s %s %s %s" % (n(W * 0.93), n(H * 1.06), n(W * 0.8), n(H * 1.02), n(W * 0.74), n(H * 0.86))
        + "C%s %s %s %s %s %s" % (n(W * 0.68), n(H * 0.74), n(W * 0.32), n(H * 0.74), n(W * 0.26), n(H * 0.86))
        + "C%s %s %s %s %s %s" % (n(W * 0.2), n(H * 1.02), n(W * 0.07), n(H * 1.06), n(W * 0.03), n(H * 0.92))
        + "C%s %s %s %s %s %s" % (n(-W * 0.02), n(H * 0.7), n(W * 0.05), n(H * 0.1), n(W * 0.22), n(H * 0.05))
        + "Z"
    )
    out = [_body(sv, d, m, 12)]
    out.append(dpad(sv, W * 0.22, H * 0.36, W * 0.15, shade(c, -0.5)))
    s = W * 0.055
    cols = P.get("face", [P["acc"]] * 4)
    for (dx, dy), col in zip(((0, -s), (s, 0), (0, s), (-s, 0)), cols):
        out.append(button(sv, W * 0.78 + dx, H * 0.36 + dy, W * 0.026, col))
    out.append(stick(sv, W * 0.36, H * 0.62, W * 0.045, shade(c, -0.35)))
    out.append(stick(sv, W * 0.64, H * 0.62, W * 0.045, shade(c, -0.35)))
    out.append(pill(sv, W * 0.43, H * 0.3, W * 0.05, W * 0.018, shade(c, -0.4)))
    out.append(pill(sv, W * 0.57, H * 0.3, W * 0.05, W * 0.018, shade(c, -0.4)))
    out.append(led(sv, W * 0.5, H * 0.18, 4, P.get("ledc", P["acc"])))
    return "".join(out), W, H * 1.05 + 12


def pixel_box(sv, P):
    """A tiny fantasy computer: a chunky screen box with a pixel sprite and a keyboard."""
    c = P["shell"]
    m = Mat(sv, c)
    W, H = 440, 340
    d = rrect_d(0, 0, W, H, 40)
    out = [_body(sv, d, m, 12)]
    out.append(rect(36, 32, W - 72, H - 110, 18, shade(c, -0.6)))
    out.append(screen(sv, 50, 46, W - 100, H - 138, 10, P["tint"]))
    out.append(_screen_content(sv, P, 50, 46, W - 100, H - 138, "pixels"))
    for i in range(4):
        out.append(rect(60 + i * 36, H - 52, 26, 18, 4, [P["acc"], shade(P["acc"], 0.4), mix(P["acc"], "#ffd166", 0.6), mix(P["acc"], "#7ae7c7", 0.6)][i]))
    out.append(slots(W - 160, H - 56, 110, 26, 7, m.dark))
    return "".join(out), W, H + 12


def board(sv, P):
    """An open-hardware console: a bare circuit board."""
    c = P.get("board", "#1f5f55")
    W, H = 600, 380
    out = []
    d = rrect_d(0, 0, W, H, 18)
    bm = Mat(sv, c)
    out.append(path(d, bm.deep, transform="translate(0 10)"))
    out.append(path(d, sv.lin([(0, shade(c, 0.12)), (1, shade(c, -0.15))])))
    tr = shade(c, 0.35)
    for i in range(9):
        y = 40 + i * 34
        out.append(path("M20 %sH%sL%s %sH%s" % (n(y), n(150 + i * 12), n(180 + i * 12), n(y + 18), n(W - 20)), "none", stroke=tr, stroke_width="2.5", stroke_opacity="0.45"))
    chips = [(150, 90, 150, 110), (350, 70, 110, 80), (360, 210, 90, 90), (60, 230, 120, 60)]
    for x, y, w, h in chips:
        out.append(rect(x + 3, y + 5, w, h, 6, "#000000", fill_opacity="0.4"))
        out.append(rect(x, y, w, h, 6, sv.lin([(0, "#3a3d44"), (1, "#16181c")])))
        for k in range(int(w // 14)):
            out.append(rect(x + 7 + k * 14, y - 7, 6, 8, 1, "#c9cbd1"))
            out.append(rect(x + 7 + k * 14, y + h - 1, 6, 8, 1, "#c9cbd1"))
        out.append(circle(x + 12, y + 12, 4, "#5a5e68"))
    for i, col in enumerate([P["acc"], "#ffd166", "#6ee7a8"]):
        out.append(led(sv, W - 60 - i * 28, H - 40, 6, col))
    out.append(rect(W - 110, 30, 80, 50, 8, sv.lin([(0, "#d8dbe2"), (1, "#9ba1ad")])))
    out.append(rect(W - 98, 44, 56, 22, 4, "#2b2e35"))
    for x, y in ((20, 20), (W - 20, 20), (20, H - 20), (W - 20, H - 20)):
        out.append(circle(x, y, 8, "#d8c27a") + circle(x, y, 4, shade(c, -0.6)))
    return "".join(out), W, H + 10


# ====================================================================== collections


def cards(sv, P):
    """A fanned hand of three game boxes."""
    W, H = 520, 420
    cols = P.get("cards", [shade(P["acc"], -0.25), P["acc"], shade(P["acc"], 0.25)])
    out = []
    for i, (rot, col) in enumerate(zip((-14, 0, 14), cols)):
        cw, ch = 210, 290
        cm = Mat(sv, col)
        art = sv.lin([(0, shade(col, 0.35)), (1, shade(col, -0.35))], 0, 0, 1, 1)
        body = (
            rect(-cw / 2 + 6, -ch + 12, cw, ch, 16, "#000000", fill_opacity="0.35")
            + rect(-cw / 2, -ch, cw, ch, 16, art)
            + circle(cw * 0.18, -ch * 0.72, cw * 0.16, "#ffffff", fill_opacity="0.35")
            + path("M%s %sL%s %sL%s %sL%s %sL%s %sZ" % (n(-cw / 2), n(-ch * 0.22), n(-cw * 0.15), n(-ch * 0.55), n(cw * 0.08), n(-ch * 0.36), n(cw * 0.25), n(-ch * 0.5), n(cw / 2), n(-ch * 0.22)), shade(col, -0.45), fill_opacity="0.6")
            + rect(-cw / 2, -ch * 0.22, cw, ch * 0.22, 0, shade(col, -0.55), fill_opacity="0.7")
            + rect(-cw * 0.38, -ch * 0.15, cw * 0.5, 10, 5, "#ffffff", fill_opacity="0.7")
            + rect(-cw * 0.38, -ch * 0.08, cw * 0.3, 8, 4, "#ffffff", fill_opacity="0.4")
            + rect(-cw / 2, -ch, cw, ch, 16, "none", stroke="#ffffff", stroke_opacity="0.25", stroke_width="2")
        )
        out.append(group(body, W / 2 + (i - 1) * 110, H - 20, rot=rot))
    return "".join(out), W, H


def star(sv, P):
    W = H = 440
    cx, cy = W / 2, H / 2 + 10
    R, r = 190, 82
    pts = []
    for i in range(10):
        a = -math.pi / 2 + i * math.pi / 5
        rr = R if i % 2 == 0 else r
        pts.append((cx + rr * math.cos(a), cy + rr * math.sin(a)))
    d = round_poly(pts, [26 if i % 2 == 0 else 10 for i in range(10)])
    col = P.get("starc", "#ffc44d")
    out = [
        path(d, shade(col, -0.6), transform="translate(8 14)", fill_opacity="0.55"),
        path(d, sv.lin([(0, shade(col, 0.4)), (0.55, col), (1, shade(col, -0.3))], 0, 0, 0.6, 1)),
    ]
    inner = round_poly([(cx + (x - cx) * 0.62, cy + (y - cy) * 0.62) for x, y in pts], [16 if i % 2 == 0 else 6 for i in range(10)])
    out.append(path(inner, "#ffffff", fill_opacity="0.18"))
    out.append(path(d, "none", stroke="#ffffff", stroke_opacity="0.45", stroke_width="3"))
    out.append(star4(W * 0.86, H * 0.16, 26, "#ffffff", "0.85"))
    out.append(star4(W * 0.12, H * 0.78, 16, "#ffffff", "0.6"))
    return "".join(out), W, H


def play(sv, P):
    W = H = 440
    cx, cy, R = W / 2, H / 2, 180
    acc = P["acc"]
    out = [circle(cx + 6, cy + 12, R, "#000000", fill_opacity="0.35")]
    out.append(circle(cx, cy, R, sv.lin([(0, shade(P["glow"], 0.1)), (1, shade(P["glow"], -0.45))])))
    out.append(circle(cx, cy, R - 30, "none", stroke="#ffffff", stroke_opacity="0.14", stroke_width="16"))
    # Progress arc (about three quarters round).
    a0, a1 = -math.pi / 2, -math.pi / 2 + math.pi * 1.5
    rr = R - 30
    d = "M%s %sA%s %s 0 1 1 %s %s" % (n(cx + rr * math.cos(a0)), n(cy + rr * math.sin(a0)), n(rr), n(rr), n(cx + rr * math.cos(a1)), n(cy + rr * math.sin(a1)))
    out.append(path(d, "none", stroke=shade(acc, 0.3), stroke_width="16", stroke_linecap="round"))
    out.append(circle(cx + rr * math.cos(a1), cy + rr * math.sin(a1), 14, "#ffffff"))
    t = R * 0.42
    tri = round_poly([(cx - t * 0.6, cy - t), (cx + t, cy), (cx - t * 0.6, cy + t)], 14)
    out.append(path(tri, "#ffffff"))
    out.append(ellipse(cx - R * 0.35, cy - R * 0.55, R * 0.35, R * 0.14, "#ffffff", fill_opacity="0.18", transform="rotate(-30 %s %s)" % (n(cx - R * 0.35), n(cy - R * 0.55))))
    return "".join(out), W, H


def drive(sv, P):
    W, H = 520, 360
    c = P.get("shell", "#2c3038")
    m = Mat(sv, c)
    out = []
    d = rrect_d(0, 110, W, 170, 22)
    out.append(_body(sv, d, m, 14))
    out.append(rect(30, 140, W * 0.55, 110, 10, sv.lin([(0, shade(P["acc"], 0.2)), (1, shade(P["acc"], -0.25))])))
    for i in range(3):
        out.append(rect(50, 160 + i * 28, W * 0.35 - i * 40, 10, 5, "#ffffff", fill_opacity="0.55" if i == 0 else "0.3"))
    for i in range(6):
        out.append(rect(W * 0.66 + i * 26, 150, 16, 100, 4, m.dark))
    out.append(led(sv, W - 36, 250, 5, "#6ee7a8"))
    # Badge with a downward arrow.
    bx, by, br = W - 90, 96, 70
    out.append(circle(bx + 4, by + 8, br, "#000000", fill_opacity="0.35"))
    out.append(circle(bx, by, br, sv.lin([(0, "#7ef0b8"), (1, "#22a874")])))
    out.append(path("M%s %sV%sM%s %sL%s %sL%s %s" % (n(bx), n(by - 32), n(by + 28), n(bx - 26), n(by + 4), n(bx), n(by + 30), n(bx + 26), n(by + 4)), "none", stroke="#ffffff", stroke_width="14", stroke_linecap="round", stroke_linejoin="round"))
    return "".join(out), W + 10, H


def pc_combo(sv, P):
    mon, mw, mh = monitor(sv, P)
    c = P.get("tower", "#2a2e36")
    m = Mat(sv, c)
    TW, TH = 200, 440
    d = rrect_d(0, 0, TW, TH, 14)
    t = path(d, m.front) + highlight_rim(d, m, 2, 0.3)
    t += rect(TW * 0.12, 30, TW * 0.76, TH * 0.5, 8, sv.lin([(0, "#0b0d12"), (1, "#151924")]))
    t += rect(TW * 0.12, 30, TW * 0.76, TH * 0.5, 8, sv.rad([(0, P["acc"], 0.6), (1, P["acc"], 0)], cx=0.5, cy=0.6, r=0.6))
    for i in range(3):
        t += circle(TW / 2, 70 + i * 62, 24, "none", stroke=shade(P["acc"], 0.3), stroke_width="3", stroke_opacity="0.7")
    t += slots(TW * 0.15, TH * 0.66, TW * 0.7, TH * 0.24, 6, m.dark, vertical=False)
    t += led(sv, TW / 2, TH - 22, 4, P["acc"]) + outline(d, m)
    return group(mon, 0, TH - mh + 20) + group(t, mw + 30, 0), mw + 30 + TW, TH + 20


def shelf(sv, P):
    """A row of game boxes on a shelf (a store's or launcher's library)."""
    import random

    W, H = 560, 400
    acc = P["acc"]
    rnd = random.Random(P.get("seed", ""))
    out = []
    widths = [64, 54, 70, 58]
    heights = [300, 270, 320, 285]
    cols = [shade(acc, -0.35), mix(acc, P["glow"], 0.5), shade(acc, 0.15), shade(P["glow"], -0.4)]
    rnd.shuffle(widths)
    rnd.shuffle(heights)
    rnd.shuffle(cols)
    x = 70
    for i, (w, h, col) in enumerate(zip(widths, heights, cols)):
        cm = Mat(sv, col)
        out.append(rect(x, H - 40 - h, w, h, 6, cm.front))
        out.append(rect(x + 8, H - 40 - h + 22, w - 16, 10, 4, "#ffffff", fill_opacity="0.55"))
        out.append(rect(x + 8, H - 40 - h + 40, w - 26, 7, 3, "#ffffff", fill_opacity="0.3"))
        out.append(rect(x, H - 40 - h, 6, h, 3, "#ffffff", fill_opacity="0.18"))
        out.append(star4(x + w / 2, H - 90, w * 0.18, "#ffffff", "0.5"))
        x += w + 8
    # A leaning box and a face-out cover.
    col = shade(acc, 0.35)
    cm = Mat(sv, col)
    lean = rect(0, -290, 60, 290, 6, cm.front) + rect(8, -268, 44, 10, 4, "#ffffff", fill_opacity="0.55")
    out.append(group(lean, x + 10, H - 40, rot=14))
    fc = mix(acc, P["glow"], 0.25)
    cover = sv.lin([(0, shade(fc, 0.3)), (1, shade(fc, -0.35))], 0, 0, 1, 1)
    out.append(rect(x + 110, H - 330, 200, 290, 12, cover))
    out.append(circle(x + 260, H - 270, 30, "#ffffff", fill_opacity="0.4"))
    out.append(path("M%s %sL%s %sL%s %sL%s %sZ" % (n(x + 110), n(H - 100), n(x + 170), n(H - 190), n(x + 230), n(H - 130), n(x + 310), n(H - 100)), shade(fc, -0.5), fill_opacity="0.7"))
    out.append(rect(x + 110, H - 330, 200, 290, 12, "none", stroke="#ffffff", stroke_opacity="0.25", stroke_width="2"))
    pm = Mat(sv, P.get("plank", "#3a3f4a"))
    out.append(rect(30, H - 42, W + 40, 26, 6, pm.front))
    out.append(line(36, H - 41, W + 64, H - 41, pm.edge, 2, stroke_opacity="0.6"))
    body = "".join(out)
    if rnd.random() < 0.5:
        body = '<g transform="matrix(-1 0 0 1 %s 0)">%s</g>' % (n(W + 100), body)
    return body, W + 100, H


def stream(sv, P):
    mon, mw, mh = monitor(sv, dict(P, screen="tiles"))
    out = [mon]
    cx, cy = mw - 40, 40
    for i, r in enumerate((40, 80, 120)):
        d = "M%s %sA%s %s 0 0 1 %s %s" % (n(cx - r * 0.7), n(cy + r * 0.7), n(r), n(r), n(cx + r * 0.7), n(cy + r * 0.7))
        out.append(path(d, "none", stroke=shade(P["acc"], 0.3), stroke_width="14", stroke_linecap="round", stroke_opacity=str(1 - i * 0.22), transform="rotate(-45 %s %s)" % (n(cx), n(cy))))
    out.append(circle(cx - 2, cy + 2, 12, shade(P["acc"], 0.3)))
    return group("".join(out), 0, 90), mw + 60, mh + 90


def server(sv, P):
    W = 460
    c = P.get("shell", "#2a2e36")
    m = Mat(sv, c)
    out = []
    for i in range(3):
        y = i * 120
        d = rrect_d(0, y, W, 100, 14)
        out.append(_body(sv, d, m, 8))
        for k in range(10):
            out.append(rect(30 + k * 18, y + 24, 10, 52, 3, m.dark))
        out.append(rect(240, y + 38, 120, 24, 6, m.deep))
        for k, col in enumerate([P["acc"], "#6ee7a8", "#ffd166"]):
            out.append(led(sv, 390 + k * 22, y + 50, 4, col if (i + k) % 2 == 0 else shade(col, -0.5)))
    return "".join(out), W, 360 + 10


def puzzle(sv, P):
    W = H = 400
    s = 280
    x0, y0 = 60, 80
    k = s * 0.18
    d = (
        "M%s %s" % (n(x0), n(y0))
        + "H%s" % n(x0 + s * 0.36)
        + "A%s %s 0 1 1 %s %s" % (n(k), n(k), n(x0 + s * 0.64), n(y0))
        + "H%s V%s" % (n(x0 + s), n(y0 + s * 0.36))
        + "A%s %s 0 1 1 %s %s" % (n(k), n(k), n(x0 + s), n(y0 + s * 0.64))
        + "V%s H%s" % (n(y0 + s), n(x0))
        + "Z"
    )
    col = P["acc"]
    out = [
        path(d, shade(col, -0.6), transform="translate(8 14)", fill_opacity="0.55"),
        path(d, sv.lin([(0, shade(col, 0.35)), (1, shade(col, -0.25))], 0, 0, 1, 1), stroke="#ffffff", stroke_opacity="0.35", stroke_width="3", stroke_linejoin="round"),
    ]
    return "".join(out), W + 40, H + 30


def apps(sv, P):
    W = H = 420
    out = []
    cols = [P["acc"], shade(P["acc"], 0.3), shade(P["glow"], -0.2), mix(P["acc"], "#ffd166", 0.4), mix(P["acc"], "#6ee7a8", 0.4)]
    for j in range(3):
        for i in range(3):
            col = cols[(i * 2 + j) % len(cols)]
            x, y = 20 + i * 136, 20 + j * 136
            g = sv.lin([(0, shade(col, 0.25)), (1, shade(col, -0.25))])
            out.append(rect(x + 4, y + 8, 110, 110, 28, "#000000", fill_opacity="0.35"))
            out.append(rect(x, y, 110, 110, 28, g))
            out.append(rect(x + 10, y + 8, 90, 34, 16, "#ffffff", fill_opacity="0.14"))
    return "".join(out), W, H


def plus(sv, P):
    W, H = 360, 440
    out = [rect(10, 10, W - 20, H - 20, 26, "#ffffff", fill_opacity="0.06")]
    out.append(rect(10, 10, W - 20, H - 20, 26, "none", stroke=shade(P["acc"], 0.3), stroke_width="6", stroke_dasharray="26 18", stroke_linecap="round"))
    cx, cy = W / 2, H / 2
    out.append(circle(cx, cy, 80, sv.lin([(0, shade(P["acc"], 0.3)), (1, shade(P["acc"], -0.25))])))
    out.append(path("M%s %sH%sM%s %sV%s" % (n(cx - 40), n(cy), n(cx + 40), n(cx), n(cy - 40), n(cy + 40)), "none", stroke="#ffffff", stroke_width="16", stroke_linecap="round"))
    return "".join(out), W, H


def cubes(sv, P):
    def cube_at(x, y, s, col):
        h = s * 0.5
        top = "M%s %sL%s %sL%s %sL%s %sZ" % (n(x), n(y - h), n(x + s), n(y), n(x), n(y + h), n(x - s), n(y))
        left = "M%s %sL%s %sL%s %sL%s %sZ" % (n(x - s), n(y), n(x), n(y + h), n(x), n(y + h + s * 1.1), n(x - s), n(y + s * 1.1))
        right = "M%s %sL%s %sL%s %sL%s %sZ" % (n(x + s), n(y), n(x), n(y + h), n(x), n(y + h + s * 1.1), n(x + s), n(y + s * 1.1))
        return path(left, shade(col, -0.2)) + path(right, shade(col, -0.42)) + path(top, shade(col, 0.22)) + path(top, "none", stroke="#ffffff", stroke_opacity="0.3", stroke_width="2")

    a, b = P["acc"], P.get("cube2", shade(P["glow"], -0.1))
    s = 100
    out = cube_at(150, 220, s, b) + cube_at(350, 220, s, a) + cube_at(250, 110, s, shade(a, 0.25))
    return out, 500, 440


def cartridge(sv, P):
    W, H = 400, 440
    col = P.get("shell", "#3a3f4a")
    m = Mat(sv, col)
    out = []
    # A disc peeking out behind.
    out.append(disc(sv, W * 0.72, H * 0.38, 150, P["acc"]))
    d = round_poly([(40, 60), (300, 60), (300, 400), (60, 400), (40, 380)], [18, 18, 10, 10, 10])
    out.append(_body(sv, d, m, 12))
    for i in range(8):
        out.append(rect(70 + i * 26, 74, 14, 46, 4, m.dark))
    out.append(rect(70, 140, 200, 180, 10, sv.lin([(0, shade(P["acc"], 0.35)), (1, shade(P["acc"], -0.3))], 0, 0, 1, 1)))
    out.append(star4(170, 215, 46, "#ffffff", "0.85"))
    out.append(rect(90, 280, 120, 10, 5, "#ffffff", fill_opacity="0.6"))
    out.append(rect(100, 350, 160, 26, 6, m.deep))
    return "".join(out), W + 40, H


DEVICES = {
    "console": console,
    "tower": tower,
    "cube": cube,
    "wedge": wedge,
    "disc_console": disc_console,
    "floppy_console": floppy_console,
    "visor": visor,
    "handheld_tall": handheld_tall,
    "calculator": calculator,
    "handheld_wide": handheld_wide,
    "handheld_slim": handheld_slim,
    "handheld_landscape": handheld_landscape,
    "hump": hump,
    "clamshell": clamshell,
    "hybrid": hybrid,
    "tablet": tablet,
    "computer": computer,
    "monitor": monitor,
    "arcade": arcade,
    "vector": vector,
    "gamepad": gamepad,
    "pixel_box": pixel_box,
    "board": board,
    "cards": cards,
    "star": star,
    "play": play,
    "drive": drive,
    "pc_combo": pc_combo,
    "shelf": shelf,
    "stream": stream,
    "server": server,
    "puzzle": puzzle,
    "apps": apps,
    "plus": plus,
    "cubes": cubes,
    "cartridge": cartridge,
}
