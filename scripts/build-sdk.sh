#!/usr/bin/env bash
# Build the cached native SDK used by UI development. Does not compile Swift.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
SDK_DIR="${DESKFUSE_SDK_DIR:-$ROOT_DIR/apps/deskfuse-macos/SDK}"
SDK_VERSION="${DESKFUSE_SDK_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -n1)}"
TARGET="${DESKFUSE_SDK_TARGET:-aarch64-apple-darwin}"
mkdir -p "$SDK_DIR"

echo "Building deskfuse-collaboration ($TARGET)..."
cargo build --locked --release --package deskfuse-collaboration --target "$TARGET"

CORE_LIB="$ROOT_DIR/target/$TARGET/release/libdeskfuse_collaboration.a"
if [[ -f "$CORE_LIB" ]]; then
  cp "$CORE_LIB" "$SDK_DIR/libdeskfuse_collaboration.a"
fi

echo "Building input-only helper..."
cargo build --locked --release --package deskfuse-input-macos --target "$TARGET"
HELPER="$ROOT_DIR/target/$TARGET/release/deskfuse-input-macos"
if [[ -f "$HELPER" ]]; then
  cp "$HELPER" "$SDK_DIR/deskfuse-input-macos"
fi

echo "Packaging SDK manifest..."
cargo run --locked --quiet --package deskfuse-sdk -- \
  --version "$SDK_VERSION" \
  --out-dir "$SDK_DIR" \
  --root "$ROOT_DIR"

echo "SDK ready: $SDK_DIR/manifest.json"
