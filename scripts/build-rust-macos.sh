#!/usr/bin/env bash
# Build the production Rust library/helper and Swift bindings, without Swift.
# Cargo owns all Rust incrementality; DESKFUSE_FORCE_RUST=1 is the explicit
# escape hatch that drops the production artifacts and regenerates bindings.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
CONFIGURATION="${1:-debug}"
[[ "$CONFIGURATION" == debug || "$CONFIGURATION" == release ]] || { echo "expected debug or release" >&2; exit 2; }
case "${DESKFUSE_MACOS_ARCH:-arm64}" in
  arm64) TARGET_TRIPLE=aarch64-apple-darwin ;;
  x86_64) TARGET_TRIPLE=x86_64-apple-darwin ;;
  *) echo "error: DESKFUSE_MACOS_ARCH must be arm64 or x86_64" >&2; exit 2 ;;
esac
PROFILE_FLAGS=""
if [[ "$CONFIGURATION" == release ]]; then PROFILE_FLAGS="--release"; fi
# Resolve Cargo's configured target directory (including CARGO_TARGET_DIR).
TARGET_DIR="$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
RUST_OUTPUT="$TARGET_DIR/$TARGET_TRIPLE/$CONFIGURATION"

# --rebuild-rust must recompile rather than trust Cargo's fingerprints.
if [[ "${DESKFUSE_FORCE_RUST:-0}" == 1 ]]; then
  "$ROOT_DIR/scripts/cargo-macos.sh" clean --target "$TARGET_TRIPLE" \
    -p deskfuse-ffi -p deskfuse-engine
fi

# Cargo owns source, build-script, feature, toolchain and flag invalidation.
# Keep CLI features in a separate invocation: workspace feature unification
# must not pull bindgen's template/parser stack into the production FFI library.
"$ROOT_DIR/scripts/cargo-macos.sh" build --locked -p deskfuse-ffi --lib --target "$TARGET_TRIPLE" $PROFILE_FLAGS
"$ROOT_DIR/scripts/cargo-macos.sh" build --locked -p deskfuse-ffi --bin deskfuse-input-service --target "$TARGET_TRIPLE" $PROFILE_FLAGS
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --locked -p deskfuse-bindgen --bin deskfuse-bindgen --target "$HOST_TRIPLE"
python3 "$ROOT_DIR/scripts/generate-swift-bindings.py" \
  --library "$RUST_OUTPUT/libdeskfuse_ffi.dylib" \
  --bindgen "$TARGET_DIR/$HOST_TRIPLE/debug/deskfuse-bindgen"
printf '%s\n' "${DESKFUSE_INPUT_BUILD_ID:-dev-$(git rev-parse --short=12 HEAD)}" > "$RUST_OUTPUT/build-id"
