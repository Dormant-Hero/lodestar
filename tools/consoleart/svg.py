"""SVG building blocks for the console artwork: colours, gradients, shapes and the small
parts devices are made of (screens, buttons, d-pads, sticks, lights, discs)."""
import colorsys
import math


# ---------------------------------------------------------------------- colours


def hexc(r, g, b):
    c = lambda v: max(0, min(255, int(round(v * 255))))
    return "#%02x%02x%02x" % (c(r), c(g), c(b))


def rgb(c):
    c = c.lstrip("#")
    return tuple(int(c[i : i + 2], 16) / 255 for i in (0, 2, 4))


def hsl(h, s, l):
    r, g, b = colorsys.hls_to_rgb((h % 360) / 360, max(0, min(1, l)), max(0, min(1, s)))
    return hexc(r, g, b)


def mix(a, b, t):
    a, b = rgb(a), rgb(b)
    return hexc(*(x + (y - x) * t for x, y in zip(a, b)))


def shade(c, amt):
    """amt > 0 lightens towards white, < 0 darkens towards black."""
    return mix(c, "#ffffff", amt) if amt >= 0 else mix(c, "#000000", -amt)


def luma(c):
    r, g, b = rgb(c)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def n(v):
    """Compact number."""
    s = "%.1f" % v
    s = s.rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def attrs(kw):
    out = []
    for k, v in kw.items():
        if v is None:
            continue
        k = k.rstrip("_").replace("_", "-")
        if isinstance(v, float):
            v = n(v) if abs(v) >= 1 or v == 0 else ("%.3f" % v).rstrip("0").rstrip(".")
        out.append('%s="%s"' % (k, v))
    return (" " + " ".join(out)) if out else ""


# ---------------------------------------------------------------------- document


class Svg:
    """Collects <defs> (deduplicated gradients) for one file."""

    def __init__(self):
        self.defs = []
        self.keys = {}

    def _stops(self, stops):
        out = []
        for s in stops:
            off, col = s[0], s[1]
            op = s[2] if len(s) > 2 else 1
            o = "" if op >= 1 else ' stop-opacity="%s"' % ("%.3f" % op).rstrip("0").rstrip(".")
            out.append('<stop offset="%s" stop-color="%s"%s/>' % (n(off * 100) + "%", col, o))
        return "".join(out)

    def _add(self, key, make):
        if key not in self.keys:
            gid = "g%d" % len(self.keys)
            self.keys[key] = gid
            self.defs.append(make(gid))
        return "url(#%s)" % self.keys[key]

    def lin(self, stops, x1=0, y1=0, x2=0, y2=1, user=False):
        key = ("l", tuple(map(tuple, stops)), x1, y1, x2, y2, user)
        u = ' gradientUnits="userSpaceOnUse"' if user else ""
        return self._add(
            key,
            lambda gid: '<linearGradient id="%s" x1="%s" y1="%s" x2="%s" y2="%s"%s>%s</linearGradient>'
            % (gid, n(x1) if user else x1, n(y1) if user else y1, n(x2) if user else x2, n(y2) if user else y2, u, self._stops(stops)),
        )

    def rad(self, stops, cx=0.5, cy=0.5, r=0.5, user=False, fx=None, fy=None):
        key = ("r", tuple(map(tuple, stops)), cx, cy, r, user, fx, fy)
        u = ' gradientUnits="userSpaceOnUse"' if user else ""
        f = ""
        if fx is not None:
            f = ' fx="%s" fy="%s"' % (fx, fy)
        return self._add(
            key,
            lambda gid: '<radialGradient id="%s" cx="%s" cy="%s" r="%s"%s%s>%s</radialGradient>'
            % (gid, n(cx) if user else cx, n(cy) if user else cy, n(r) if user else r, f, u, self._stops(stops)),
        )

    def clip(self, d):
        """A clip path of the path data d (user space); returns the url() to use as clip-path."""
        return self._add(("c", d), lambda gid: '<clipPath id="%s"><path d="%s"/></clipPath>' % (gid, d))

    def doc(self, w, h, body, view=None):
        vb = view or (0, 0, w, h)
        defs = "<defs>%s</defs>" % "".join(self.defs) if self.defs else ""
        return '<svg xmlns="http://www.w3.org/2000/svg" width="%s" height="%s" viewBox="%s">%s%s</svg>\n' % (
            n(w),
            n(h),
            " ".join(n(v) for v in vb),
            defs,
            body,
        )


# ---------------------------------------------------------------------- shapes


def rect(x, y, w, h, r=0, fill="#fff", **kw):
    rr = min(r, w / 2, h / 2)
    return "<rect%s/>" % attrs(dict(x=n(x), y=n(y), width=n(w), height=n(h), rx=n(rr) if rr else None, fill=fill, **kw))


def circle(cx, cy, r, fill="#fff", **kw):
    return "<circle%s/>" % attrs(dict(cx=n(cx), cy=n(cy), r=n(r), fill=fill, **kw))


def ellipse(cx, cy, rx, ry, fill="#fff", **kw):
    return "<ellipse%s/>" % attrs(dict(cx=n(cx), cy=n(cy), rx=n(rx), ry=n(ry), fill=fill, **kw))


def path(d, fill="#fff", **kw):
    return "<path%s/>" % attrs(dict(d=d, fill=fill, **kw))


def line(x1, y1, x2, y2, stroke="#fff", width=2, **kw):
    return "<path%s/>" % attrs(
        dict(d="M%s %sL%s %s" % (n(x1), n(y1), n(x2), n(y2)), fill="none", stroke=stroke, stroke_width=n(width), **kw)
    )


def group(body, tx=0, ty=0, s=1, rot=None, **kw):
    t = []
    if tx or ty:
        t.append("translate(%s %s)" % (n(tx), n(ty)))
    if rot:
        t.append("rotate(%s)" % n(rot))
    if s != 1:
        t.append("scale(%s)" % ("%.4f" % s).rstrip("0").rstrip("."))
    return "<g%s>%s</g>" % (attrs(dict(transform=" ".join(t) if t else None, **kw)), body)


def circ_d(cx, cy, r):
    return "M%s %sa%s %s 0 1 0 %s 0a%s %s 0 1 0 %s 0Z" % (n(cx - r), n(cy), n(r), n(r), n(2 * r), n(r), n(r), n(-2 * r))


def round_poly(pts, radii, close=True):
    """Polygon with rounded corners (quadratic corners), radii per point."""
    k = len(pts)
    if isinstance(radii, (int, float)):
        radii = [radii] * k
    out = []
    for i in range(k):
        p = pts[i]
        a = pts[i - 1]
        b = pts[(i + 1) % k]
        r = radii[i]
        da = math.hypot(a[0] - p[0], a[1] - p[1]) or 1
        db = math.hypot(b[0] - p[0], b[1] - p[1]) or 1
        ta = min(r / da, 0.5)
        tb = min(r / db, 0.5)
        p1 = (p[0] + (a[0] - p[0]) * ta, p[1] + (a[1] - p[1]) * ta)
        p2 = (p[0] + (b[0] - p[0]) * tb, p[1] + (b[1] - p[1]) * tb)
        out.append(("L" if out else "M") + "%s %s" % (n(p1[0]), n(p1[1])))
        if r > 0:
            out.append("Q%s %s %s %s" % (n(p[0]), n(p[1]), n(p2[0]), n(p2[1])))
    return "".join(out) + ("Z" if close else "")


def rrect_d(x, y, w, h, r):
    """Rounded rectangle as path data; r may be one radius or [tl, tr, br, bl]."""
    if isinstance(r, (int, float)):
        r = [r] * 4
    return round_poly([(x, y), (x + w, y), (x + w, y + h), (x, y + h)], r)


# ---------------------------------------------------------------------- materials


class Mat:
    """Fills for a plastic shell of colour c (lit from above)."""

    def __init__(self, sv, c, gloss=0.0):
        self.c = c
        lt = luma(c)
        k = 0.55 if lt > 0.6 else 1.0
        self.front = sv.lin([(0, shade(c, 0.1 * k)), (0.55, c), (1, shade(c, -0.16 * k - 0.04))])
        self.top = sv.lin([(0, shade(c, 0.32 * k)), (1, shade(c, 0.14 * k))])
        self.side = sv.lin([(0, shade(c, -0.1)), (1, shade(c, -0.3))])
        self.edge = shade(c, 0.5 if lt < 0.5 else 0.7)
        self.line = shade(c, -0.28 if lt > 0.25 else -0.45)
        self.dark = shade(c, -0.42)
        self.deep = shade(c, -0.65)
        self.light = shade(c, 0.22)
        self.outline = shade(c, -0.55)
        self.gloss = gloss


def outline(d, m, w=2.0):
    return path(d, "none", stroke=m.outline, stroke_width=n(w), stroke_opacity="0.55", stroke_linejoin="round")


def highlight_rim(d, m, w=2.0, op=0.35):
    return path(d, "none", stroke=m.edge, stroke_width=n(w), stroke_opacity=str(op), stroke_linejoin="round")


def screen(sv, x, y, w, h, r, tint, glow=True, bezel=None, bezel_pad=0):
    """Dark glass with a soft inner glow and a diagonal reflection."""
    out = []
    if bezel:
        out.append(rect(x - bezel_pad, y - bezel_pad, w + 2 * bezel_pad, h + 2 * bezel_pad, r + bezel_pad * 0.6, bezel))
    base = sv.lin([(0, shade(tint, -0.78)), (1, shade(tint, -0.9))])
    out.append(rect(x, y, w, h, r, base))
    if glow:
        g = sv.rad([(0, tint, 0.55), (0.6, tint, 0.14), (1, tint, 0)], cx=0.42, cy=0.38, r=0.75)
        out.append(rect(x, y, w, h, r, g))
    # Reflection: a soft diagonal sheen over the upper-left part.
    refl = sv.lin([(0, "#ffffff", 0.16), (1, "#ffffff", 0)], 0, 0, 1, 1)
    d = "M%s %sL%s %sL%s %sL%s %sZ" % (
        n(x + r * 0.4), n(y + r * 0.4), n(x + w * 0.62), n(y + r * 0.4), n(x + w * 0.28), n(y + h - r * 0.4), n(x + r * 0.4), n(y + h - r * 0.4)
    )
    out.append(path(d, refl))
    out.append(rect(x + 0.5, y + 0.5, w - 1, h - 1, r, "none", stroke="#ffffff", stroke_opacity="0.10", stroke_width="1.5"))
    return "".join(out)


def led(sv, x, y, r, col):
    halo = sv.rad([(0, col, 0.55), (0.4, col, 0.18), (1, col, 0)])
    return circle(x, y, r * 3.4, halo) + circle(x, y, r, col) + circle(x - r * 0.3, y - r * 0.3, r * 0.38, "#ffffff", fill_opacity="0.7")


def button(sv, x, y, r, col, label_dot=False):
    g = sv.lin([(0, shade(col, 0.28)), (1, shade(col, -0.22))])
    out = [
        circle(x, y + r * 0.16, r * 1.08, shade(col, -0.62), fill_opacity="0.55"),
        circle(x, y, r, g),
        ellipse(x - r * 0.15, y - r * 0.42, r * 0.55, r * 0.3, "#ffffff", fill_opacity="0.28"),
    ]
    return "".join(out)


def pill(sv, cx, cy, w, h, col, rot=0):
    g = sv.lin([(0, shade(col, 0.25)), (1, shade(col, -0.25))])
    body = rect(-w / 2, -h / 2 + h * 0.15, w, h, h / 2, shade(col, -0.6), fill_opacity="0.5") + rect(-w / 2, -h / 2, w, h, h / 2, g)
    return group(body, cx, cy, rot=rot)


def dpad(sv, cx, cy, s, col):
    a = s / 3.0
    h = s / 2.0
    pts = [
        (-a / 2, -h), (a / 2, -h), (a / 2, -a / 2), (h, -a / 2), (h, a / 2), (a / 2, a / 2),
        (a / 2, h), (-a / 2, h), (-a / 2, a / 2), (-h, a / 2), (-h, -a / 2), (-a / 2, -a / 2),
    ]
    rr = [a * 0.22, a * 0.22, 0, a * 0.22, a * 0.22, 0, a * 0.22, a * 0.22, 0, a * 0.22, a * 0.22, 0]
    d = round_poly(pts, rr)
    g = sv.lin([(0, shade(col, 0.22)), (1, shade(col, -0.2))])
    body = (
        path(d, shade(col, -0.6), fill_opacity="0.55", transform="translate(0 %s)" % n(s * 0.05))
        + path(d, g)
        + circle(0, 0, a * 0.28, shade(col, -0.25))
        + path(d, "none", stroke="#ffffff", stroke_opacity="0.18", stroke_width=n(max(1, s * 0.02)))
    )
    return group(body, cx, cy)


def stick(sv, cx, cy, r, col):
    ring = shade(col, -0.55)
    cap = sv.rad([(0, shade(col, 0.3)), (0.7, col), (1, shade(col, -0.3))], cx=0.45, cy=0.38, r=0.62)
    return (
        circle(cx, cy, r * 1.25, ring, fill_opacity="0.9")
        + circle(cx, cy + r * 0.08, r, shade(col, -0.5))
        + circle(cx, cy, r * 0.92, cap)
        + circle(cx, cy, r * 0.55, "none", stroke=shade(col, -0.25), stroke_width=n(max(1, r * 0.1)), stroke_opacity="0.6")
    )


def slots(x, y, w, h, k, col, gap=None, vertical=True, op="0.75"):
    """k parallel slits filling a box."""
    out = []
    if vertical:
        step = w / k
        sw = gap or step * 0.42
        for i in range(k):
            out.append(rect(x + i * step + (step - sw) / 2, y, sw, h, sw / 2, col, fill_opacity=op))
    else:
        step = h / k
        sh = gap or step * 0.42
        for i in range(k):
            out.append(rect(x, y + i * step + (step - sh) / 2, w, sh, sh / 2, col, fill_opacity=op))
    return "".join(out)


def disc(sv, cx, cy, r, tint):
    """An optical disc with a soft rainbow sheen."""
    sheen = sv.lin(
        [
            (0, "#e9edf3"),
            (0.25, mix("#e9edf3", tint, 0.35)),
            (0.45, "#f7f2e8"),
            (0.6, mix("#cfe8f0", tint, 0.2)),
            (0.8, "#d9dce6"),
            (1, "#aeb4c2"),
        ],
        0, 0, 1, 1,
    )
    d = circ_d(cx, cy, r) + circ_d(cx, cy, r * 0.14)
    out = [
        path(d, sheen, fill_rule="evenodd"),
        path(circ_d(cx, cy, r * 0.36) + circ_d(cx, cy, r * 0.14), "#ffffff", fill_rule="evenodd", fill_opacity="0.35"),
        circle(cx, cy, r * 0.36, "none", stroke="#8c93a3", stroke_opacity="0.6", stroke_width=n(r * 0.012)),
        circle(cx, cy, r - r * 0.01, "none", stroke="#6f7686", stroke_opacity="0.6", stroke_width=n(r * 0.02)),
    ]
    # Two light streaks.
    hi = sv.lin([(0, "#ffffff", 0), (0.5, "#ffffff", 0.55), (1, "#ffffff", 0)], 0, 0, 1, 0)
    for ang in (-35, 145):
        out.append(group(rect(-r * 0.05, -r * 0.92, r * 0.1, r * 0.5, r * 0.05, hi), cx, cy, rot=ang))
    return "".join(out)


def floor_shadow(sv, cx, cy, rx, ry, op=0.55):
    g = sv.rad([(0, "#000000", op), (0.55, "#000000", op * 0.45), (1, "#000000", 0)])
    return ellipse(cx, cy, rx, ry, g)


def star4(cx, cy, r, fill="#fff", op="1"):
    """A four-pointed sparkle (Lodestar's star motif)."""
    k = r * 0.22
    d = "M%s %sQ%s %s %s %sQ%s %s %s %sQ%s %s %s %sQ%s %s %s %sZ" % (
        n(cx), n(cy - r),
        n(cx + k), n(cy - k), n(cx + r), n(cy),
        n(cx + k), n(cy + k), n(cx), n(cy + r),
        n(cx - k), n(cy + k), n(cx - r), n(cy),
        n(cx - k), n(cy - k), n(cx), n(cy - r),
    )
    return path(d, fill, fill_opacity=op)
