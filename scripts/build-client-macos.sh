#!/usr/bin/env bash
# Build the macOS client. --use-sdk skips Rust when the SDK manifest is present.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
USE_SDK=0
for arg in "$@"; do
  case "$arg" in
    --use-sdk) USE_SDK=1 ;;
  esac
done
SDK_DIR="$ROOT_DIR/apps/unidesk-macos/SDK"
if [[ "$USE_SDK" -eq 1 && -f "$SDK_DIR/manifest.json" ]]; then
  echo "SDK cache hit: $SDK_DIR/manifest.json"
else
  "$ROOT_DIR/scripts/build-sdk.sh"
fi
if [[ -x "$ROOT_DIR/scripts/build-native-macos.sh" ]]; then
  echo "Note: production app still links unidesk-ffi until the host is wired to the SDK."
  echo "Run scripts/build-native-macos.sh for a full client build."
fi
