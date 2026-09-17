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

check_absent unidesk-collaboration \
  '(^| )(axum|image|unidesk-workflow|unidesk-content-library|unidesk-platform-macos|unidesk-lan-web)( v|$)' \
  "unidesk-collaboration does not depend on extensions or platform implementations"

check_absent unidesk-input-macos \
  '(^| )(axum|unidesk-workflow|unidesk-content-library|unidesk-lan-web|unidesk-extension-update|unidesk-engine)( v|$)' \
  "unidesk-input-macos does not depend on product extensions or the full engine"

check_absent unidesk-platform \
  '(^| )(core-graphics|core-graphics-rs|winapi|cocoa|objc2-app-kit)( v|$)' \
  "unidesk-platform has no concrete OS adapter crates"

exit "$fail"
