#!/usr/bin/env bash
# Build the standard library for the same minimum OS as the app. Homebrew's
# prebuilt std can otherwise silently raise the deployment target to macOS 26.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
export MACOSX_DEPLOYMENT_TARGET=14.0
RUST_SYSROOT="$(rustc --print sysroot)"
[[ -f "$RUST_SYSROOT/lib/rustlib/src/rust/library/Cargo.toml" ]] || {
  echo "error: matching rust-src is required (rustup component add rust-src)" >&2
  exit 1
}
ACTION="${1:-build}"
shift
# The pinned stable compiler uses its matching source; bootstrap is scoped to
# this invocation solely to enable Cargo's standard-library build facility.
RUSTC_BOOTSTRAP=1 cargo "$ACTION" -Z build-std=std,panic_unwind "$@"
