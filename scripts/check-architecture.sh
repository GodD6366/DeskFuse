#!/usr/bin/env bash
# Architecture gates from the collaboration refactor plan.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
fail=0

check_absent() {
  local package="$1"
  local pattern="$2"
  local label="$3"
  if cargo tree --package "$package" --edges normal 2>/dev/null | grep -E "$pattern" >/dev/null; then
    echo "FAIL: $label"
    cargo tree --package "$package" --edges normal | grep -E "$pattern" || true
    fail=1
  else
    echo "PASS: $label"
  fi
}

check_absent deskfuse-collaboration \
  '(^| )(axum|image|deskfuse-workflow|deskfuse-content-library|deskfuse-platform-macos|deskfuse-lan-web)( v|$)' \
  "deskfuse-collaboration does not depend on extensions or platform implementations"

check_absent deskfuse-input-macos \
  '(^| )(axum|deskfuse-workflow|deskfuse-content-library|deskfuse-lan-web|deskfuse-extension-update|deskfuse-engine)( v|$)' \
  "deskfuse-input-macos does not depend on product extensions or the full engine"

check_absent deskfuse-platform \
  '(^| )(core-graphics|core-graphics-rs|winapi|cocoa|objc2-app-kit)( v|$)' \
  "deskfuse-platform has no concrete OS adapter crates"

exit "$fail"
