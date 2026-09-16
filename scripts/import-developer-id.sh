#!/usr/bin/env bash
# Import a Developer ID Application .p12 into an ephemeral keychain for CI.
# Secrets: APPLE_DEVELOPER_ID_P12 (base64 or raw PKCS#12) and
# APPLE_DEVELOPER_ID_P12_PASSWORD. See docs/NOTARIZE.md.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: Developer ID import must run on Darwin" >&2
  exit 1
fi

if [[ -z "${APPLE_DEVELOPER_ID_P12:-}" ]]; then
  echo "error: APPLE_DEVELOPER_ID_P12 is empty. Create a Developer ID Application certificate and add the secrets listed in docs/NOTARIZE.md" >&2
  exit 1
fi
if [[ -z "${APPLE_DEVELOPER_ID_P12_PASSWORD:-}" ]]; then
  echo "error: APPLE_DEVELOPER_ID_P12_PASSWORD is empty" >&2
  exit 1
fi

STATE_DIR="${UNIDESK_SIGNING_STATE_DIR:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}/unidesk-codesign}"
mkdir -p "$STATE_DIR"
chmod 700 "$STATE_DIR"
STATE_FILE="$STATE_DIR/state"
KEYCHAIN_PATH="$STATE_DIR/unidesk.keychain-db"
P12_PATH="$STATE_DIR/developer-id.p12"
KEYCHAIN_PASSWORD="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"

python3 - "$P12_PATH" <<'PY'
import base64, os, pathlib, sys
compact = "".join(os.environ["APPLE_DEVELOPER_ID_P12"].split())
path = pathlib.Path(sys.argv[1])
try:
    path.write_bytes(base64.b64decode(compact, validate=True))
except Exception:
    raise SystemExit("error: APPLE_DEVELOPER_ID_P12 must be standard base64 of the exported .p12") from None
if path.stat().st_size < 64:
    raise SystemExit("error: decoded Developer ID .p12 is too small")
PY
chmod 600 "$P12_PATH"

security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
security set-keychain-settings -lut 21600 "$KEYCHAIN_PATH"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH"
# Keep the runner's existing search list so Apple intermediates stay visible.
EXISTING_KEYCHAINS="$(security list-keychains -d user | tr -d '"')"
# shellcheck disable=SC2086
security list-keychains -d user -s "$KEYCHAIN_PATH" $EXISTING_KEYCHAINS
security import "$P12_PATH" -P "$APPLE_DEVELOPER_ID_P12_PASSWORD" \
  -A -t cert -f pkcs12 -k "$KEYCHAIN_PATH"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN_PATH" >/dev/null

IDENTITY_HASH="$(security find-identity -v -p codesigning "$KEYCHAIN_PATH" | awk '
  /Developer ID Application:/ { print $2; count++; if (count > 1) exit }
  END { if (count != 1) exit 1 }
')" || {
  echo "error: expected exactly one Developer ID Application identity in the imported keychain" >&2
  security find-identity -v -p codesigning "$KEYCHAIN_PATH" >&2 || true
  exit 1
}

umask 077
{
  printf 'KEYCHAIN_PATH=%s\n' "$KEYCHAIN_PATH"
  printf 'P12_PATH=%s\n' "$P12_PATH"
} > "$STATE_FILE"
printf '%s\n' "$KEYCHAIN_PASSWORD" > "$STATE_DIR/keychain-password"
chmod 600 "$STATE_FILE" "$STATE_DIR/keychain-password"

if [[ -n "${GITHUB_ENV:-}" ]]; then
  {
    printf 'CODESIGN_IDENTITY=%s\n' "$IDENTITY_HASH"
    printf 'UNIDESK_SIGNING_STATE_DIR=%s\n' "$STATE_DIR"
  } >> "$GITHUB_ENV"
fi

printf 'CODESIGN_IDENTITY=%s\n' "$IDENTITY_HASH"
printf 'imported Developer ID Application into %s\n' "$KEYCHAIN_PATH"
