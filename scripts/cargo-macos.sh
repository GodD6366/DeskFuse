#!/usr/bin/env bash
# Keep the deployment target at macOS 14 without rebuilding std.
# rust-src / -Z build-std is intentionally not used.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/rust-toolchain-env.sh"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
ACTION="${1:-build}"
shift
cargo "$ACTION" "$@"
