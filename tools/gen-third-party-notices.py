#!/usr/bin/env python3
"""Regenerate THIRD_PARTY_NOTICES.md (repo root) from Cargo.lock and tools/third-party/.

Run from anywhere, offline (only `cargo metadata` / `cargo tree --offline` are used; every crate is already in
~/.cargo/registry after a build):

    python3 -I tools/gen-third-party-notices.py

What it writes:
  * tools/third-party/intro.md verbatim (fonts, built-in data, Playnite / ES-DE notices, services);
  * every Rust library in the Linux release build, grouped by licence, with the copyright lines
    found in its licence files (else its authors) — "Built into Lodestar" and "Used only while
    building";
  * the full text of each licence once (tools/third-party/licenses/<SPDX id>.txt; a missing text
    stops the script, so a new licence is never left out);
  * extra notices some libraries ship (NOTICE files, font licences, SDL_GameControllerDB, ring,
    SQLite's public-domain blessing);
  * the dependency graph comes from `cargo tree` (enabled features only), package details
    (licence, authors, files) from `cargo metadata`;
  * a last comment line listing every Cargo.lock package and why it is (or isn't) shipped; the
    test `credits::tests::notices_cover_cargo_lock` compares it with Cargo.lock.
"""

import json
import os
import re
import subprocess
import sys
import textwrap
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HERE = os.path.join(ROOT, "tools", "third-party")
TARGET = "x86_64-unknown-linux-gnu"
OUT = os.path.join(ROOT, "THIRD_PARTY_NOTICES.md")

# Preferred order of the alternatives of an "A OR B" licence (only for sorting the groups).
PREF = ["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "Zlib", "BSD-2-Clause",
        "BSD-3-Clause", "ISC", "0BSD", "Unlicense", "GPL-2.0-only"]
# Copyleft alternatives Lodestar does not take (the other alternative is used instead).
NOT_TAKEN = {"GPL-2.0-only"}

# Files shipped by some libraries that must be reproduced as they are (crate name, relative path,
# heading). Their content is printed verbatim under "Extra notices".
EXTRA = [
    ("epaint_default_fonts", "fonts/Hack-Regular.txt", "Hack font (egui's monospace font)"),
    ("epaint_default_fonts", "fonts/emoji-icon-font-mit-license.txt", "emoji-icon-font (egui's icon glyphs)"),
    ("epaint_default_fonts", "fonts/OFL.txt", "Noto Emoji (egui's emoji font): SIL Open Font License 1.1"),
    ("epaint_default_fonts", "fonts/UFL.txt", "Ubuntu Light (egui's text font): Ubuntu Font Licence 1.0"),
    ("gilrs", "SDL_GameControllerDB/LICENSE", "SDL_GameControllerDB (controller mappings built in by gilrs)"),
    ("ring", "LICENSE", "ring"),
    ("ring", "LICENSE-other-bits", "ring (other parts)"),
    ("ring", "LICENSE-BoringSSL", "ring (code from BoringSSL)"),
]
SQLITE_NOTE = """SQLite (built in by libsqlite3-sys, `bundled`) is in the public domain. From its source:

    The author disclaims copyright to this source code.  In place of
    a legal notice, here is a blessing:

       May you do good and not evil.
       May you find forgiveness for yourself and forgive others.
       May you share freely, never taking more than you give.
"""

LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|copyright|notice|unlicense)", re.I)
NOTICE_FILE = re.compile(r"^notice", re.I)


def norm_expr(expr):
    """'MIT/Apache-2.0' → 'MIT OR Apache-2.0'; the alternatives of a plain OR in PREF order."""
    e = (expr or "").strip()
    if not e:
        return "Unknown"
    e = re.sub(r"\s*/\s*", " OR ", e)
    if "(" in e or " AND " in e:
        return e
    parts = [p.strip() for p in e.split(" OR ")]
    parts = sorted(set(parts), key=lambda p: (PREF.index(p) if p in PREF else len(PREF), p))
    return " OR ".join(parts)


def ids_of(expr):
    """SPDX ids whose text the notices need (copyleft alternatives that aren't taken left out)."""
    out = []
    for tok in re.findall(r"[A-Za-z0-9.+-]+", expr):
        if tok in ("AND", "OR", "WITH"):
            continue
        if tok in NOT_TAKEN and " OR " in expr:
            continue
        if tok not in out:
            out.append(tok)
    return out


def copyright_lines(manifest_dir):
    lines = []
    try:
        names = sorted(os.listdir(manifest_dir))
    except OSError:
        return lines
    for n in names:
        p = os.path.join(manifest_dir, n)
        if not (LICENSE_FILE.match(n) and os.path.isfile(p)) or "gpl" in n.lower():
            continue  # (a GPL text is an alternative Lodestar doesn't take; its lines are the FSF's)
        try:
            text = open(p, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        rows = text.splitlines()
        at = -1
        while at + 1 < len(rows):
            at += 1
            l = rows[at].strip().lstrip("#*/ ").strip()
            while (l.endswith(" or") or l.endswith(",")) and at + 1 < len(rows) and rows[at + 1].strip():
                at += 1
                l += " " + rows[at].strip().lstrip("#*/ ").strip()  # wrapped line
            low = l.lower()
            if not (low.startswith("copyright") or l.startswith("©")):
                continue
            if any(t in low for t in ("notice", "[", "<year", "{", "holder", "owner", "license", "licence",
                                      "copyright and", "copyright (c) <", "permission")):
                # templates ("Copyright [yyyy] [name of copyright owner]") and legal prose
                if not re.search(r"(19|20)\d\d", l) or "[" in l or "{" in l:
                    continue
            if len(l) > 300 or len(l) < 12 or "free software foundation" in low:
                continue
            l = re.sub(r"\s+", " ", l)
            if not any(l in x for x in lines):
                lines.append(l)
    return lines[:6]


def notice_files(manifest_dir):
    out = []
    try:
        for n in sorted(os.listdir(manifest_dir)):
            p = os.path.join(manifest_dir, n)
            if NOTICE_FILE.match(n) and os.path.isfile(p):
                out.append((n, open(p, encoding="utf-8", errors="replace").read().strip()))
    except OSError:
        pass
    return out


def main():
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--offline", "--locked",
         "--filter-platform", TARGET],
        cwd=ROOT, check=True, capture_output=True).stdout)
    pkgs = {p["id"]: p for p in meta["packages"]}
    root = meta["resolve"]["root"]

    # `cargo metadata` lists optional dependencies even when their feature is off, so the
    # dependency graph itself comes from `cargo tree`, which follows the enabled features.
    by_key = {(p["name"], p["version"]): pid for pid, p in pkgs.items()}

    def tree(edges):
        r = subprocess.run(
            ["cargo", "tree", "--offline", "--locked", "--target", TARGET, "-e", edges,
             "--prefix", "none", "--format", "{p}"],
            cwd=ROOT, check=True, capture_output=True, text=True).stdout
        out = set()
        for line in r.splitlines():
            w = line.split()
            if len(w) >= 2 and w[1].startswith("v"):
                out.add(by_key[(w[0], w[1][1:])])
        out.discard(root)
        return out

    shipped = tree("normal,no-proc-macro")
    build = tree("normal,build") - shipped

    def entry(pid):
        p = pkgs[pid]
        d = os.path.dirname(p["manifest_path"])
        cr = copyright_lines(d)
        if not cr:
            authors = [re.sub(r"\s*<[^>]*>", "", a).strip() for a in p.get("authors") or []]
            authors = [a for a in authors if a]
            cr = ["Copyright (c) " + (", ".join(authors) if authors else f"the {p['name']} developers")]
        return {
            "name": p["name"], "version": p["version"], "expr": norm_expr(p.get("license")),
            "repo": p.get("repository") or p.get("homepage") or f"https://crates.io/crates/{p['name']}",
            "copyright": cr, "dir": d, "notices": notice_files(d),
        }

    ship = sorted((entry(i) for i in shipped), key=lambda e: (e["name"], e["version"]))
    bld = sorted((entry(i) for i in build), key=lambda e: (e["name"], e["version"]))
    by_name = {}
    for e in ship + bld:
        by_name.setdefault(e["name"], e)

    needed = []
    for e in ship + bld:
        for i in ids_of(e["expr"]):
            if i not in needed:
                needed.append(i)
    needed += [i for i in ("MIT", "Bitstream-Vera", "OFL-1.1", "Ubuntu-font-1.0") if i not in needed]
    texts = {}
    missing = []
    for i in needed:
        f = os.path.join(HERE, "licenses", f"{i}.txt")
        if os.path.isfile(f):
            texts[i] = open(f, encoding="utf-8").read().strip()
        else:
            missing.append(i)
    if missing:
        sys.exit(f"No licence text for {', '.join(missing)}: add tools/third-party/licenses/<id>.txt")

    out = [open(os.path.join(HERE, "intro.md"), encoding="utf-8").read().rstrip(), ""]

    def section(title, entries, blurb):
        out.append(f"## {title}")
        out.append("")
        out.extend(textwrap.wrap(blurb, 92, break_on_hyphens=False))
        out.append("")
        groups = {}
        for e in entries:
            groups.setdefault(e["expr"], []).append(e)
        order = sorted(groups, key=lambda k: (-len(groups[k]), k))
        for expr in order:
            out.append(f"### {expr} ({len(groups[expr])})")
            out.append("")
            for e in groups[expr]:
                cr = "; ".join(e["copyright"])
                out.append(f"- **{e['name']}** {e['version']} — {cr} — {e['repo']}")
            out.append("")

    section("Rust libraries built into Lodestar", ship,
            f"{len(ship)} libraries, grouped by licence. Where a library offers a choice (\"A OR B\"), "
            "any of the offered licences may be used; Lodestar takes the permissive ones (for "
            "self_cell: Apache-2.0, not GPL-2.0).")
    section("Rust libraries used only while building", bld,
            f"{len(bld)} build tools and code generators. They run on the build machine and are not "
            "part of the program; listed for completeness.")

    out.append("## Licence texts")
    out.append("")
    out.extend(textwrap.wrap("Each licence once. Where a text starts with a copyright placeholder, the "
                             "copyright lines are the ones listed with each library above.", 92))
    out.append("")
    for i in needed:
        out.append(f"### {i}")
        out.append("")
        out.append("```")
        out.append(texts[i])
        out.append("```")
        out.append("")

    out.append("## Extra notices")
    out.append("")
    for crate, rel, heading in EXTRA:
        e = by_name.get(crate)
        if not e:
            continue
        p = os.path.join(e["dir"], rel)
        if not os.path.isfile(p):
            sys.exit(f"{crate}: {rel} not found")
        out.append(f"### {heading}")
        out.append("")
        out.append("```")
        out.append(open(p, encoding="utf-8", errors="replace").read().strip())
        out.append("```")
        out.append("")
    if "libsqlite3-sys" in by_name:
        out.append("### SQLite")
        out.append("")
        out.append(SQLITE_NOTE.rstrip())
        out.append("")
    for e in ship + bld:
        for n, text in e["notices"]:
            out.append(f"### {e['name']} {e['version']}: {n}")
            out.append("")
            out.append("```")
            out.append(text)
            out.append("```")
            out.append("")

    # Every Cargo.lock package and what it is: s = built in, b = build only, o = not in the Linux
    # release build (other platforms, tests).
    lock = tomllib.load(open(os.path.join(ROOT, "Cargo.lock"), "rb"))
    ship_keys = {(e["name"], e["version"]) for e in ship}
    bld_keys = {(e["name"], e["version"]) for e in bld}
    rootname = pkgs[root]["name"]
    marks = []
    for p in sorted(lock["package"], key=lambda p: (p["name"], p["version"])):
        k = (p["name"], p["version"])
        if p["name"] == rootname and "source" not in p:
            continue
        m = "s" if k in ship_keys else "b" if k in bld_keys else "o"
        marks.append(f"{m}:{p['name']}@{p['version']}")
    out.append("<!-- cargo-lock (s = built in, b = build only, o = not in the Linux release build):")
    row = ""
    for m in marks:
        if row and len(row) + len(m) > 110:
            out.append(row)
            row = ""
        row = f"{row} {m}" if row else m
    if row:
        out.append(row)
    out.append("-->")
    out.append("")

    text = "\n".join(out)
    text = re.sub(r"[ \t]+\n", "\n", text)
    with open(OUT, "w", encoding="utf-8") as f:
        f.write(text)
    print(f"{OUT}: {len(ship)} built in, {len(bld)} build-only, {len(needed)} licence texts, "
          f"{len(text) // 1024} KB")


if __name__ == "__main__":
    main()
