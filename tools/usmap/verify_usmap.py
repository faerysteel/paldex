#!/usr/bin/env python3
"""Structurally validate a .usmap and report what it contains.

Walking the file end to end is the point: the name/enum/struct sections are
length-prefixed back to back, so if the cursor lands exactly on each section
boundary the preceding section must have been encoded correctly. A file that
merely *contains* the string "ZukanIndex" proves nothing -- that can be a
coincidental byte match inside compressed or misaligned data.

Exits non-zero if the file is malformed or a required name is missing.

Usage:
    verify_usmap.py <file.usmap> [--require NAME ...]
"""

from __future__ import annotations

import argparse
import hashlib
import struct
import sys

USMAP_MAGIC = 0x30C4

# Enums whose values are known independently of Palworld, so a correct decode
# is evidence the UEnum::Names() offset is right rather than merely plausible.
KNOWN_ENUMS = {
    "EInterpCurveMode": [
        "CIM_Linear",
        "CIM_CurveAuto",
        "CIM_Constant",
        "CIM_CurveUser",
        "CIM_CurveBreak",
        "CIM_CurveAutoClamped",
        "CIM_MAX",
    ],
}


class Malformed(Exception):
    pass


def parse(data: bytes) -> dict:
    if len(data) < 12:
        raise Malformed(f"file is {len(data)} bytes, too short for a 12-byte header")

    magic, version, compression, csize, dsize = struct.unpack_from("<HBBII", data, 0)
    if magic != USMAP_MAGIC:
        raise Malformed(f"bad magic 0x{magic:04X}, expected 0x{USMAP_MAGIC:04X}")
    if compression != 0:
        raise Malformed(f"compression method {compression} is not supported by this checker")
    if csize != dsize:
        raise Malformed(f"uncompressed file disagrees with itself: {csize} != {dsize}")
    if len(data) != dsize + 12:
        raise Malformed(f"header says {dsize} + 12 bytes, file is {len(data)}")

    off = 12
    (name_count,) = struct.unpack_from("<i", data, off)
    off += 4
    if name_count < 0:
        raise Malformed(f"negative name count {name_count}")

    names: list[str] = []
    for i in range(name_count):
        if off >= len(data):
            raise Malformed(f"ran off the end reading name {i}/{name_count}")
        length = data[off]
        off += 1
        names.append(data[off : off + length].decode("utf-8", "replace"))
        off += length

    (enum_count,) = struct.unpack_from("<I", data, off)
    off += 4
    enums: dict[str, list[str]] = {}
    total_entries = 0
    empty = 0
    for _ in range(enum_count):
        (name_idx,) = struct.unpack_from("<i", data, off)
        off += 4
        (count,) = struct.unpack_from("<i", data, off)
        off += 4
        if count < 0 or off + 4 * count > len(data):
            raise Malformed(f"enum '{names[name_idx]}' has impossible entry count {count}")
        values = [struct.unpack_from("<i", data, off + 4 * k)[0] for k in range(count)]
        off += 4 * count
        total_entries += count
        if count == 0:
            empty += 1
        enums[names[name_idx]] = [names[v] for v in values]

    (struct_count,) = struct.unpack_from("<I", data, off)
    off += 4

    return {
        "version": version,
        "size": len(data),
        "names": names,
        "enums": enums,
        "struct_count": struct_count,
        "enum_entries": total_entries,
        "empty_enums": empty,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("path")
    ap.add_argument("--require", nargs="*", default=["ZukanIndex", "ZukanIndexSuffix"])
    args = ap.parse_args()

    data = open(args.path, "rb").read()
    try:
        info = parse(data)
    except Malformed as e:
        print(f"MALFORMED: {e}", file=sys.stderr)
        return 1

    print(f"file           {args.path}")
    print(f"size           {info['size']:,} bytes")
    print(f"sha256         {hashlib.sha256(data).hexdigest()}")
    print(f"usmap version  {info['version']}")
    print(f"names          {len(info['names']):,}")
    print(f"enums          {len(info['enums']):,}  ({info['enum_entries']:,} entries, {info['empty_enums']} empty)")
    print(f"structs        {info['struct_count']:,}")

    failures: list[str] = []

    # An all-empty enum table is the signature of a wrong UEnum::Names() offset.
    # It still parses cleanly, so only a content check catches it.
    if info["enums"] and info["enum_entries"] == 0:
        failures.append("every enum is empty -- UEnum::Names() offset is almost certainly wrong")

    print("\nknown-value enum checks:")
    for name, expected in KNOWN_ENUMS.items():
        got = info["enums"].get(name)
        if got is None:
            print(f"  {name}: absent (skipped)")
        elif got == expected:
            print(f"  {name}: OK ({len(got)} values)")
        else:
            print(f"  {name}: MISMATCH\n    expected {expected}\n    got      {got}")
            failures.append(f"{name} decoded incorrectly")

    if "EPalElementType" in info["enums"]:
        print(f"  EPalElementType: {info['enums']['EPalElementType']}")

    print("\nrequired names:")
    for want in args.require:
        present = want in info["names"]
        print(f"  {want}: {'present' if present else 'MISSING'}")
        if not present:
            failures.append(f"required name {want!r} missing")

    if failures:
        print("\nFAILED:", file=sys.stderr)
        for f in failures:
            print(f"  - {f}", file=sys.stderr)
        return 1

    print("\nOK -- usmap is structurally valid and contains the required schema.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
