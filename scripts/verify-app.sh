#!/usr/bin/env bash
# Fast-path verification of an already-built DeskFuse.app bundle.
#
# This is the subset of scripts/verify-dmg.sh that does not require a mounted
# disk image. It is used by the local --fast packaging flow; distributable DMGs
# still go through verify-dmg.sh.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

for command_name in codesign lipo otool python3; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "error: required command not found: $command_name" >&2
    exit 127
  fi
done

APP_BUNDLE="${1:-}"
if [[ -z "$APP_BUNDLE" || ! -d "$APP_BUNDLE" ]]; then
  echo "usage: scripts/verify-app.sh <DeskFuse.app>" >&2
  exit 2
fi
APP_BUNDLE="$(python3 -c 'import os,sys; print(os.path.abspath(sys.argv[1]))' "$APP_BUNDLE")"
CONTENTS_DIR="$APP_BUNDLE/Contents"
BINARY_PATH="$CONTENTS_DIR/MacOS/deskfuse-app"
INFO_PLIST="$CONTENTS_DIR/Info.plist"
python3 "$ROOT_DIR/scripts/macos-rpaths.py" "$BINARY_PATH" "$CONTENTS_DIR/MacOS/deskfuse-ocr-helper"

MACOS_ARCH="${DESKFUSE_MACOS_ARCH:-arm64}"
case "$MACOS_ARCH" in
  arm64) EXPECTED_ARCH="arm64" ;;
  x86_64) EXPECTED_ARCH="x86_64" ;;
  *) echo "error: DESKFUSE_MACOS_ARCH must be arm64 or x86_64" >&2; exit 2 ;;
esac

for resource in LICENSE LICENSE-MIT LICENSE-APACHE THIRD_PARTY_NOTICES.md LICENSE-LUCIDE THIRD_PARTY_DEPENDENCIES.txt THIRD_PARTY_UI.txt DeskFuse.icns DeskFuse-icon-1024.png DeskFuse-icon-light-1024.png deskfuse-status-icon.png; do
  [[ -s "$CONTENTS_DIR/Resources/$resource" ]] || { echo "error: bundle resource missing: $resource" >&2; exit 1; }
done

[[ -f "$CONTENTS_DIR/Resources/update-channel" ]] || { echo "error: update channel missing" >&2; exit 1; }
UPDATE_CHANNEL="$(cat "$CONTENTS_DIR/Resources/update-channel")"
# 局域网更新改为设备间互传正在运行的应用，任何通道都不再内嵌更新包。
case "$UPDATE_CHANNEL" in
  lan|github|manual)
    [[ ! -e "$CONTENTS_DIR/Resources/LanUpdate" ]] || {
      echo "error: $UPDATE_CHANNEL package contains an embedded update payload" >&2; exit 1;
    }
    ;;
  *) echo "error: invalid update channel" >&2; exit 1 ;;
esac

python3 - "$CONTENTS_DIR/Resources/Metadata.appintents/extract.actionsdata" <<'PYINTENTS'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1])
if not path.is_file() or not path.stat().st_size:
    raise SystemExit("error: App Intents metadata missing from bundle")
payload = json.loads(path.read_text())
text = json.dumps(payload, ensure_ascii=False)
if "RunWorkflowIntent" not in text or "WorkflowEntity" not in text:
    raise SystemExit("error: image workflow intent/entity missing from bundle metadata")
PYINTENTS

[[ -x "$BINARY_PATH" ]] || { echo "error: deskfuse-app missing or not executable" >&2; exit 1; }
[[ -f "$INFO_PLIST" ]] || { echo "error: Info.plist missing" >&2; exit 1; }

python3 - "$CONTENTS_DIR/Resources" <<'PYICONS'
from pathlib import Path
import struct, sys
import xml.etree.ElementTree as ET
resources = Path(sys.argv[1])
for name, size, png_name in (("DeskFuse-icon", 1024, "DeskFuse-icon-1024.png"),
                             ("DeskFuse-icon-light", 1024, "DeskFuse-icon-light-1024.png"),
                             ("deskfuse-status-icon", 36, "deskfuse-status-icon.png")):
    root = ET.parse(resources / f"{name}.svg").getroot()
    if root.tag != "{http://www.w3.org/2000/svg}svg":
        raise SystemExit(f"error: invalid SVG source: {name}")
    header = (resources / png_name).read_bytes()[:24]
    if (len(header) != 24 or header[:8] != b"\x89PNG\r\n\x1a\n"
            or header[12:16] != b"IHDR" or struct.unpack(">II", header[16:24]) != (size, size)):
        raise SystemExit(f"error: invalid {size}px icon: {png_name}")
print("icons: application SVG/1024px/ICNS and separate monochrome SVG/36px template")
PYICONS

[[ ! -e "$CONTENTS_DIR/Resources/DeskFuse.entitlements" ]] || {
  echo "error: entitlements must be used for signing, not shipped in Contents/Resources" >&2
  exit 1
}

ARCHES="$(lipo -archs "$BINARY_PATH")"
[[ "$ARCHES" == "$EXPECTED_ARCH" ]] || { echo "error: expected $EXPECTED_ARCH-only binary, got: $ARCHES" >&2; exit 1; }

python3 - "$INFO_PLIST" <<'PYINPUTSERVICE'
import plistlib, sys
with open(sys.argv[1], 'rb') as source:
    services = plistlib.load(source).get('NSBonjourServices', [])
if not {'_deskfuse._udp', '_deskfuse-input._udp'}.issubset(services):
    raise SystemExit('error: missing control/input Bonjour declaration')
print('Bonjour: separate control and input services declared')
PYINPUTSERVICE

codesign --verify --deep --strict "$APP_BUNDLE"

python3 "$ROOT_DIR/scripts/verify-input-service.py" "$CONTENTS_DIR/MacOS/deskfuse-input-service"

# Match the Info.plist contract enforced for DMGs so the fast path cannot ship
# a bundle that fails the distribution verifier.
if command -v /usr/libexec/PlistBuddy >/dev/null 2>&1; then
  DISPLAY_NAME="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleDisplayName' "$INFO_PLIST")"
  ICON_FILE="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIconFile' "$INFO_PLIST")"
  BUNDLE_NAME="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleName' "$INFO_PLIST")"
  BUNDLE_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$INFO_PLIST")"
  EXECUTABLE="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$INFO_PLIST")"
  MIN_SYSTEM_VERSION="$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$INFO_PLIST")"
  AUTOMATION_DESCRIPTION="$(/usr/libexec/PlistBuddy -c 'Print :NSAppleEventsUsageDescription' "$INFO_PLIST")"
  LOCAL_NETWORK_DESCRIPTION="$(/usr/libexec/PlistBuddy -c 'Print :NSLocalNetworkUsageDescription' "$INFO_PLIST")"
  BONJOUR_SERVICE="$(/usr/libexec/PlistBuddy -c 'Print :NSBonjourServices:0' "$INFO_PLIST")"
  [[ "$DISPLAY_NAME" == "DeskFuse" ]] || { echo "error: wrong display name: $DISPLAY_NAME" >&2; exit 1; }
  [[ "$ICON_FILE" == "DeskFuse.icns" ]] || { echo "error: wrong icon file: $ICON_FILE" >&2; exit 1; }
  [[ "$BUNDLE_NAME" == "DeskFuse" ]] || { echo "error: wrong bundle name: $BUNDLE_NAME" >&2; exit 1; }
  [[ "$BUNDLE_ID" == "com.deskfuse.app" ]] || { echo "error: wrong bundle id: $BUNDLE_ID" >&2; exit 1; }
  [[ "$EXECUTABLE" == "deskfuse-app" ]] || { echo "error: wrong executable: $EXECUTABLE" >&2; exit 1; }
  [[ "$MIN_SYSTEM_VERSION" == "14.0" ]] || { echo "error: minimum macOS must be 14.0: $MIN_SYSTEM_VERSION" >&2; exit 1; }
  [[ -n "$AUTOMATION_DESCRIPTION" ]] || { echo "error: NSAppleEventsUsageDescription missing" >&2; exit 1; }
  [[ -n "$LOCAL_NETWORK_DESCRIPTION" ]] || { echo "error: NSLocalNetworkUsageDescription missing" >&2; exit 1; }
  [[ "$BONJOUR_SERVICE" == "_deskfuse._udp" ]] || { echo "error: wrong Bonjour service: $BONJOUR_SERVICE" >&2; exit 1; }
  if UI_ELEMENT="$(/usr/libexec/PlistBuddy -c 'Print :LSUIElement' "$INFO_PLIST" 2>/dev/null)"; then
    [[ "$UI_ELEMENT" == "false" ]] || { echo "error: LSUIElement must be false when present" >&2; exit 1; }
  fi
fi

python3 - "$APP_BUNDLE" <<'PYAUTOMATION'
import plistlib, subprocess, sys
result = subprocess.run(["codesign", "-d", "--entitlements", ":-", sys.argv[1]], capture_output=True, check=True)
entitlements = plistlib.loads(result.stdout)
if entitlements.get("com.apple.security.automation.apple-events") is not True:
    raise SystemExit("error: Finder automation entitlement missing from signed app")
print("Finder automation: signed Apple Events entitlement present")
PYAUTOMATION

for helper in deskfuse-update-helper deskfuse-input-service deskfuse-ocr-helper deskfuse-history-files deskfuse-mygo; do
  [[ -x "$CONTENTS_DIR/MacOS/$helper" ]] || { echo "error: $helper missing" >&2; exit 1; }
  [[ "$(lipo -archs "$CONTENTS_DIR/MacOS/$helper")" == "$EXPECTED_ARCH" ]] || {
    echo "error: $helper architecture" >&2; exit 1;
  }
  codesign --verify --strict "$CONTENTS_DIR/MacOS/$helper"
done

# Rust must stay statically linked, and the native client must keep the modern
# SDK/deployment contract that verify-dmg.sh enforces for distribution.
python3 - "$BINARY_PATH" "$CONTENTS_DIR/MacOS/deskfuse-update-helper" "$CONTENTS_DIR/MacOS/deskfuse-input-service" "$CONTENTS_DIR/MacOS/deskfuse-ocr-helper" "$CONTENTS_DIR/MacOS/deskfuse-history-files" "$CONTENTS_DIR/MacOS/deskfuse-mygo" <<'PYNATIVE'
import re, subprocess, sys
for binary in sys.argv[1:]:
    load = subprocess.check_output(["otool", "-l", binary], text=True)
    versions = re.findall(r"\bminos\s+(\d+\.\d+)", load)
    if not versions or any(tuple(map(int, v.split("."))) > (14, 0) for v in versions):
        raise SystemExit(f"error: unsupported deployment target in {binary}: {versions}")
    sdks = re.findall(r"\bsdk\s+(\d+\.\d+)", load)
    if binary.endswith("/deskfuse-app") and (not sdks or any(int(v.split(".")[0]) < 26 for v in sdks)):
        raise SystemExit(f"error: native Liquid Glass requires a linked macOS 26+ SDK: {sdks}")
    links = subprocess.check_output(["otool", "-L", binary], text=True)
    if re.search(r"gpui|libdeskfuse_ffi", links, re.I):
        raise SystemExit(f"error: unexpected client dynamic dependency: {binary}")
    if binary.endswith("/deskfuse-mygo"):
        foreign = [line.strip() for line in links.splitlines()[1:]
                   if line.strip() and not line.strip().startswith(("/usr/lib/", "/System/Library/"))]
        if foreign:
            raise SystemExit(f"error: WebView client links a non-system library: {foreign}")
    if binary.endswith("/deskfuse-app") and "SwiftUI.framework" not in links:
        raise SystemExit("error: native SwiftUI framework dependency missing")
    print(f"Mach-O: {binary.rsplit('/', 1)[-1]} minimum macOS {versions[0]}, SDK {sdks[0] if sdks else 'unknown'}")
PYNATIVE

python3 - "$CONTENTS_DIR/Resources/release.json" "$INFO_PLIST" <<'PYMETA'
import json, plistlib, sys, uuid
with open(sys.argv[1], encoding="utf-8") as source:
    release = json.load(source)
with open(sys.argv[2], "rb") as source:
    info = plistlib.load(source)
if set(release) != {"version", "build_number", "release_id"}:
    raise SystemExit("error: invalid bundled release fields")
if type(release["build_number"]) is not int or release["build_number"] <= 0:
    raise SystemExit("error: invalid bundled release number")
if release["version"] != info["CFBundleShortVersionString"] or str(release["build_number"]) != info["CFBundleVersion"]:
    raise SystemExit("error: bundled release differs from Info.plist")
if not uuid.UUID(release["release_id"]).int:
    raise SystemExit("error: empty release identity")
print(f"release: {release['version']} build {release['build_number']} ({release['release_id']})")
PYMETA


SIGNING_INFO="$(codesign -dv --verbose=4 "$APP_BUNDLE" 2>&1)"
SIGNING_AUTHORITY="$(printf '%s\n' "$SIGNING_INFO" | sed -n 's/^Authority=//p' | head -n 1)"
TEAM_ID="$(printf '%s\n' "$SIGNING_INFO" | sed -n 's/^TeamIdentifier=//p' | head -n 1)"
RUNTIME_FLAG="$(printf '%s\n' "$SIGNING_INFO" | sed -n 's/^CodeDirectory.*flags=//p' | head -n 1)"
if [[ "$SIGNING_AUTHORITY" == Developer\ ID\ Application:* ]]; then
  [[ "$RUNTIME_FLAG" == *runtime* ]] || { echo "error: Developer ID app is missing the hardened runtime flag" >&2; exit 1; }
  printf '%s\n' "$SIGNING_INFO" | grep -q '^Timestamp=' || { echo "error: Developer ID app is missing a secure timestamp" >&2; exit 1; }
  SIGNING_STATUS="$SIGNING_AUTHORITY (Team ID ${TEAM_ID:-unknown})"
elif [[ -n "$SIGNING_AUTHORITY" && "$TEAM_ID" != "not set" ]]; then
  SIGNING_STATUS="$SIGNING_AUTHORITY (Team ID $TEAM_ID)"
else
  SIGNING_STATUS="ad-hoc"
fi

echo "verified app: $APP_BUNDLE"
echo "architecture: $ARCHES"
echo "update channel: $UPDATE_CHANNEL"
echo "signing: $SIGNING_STATUS"
