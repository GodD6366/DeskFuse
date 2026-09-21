#!/usr/bin/env python3
"""Compile the iconset with Apple's native ICNS encoder."""
from pathlib import Path
import struct
import subprocess

ROOT = Path(__file__).resolve().parent.parent
GENERATED = ROOT / "resources/generated"
# PNG-backed ICNS representations, including each macOS Retina variant.
# Format reference: Pillow's PIL.IcnsImagePlugin.IcnsFile.SIZES.
REPRESENTATIONS = [
    (b"icp4", 16, 1), (b"ic11", 16, 2),
    (b"icp5", 32, 1), (b"ic12", 32, 2),
    (b"ic07", 128, 1), (b"ic13", 128, 2),
    (b"ic08", 256, 1), (b"ic14", 256, 2),
    (b"ic09", 512, 1), (b"ic10", 512, 2),
]
for kind, size, scale in REPRESENTATIONS:
    suffix = "@2x" if scale == 2 else ""
    path = GENERATED / "DeskFuse.iconset" / f"icon_{size}x{size}{suffix}.png"
    png = path.read_bytes()
    if png[:8] != b"\x89PNG\r\n\x1a\n" or struct.unpack(">II", png[16:24]) != (size * scale,) * 2:
        raise SystemExit(f"error: invalid icon representation: {path}")
output = GENERATED / "DeskFuse.icns"
subprocess.run(["iconutil", "--convert", "icns", "--output", str(output),
                str(GENERATED / "DeskFuse.iconset")], check=True)
