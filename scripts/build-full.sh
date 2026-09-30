#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
"$ROOT_DIR/scripts/check-architecture.sh"
cargo build --locked --package deskfuse-collaboration
cargo build --locked --package deskfuse-content-library
cargo build --locked --package deskfuse-workflow
cargo build --locked --package deskfuse-lan-web
cargo build --locked --package deskfuse-extension-update
cargo build --locked --package deskfuse-input-macos
"$ROOT_DIR/scripts/build-sdk.sh"
