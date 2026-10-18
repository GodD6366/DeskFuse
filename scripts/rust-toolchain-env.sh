#!/usr/bin/env bash
# Source after cd to the repository root. Homebrew's cargo/rustc do not honor
# rust-toolchain.toml; prefer the already-installed rustup toolchain explicitly.
# Do not install or change the user's default toolchain here.
if ! command -v rustup >/dev/null 2>&1; then
  echo "error: rustup is required to select the repository's pinned Rust toolchain" >&2
  return 1
fi
DESKFUSE_RUST_TOOLCHAIN_BIN="$(dirname "$(rustup which cargo)")" || return 1
[[ -x "$DESKFUSE_RUST_TOOLCHAIN_BIN/cargo" && -x "$DESKFUSE_RUST_TOOLCHAIN_BIN/rustc" ]] || {
  echo "error: the repository's pinned Rust toolchain is not installed" >&2
  return 1
}
export PATH="$DESKFUSE_RUST_TOOLCHAIN_BIN:$PATH"
unset DESKFUSE_RUST_TOOLCHAIN_BIN
# Every cargo invocation must share the same deployment target, otherwise the
# same crate gets rebuilt under multiple fingerprints (and stale artifacts look
# interchangeable). Packaging and the native client already require 14.0.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
