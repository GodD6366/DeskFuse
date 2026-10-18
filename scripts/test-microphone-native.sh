#!/usr/bin/env bash
# Synthetic codec and isolated RPC regressions; no microphone capture or HAL installation.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Match the native build entry point even when --built regenerates CMake files.
if [[ -z "${DEVELOPER_DIR:-}" && "$(xcode-select -p)" == */CommandLineTools ]]; then
  for candidate in /Applications/Xcode.app/Contents/Developer /Applications/Xcode-beta.app/Contents/Developer; do
    if [[ -d "$candidate" ]]; then export DEVELOPER_DIR="$candidate"; break; fi
  done
fi
ARCH="${DESKFUSE_MACOS_ARCH:-$(uname -m)}"
[[ "$#" == 0 || ( "$#" == 1 && "$1" == --built ) ]] || { echo 'usage: test-microphone-native.sh [--built]' >&2; exit 2; }
[[ "$ARCH" == "$(uname -m)" ]] || { echo 'Run native tests on the target architecture' >&2; exit 2; }
if [[ "${1:-}" != --built ]]; then "$ROOT/scripts/build-roc-microphone.sh" "$ARCH"; fi
BUILD="$ROOT/target/roc-microphone/build-$ARCH"
cmake --build "$BUILD" --target microphone-codec-test microphone-rpc-reconnect-test --parallel "${DESKFUSE_ROC_BUILD_JOBS:-8}"
"$BUILD/microphone/microphone-codec-test"
GRPC_EXPERIMENTS=-event_engine_listener "$BUILD/microphone/microphone-rpc-reconnect-test"
GRPC_EXPERIMENTS=event_engine_listener "$BUILD/microphone/microphone-rpc-reconnect-test"
