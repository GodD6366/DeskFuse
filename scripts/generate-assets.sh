#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RESOURCE_DIR="$ROOT_DIR/resources"
GENERATED_DIR="$RESOURCE_DIR/generated"
ICONSET_DIR="$GENERATED_DIR/DeskFuse.iconset"

for command_name in sips python3; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "error: required command not found: $command_name" >&2
    exit 127
  fi
done

rm -rf "$ICONSET_DIR"
mkdir -p "$GENERATED_DIR" "$ICONSET_DIR"

python3 "$ROOT_DIR/scripts/export-center-icon.py"
sips -s format png "$RESOURCE_DIR/DeskFuse-icon.svg" --out "$GENERATED_DIR/DeskFuse-icon-1024.png" >/dev/null
# Render the dedicated monochrome template at native 2x (36px for 18pt).
# Never derive the menu bar glyph from the colored application artwork.
sips -s format png "$RESOURCE_DIR/deskfuse-status-icon.svg" --out "$GENERATED_DIR/deskfuse-status-icon.png" >/dev/null
sips -s format png "$RESOURCE_DIR/dmg-background.svg" --out "$GENERATED_DIR/dmg-background.png" >/dev/null

for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$GENERATED_DIR/DeskFuse-icon-1024.png" --out "$ICONSET_DIR/icon_${size}x${size}.png" >/dev/null
  double_size=$((size * 2))
  sips -z "$double_size" "$double_size" "$GENERATED_DIR/DeskFuse-icon-1024.png" --out "$ICONSET_DIR/icon_${size}x${size}@2x.png" >/dev/null
done

# Keep PNG alpha intact even for 16/32 pt representations.
python3 "$ROOT_DIR/scripts/pack-icon.py"
printf 'generated assets in %s\n' "$GENERATED_DIR"
