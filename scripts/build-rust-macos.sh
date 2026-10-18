#!/usr/bin/env bash
# Build the production Rust library/helper and Swift bindings, without Swift.
# Cargo owns all Rust incrementality; DESKFUSE_FORCE_RUST=1 is the explicit
# escape hatch that drops the production artifacts and regenerates bindings.
#
# Cache policy: a cache-corruption signature triggers a scoped clean + retry,
# then a full target clean + retry. Ordinary source errors never clean caches.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/rust-toolchain-env.sh"
source "$ROOT_DIR/scripts/build-cache.sh"
CONFIGURATION="${1:-debug}"
[[ "$CONFIGURATION" == debug || "$CONFIGURATION" == release ]] || { echo "expected debug or release" >&2; exit 2; }
case "${DESKFUSE_MACOS_ARCH:-arm64}" in
  arm64) TARGET_TRIPLE=aarch64-apple-darwin ;;
  x86_64) TARGET_TRIPLE=x86_64-apple-darwin ;;
  *) echo "error: DESKFUSE_MACOS_ARCH must be arm64 or x86_64" >&2; exit 2 ;;
esac
PROFILE_FLAGS=""
if [[ "$CONFIGURATION" == release ]]; then PROFILE_FLAGS="--release"; fi
# Resolve Cargo's configured target directory (including CARGO_TARGET_DIR) once.
TARGET_DIR="${DESKFUSE_TARGET_DIR:-$(deskfuse_cache_target_dir)}"
RUST_OUTPUT="$TARGET_DIR/$TARGET_TRIPLE/$CONFIGURATION"
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
BINDGEN_BIN="$TARGET_DIR/$HOST_TRIPLE/debug/deskfuse-bindgen"
BINDGEN_FINGERPRINT_FILE="$TARGET_DIR/native-macos/.bindgen-fingerprint-$HOST_TRIPLE"

# --rebuild-rust must recompile rather than trust Cargo's fingerprints.
if [[ "${DESKFUSE_FORCE_RUST:-0}" == 1 ]]; then
  deskfuse_cache_clean rust-target "$TARGET_TRIPLE" deskfuse-ffi deskfuse-engine deskfuse-update
  deskfuse_cache_clean rust-host-tools
  deskfuse_cache_clean bindings || true
fi

# Refuse to reuse artifacts produced by a different rustc/SDK/target environment.
deskfuse_rust_cache_guard "$TARGET_TRIPLE" "$CONFIGURATION"

bindgen_fingerprint() {
  {
    find "$ROOT_DIR/tools/deskfuse-bindgen" -type f \( -name '*.rs' -o -name 'Cargo.toml' \) -print0 \
      | xargs -0 shasum -a 256
    shasum -a 256 "$ROOT_DIR/Cargo.lock"
  } | LC_ALL=C sort | shasum -a 256 | awk '{print $1}'
}

bindgen_is_cached() {
  [[ -x "$BINDGEN_BIN" && -f "$BINDGEN_FINGERPRINT_FILE" ]] \
    && [[ "$(cat "$BINDGEN_FINGERPRINT_FILE")" == "$(bindgen_fingerprint)" ]]
}

# Cargo owns source, build-script, feature, toolchain and flag invalidation.
# Keep CLI features in a separate invocation: workspace feature unification
# must not pull bindgen's template/parser stack into the production FFI library.
build_rust_artifacts() {
  "$ROOT_DIR/scripts/cargo-macos.sh" build --locked -p deskfuse-ffi --lib --bin deskfuse-input-service \
    --target "$TARGET_TRIPLE" $PROFILE_FLAGS || return $?
  if ! bindgen_is_cached; then
    cargo build --locked -p deskfuse-bindgen --bin deskfuse-bindgen --target "$HOST_TRIPLE" || return $?
  fi
}

attempt=1
while :; do
  if deskfuse_run_logged "rust-$CONFIGURATION-attempt$attempt" build_rust_artifacts; then
    break
  else
    status=$?
  fi
  deskfuse_cache_note_failure "rust-$CONFIGURATION" "${DESKFUSE_LAST_LOG:-}"
  if [[ "$attempt" -ge 3 ]] || ! deskfuse_failure_is_cache_suspect "${DESKFUSE_LAST_LOG:-/dev/null}" rust; then
    echo "rust build failed outside the cache-cleanup path; cache left intact" >&2
    echo "log: ${DESKFUSE_LAST_LOG:-unknown}" >&2
    exit "$status"
  fi
  case "$attempt" in
    1)
      echo "rust cache: corruption signature detected; cleaning the affected packages and retrying" >&2
      deskfuse_cache_clean rust-target "$TARGET_TRIPLE" deskfuse-ffi deskfuse-engine deskfuse-update
      ;;
    2)
      echo "rust cache: failure persists; cleaning the whole $TARGET_TRIPLE target and retrying" >&2
      deskfuse_cache_clean rust-target "$TARGET_TRIPLE"
      ;;
  esac
  deskfuse_cache_clean rust-host-tools
  deskfuse_cache_clean swift-fingerprint "$TARGET_TRIPLE" release
  deskfuse_cache_clean swift-fingerprint "$TARGET_TRIPLE" debug
  attempt=$((attempt + 1))
done
deskfuse_rust_cache_record
rm -f "$TARGET_DIR/.deskfuse-cache/last-failure.txt"

# A successful retry may have rebuilt the generator without a receipt yet; the
# binary is the authority. Record the current source fingerprint so the next
# run can skip the cargo invocation entirely.
if [[ ! -x "$BINDGEN_BIN" ]]; then
  echo "error: expected Rust binding generator missing: $BINDGEN_BIN" >&2
  exit 1
fi
mkdir -p "$(dirname "$BINDGEN_FINGERPRINT_FILE")"
printf '%s\n' "$(bindgen_fingerprint)" > "$BINDGEN_FINGERPRINT_FILE"
python3 "$ROOT_DIR/scripts/generate-swift-bindings.py" \
  --library "$RUST_OUTPUT/libdeskfuse_ffi.dylib" \
  --bindgen "$BINDGEN_BIN"
printf '%s\n' "${DESKFUSE_INPUT_BUILD_ID:-dev-$(git rev-parse --short=12 HEAD)}" > "$RUST_OUTPUT/build-id"
