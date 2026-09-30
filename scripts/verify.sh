#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
"$ROOT_DIR/scripts/check-architecture.sh"
cargo test --locked --package deskfuse-collaboration
cargo test --locked --package deskfuse-store-core
cargo test --locked --package deskfuse-store-library
cargo test --locked --package deskfuse-platform --lib
