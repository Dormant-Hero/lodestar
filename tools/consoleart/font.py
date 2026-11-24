"""A small TrueType reader: glyph outlines (glyf), advances (hmtx), cmap and GPOS pair
kerning, enough to set a line of text as SVG paths. Standard library only.

Lodestar's SVG renderer (resvg) is built without text support, so the console artwork's
names are outlined here, in Titillium Web (SIL OFL 1.1, bundled in assets/fonts).
"""
import struct


class Font:
    def __init__(self, path):
        with open(path, "rb") as f:
            self.d = d = f.read()
        n = struct.unpack(">H", d[4:6])[0]
        self.t = {}
        for i in range(n):
            rec = d[12 + 16 * i : 28 + 16 * i]
            tag = rec[:4].decode("latin-1")
            off, ln = struct.unpack(">II", rec[8:16])
            self.t[tag] = off
        head = self.t["head"]
        self.upem = self.u16(head + 18)
        self.locfmt = self.i16(head + 50)
        self.nglyphs = self.u16(self.t["maxp"] + 4)
        hhea = self.t["hhea"]
        self.ascent = self.i16(hhea + 4)
        self.descent = self.i16(hhea + 6)
        nhm = self.u16(hhea + 34)
        hm = self.t["hmtx"]
        self.adv = [self.u16(hm + 4 * min(i, nhm - 1)) for i in range(self.nglyphs)]
        os2 = self.t.get("OS/2")
        self.cap = self.i16(os2 + 88) if os2 is not None and self.u16(os2) >= 2 else int(self.upem * 0.7)
        self.cmap = self._cmap()
        self.loca = self._loca()
        self.kern = self._gpos_kern()
        self._outline_cache = {}

    def u16(self, o):
        return struct.unpack(">H", self.d[o : o + 2])[0]

    def i16(self, o):
        return struct.unpack(">h", self.d[o : o + 2])[0]

    def u32(self, o):
        return struct.unpack(">I", self.d[o : o + 4])[0]

    # ------------------------------------------------------------------ tables

    def _cmap(self):
        base = self.t["cmap"]
        n = self.u16(base + 2)
        best = None
        for i in range(n):
            pid, eid, off = struct.unpack(">HHI", self.d[base + 4 + 8 * i : base + 12 + 8 * i])
            fmt = self.u16(base + off)
            if fmt == 4 and (pid, eid) in ((3, 1), (0, 3), (0, 4)) and best is None:
                best = base + off
        out = {}
        o = best
        segx2 = self.u16(o + 6)
        ends = o + 14
        starts = ends + segx2 + 2
        deltas = starts + segx2
        ranges = deltas + segx2
        for s in range(segx2 // 2):
            end = self.u16(ends + 2 * s)
            start = self.u16(starts + 2 * s)
            delta = self.i16(deltas + 2 * s)
            ro = self.u16(ranges + 2 * s)
            for c in range(start, end + 1):
                if c == 0xFFFF:
                    continue
                if ro == 0:
                    g = (c + delta) & 0xFFFF
                else:
                    a = ranges + 2 * s + ro + 2 * (c - start)
                    g = self.u16(a)
                    if g:
                        g = (g + delta) & 0xFFFF
                if g:
                    out[c] = g
        return out

    def _loca(self):
        o = self.t["loca"]
        if self.locfmt == 0:
            return [self.u16(o + 2 * i) * 2 for i in range(self.nglyphs + 1)]
        return [self.u32(o + 4 * i) for i in range(self.nglyphs + 1)]

    def _coverage(self, o):
        fmt = self.u16(o)
        cov = {}
        if fmt == 1:
            for i in range(self.u16(o + 2)):
                cov[self.u16(o + 4 + 2 * i)] = i
        else:
            for i in range(self.u16(o + 2)):
                s, e, idx = struct.unpack(">HHH", self.d[o + 4 + 6 * i : o + 10 + 6 * i])
                for g in range(s, e + 1):
                    cov[g] = idx + g - s
        return cov

    def _classdef(self, o):
        fmt = self.u16(o)
        cls = {}
        if fmt == 1:
            start = self.u16(o + 2)
            for i in range(self.u16(o + 4)):
                cls[start + i] = self.u16(o + 6 + 2 * i)
        else:
            for i in range(self.u16(o + 2)):
                s, e, c = struct.unpack(">HHH", self.d[o + 4 + 6 * i : o + 10 + 6 * i])
                for g in range(s, e + 1):
                    cls[g] = c
        return cls

    @staticmethod
    def _vsize(vf):
        return 2 * bin(vf & 0xFF).count("1")

    @staticmethod
    def _xadv_at(vf):
        """Offset of XAdvance inside a value record (None when absent)."""
        if not vf & 4:
            return None
        return 2 * bin(vf & 3).count("1")

    def _gpos_kern(self):
        """[(coverage, kind, data)] pair-adjustment subtables of the 'kern' feature."""
        base = self.t.get("GPOS")
        if base is None:
            return []
        fl = base + self.u16(base + 6)
        ll = base + self.u16(base + 8)
        lookups = set()
        for i in range(self.u16(fl)):
            tag = self.d[fl + 2 + 6 * i : fl + 6 + 6 * i]
            if tag == b"kern":
                fo = fl + self.u16(fl + 6 + 6 * i)
                for j in range(self.u16(fo + 2)):
                    lookups.add(self.u16(fo + 4 + 2 * j))
        subs = []
        for li in sorted(lookups):
            lo = ll + self.u16(ll + 2 + 2 * li)
            ltype = self.u16(lo)
            for si in range(self.u16(lo + 4)):
                so = lo + self.u16(lo + 6 + 2 * si)
                t = ltype
                if t == 9:
                    t = self.u16(so + 2)
                    so = so + self.u32(so + 4)
                if t != 2:
                    continue
                fmt = self.u16(so)
                cov = self._coverage(so + self.u16(so + 2))
                vf1, vf2 = self.u16(so + 4), self.u16(so + 6)
                xa = self._xadv_at(vf1)
                rec = self._vsize(vf1) + self._vsize(vf2)
                if fmt == 1:
                    pairs = {}
                    for g, ci in cov.items():
                        ps = so + self.u16(so + 10 + 2 * ci)
                        m = {}
                        for k in range(self.u16(ps)):
                            r = ps + 2 + k * (2 + rec)
                            second = self.u16(r)
                            m[second] = self.i16(r + 2 + xa) if xa is not None else 0
                        pairs[g] = m
                    subs.append((cov, 1, pairs))
                elif fmt == 2:
                    c1 = self._classdef(so + self.u16(so + 8))
                    c2 = self._classdef(so + self.u16(so + 10))
                    n1, n2 = self.u16(so + 12), self.u16(so + 14)
                    rows = so + 16
                    subs.append((cov, 2, (c1, c2, n2, rows, rec, xa)))
        return subs

    def kerning(self, a, b):
        for cov, kind, data in self.kern:
            if a not in cov:
                continue
            if kind == 1:
                m = data.get(a, {})
                if b in m:
                    return m[b]
                continue
            c1, c2, n2, rows, rec, xa = data
            if xa is None:
                return 0
            k1, k2 = c1.get(a, 0), c2.get(b, 0)
            return self.i16(rows + (k1 * n2 + k2) * rec + xa)
        return 0

    # ------------------------------------------------------------------ outlines

    def glyph(self, ch):
        return self.cmap.get(ord(ch), 0)

    def _contours(self, g, depth=0):
        """Contours of glyph g in font units: [[(x, y, on_curve) …] …]."""
        o0, o1 = self.loca[g], self.loca[g + 1]
        if o1 <= o0 or depth > 8:
            return []
        o = self.t["glyf"] + o0
        nc = self.i16(o)
        if nc >= 0:
            ends = [self.u16(o + 10 + 2 * i) for i in range(nc)]
            npts = ends[-1] + 1 if ends else 0
            p = o + 10 + 2 * nc
            p += 2 + self.u16(p)
            flags = []
            while len(flags) < npts:
                f = self.d[p]
                p += 1
                flags.append(f)
                if f & 8:
                    r = self.d[p]
                    p += 1
                    flags.extend([f] * r)
            xs, ys = [], []
            v = 0
            for f in flags:
                if f & 2:
                    dv = self.d[p]
                    p += 1
                    v += dv if f & 16 else -dv
                elif not f & 16:
                    v += self.i16(p)
                    p += 2
                xs.append(v)
            v = 0
            for f in flags:
                if f & 4:
                    dv = self.d[p]
                    p += 1
                    v += dv if f & 32 else -dv
                elif not f & 32:
                    v += self.i16(p)
                    p += 2
                ys.append(v)
            out, s = [], 0
            for e in ends:
                out.append([(xs[i], ys[i], bool(flags[i] & 1)) for i in range(s, e + 1)])
                s = e + 1
            return out
        # Composite glyph.
        out = []
        p = o + 10
        while True:
            flags, gi = self.u16(p), self.u16(p + 2)
            p += 4
            if flags & 1:
                a1, a2 = self.i16(p), self.i16(p + 2)
                p += 4
            else:
                a1, a2 = struct.unpack(">bb", self.d[p : p + 2])
                p += 2
            m = [1.0, 0.0, 0.0, 1.0]
            f2 = lambda q: self.i16(q) / 16384.0
            if flags & 8:
                m = [f2(p), 0.0, 0.0, f2(p)]
                p += 2
            elif flags & 0x40:
                m = [f2(p), 0.0, 0.0, f2(p + 2)]
                p += 4
            elif flags & 0x80:
                m = [f2(p), f2(p + 2), f2(p + 4), f2(p + 6)]
                p += 8
            dx, dy = (a1, a2) if flags & 2 else (0, 0)
            for c in self._contours(gi, depth + 1):
                out.append([(m[0] * x + m[2] * y + dx, m[1] * x + m[3] * y + dy, on) for x, y, on in c])
            if not flags & 0x20:
                break
        return out

    def contours(self, g):
        if g not in self._outline_cache:
            self._outline_cache[g] = self._contours(g)
        return self._outline_cache[g]

    # ------------------------------------------------------------------ text

    def layout(self, text, size, spacing=0.0):
        """[(glyph, x)] in pixels for `text` at `size` px (letter `spacing` in ems) and the width."""
        s = size / self.upem
        x = 0.0
        out = []
        prev = None
        for ch in text:
            g = self.glyph(ch)
            if prev is not None:
                x += self.kerning(prev, g) * s
            out.append((g, x))
            x += self.adv[g] * s + spacing * size
            prev = g
        if text:
            x -= spacing * size
        return out, x

    def width(self, text, size, spacing=0.0):
        return self.layout(text, size, spacing)[1]

    def path(self, text, x0, baseline, size, spacing=0.0):
        """SVG path data of `text` with its left edge at x0 on `baseline`."""
        s = size / self.upem
        glyphs, _ = self.layout(text, size, spacing)
        parts = []
        f = lambda v: ("%.1f" % v).rstrip("0").rstrip(".")
        for g, gx in glyphs:
            for c in self.contours(g):
                pts = [(x0 + gx + x * s, baseline - y * s, on) for x, y, on in c]
                parts.append(_contour_path(pts, f))
        return "".join(parts)


def _contour_path(pts, f):
    n = len(pts)
    if n == 0:
        return ""
    # Start on an on-curve point (or the midpoint of two off-curve ones).
    start = next((i for i, p in enumerate(pts) if p[2]), None)
    if start is None:
        a, b = pts[0], pts[1 % n]
        first = ((a[0] + b[0]) / 2, (a[1] + b[1]) / 2)
        seq = pts
    else:
        first = pts[start][:2]
        seq = pts[start + 1 :] + pts[: start + 1]
    out = ["M%s %s" % (f(first[0]), f(first[1]))]
    ctrl = None
    for x, y, on in seq:
        if on:
            if ctrl is None:
                out.append("L%s %s" % (f(x), f(y)))
            else:
                out.append("Q%s %s %s %s" % (f(ctrl[0]), f(ctrl[1]), f(x), f(y)))
                ctrl = None
        else:
            if ctrl is not None:
                mx, my = (ctrl[0] + x) / 2, (ctrl[1] + y) / 2
                out.append("Q%s %s %s %s" % (f(ctrl[0]), f(ctrl[1]), f(mx), f(my)))
            ctrl = (x, y)
    if ctrl is not None:
        out.append("Q%s %s %s %s" % (f(ctrl[0]), f(ctrl[1]), f(first[0]), f(first[1])))
    out.append("Z")
    return "".join(out)
