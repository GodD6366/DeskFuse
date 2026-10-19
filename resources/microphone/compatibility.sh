#!/bin/bash
# 麦克风驱动身份。
# - 当前 DeskFuse 驱动身份来自同目录的 driver-identity.json，用于构建、打包和安装；
# - 旧安装的驱动身份只来自 previous-installation.json（参数 1），仅用于安装/卸载时清理遗留驱动。
MICROPHONE_RESOURCES_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MICROPHONE_IDENTITY_FILE="$MICROPHONE_RESOURCES_DIR/driver-identity.json"
MICROPHONE_COMPATIBILITY_FILE="${1:-$MICROPHONE_RESOURCES_DIR/previous-installation.json}"
MICROPHONE_BUNDLE_NAME="$(/usr/bin/plutil -extract bundleName raw -o - "$MICROPHONE_IDENTITY_FILE")" || return 1
MICROPHONE_BUNDLE_ID="$(/usr/bin/plutil -extract bundleIdentifier raw -o - "$MICROPHONE_IDENTITY_FILE")" || return 1
MICROPHONE_DEVICE_UID="$(/usr/bin/plutil -extract deviceUID raw -o - "$MICROPHONE_IDENTITY_FILE")" || return 1
LEGACY_MICROPHONE_BUNDLE_NAME="$(/usr/bin/plutil -extract microphone.bundleName raw -o - "$MICROPHONE_COMPATIBILITY_FILE")" || return 1
LEGACY_MICROPHONE_BUNDLE_ID="$(/usr/bin/plutil -extract microphone.bundleIdentifier raw -o - "$MICROPHONE_COMPATIBILITY_FILE")" || return 1
for value in "$MICROPHONE_BUNDLE_NAME" "$LEGACY_MICROPHONE_BUNDLE_NAME"; do
  [[ "$value" =~ ^[A-Za-z][A-Za-z0-9]+$ ]] || { echo 'Invalid microphone driver identity' >&2; return 1; }
done
for value in "$MICROPHONE_BUNDLE_ID" "$MICROPHONE_DEVICE_UID" "$LEGACY_MICROPHONE_BUNDLE_ID"; do
  [[ "$value" =~ ^[A-Za-z0-9.]+$ ]] || { echo 'Invalid microphone driver identity' >&2; return 1; }
done
# 新旧身份必须不同，否则清理旧驱动会误删当前驱动。
[[ "$MICROPHONE_BUNDLE_NAME" != "$LEGACY_MICROPHONE_BUNDLE_NAME" && "$MICROPHONE_BUNDLE_ID" != "$LEGACY_MICROPHONE_BUNDLE_ID" ]] || {
  echo 'Microphone driver identity must differ from the previous installation' >&2
  return 1
}
