#!/usr/bin/env bash
# Submit a Developer ID signed DMG to Apple notarytool and staple the ticket.
# Preferred credentials: APPLE_API_KEY (base64 of AuthKey_*.p8 or raw PEM),
# APPLE_API_KEY_ID, APPLE_API_ISSUER. Apple ID fallback: APPLE_ID,
# APPLE_APP_SPECIFIC_PASSWORD, APPLE_TEAM_ID. See docs/NOTARIZE.md.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: notarization must run on Darwin" >&2
  exit 1
fi

for command_name in xcrun python3; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "error: required command not found: $command_name" >&2
    exit 127
  fi
done

DMG_PATH="${1:-${DMG_PATH:-$ROOT_DIR/dist/DeskFuse-arm64.dmg}}"
if [[ ! -f "$DMG_PATH" ]]; then
  echo "error: DMG not found: $DMG_PATH" >&2
  exit 1
fi
DMG_PATH="$(python3 -c 'import os,sys; print(os.path.abspath(sys.argv[1]))' "$DMG_PATH")"

STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/deskfuse-notarize.XXXXXX")"
cleanup() {
  rm -rf "$STAGING_DIR"
}
trap cleanup EXIT

submit_args=()
if [[ -n "${APPLE_API_KEY:-}" && -n "${APPLE_API_KEY_ID:-}" && -n "${APPLE_API_ISSUER:-}" ]]; then
  KEY_PATH="$STAGING_DIR/AuthKey.p8"
  python3 - "$KEY_PATH" <<'PY'
import base64, os, pathlib, sys
raw = os.environ["APPLE_API_KEY"]
path = pathlib.Path(sys.argv[1])
if raw.lstrip().startswith("-----BEGIN"):
    path.write_text(raw if raw.endswith("\n") else raw + "\n")
else:
    compact = "".join(raw.split())
    try:
        path.write_bytes(base64.b64decode(compact, validate=True))
    except Exception:
        raise SystemExit("error: APPLE_API_KEY must be the .p8 PEM or its standard base64") from None
    text = path.read_text()
    if "BEGIN" not in text:
        raise SystemExit("error: decoded APPLE_API_KEY is not a PEM private key")
path.chmod(0o600)
PY
  submit_args=(--key "$KEY_PATH" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER")
elif [[ -n "${APPLE_ID:-}" && -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" && -n "${APPLE_TEAM_ID:-}" ]]; then
  submit_args=(--apple-id "$APPLE_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" --team-id "$APPLE_TEAM_ID")
else
  echo "error: notarization credentials missing. Set APPLE_API_KEY + APPLE_API_KEY_ID + APPLE_API_ISSUER (recommended) or APPLE_ID + APPLE_APP_SPECIFIC_PASSWORD + APPLE_TEAM_ID. See docs/NOTARIZE.md" >&2
  exit 1
fi

echo "notarization: submitting $DMG_PATH"
if ! xcrun notarytool submit "$DMG_PATH" "${submit_args[@]}" --wait --timeout 30m; then
  echo "error: notarytool submit failed" >&2
  xcrun notarytool history "${submit_args[@]}" --limit 3 >&2 || true
  exit 1
fi

xcrun stapler staple "$DMG_PATH"
xcrun stapler validate "$DMG_PATH"
echo "notarization: stapled $DMG_PATH"
