#!/usr/bin/env bash
# Build the macOS client. Rust is always built through Cargo incremental
# invalidation; --rebuild-rust additionally drops the production artifacts.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
CONFIGURATION=debug
USE_SDK=1
for arg in "$@"; do
  case "$arg" in
    --use-sdk) USE_SDK=1 ;;
    --rebuild-rust) USE_SDK=0 ;;
    debug|release) CONFIGURATION="$arg" ;;
  esac
done
if [[ "$USE_SDK" -eq 0 ]]; then
  DESKFUSE_FORCE_RUST=1 "$ROOT_DIR/scripts/build-native-macos.sh" "$CONFIGURATION"
else
  "$ROOT_DIR/scripts/build-native-macos.sh" "$CONFIGURATION"
fi
