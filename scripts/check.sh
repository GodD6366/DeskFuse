#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/rust-toolchain-env.sh"

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is not installed; install Rust 1.97.1 before running checks" >&2
  exit 127
fi

if ! command -v rustc >/dev/null 2>&1; then
  echo "error: rustc is not installed; install Rust 1.97.1 before running checks" >&2
  exit 127
fi

printf '== rustc ==\n'
rustc --version
printf '== cargo ==\n'
cargo --version
printf '== cargo fmt ==\n'
cargo fmt --all -- --check
printf '== cargo check ==\n'
cargo check --locked --workspace --all-targets
printf '== cargo test ==\n'
cargo test --workspace --all-targets --locked
printf '== cargo clippy ==\n'
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

printf '== ui tokens ==\n'
"$ROOT_DIR/scripts/check-ui-tokens.sh"

"$ROOT_DIR/scripts/build-native-macos.sh" debug
"$ROOT_DIR/scripts/test-microphone-native.sh" --built
printf '== RDP bridge regression ==\n'
"$ROOT_DIR/scripts/build-freerdp-macos.sh" --test
printf '== RemoteApplication Swift tests ==\n'
python3 "$ROOT_DIR/scripts/test-remote-application.py"
