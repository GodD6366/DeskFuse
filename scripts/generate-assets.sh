#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RESOURCE_DIR="$ROOT_DIR/resources"
GENERATED_DIR="$RESOURCE_DIR/generated"
ICONSET_DIR="$GENERATED_DIR/DeskFuse.iconset"

if ! command -v python3 >/dev/null 2>&1; then
  echo "error: required command not found: python3" >&2
  exit 127
fi

mkdir -p "$GENERATED_DIR"
ASSET_FINGERPRINT_FILE="$GENERATED_DIR/.assets-fingerprint"
ASSET_FINGERPRINT="$(python3 - "$ROOT_DIR" <<'PY'
import hashlib, pathlib, sys
root = pathlib.Path(sys.argv[1])
hasher = hashlib.sha256()
for relative in (
    "resources/DeskFuse-icon.svg",
    "resources/DeskFuse-icon-light.svg",
    "resources/deskfuse-status-icon.svg",
    "resources/dmg-background.svg",
    "scripts/export-center-icon.py",
    "scripts/pack-icon.py",
    "scripts/generate-assets.sh",
):
    hasher.update(relative.encode())
    hasher.update((root / relative).read_bytes())
print(hasher.hexdigest())
PY
)"
if [[ "${DESKFUSE_FORCE_ASSETS:-0}" != 1 \
  && -f "$ASSET_FINGERPRINT_FILE" \
  && "$(cat "$ASSET_FINGERPRINT_FILE")" == "$ASSET_FINGERPRINT" \
  && -s "$GENERATED_DIR/DeskFuse.icns" \
  && -s "$GENERATED_DIR/DeskFuse-icon-1024.png" \
  && -s "$GENERATED_DIR/DeskFuse-icon-light-1024.png" \
  && -s "$GENERATED_DIR/deskfuse-status-icon.png" \
  && -s "$GENERATED_DIR/dmg-background.tiff" \
  && -s "$RESOURCE_DIR/macos_center_icon.svg" ]]; then
  echo "asset cache hit"
  exit 0
fi

for required in sips tiffutil; do
  if ! command -v "$required" >/dev/null 2>&1; then
    echo "error: required command not found: $required" >&2
    exit 127
  fi
done

rm -rf "$ICONSET_DIR"
mkdir -p "$ICONSET_DIR"

python3 "$ROOT_DIR/scripts/export-center-icon.py"
sips -s format png "$RESOURCE_DIR/DeskFuse-icon.svg" --out "$GENERATED_DIR/DeskFuse-icon-1024.png" >/dev/null
sips -s format png "$RESOURCE_DIR/DeskFuse-icon-light.svg" --out "$GENERATED_DIR/DeskFuse-icon-light-1024.png" >/dev/null
# Render the dedicated monochrome template at native 2x (36px for 18pt).
# Never derive the menu bar glyph from the colored application artwork.
sips -s format png "$RESOURCE_DIR/deskfuse-status-icon.svg" --out "$GENERATED_DIR/deskfuse-status-icon.png" >/dev/null
# Finder reads a multi-representation TIFF and picks the 2x image on Retina
# displays; a single 1x PNG would be upscaled and look blurry.
sips -s format png "$RESOURCE_DIR/dmg-background.svg" --out "$GENERATED_DIR/dmg-background.png" >/dev/null
python3 - "$RESOURCE_DIR/dmg-background.svg" "$GENERATED_DIR/dmg-background@2x.svg" <<'PY2X'
import re, sys
from pathlib import Path
source = Path(sys.argv[1]).read_text()
# Double only the root intrinsic size; the viewBox keeps the 1x coordinates.
retina, count = re.subn(r'<svg([^>]*?) width="920" height="560"', r'<svg\1 width="1840" height="1120"', source, count=1)
if count != 1:
    raise SystemExit("error: dmg-background.svg must declare width=920 height=560")
Path(sys.argv[2]).write_text(retina)
PY2X
sips -s format png "$GENERATED_DIR/dmg-background@2x.svg" --out "$GENERATED_DIR/dmg-background@2x.png" >/dev/null
tiffutil -cathidpicheck "$GENERATED_DIR/dmg-background.png" "$GENERATED_DIR/dmg-background@2x.png" \
  -out "$GENERATED_DIR/dmg-background.tiff" >/dev/null

for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$GENERATED_DIR/DeskFuse-icon-light-1024.png" --out "$ICONSET_DIR/icon_${size}x${size}.png" >/dev/null
  double_size=$((size * 2))
  sips -z "$double_size" "$double_size" "$GENERATED_DIR/DeskFuse-icon-light-1024.png" --out "$ICONSET_DIR/icon_${size}x${size}@2x.png" >/dev/null
done

# Keep PNG alpha intact even for 16/32 pt representations.
python3 "$ROOT_DIR/scripts/pack-icon.py"
printf '%s\n' "$ASSET_FINGERPRINT" > "$ASSET_FINGERPRINT_FILE"
printf 'generated assets in %s\n' "$GENERATED_DIR"
