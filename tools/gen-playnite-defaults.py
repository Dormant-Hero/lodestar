#!/usr/bin/env python3
"""Build assets/playnite/default-theme.bin: Playnite's own default Desktop and Fullscreen theme
XAML (MIT licence, (c) Josef Nemec, github.com/JosefNemec/Playnite), which imported Playnite
themes inherit every file they don't ship from.

Usage: python3 -I tools/gen-playnite-defaults.py PLAYNITE_CHECKOUT [OUT]

PLAYNITE_CHECKOUT is a clone of github.com/JosefNemec/Playnite (a sparse checkout of
source/Playnite.DesktopApp/Themes and source/Playnite.FullscreenApp/Themes is enough).
Only the resource dictionaries the importer reads are kept (Constants, Media, Common, the derived
styles and the views; not the stock WPF control styles), with XML comments removed. Format: one
raw-deflate stream of `PNDT1\n` then, per file, `<mode>/<path>\t<bytes>\n<text>`.
"""
import os
import re
import sys
import zlib

ROOTS = {
    "desktop": "source/Playnite.DesktopApp/Themes/Desktop/Default",
    "fullscreen": "source/Playnite.FullscreenApp/Themes/Fullscreen/Default",
}
KEEP_DIRS = ("", "DerivedStyles", "Views")
EXTRA = {"desktop": ["DefaultControls/TextBlock.xaml"], "fullscreen": []}


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    src = sys.argv[1]
    here = os.path.dirname(os.path.abspath(__file__))
    out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "..", "assets", "playnite", "default-theme.bin")
    parts = [b"PNDT1\n"]
    raw = 0
    for mode, rel_root in ROOTS.items():
        root = os.path.join(src, rel_root)
        files = []
        for d in KEEP_DIRS:
            folder = os.path.join(root, d)
            for name in sorted(os.listdir(folder)):
                if name.lower().endswith(".xaml") and os.path.isfile(os.path.join(folder, name)):
                    files.append(f"{d}/{name}" if d else name)
        files += EXTRA[mode]
        for rel in files:
            with open(os.path.join(root, rel), encoding="utf-8-sig") as f:
                text = f.read()
            text = re.sub(r"<!--.*?-->", "", text, flags=re.S)
            text = re.sub(r"[ \t]+\n", "\n", text)
            text = re.sub(r"\n{3,}", "\n\n", text)
            data = text.encode("utf-8")
            raw += len(data)
            parts.append(f"{mode}/{rel}\t{len(data)}\n".encode("utf-8"))
            parts.append(data)
    blob = b"".join(parts)
    c = zlib.compressobj(9, zlib.DEFLATED, -15)
    packed = c.compress(blob) + c.flush()
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "wb") as f:
        f.write(packed)
    print(f"{out}: {len(packed)} bytes ({raw} bytes of XAML)")


if __name__ == "__main__":
    main()
