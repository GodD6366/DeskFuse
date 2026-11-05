#!/usr/bin/env bash
# Point target/ at a runner-local directory so roc, Rust and Swift artifacts
# survive actions/checkout's clean. Self-hosted only: every job runs as the same
# user, and a directory must never be shared by jobs running at the same time.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
NAME="${1:?usage: ci-build-cache.sh JOB_NAME}"
CACHE="$HOME/deskfuse-ci-target/$NAME"
mkdir -p "$CACHE"
FREE_KB="$(df -Pk "$CACHE" | awk 'NR==2 {print $4}')"
if (( FREE_KB < 50 * 1024 * 1024 )); then
  echo "ci-build-cache: low disk space, clearing $CACHE" >&2
  rm -rf "$CACHE"
  mkdir -p "$CACHE"
fi
rm -rf target
ln -s "$CACHE" target

# Swift Build keeps its scratch tree inside the checkout, so a clean checkout
# recompiles the whole app module every run. Keep it beside the Rust cache.
SWIFT_SCRATCH="$ROOT_DIR/apps/deskfuse-macos/.build"
mkdir -p "$CACHE/swift" "$(dirname "$SWIFT_SCRATCH")"
rm -rf "$SWIFT_SCRATCH"
ln -s "$CACHE/swift" "$SWIFT_SCRATCH"

echo "ci-build-cache: $CACHE"
