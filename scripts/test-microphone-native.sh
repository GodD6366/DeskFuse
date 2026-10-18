#!/usr/bin/env bash
# Synthetic codec regression; no microphone capture or HAL installation.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARCH="${DESKFUSE_MACOS_ARCH:-$(uname -m)}"
[[ "$#" == 0 || ( "$#" == 1 && "$1" == --built ) ]] || { echo 'usage: test-microphone-native.sh [--built]' >&2; exit 2; }
[[ "$ARCH" == "$(uname -m)" ]] || { echo 'Run codec tests on the target architecture' >&2; exit 2; }
if [[ "${1:-}" != --built ]]; then "$ROOT/scripts/build-roc-microphone.sh" "$ARCH"; fi
BUILD="$ROOT/target/roc-microphone/build-$ARCH"
cmake --build "$BUILD" --target microphone-codec-test --parallel "${DESKFUSE_ROC_BUILD_JOBS:-8}"
"$BUILD/microphone/microphone-codec-test"
