#!/usr/bin/env python3
"""Build assets/emulation/romdb.bin, Lodestar's embedded ROM hash database.

Source data: Playnite's emulation databases (`Emulation/Database/*.db`, SQLite, table
`DatGame(Id, Name, Region, ReleaseYear, Serial, RomCrc, RomName)`), which Playnite generated
from No-Intro / Redump / libretro DATs. Playnite is MIT-licensed (Copyright (c) Josef Nemec).

Run (read-only on the source; isolated interpreter):
    python3 -I scripts/gen-romdb.py <Playnite>/Emulation/Database assets/emulation/romdb.bin

File layout (all integers little-endian):
    b"LSROMDB1"
    u32  database count
    per database:  u16 name length, UTF-8 name ("Sony - PlayStation 2"),
                   u32 blob offset (from the start of the file), u32 compressed length,
                   u32 uncompressed length, u32 row count
    the blobs: raw deflate (no zlib header) of one database each

A database blob, inflated (rows sorted by name so neighbours compress well; n = row count):
    ceil(n / 8) bytes   CRC presence bitmap (bit i % 8 of byte i / 8 set = row i has a CRC)
    n * 4 bytes         CRC32 of each row, u32 little-endian (0 when absent)
    UTF-8 text          five columns separated by NUL, one value per row separated by '\\n':
                        name, region, year, serial, rom name
The rom name column is "" when it equals name + the database's most common suffix (stored in
the header line of the column as its first value: the column has rows + 1 values), "+<suffix>"
when it is name + another suffix, and the full name otherwise.
Lodestar builds its CRC / serial / rom-name indexes when a database is first used.
"""

import collections
import glob
import os
import sqlite3
import struct
import sys
import zlib


def clean(v):
    if v is None:
        return ""
    v = str(v)
    return v.replace("\x00", "").replace("\n", " ").replace("\r", " ").replace("\t", " ").strip()


def norm_crc(v):
    v = clean(v).lower()
    if len(v) > 8 and all(c in "0123456789abcdef" for c in v[:8]):
        v = v[:8]  # one broken row: "2adc33eaA"
    if len(v) == 8 and all(c in "0123456789abcdef" for c in v):
        return v
    return ""


def norm_year(v):
    v = clean(v)
    return v[:4] if len(v) >= 4 and v[:4].isdigit() else ""


def build(path):
    con = sqlite3.connect("file:" + path + "?mode=ro", uri=True)
    rows = []
    for name, region, year, serial, crc, rom in con.execute(
        "SELECT Name, Region, ReleaseYear, Serial, RomCrc, RomName FROM DatGame"
    ):
        name = clean(name)
        if not name:
            continue
        rows.append((name, clean(region), norm_year(year), clean(serial), norm_crc(crc), clean(rom)))
    con.close()
    rows = sorted(set(rows))
    suffixes = collections.Counter(r[5][len(r[0]):] for r in rows if r[5].startswith(r[0]))
    common = suffixes.most_common(1)[0][0] if suffixes else ""
    roms = [common]
    for r in rows:
        name, rom = r[0], r[5]
        if rom == name + common:
            roms.append("")
        elif rom.startswith(name):
            roms.append("+" + rom[len(name):])
        else:
            roms.append(rom)
    cols = [[r[i] for r in rows] for i in (0, 1, 2, 3)] + [roms]
    text = "\x00".join("\n".join(c) for c in cols)
    bitmap = bytearray((len(rows) + 7) // 8)
    crcs = bytearray()
    for i, r in enumerate(rows):
        if r[4]:
            bitmap[i // 8] |= 1 << (i % 8)
            crcs += struct.pack("<I", int(r[4], 16))
        else:
            crcs += b"\0\0\0\0"
    raw = bytes(bitmap) + bytes(crcs) + text.encode("utf-8")
    comp = zlib.compressobj(9, zlib.DEFLATED, -15, 9)
    blob = comp.compress(raw) + comp.flush()
    return len(rows), raw, blob


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: gen-romdb.py <Database dir> <out romdb.bin>")
    src, out = sys.argv[1], sys.argv[2]
    entries = []
    for path in sorted(glob.glob(os.path.join(src, "*.db"))):
        if os.path.getsize(path) == 0:
            continue
        name = os.path.basename(path)[:-3]
        count, raw, blob = build(path)
        if count:
            entries.append((name, count, raw, blob))
    header = bytearray(b"LSROMDB1" + struct.pack("<I", len(entries)))
    for name, *_ in entries:
        header += struct.pack("<H", len(name.encode())) + name.encode() + b"\0" * 16
    offset = len(header)
    pos = 12
    body = bytearray()
    for name, count, raw, blob in entries:
        pos += 2 + len(name.encode())
        struct.pack_into("<IIII", header, pos, offset + len(body), len(blob), len(raw), count)
        pos += 16
        body += blob
    data = bytes(header) + bytes(body)
    os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
    with open(out, "wb") as f:
        f.write(data)
    total = sum(e[1] for e in entries)
    print(f"{len(entries)} databases, {total} rows, {len(data)} bytes -> {out}")


if __name__ == "__main__":
    main()
