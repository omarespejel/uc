#!/usr/bin/env python3
"""Pack a Cairo corelib checkout into crates/uc-cli/src/embedded_corelib.bin.

Usage:
    scripts/pack_embedded_corelib.py [--corelib third_party/corelib] [--out crates/uc-cli/src/embedded_corelib.bin]

Format (read by `unpack_embedded_corelib` in crates/uc-cli/src/main.rs):
    u32 count, then `count` entries of: u32 path_len, path bytes, u32 data_len, data bytes.
The corelib version must match the `cairo-lang-*` crates pinned in Cargo.toml; the binary
extracts it under `<uc cache>/embedded-corelib-v<cairo version>-<pack digest>/`.
"""
import argparse
import os
import struct
import sys
from pathlib import Path

TOP_LEVEL = [".tool-versions", "Scarb.lock", "Scarb.toml", "cairo_project.toml"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--corelib", default="third_party/corelib")
    ap.add_argument("--out", default="crates/uc-cli/src/embedded_corelib.bin")
    args = ap.parse_args()
    root = Path(args.corelib)
    if not (root / "src" / "lib.cairo").is_file():
        sys.exit(f"{root} does not look like a corelib checkout (no src/lib.cairo)")
    entries = []
    for rel in TOP_LEVEL:
        p = root / rel
        if p.is_file():
            entries.append((rel, p.read_bytes()))
    for dirpath, _, files in os.walk(root / "src"):
        for name in sorted(files):
            p = Path(dirpath) / name
            entries.append((str(p.relative_to(root)), p.read_bytes()))
    entries.sort(key=lambda e: (not e[0].startswith("src/"), e[0]))
    blob = bytearray(struct.pack("<I", len(entries)))
    for rel, data in entries:
        rel_b = rel.encode()
        blob += struct.pack("<I", len(rel_b)) + rel_b + struct.pack("<I", len(data)) + data
    Path(args.out).write_bytes(blob)
    version = next((line.split('"')[1] for line in (root / "Scarb.toml").read_text().splitlines() if line.startswith("version")), "?")
    print(f"packed {len(entries)} files ({len(blob)} bytes) from corelib {version} into {args.out}")


if __name__ == "__main__":
    main()
