#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
"$ROOT_DIR/scripts/check-architecture.sh"
cargo build --locked --package unidesk-collaboration
cargo build --locked --package unidesk-content-library
cargo build --locked --package unidesk-workflow
cargo build --locked --package unidesk-lan-web
cargo build --locked --package unidesk-extension-update
cargo build --locked --package unidesk-input-macos
"$ROOT_DIR/scripts/build-sdk.sh"
