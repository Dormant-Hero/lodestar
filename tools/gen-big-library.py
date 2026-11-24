#!/usr/bin/env python3
"""Make a big synthetic Lodestar test home for performance checks.

    tools/gen-big-library.py BASE_HOME OUT_HOME [--games 5000] [--emulated 0.4]

BASE_HOME is a copy of a test home (e.g. ~/.cache/lodestar-launchtest copied to /tmp). Its
games are cloned (new ids, names, tags, play time, scores, years) until the library holds
--games games; a share of them become emulated games (several consoles). Every clone gets
its own artwork paths: the base game's images are copied once into OUT_HOME/data/bigart and
hard-linked per clone, so the image cache sees thousands of distinct files without the disk
filling up. Nothing outside OUT_HOME is written; the base images are only read.

Run Lodestar on it with --no-scan (a scan would treat the clones as gone from their store):

    LODESTAR_HOME=OUT_HOME ./target/release/lodestar --no-scan --screenshot x.png
"""
import argparse
import json
import os
import random
import shutil
import sqlite3
import sys

PLATFORMS = [
    "sony_playstation2", "sony_playstation", "nintendo_snes", "nintendo_gameboyadvance",
    "nintendo_ds", "nintendo_gamecube", "nintendo_wii", "sega_genesis", "nintendo_64",
    "sony_psp", "nintendo_switch", "sony_playstation3",
]
GENRES = ["Action", "Adventure", "RPG", "Strategy", "Puzzle", "Racing", "Sports",
          "Shooter", "Platformer", "Simulation", "Fighting", "Indie"]
DEVS = [f"Studio {c}" for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ"]
WORDS = ["Star", "Dark", "Legend", "Quest", "Night", "Iron", "Dragon", "Shadow", "Crystal",
         "River", "Storm", "Last", "Lost", "Hollow", "Ember", "Sky", "Tide", "Echo", "Rune"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("base")
    ap.add_argument("out")
    ap.add_argument("--games", type=int, default=5000)
    ap.add_argument("--emulated", type=float, default=0.4)
    a = ap.parse_args()
    if os.path.exists(a.out):
        sys.exit(f"{a.out} exists; pick a new folder")
    shutil.copytree(a.base, a.out, symlinks=True)
    db = sqlite3.connect(os.path.join(a.out, "data", "library.db"))
    db.row_factory = sqlite3.Row
    cols = [r[1] for r in db.execute("PRAGMA table_info(games)")]
    for col, decl in (("edits_json", "TEXT NOT NULL DEFAULT '{}'"), ("emu_json", "TEXT")):
        if col not in cols:  # an older test home: add what Lodestar's own upgrade would
            db.execute(f"ALTER TABLE games ADD COLUMN {col} {decl}")
            cols.append(col)
    base = [dict(r) for r in db.execute("SELECT * FROM games")]
    if not base:
        sys.exit("base library is empty")
    art_dir = os.path.join(a.out, "data", "bigart")
    os.makedirs(art_dir, exist_ok=True)

    # One local copy of every base image (read-only access to the originals).
    local = {}

    def localise(path):
        if not path or not os.path.isfile(path):
            return None
        if path not in local:
            dst = os.path.join(art_dir, f"base{len(local)}{os.path.splitext(path)[1]}")
            shutil.copyfile(path, dst)
            local[path] = dst
        return local[path]

    rnd = random.Random(42)
    next_id = (db.execute("SELECT max(id) FROM games").fetchone()[0] or 0) + 1
    n_new = max(0, a.games - len(base))
    rows = []
    for i in range(n_new):
        src = dict(base[i % len(base)])
        gid = next_id + i
        name = f"{rnd.choice(WORDS)} {rnd.choice(WORDS)} {i}"
        src["id"] = gid
        src["source_id"] = f"perf-{gid}"
        src["name"] = name
        src["sort_name"] = name.lower()
        src["user_name"] = None
        src["installed"] = 1 if rnd.random() < 0.6 else 0
        src["playtime_secs"] = rnd.choice([0, 0, 0, rnd.randint(60, 400000)])
        src["play_count"] = rnd.randint(0, 40) if src["playtime_secs"] else 0
        src["last_played"] = 1700000000 + rnd.randint(0, 60000000) if src["playtime_secs"] else None
        src["favorite"] = 1 if rnd.random() < 0.05 else 0
        src["hidden"] = 0
        src["user_rating"] = rnd.choice([None, None, None, 1, 2, 3, 4, 5])
        src["tags_json"] = json.dumps(rnd.sample(["co-op", "backlog", "finished", "retro", "kids", "vr"], rnd.randint(0, 2)))
        src["size_bytes"] = rnd.randint(10**6, 10**11)
        meta = {"genres": rnd.sample(GENRES, rnd.randint(1, 3)), "developer": rnd.choice(DEVS),
                "publisher": rnd.choice(DEVS), "release_year": rnd.randint(1985, 2026),
                "description": f"Synthetic test game number {i}. " * 8}
        src["meta_json"] = json.dumps(meta)
        src["fetched_meta_json"] = "{}"
        # Distinct art paths, hard-linked to the base images.
        art = json.loads(src.get("art_json") or "{}")
        for k in ("art_json", "user_art_json", "fetched_art_json"):
            for kind, p in json.loads(src.get(k) or "{}").items():
                art.setdefault(kind, p)
        new_art = {}
        for kind, p in art.items():
            if not isinstance(p, str):
                continue
            lp = localise(p)
            if lp:
                dst = os.path.join(art_dir, f"g{gid}-{kind}{os.path.splitext(lp)[1]}")
                os.link(lp, dst)
                new_art[kind] = dst
        src["art_json"] = json.dumps(new_art)
        src["user_art_json"] = "{}"
        src["fetched_art_json"] = "{}"
        if rnd.random() < a.emulated and "emu_json" in cols:
            plat = rnd.choice(PLATFORMS)
            src["source"] = "emulator"
            src["platform"] = plat
            src["launch_json"] = None
            src["install_dir"] = None
            src["installed"] = 1
            src["emu_json"] = json.dumps({"platform": plat, "folder": "perf", "rom": f"/nonexistent/perf/{gid}.iso",
                                          "files": [], "emulator": "", "profile": ""})
        rows.append(src)
    present = [c for c in cols if c in rows[0]] if rows else cols
    q = f"INSERT INTO games ({','.join(present)}) VALUES ({','.join('?' for _ in present)})"
    db.executemany(q, [[r.get(c) for c in present] for r in rows])
    db.commit()
    total = db.execute("SELECT count(*) FROM games").fetchone()[0]
    print(f"{a.out}: {total} games ({len(local)} distinct images, {n_new} clones)")


if __name__ == "__main__":
    main()
