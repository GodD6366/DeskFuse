#!/usr/bin/env python3
"""Convert DeskFuse's existing app PNG into a multi-resolution Windows ICO.

No artwork is generated or redrawn. Uses Pillow for Lanczos downsampling; the
ICO stores each RGBA size as PNG so transparent edges remain intact. Windows
NotifyIcon uses the colored app mark, not the macOS black template, making it
legible on both light and dark system trays without theme guessing.
"""
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import struct

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "resources/generated/DeskFuse-icon-1024.png"
ASSETS = ROOT / "apps/deskfuse-windows/Assets"
SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)


def main() -> None:
    with Image.open(SOURCE) as source:
        source.load()
        if source.size != (1024, 1024):
            raise SystemExit("Expected the existing 1024 x 1024 app icon PNG")
        artwork = source.convert("RGBA")
    frames = []
    for size in SIZES:
        image = artwork.resize((size, size), Image.Resampling.LANCZOS)
        output = BytesIO()
        image.save(output, format="PNG", compress_level=9)
        frames.append(output.getvalue())
    offset = 6 + 16 * len(frames)
    directory = bytearray(struct.pack("<HHH", 0, 1, len(frames)))
    for size, png in zip(SIZES, frames):
        dimension = size if size < 256 else 0
        directory += struct.pack("<BBBBHHII", dimension, dimension, 0, 0, 1, 32, len(png), offset)
        offset += len(png)
    payload = bytes(directory) + b"".join(frames)
    # A format check, not a Windows runtime/visual test.
    with Image.open(BytesIO(payload)) as decoded:
        if decoded.format != "ICO" or decoded.ico.sizes() != {(size, size) for size in SIZES}:
            raise SystemExit("ICO directory or representations invalid")
        for size in SIZES:
            if decoded.ico.getimage((size, size)).size != (size, size):
                raise SystemExit("ICO representation failed decoding")
    ASSETS.mkdir(parents=True, exist_ok=True)
    icon = ASSETS / "DeskFuse.ico"
    temporary = icon.with_suffix(".ico.tmp")
    temporary.write_bytes(payload)
    temporary.replace(icon)
    manifest = {
        "source": str(SOURCE.relative_to(ROOT)),
        "source_sha256": sha256(SOURCE.read_bytes()).hexdigest(),
        "output": str(icon.relative_to(ROOT)),
        "output_sha256": sha256(payload).hexdigest(),
        "sizes": list(SIZES),
        "encoding": "PNG-backed 32-bit RGBA ICO",
        "tray": "Existing colored app mark; not the macOS monochrome template",
    }
    (ASSETS / "icons-manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    print(f"Generated {icon} ({len(payload)} bytes; {len(SIZES)} representations)")
    print(f"SHA-256 {manifest['output_sha256']}")


if __name__ == "__main__":
    main()
