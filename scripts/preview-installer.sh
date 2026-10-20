#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREVIEW_DIR="$(mktemp -d "${TMPDIR:-/tmp}/deskfuse-installer-preview.XXXXXX")"
trap 'rm -rf "$PREVIEW_DIR"' EXIT
SOURCE_DIR="$ROOT_DIR/apps/deskfuse-macos/Sources/DeskFuse"
swiftc -swift-version 6 -parse-as-library \
  "$SOURCE_DIR/AppInstallation.swift" "$SOURCE_DIR/InstallationWindow.swift" \
  "$SOURCE_DIR/DesignSystem/DFTokens.swift" "$SOURCE_DIR/DesignSystem/DFInstallerStyle.swift" \
  "$ROOT_DIR/scripts/preview-installer.swift" -o "$PREVIEW_DIR/preview"
# One AppKit process per capture avoids shared layer-cache dirty-region snapshots.
for state in welcome installing installed welcome-en failed; do
  "$PREVIEW_DIR/preview" "$ROOT_DIR/resources/generated/DeskFuse-icon-1024.png" \
    "${1:-$ROOT_DIR/design/current/installer}" "$state"
done
