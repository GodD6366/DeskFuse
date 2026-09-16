#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
"$ROOT_DIR/scripts/check-architecture.sh"
cargo test --locked --package unidesk-collaboration
cargo test --locked --package unidesk-store-core
cargo test --locked --package unidesk-store-library
cargo test --locked --package unidesk-platform --lib
