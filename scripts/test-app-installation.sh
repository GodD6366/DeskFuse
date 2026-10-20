#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/deskfuse-installer-check.XXXXXX")"
trap 'rm -rf "$TEST_DIR"' EXIT
swiftc -swift-version 6 -parse-as-library \
  "$ROOT_DIR/apps/deskfuse-macos/Sources/DeskFuse/AppInstallation.swift" \
  "$ROOT_DIR/scripts/tests/AppInstallationTests.swift" \
  -o "$TEST_DIR/installation-tests"
"$TEST_DIR/installation-tests" "$@"
