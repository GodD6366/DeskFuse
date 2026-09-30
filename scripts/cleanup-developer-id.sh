#!/usr/bin/env bash
# Remove the ephemeral Developer ID keychain and PKCS#12 copy created by
# scripts/import-developer-id.sh. Safe to run when import never happened.
set -euo pipefail

STATE_DIR="${DESKFUSE_SIGNING_STATE_DIR:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}/deskfuse-codesign}"
if [[ ! -d "$STATE_DIR" ]]; then
  echo "signing cleanup: no imported Developer ID state"
  exit 0
fi

STATE_FILE="$STATE_DIR/state"
if [[ -f "$STATE_FILE" ]]; then
  # shellcheck disable=SC1090
  source "$STATE_FILE"
fi
if [[ -n "${KEYCHAIN_PATH:-}" && -f "$KEYCHAIN_PATH" ]]; then
  security delete-keychain "$KEYCHAIN_PATH" >/dev/null 2>&1 || true
fi
find "$STATE_DIR" -name '*.keychain-db' -exec security delete-keychain {} \; >/dev/null 2>&1 || true
rm -rf "$STATE_DIR"
echo "signing cleanup: removed ephemeral Developer ID keychain"
