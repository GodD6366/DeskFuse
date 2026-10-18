#!/usr/bin/env bash
# Explicit cache maintenance for the macOS build. Never touches source files,
# Cargo.lock, committed resources or delivered packages.
#
# Defaults to a dry run; pass --yes to actually delete anything.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/rust-toolchain-env.sh"
source "$ROOT_DIR/scripts/build-cache.sh"

usage() {
  cat >&2 <<'USAGE'
usage: scripts/clean-build-cache.sh [scope ...] [--dry-run|--yes]

scopes:
  --rust                 clean Rust artifacts (requires --target)
  --target arm64|x86_64|both   target for --rust/--freerdp (default: both)
  --swift                remove apps/deskfuse-macos/.build
  --bindings             remove the Swift bindings receipt
  --freerdp              remove the vendored FreeRDP archive + matching build
  --assets              remove the generated asset receipt
  --host-tools           remove host-side packaging tools (bindgen/update)
  --incremental          remove target/*/debug/incremental directories
  --dist-old             remove all but the newest 3 dist/*-build* directories
  --all                  rust + swift + bindings + freerdp + assets + host-tools

  --prune                alias for --incremental --dist-old
  --dry-run              print actions without deleting (default without --yes)
  --yes                  actually delete
USAGE
  exit 2
}

WANT_RUST=0
WANT_SWIFT=0
WANT_BINDINGS=0
WANT_FREERDP=0
WANT_ASSETS=0
WANT_HOST_TOOLS=0
WANT_INCREMENTAL=0
WANT_DIST_OLD=0
ARCH_CHOICE=both

if [[ "$#" -eq 0 ]]; then
  usage
fi
while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --rust) WANT_RUST=1 ;;
    --swift) WANT_SWIFT=1 ;;
    --bindings) WANT_BINDINGS=1 ;;
    --freerdp) WANT_FREERDP=1 ;;
    --assets) WANT_ASSETS=1 ;;
    --host-tools) WANT_HOST_TOOLS=1 ;;
    --incremental) WANT_INCREMENTAL=1 ;;
    --dist-old) WANT_DIST_OLD=1 ;;
    --all)
      WANT_RUST=1; WANT_SWIFT=1; WANT_BINDINGS=1
      WANT_FREERDP=1; WANT_ASSETS=1; WANT_HOST_TOOLS=1
      ;;
    --prune) WANT_INCREMENTAL=1; WANT_DIST_OLD=1 ;;
    --target)
      shift
      [[ "$#" -gt 0 ]] || usage
      ARCH_CHOICE="$1"
      ;;
    --target=*) ARCH_CHOICE="${1#--target=}" ;;
    --dry-run) DESKFUSE_DRY_RUN=1 ;;
    --yes) DESKFUSE_DRY_RUN=0 ;;
    -h|--help) usage ;;
    *) usage ;;
  esac
  shift
done

case "$ARCH_CHOICE" in
  arm64) TARGET_CHOICES=(aarch64-apple-darwin); FREERDP_ARCHS=(arm64) ;;
  x86_64) TARGET_CHOICES=(x86_64-apple-darwin); FREERDP_ARCHS=(x86_64) ;;
  both) TARGET_CHOICES=(aarch64-apple-darwin x86_64-apple-darwin); FREERDP_ARCHS=(arm64 x86_64) ;;
  *) echo "error: --target must be arm64, x86_64 or both" >&2; exit 2 ;;
esac

if [[ "${DESKFUSE_DRY_RUN:-1}" == 1 ]]; then
  echo "dry run: pass --yes to delete"
  export DESKFUSE_DRY_RUN=1
else
  export DESKFUSE_DRY_RUN=0
fi

TARGET_DIR="$(deskfuse_cache_target_dir)"
echo "target directory: $TARGET_DIR"

if [[ "$WANT_RUST" == 1 ]]; then
  for triple in "${TARGET_CHOICES[@]}"; do
    deskfuse_cache_clean rust-target "$triple"
  done
fi
if [[ "$WANT_HOST_TOOLS" == 1 ]]; then
  deskfuse_cache_clean rust-host-tools
fi
if [[ "$WANT_SWIFT" == 1 ]]; then
  deskfuse_cache_clean swift-build
fi
if [[ "$WANT_BINDINGS" == 1 ]]; then
  deskfuse_cache_clean bindings
fi
if [[ "$WANT_FREERDP" == 1 ]]; then
  for arch in "${FREERDP_ARCHS[@]}"; do
    deskfuse_cache_clean freerdp "$arch"
  done
fi
if [[ "$WANT_ASSETS" == 1 ]]; then
  deskfuse_cache_clean assets
fi
if [[ "$WANT_INCREMENTAL" == 1 ]]; then
  for directory in "$TARGET_DIR"/debug/incremental "$TARGET_DIR"/*/debug/incremental; do
    [[ -d "$directory" ]] || continue
    deskfuse_remove_path "$directory" "$TARGET_DIR"
  done
fi
if [[ "$WANT_DIST_OLD" == 1 ]]; then
  kept=0
  while IFS= read -r directory; do
    [[ -n "$directory" ]] || continue
    kept=$((kept + 1))
    if [[ "$kept" -le 3 ]]; then
      continue
    fi
    deskfuse_remove_path "$directory" "$ROOT_DIR/dist"
  done < <(ls -1dt "$ROOT_DIR"/dist/*-build* 2>/dev/null || true)
fi

echo "cache cleanup complete (dry run: ${DESKFUSE_DRY_RUN})"
