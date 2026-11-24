#!/usr/bin/env python3
"""Generate Lodestar's own console artwork (assets/consoles/).

    python3 -I tools/gen-console-art.py [--sheet OUT.svg]

For every console Lodestar knows (src/emulation/platforms.rs) and every built-in collection
(All Games, Favourites, stores …) four SVG pictures are drawn, all original work: a stylised
device illustration, the name set in Titillium Web (outlined: Lodestar's SVG renderer has no
text support) and a colour scheme of its own:

    card/<key>.svg    600 x 800  portrait card: illustration, name, maker and year
    hero/<key>.svg   1920 x 1080 wide background: illustration on the right, no text
    mark/<key>.svg   (wide)      the name as a wordmark (white, on transparent)
    device/<key>.svg  800 x 600  the illustration alone (on transparent)

They are also packed into assets/consoles/console-art.bin ("LSART1\n", an index of
"<path> <length>" lines, a blank line, then one raw-deflate stream of the files back to back in
index order), which Lodestar embeds
(src/theme/console_art.rs). --sheet writes a contact sheet of every card for checking.
Standard library only; the output is deterministic.
"""
import math
import os
import random
import re
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "consoleart"))

import catalog  # noqa: E402
from devices import DEVICES  # noqa: E402
from font import Font  # noqa: E402
from svg import Svg, circle, ellipse, floor_shadow, group, hsl, mix, n, path, rect, shade, star4  # noqa: E402

ROOT = os.path.dirname(HERE)
OUT = os.path.join(ROOT, "assets", "consoles")
FONTS = os.path.join(ROOT, "assets", "fonts", "titillium-web")
SEMI = Font(os.path.join(FONTS, "TitilliumWeb-SemiBold.ttf"))
BOLD = Font(os.path.join(FONTS, "TitilliumWeb-Bold.ttf"))
REG = Font(os.path.join(FONTS, "TitilliumWeb-Regular.ttf"))
KINDS = ("card", "hero", "mark", "device")


def platforms():
    """(id, name, short) of every platform in platforms.rs."""
    src = open(os.path.join(ROOT, "src", "emulation", "platforms.rs"), encoding="utf-8").read()
    return re.findall(r'plat!\("([^"]+)", "([^"]+)", "([^"]+)"', src)


def palette(e):
    h, s = e["hue"], e["sat"]
    return dict(
        bg0=hsl(h, s * 0.55, 0.075),
        bg1=hsl(h + 10, s * 0.62, 0.21),
        glow=hsl(h, s, 0.5),
        acc=hsl(h, min(1, s + 0.2), 0.64),
        light=hsl(h, min(1, s + 0.1), 0.8),
        tint=hsl(h, s, 0.58),
    )


def params(e, pal):
    P = dict(e["params"])
    P.setdefault("seed", e["key"])
    P.setdefault("acc", pal["acc"])
    P.setdefault("glow", pal["glow"])
    P.setdefault("tint", pal["tint"])
    P.setdefault("shell", shade(pal["glow"], -0.55))
    return P


def device(sv, e, pal, box):
    """The illustration fitted into box (x, y, w, h), centred, with a floor shadow."""
    body, w, h = DEVICES[e["dev"]](sv, params(e, pal))
    bx, by, bw, bh = box
    s = min(bw / w, bh / h)
    x = bx + (bw - w * s) / 2
    y = by + (bh - h * s) / 2
    sh = floor_shadow(sv, bx + bw / 2, y + h * s, w * s * 0.56, max(14, h * s * 0.07), 0.6)
    return sh + group(body, x, y, s)


def sparkles(e, x, y, w, h, k, scale=1.0):
    rnd = random.Random(e["key"])
    out = []
    for i in range(k):
        px, py = x + rnd.random() * w, y + rnd.random() * h
        if i % 4 == 0:
            out.append(star4(px, py, (6 + rnd.random() * 10) * scale, "#ffffff", "%.2f" % (0.25 + rnd.random() * 0.4)))
        else:
            out.append(circle(px, py, (1 + rnd.random() * 1.8) * scale, "#ffffff", fill_opacity="%.2f" % (0.15 + rnd.random() * 0.35)))
    return "".join(out)


def subline(e):
    parts = [e["sub"]] + ([e["year"]] if e["year"] else [])
    return " · ".join(p for p in parts if p).upper()


def fit_lines(font, text, max_w, sizes, max_lines=2):
    """(size, [lines]) of the biggest size at which text fits max_w in ≤ max_lines lines."""
    words = text.split()
    for size in sizes:
        if font.width(text, size) <= max_w:
            return size, [text]
        if max_lines >= 2 and len(words) > 1:
            best = None
            for i in range(1, len(words)):
                a, b = " ".join(words[:i]), " ".join(words[i:])
                wmax = max(font.width(a, size), font.width(b, size))
                if wmax <= max_w and (best is None or wmax < best[0]):
                    best = (wmax, [a, b])
            if best:
                return size, best[1]
    size = sizes[-1]
    return size, [text]


# ---------------------------------------------------------------------- the four pictures


def card(e):
    sv = Svg()
    pal = palette(e)
    W, H = 600, 800
    out = [rect(0, 0, W, H, 0, sv.lin([(0, pal["bg1"]), (0.7, mix(pal["bg1"], pal["bg0"], 0.75)), (1, pal["bg0"])]))]
    out.append(circle(300, 330, 330, sv.rad([(0, pal["glow"], 0.55), (0.5, pal["glow"], 0.16), (1, pal["glow"], 0)])))
    out.append(circle(300, 330, 236, "none", stroke="#ffffff", stroke_opacity="0.07", stroke_width="2"))
    out.append(circle(300, 330, 300, "none", stroke="#ffffff", stroke_opacity="0.04", stroke_width="2"))
    out.append(sparkles(e, 20, 20, 560, 420, 14))
    out.append(device(sv, e, pal, (56, 118, 488, 410)))
    out.append(rect(0, 520, W, 280, 0, sv.lin([(0, pal["bg0"], 0), (0.45, pal["bg0"], 0.8), (1, pal["bg0"], 0.96)])))
    size, lines = fit_lines(SEMI, e["title"], W - 96, list(range(64, 37, -2)))
    sub = subline(e)
    sub_size = 21
    while REG.width(sub, sub_size, 0.14) > W - 96 and sub_size > 14:
        sub_size -= 1
    base = 748
    out.append(path(REG.path(sub, 48, base, sub_size, 0.14), pal["light"], fill_opacity="0.9"))
    lh = size * 1.02
    y = base - sub_size - 16
    for ln in reversed(lines):
        out.append(path(SEMI.path(ln, 46, y, size), "#000000", fill_opacity="0.35", transform="translate(0 3)"))
        out.append(path(SEMI.path(ln, 46, y, size), "#f6f8fb"))
        y -= lh
    top = y + lh - SEMI.cap / SEMI.upem * size
    out.append(rect(48, top - 34, 44, 6, 3, pal["acc"]))
    out.append(star4(W - 52, 52, 13, "#ffffff", "0.55"))
    return sv.doc(W, H, "".join(out))


def hero(e, short):
    sv = Svg()
    pal = palette(e)
    W, H = 1920, 1080
    cx, cy = 1340, 540
    out = [rect(0, 0, W, H, 0, sv.lin([(0, pal["bg0"]), (0.5, mix(pal["bg0"], pal["bg1"], 0.55)), (1, pal["bg1"])], 0, 0, 1, 1))]
    out.append(circle(cx, cy, 760, sv.rad([(0, pal["glow"], 0.5), (0.45, pal["glow"], 0.15), (1, pal["glow"], 0)])))
    for r, op in ((380, "0.08"), (520, "0.055"), (680, "0.035")):
        out.append(circle(cx, cy, r, "none", stroke="#ffffff", stroke_opacity=op, stroke_width="2.5"))
    rnd = random.Random(e["key"] + "orbit")
    for r in (380, 520, 680):
        a = rnd.random() * 6.283
        out.append(circle(cx + r * math.cos(a), cy + r * math.sin(a), 5, pal["light"], fill_opacity="0.7"))
    out.append(sparkles(e, 60, 40, 1800, 600, 34, 1.6))
    if short:
        size = 420
        wdt = BOLD.width(short, size, -0.01)
        if wdt > 1500:
            size *= 1500 / wdt
        out.append(path(BOLD.path(short, 60, H + size * 0.08, size, -0.01), "#ffffff", fill_opacity="0.045"))
    out.append(device(sv, e, pal, (cx - 470, cy - 380, 940, 620)))
    out.append(rect(0, 0, 1000, H, 0, sv.lin([(0, pal["bg0"], 0.75), (1, pal["bg0"], 0)], 0, 0, 1, 0)))
    out.append(rect(0, H * 0.62, W, H * 0.38, 0, sv.lin([(0, pal["bg0"], 0), (1, pal["bg0"], 0.7)])))
    return sv.doc(W, H, "".join(out))


def mark(e):
    sv = Svg()
    pal = palette(e)
    size, lines = fit_lines(SEMI, e["title"], 1000, list(range(120, 71, -4)), 2)
    widths = [SEMI.width(ln, size) for ln in lines]
    sub = subline(e)
    sub_size = 34
    cap = SEMI.cap / SEMI.upem * size
    pad = 8
    y = pad + cap
    out = []
    for ln in lines:
        d = SEMI.path(ln, pad, y, size)
        out.append(path(d, "#000000", fill_opacity="0.3", transform="translate(0 4)"))
        out.append(path(d, "#f6f8fb"))
        y += size * 1.0
    sb = y - size + 24 + sub_size
    out.append(path(REG.path(sub, pad + 3, sb, sub_size, 0.16), pal["light"]))
    w = max(widths + [REG.width(sub, sub_size, 0.16)]) + 2 * pad + 4
    h = sb + 14
    return sv.doc(w, h, "".join(out))


def device_only(e):
    sv = Svg()
    pal = palette(e)
    return sv.doc(800, 600, device(sv, e, pal, (60, 50, 680, 480)))


# ---------------------------------------------------------------------- output


def main():
    args = sys.argv[1:]
    sheet = args[args.index("--sheet") + 1] if "--sheet" in args else None
    plats = platforms()
    by_key = {e["key"]: e for e in catalog.ALL}
    missing = [p for p, _, _ in plats if p not in by_key]
    if missing:
        sys.exit("no artwork defined for: " + ", ".join(missing))
    shorts = {p: s for p, _, s in plats}
    files = []
    for e in catalog.ALL:
        short = shorts.get(e["key"], "")
        if short == e["title"] or len(short) > 12:
            short = short if len(short) <= 12 else ""
        files.append(("card/%s.svg" % e["key"], card(e)))
        files.append(("hero/%s.svg" % e["key"], hero(e, short)))
        files.append(("mark/%s.svg" % e["key"], mark(e)))
        files.append(("device/%s.svg" % e["key"], device_only(e)))
    for k in KINDS:
        os.makedirs(os.path.join(OUT, k), exist_ok=True)
        keep = {f"{e['key']}.svg" for e in catalog.ALL}
        for f in os.listdir(os.path.join(OUT, k)):
            if f not in keep:
                os.remove(os.path.join(OUT, k, f))
    raw_total = 0
    for name, text in files:
        data = text.encode("utf-8")
        raw_total += len(data)
        p = os.path.join(OUT, name)
        old = open(p, "rb").read() if os.path.exists(p) else None
        if old != data:
            with open(p, "wb") as f:
                f.write(data)
    index = "".join("%s %d\n" % (name, len(text.encode("utf-8"))) for name, text in files) + "\n"
    c = zlib.compressobj(9, zlib.DEFLATED, -15, 9)
    packed = b"LSART1\n" + index.encode() + c.compress(b"".join(t.encode("utf-8") for _, t in files)) + c.flush()
    with open(os.path.join(OUT, "console-art.bin"), "wb") as f:
        f.write(packed)
    print("%d pictures, %d KiB of SVG, packed to %d KiB" % (len(files), raw_total // 1024, len(packed) // 1024))
    if sheet:
        cols = 10
        keys = [e["key"] for e in catalog.ALL]
        rows = (len(keys) + cols - 1) // cols
        cw, ch = 300, 400
        body = ['<rect width="100%" height="100%" fill="#111318"/>']
        for i, k in enumerate(keys):
            x, y = (i % cols) * (cw + 10) + 10, (i // cols) * (ch + 10) + 10
            body.append('<image x="%d" y="%d" width="%d" height="%d" href="%s"/>' % (x, y, cw, ch, os.path.join(OUT, "card", k + ".svg")))
        W, H = cols * (cw + 10) + 10, rows * (ch + 10) + 10
        with open(sheet, "w") as f:
            f.write('<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d">%s</svg>' % (W, H, "".join(body)))


if __name__ == "__main__":
    main()
