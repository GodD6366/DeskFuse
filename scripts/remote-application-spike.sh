#!/usr/bin/env bash
# Isolated P0 tooling. Never launches or replaces the shipping DeskFuse app.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${DEVELOPER_DIR:-}" && "$(xcode-select -p)" == */CommandLineTools ]]; then
  for candidate in /Applications/Xcode.app/Contents/Developer /Applications/Xcode-beta.app/Contents/Developer; do
    if [[ -d "$candidate" ]]; then export DEVELOPER_DIR="$candidate"; break; fi
  done
fi
MODE="${1:-run}"
if [[ $# -gt 0 ]]; then shift; fi
PACKAGE="$ROOT_DIR/apps/deskfuse-macos/RemoteApplication"
case "$MODE" in
  test) exec swift test --package-path "$PACKAGE" "$@" ;;
  build|run|package|probe) ;;
  *) echo "usage: $0 [build|run|package|test|probe [--width N --height N --fps N --seconds N]]" >&2; exit 2 ;;
esac
PRODUCT=deskfuse-remote-spike
[[ "$MODE" != probe ]] || PRODUCT=deskfuse-media-probe
SDK_VERSION="$(xcrun --sdk macosx --show-sdk-version)"
FLAGS=(--package-path "$PACKAGE" --configuration release
  -Xlinker -platform_version -Xlinker macos -Xlinker 14.2 -Xlinker "$SDK_VERSION")
swift build "${FLAGS[@]}" --product "$PRODUCT" >&2
BIN_DIR="$(swift build "${FLAGS[@]}" --show-bin-path)"
if [[ "$MODE" == probe ]]; then exec "$BIN_DIR/$PRODUCT" "$@"; fi
BUNDLE="$ROOT_DIR/artifacts/remote-application/DeskFuseRemoteSpike.app"
mkdir -p "$BUNDLE/Contents/MacOS"
cp "$BIN_DIR/$PRODUCT" "$BUNDLE/Contents/MacOS/$PRODUCT"
cat > "$BUNDLE/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>deskfuse-remote-spike</string>
  <key>CFBundleIdentifier</key><string>com.deskfuse.remote-spike</string>
  <key>CFBundleName</key><string>DeskFuse Remote Spike</string>
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
# 指定需求需跨次构建保持稳定，TCC 授权才不会失效。
# 签名身份与正式打包相同：worktree 无本目录指纹时沿用主检出，禁止退回 ad-hoc。
source "$ROOT_DIR/scripts/codesign-identity.sh"
codesign --force --sign "$CODESIGN_IDENTITY" "$BUNDLE" >&2
codesign --verify --strict "$BUNDLE"
printf '%s\n' "$BUNDLE"
if [[ "$MODE" == package ]]; then
  ARCHIVE="$ROOT_DIR/artifacts/remote-application/DeskFuseRemoteSpike-0.0.2-arm64.zip"
  /usr/bin/ditto -c -k --sequesterRsrc --keepParent "$BUNDLE" "$ARCHIVE"
  /usr/bin/shasum -a 256 "$ARCHIVE" > "$ARCHIVE.sha256"
  printf '%s\n' "$ARCHIVE"
fi
if [[ "$MODE" == run ]]; then
  pkill -x deskfuse-remote-spike 2>/dev/null || true
  /usr/bin/open -n "$BUNDLE"
fi
