#!/usr/bin/env bash
# Isolated P0 tooling. Never launches or replaces the shipping uniDesk app.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${DEVELOPER_DIR:-}" && "$(xcode-select -p)" == */CommandLineTools ]]; then
  for candidate in /Applications/Xcode.app/Contents/Developer /Applications/Xcode-beta.app/Contents/Developer; do
    if [[ -d "$candidate" ]]; then export DEVELOPER_DIR="$candidate"; break; fi
  done
fi
MODE="${1:-run}"
if [[ $# -gt 0 ]]; then shift; fi
PACKAGE="$ROOT_DIR/apps/unidesk-macos/RemoteApplication"
case "$MODE" in
  test) exec swift test --package-path "$PACKAGE" "$@" ;;
  build|run|package|probe) ;;
  *) echo "usage: $0 [build|run|package|test|probe [--width N --height N --fps N --seconds N]]" >&2; exit 2 ;;
esac
PRODUCT=unidesk-remote-spike
[[ "$MODE" != probe ]] || PRODUCT=unidesk-media-probe
SDK_VERSION="$(xcrun --sdk macosx --show-sdk-version)"
FLAGS=(--package-path "$PACKAGE" --configuration release
  -Xlinker -platform_version -Xlinker macos -Xlinker 14.2 -Xlinker "$SDK_VERSION")
swift build "${FLAGS[@]}" --product "$PRODUCT" >&2
BIN_DIR="$(swift build "${FLAGS[@]}" --show-bin-path)"
if [[ "$MODE" == probe ]]; then exec "$BIN_DIR/$PRODUCT" "$@"; fi
BUNDLE="$ROOT_DIR/artifacts/remote-application/UniDeskRemoteSpike.app"
mkdir -p "$BUNDLE/Contents/MacOS"
cp "$BIN_DIR/$PRODUCT" "$BUNDLE/Contents/MacOS/$PRODUCT"
cat > "$BUNDLE/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>unidesk-remote-spike</string>
  <key>CFBundleIdentifier</key><string>com.unidesk.remote-spike</string>
  <key>CFBundleName</key><string>uniDesk Remote Spike</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>2</string>
  <key>CFBundleShortVersionString</key><string>0.0.2</string>
  <key>LSMinimumSystemVersion</key><string>14.2</string>
  <key>NSPrincipalClass</key><string>NSApplication</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSLocalNetworkUsageDescription</key><string>在两台 Mac 之间传输远程应用画面和键鼠控制。</string>
  <key>NSScreenCaptureUsageDescription</key><string>验证应用画面采集、原生分辨率与硬件视频编解码。</string>
</dict></plist>
PLIST
# Keep the designated requirement stable across rebuilds so TCC grants survive.
# In a linked worktree reuse the main checkout's pinned certificate, then use
# the same selector as the production build. Never silently fall back to ad-hoc.
if [[ -z "${CODESIGN_IDENTITY:-}" && ! -f "$ROOT_DIR/.codesign-identity" ]]; then
  COMMON_GIT_DIR="$(git -C "$ROOT_DIR" rev-parse --path-format=absolute --git-common-dir)"
  MAIN_SIGNING_PIN="$(dirname "$COMMON_GIT_DIR")/.codesign-identity"
  if [[ -f "$MAIN_SIGNING_PIN" ]]; then export CODESIGN_IDENTITY="$(cat "$MAIN_SIGNING_PIN")"; fi
fi
source "$ROOT_DIR/scripts/codesign-identity.sh"
codesign --force --sign "$CODESIGN_IDENTITY" "$BUNDLE" >&2
codesign --verify --strict "$BUNDLE"
printf '%s\n' "$BUNDLE"
if [[ "$MODE" == package ]]; then
  ARCHIVE="$ROOT_DIR/artifacts/remote-application/UniDeskRemoteSpike-0.0.2-arm64.zip"
  /usr/bin/ditto -c -k --sequesterRsrc --keepParent "$BUNDLE" "$ARCHIVE"
  /usr/bin/shasum -a 256 "$ARCHIVE" > "$ARCHIVE.sha256"
  printf '%s\n' "$ARCHIVE"
fi
if [[ "$MODE" == run ]]; then
  pkill -x unidesk-remote-spike 2>/dev/null || true
  /usr/bin/open -n "$BUNDLE"
fi
