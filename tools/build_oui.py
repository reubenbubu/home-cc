#!/usr/bin/env python3
"""Build src-tauri/data/oui.bin, the table the app uses to tell the maker of a
device from its hardware (MAC) address. Needs only Python 3, no packages.

How to refresh the data (a few times a year is plenty):

  1. Download the four public IEEE lists into a new, empty folder:
       https://standards-oui.ieee.org/oui/oui.csv       (24-bit blocks, MA-L)
       https://standards-oui.ieee.org/oui28/mam.csv     (28-bit blocks, MA-M)
       https://standards-oui.ieee.org/oui36/oui36.csv   (36-bit blocks, MA-S)
       https://standards-oui.ieee.org/iab/iab.csv       (36-bit blocks, IAB)
  2. From the top of the repository run:
       python3 -I tools/build_oui.py <that folder> src-tauri/data/oui.bin
  3. Run the Rust tests (npm run ci:check). They check the table is sound and
     that the owner's Deco prefix 8C:90:2D is still listed as TP-Link. If a
     test names a maker that the IEEE has since renamed, update the test.
  4. Commit the new oui.bin. The app is built from the committed file, so
     builds need no network.

The same input gives the same output, except for the build date in the header.

usage: build_oui.py <dir with oui.csv mam.csv oui36.csv iab.csv> <out.bin>

File layout (all integers little-endian unless noted). src-tauri/src/net/oui.rs
reads it and refuses to compile against a file whose length does not match
its header:
  0   8  magic  b"HCOUI\\x01\\0\\0"
  8   4  n24    number of 24-bit (MA-L) records
  12  4  n28    number of 28-bit (MA-M) records
  16  4  n36    number of 36-bit (MA-S + IAB) records
  20  4  nnames number of distinct maker names
  24  4  blob_len
  28  4  build date as YYYYMMDD
  32     keys24   n24 * 3 bytes, big-endian prefix, sorted
         keys28   n28 * 4 bytes, big-endian (prefix << 4), sorted
         keys36   n36 * 5 bytes, big-endian (prefix << 4), sorted
         idx24    n24 * u16 name index
         idx28    n28 * u16
         idx36    n36 * u16
         offs     (nnames + 1) * u32 offsets into blob
         blob     UTF-8 names, concatenated
"""
import csv
import datetime
import os
import struct
import sys

if len(sys.argv) != 3:
    raise SystemExit(__doc__)
src, out = sys.argv[1], sys.argv[2]

# Names the registry uses where it does not say who the maker is.
PLACEHOLDERS = ("IEEE Registration Authority", "Private", "")


def clean(name):
    # Collapse runs of spaces, including the wide space some names contain.
    return " ".join(name.replace("　", " ").split())


def load(fn, hexlen):
    rows = {}
    dup = 0
    with open(os.path.join(src, fn), newline="", encoding="utf-8") as f:
        r = csv.reader(f)
        next(r)
        for row in r:
            if len(row) < 3:
                continue
            a = row[1].strip().upper()
            if len(a) != hexlen:
                raise SystemExit(f"{fn}: bad assignment {a!r}")
            name = clean(row[2])
            if a in rows:
                dup += 1
                # A few blocks are listed twice: keep a real name over a placeholder.
                if rows[a] not in PLACEHOLDERS:
                    continue
            rows[a] = name
    return rows, dup


l24, d24 = load("oui.csv", 6)
l28, d28 = load("mam.csv", 7)
l36, d36 = load("oui36.csv", 9)
iab, diab = load("iab.csv", 9)
for k, v in iab.items():
    l36.setdefault(k, v)

names = sorted(set(l24.values()) | set(l28.values()) | set(l36.values()))
if len(names) > 0xFFFF:
    raise SystemExit("too many names for u16 index")
nidx = {n: i for i, n in enumerate(names)}


def table(d, keybytes, shift):
    keys = bytearray()
    idx = bytearray()
    for a in sorted(d):
        keys += (int(a, 16) << shift).to_bytes(keybytes, "big")
        idx += struct.pack("<H", nidx[d[a]])
    return keys, idx


k24, i24 = table(l24, 3, 0)
k28, i28 = table(l28, 4, 4)
k36, i36 = table(l36, 5, 4)
blob = bytearray()
offs = bytearray()
for n in names:
    offs += struct.pack("<I", len(blob))
    blob += n.encode("utf-8")
offs += struct.pack("<I", len(blob))
date = int(datetime.date.today().strftime("%Y%m%d"))
hdr = b"HCOUI\x01\0\0" + struct.pack("<6I", len(l24), len(l28), len(l36), len(names), len(blob), date)
data = hdr + k24 + k28 + k36 + i24 + i28 + i36 + offs + blob
with open(out, "wb") as f:
    f.write(data)
print(
    f"24-bit {len(l24)} (dups {d24}), 28-bit {len(l28)} (dups {d28}), 36-bit {len(l36)} (dups {d36}+{diab}), "
    f"names {len(names)}, blob {len(blob)} bytes, total {len(data)} bytes"
)
