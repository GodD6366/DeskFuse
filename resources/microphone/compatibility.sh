#!/bin/bash
# Keep installed HAL identities in the existing retired-installation boundary.
MICROPHONE_COMPATIBILITY_FILE="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/previous-installation.json}"
MICROPHONE_BUNDLE_NAME="$(/usr/bin/plutil -extract microphone.bundleName raw -o - "$MICROPHONE_COMPATIBILITY_FILE")" || return 1
MICROPHONE_BUNDLE_ID="$(/usr/bin/plutil -extract microphone.bundleIdentifier raw -o - "$MICROPHONE_COMPATIBILITY_FILE")" || return 1
MICROPHONE_DEVICE_UID="$(/usr/bin/plutil -extract microphone.deviceUID raw -o - "$MICROPHONE_COMPATIBILITY_FILE")" || return 1
[[ "$MICROPHONE_BUNDLE_NAME" =~ ^[A-Za-z][A-Za-z0-9]+$ && "$MICROPHONE_BUNDLE_ID" =~ ^[A-Za-z0-9.]+$ && "$MICROPHONE_DEVICE_UID" =~ ^[A-Za-z0-9.]+$ ]] || {
  echo 'Invalid microphone compatibility identity' >&2
  return 1
}
