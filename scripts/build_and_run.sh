#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-run}"
BINARY_NAME="deskfuse-app"
# The development app shares the installed app's identity and paired devices.
BUNDLE_ID="com.deskfuse.app"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
APP_BUNDLE="$DIST_DIR/DeskFuse-dev.app"
CONTENTS_DIR="$APP_BUNDLE/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
APP_BINARY="$MACOS_DIR/$BINARY_NAME"

cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/codesign-identity.sh"
"$ROOT_DIR/scripts/generate-assets.sh"
"$ROOT_DIR/scripts/build-native-macos.sh" debug

TARGET_DIR="$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
BUILD_BINARY="$TARGET_DIR/native-macos/debug/$BINARY_NAME"
rm -rf "$APP_BUNDLE"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"
cp "$BUILD_BINARY" "$APP_BINARY"
cp "$(dirname "$BUILD_BINARY")/deskfuse-input-service" "$MACOS_DIR/deskfuse-input-service"
cp "$(dirname "$BUILD_BINARY")/build-id" "$MACOS_DIR/build-id"
codesign --force --strict --timestamp=none --identifier com.deskfuse.app.input-service --sign "$CODESIGN_IDENTITY" "$MACOS_DIR/deskfuse-input-service"
ditto "$(dirname "$BUILD_BINARY")/Metadata.appintents" "$RESOURCES_DIR/Metadata.appintents"
chmod 0755 "$APP_BINARY"

cp "$ROOT_DIR/resources/generated/DeskFuse.icns" "$RESOURCES_DIR/DeskFuse.icns"
cp "$ROOT_DIR/resources/generated/deskfuse-status-icon.png" "$RESOURCES_DIR/deskfuse-status-icon.png"
cp "$ROOT_DIR/resources/generated/DeskFuse-icon-light-1024.png" "$RESOURCES_DIR/DeskFuse-icon-light-1024.png"
ditto "$ROOT_DIR/resources/mice" "$RESOURCES_DIR/Mice"

APP_VERSION="$(python3 "$ROOT_DIR/scripts/release.py" version)"
cp "$ROOT_DIR/resources/generated/DeskFuse-icon-1024.png" "$RESOURCES_DIR/DeskFuse-icon-1024.png"
sed -e "s/@VERSION@/$APP_VERSION/g" -e 's/@BUILD_VERSION@/0/g' \
  "$ROOT_DIR/resources/Info.plist.in" > "$CONTENTS_DIR/Info.plist"

echo "signing: $CODESIGN_IDENTITY"
codesign --force --deep --strict --timestamp=none \
  --entitlements "$ROOT_DIR/resources/DeskFuse.entitlements" \
  --sign "$CODESIGN_IDENTITY" "$APP_BUNDLE"
codesign --verify --deep --strict "$APP_BUNDLE"

# Build-only validation leaves the running application untouched.
if [[ "$MODE" == "build" || "$MODE" == "--build" ]]; then
  echo "built: $APP_BUNDLE"
  exit 0
fi

# Keep the current session alive until the replacement has built and signed.
pkill -x "$BINARY_NAME" >/dev/null 2>&1 || true

open_app() {
  /usr/bin/open -n "$APP_BUNDLE"
}

case "$MODE" in
  run)
    open_app
    ;;
  --debug|debug)
    lldb -- "$APP_BINARY"
    ;;
  --logs|logs)
    open_app
    /usr/bin/log stream --info --style compact --predicate "process == \"$BINARY_NAME\""
    ;;
  --telemetry|telemetry)
    open_app
    /usr/bin/log stream --info --style compact --predicate "subsystem == \"$BUNDLE_ID\""
    ;;
  --verify|verify)
    open_app
    sleep 1
    pgrep -x "$BINARY_NAME" >/dev/null
    ;;
  *)
    echo "usage: $0 [run|--build|--debug|--logs|--telemetry|--verify]" >&2
    exit 2
    ;;
esac
