#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
source "$ROOT_DIR/resources/microphone/compatibility.sh" "$ROOT_DIR/config/previous-installation.json"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: DMG verification must run on Darwin" >&2
  exit 1
fi

for command_name in hdiutil codesign lipo python3; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "error: required command not found: $command_name" >&2
    exit 127
  fi
done

MACOS_ARCH="${DESKFUSE_MACOS_ARCH:-arm64}"
case "$MACOS_ARCH" in
  arm64) EXPECTED_ARCH="arm64"; ARTIFACT_SUFFIX="arm64" ;;
  x86_64) EXPECTED_ARCH="x86_64"; ARTIFACT_SUFFIX="x86_64" ;;
  *) echo "error: DESKFUSE_MACOS_ARCH must be arm64 or x86_64" >&2; exit 2 ;;
esac
DMG_PATH="${1:-${DMG_PATH:-$ROOT_DIR/dist/DeskFuse-$ARTIFACT_SUFFIX.dmg}}"
if [[ ! -f "$DMG_PATH" ]]; then
  echo "error: DMG not found: $DMG_PATH" >&2
  exit 1
fi

hdiutil verify "$DMG_PATH" >/dev/null
MOUNT_POINT="$(mktemp -d "${TMPDIR:-/tmp}/deskfuse-dmg.XXXXXX")"
ATTACHED=0
cleanup() {
  if [[ "$ATTACHED" -eq 1 ]]; then
    hdiutil detach "$MOUNT_POINT" -quiet || true
  fi
  rmdir "$MOUNT_POINT" 2>/dev/null || true
}
trap cleanup EXIT

hdiutil attach "$DMG_PATH" -nobrowse -readonly -mountpoint "$MOUNT_POINT" >/dev/null
ATTACHED=1

APP_BUNDLE="$MOUNT_POINT/DeskFuse.app"
CONTENTS_DIR="$APP_BUNDLE/Contents"
BINARY_PATH="$CONTENTS_DIR/MacOS/deskfuse-app"
INFO_PLIST="$CONTENTS_DIR/Info.plist"
for resource in LICENSE LICENSE-MIT LICENSE-APACHE THIRD_PARTY_NOTICES.md LICENSE-LUCIDE THIRD_PARTY_DEPENDENCIES.txt DeskFuse.icns DeskFuse-icon-1024.png DeskFuse-icon-light-1024.png deskfuse-status-icon.png; do
  [[ -s "$CONTENTS_DIR/Resources/$resource" ]] || { echo "error: bundle resource missing: $resource" >&2; exit 1; }
done

# Verify native microphone payload without loading/installing the HAL plugin.
MICROPHONE="$CONTENTS_DIR/Resources/Microphone"
for payload in '安装麦克风驱动.command' '卸载麦克风驱动.command' manage-driver.sh '双机测试说明.txt' compatibility.sh driver-identity.json previous-installation.json THIRD_PARTY_MICROPHONE.txt source-archive.json; do
  [[ -s "$MICROPHONE/$payload" ]] || { echo "error: missing microphone payload: $payload" >&2; exit 1; }
  cmp "$MICROPHONE/$payload" "$MOUNT_POINT/麦克风驱动/$payload"
done
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$MICROPHONE/${MICROPHONE_BUNDLE_NAME}.driver/Contents/Info.plist")" == "$MICROPHONE_BUNDLE_ID" ]]
codesign --verify --deep --strict "$MICROPHONE/${MICROPHONE_BUNDLE_NAME}.driver"
[[ -L "$MOUNT_POINT/麦克风驱动" && "$(readlink "$MOUNT_POINT/麦克风驱动")" == 'DeskFuse.app/Contents/Resources/Microphone' ]]
codesign --verify --strict "$CONTENTS_DIR/Frameworks/libroc-microphone.dylib"
[[ "$(lipo -archs "$CONTENTS_DIR/Frameworks/libroc-microphone.dylib")" == "$EXPECTED_ARCH" ]]
[[ "$(lipo -archs "$MICROPHONE/${MICROPHONE_BUNDLE_NAME}.driver/Contents/MacOS/${MICROPHONE_BUNDLE_NAME}")" == "$EXPECTED_ARCH" ]]
python3 - "$CONTENTS_DIR/Frameworks/libroc-microphone.dylib" "$MICROPHONE/${MICROPHONE_BUNDLE_NAME}.driver/Contents/MacOS/${MICROPHONE_BUNDLE_NAME}" <<'PYMIC'
import re,subprocess,sys
for binary in sys.argv[1:]:
    load=subprocess.check_output(['otool','-l',binary],text=True)
    versions=re.findall(r"\bminos\s+(\d+\.\d+)",load)
    if not versions or any(tuple(map(int,v.split('.'))) > (14,0) for v in versions):
        raise SystemExit('error: unsupported microphone deployment target: '+binary)
    for line in subprocess.check_output(['otool','-L',binary],text=True).splitlines()[1:]:
        name=line.strip().split(' (',1)[0]
        if not name.startswith(('/System/Library/', '/usr/lib/', '@rpath/libroc-microphone.dylib')):
            raise SystemExit('error: unpackaged microphone dependency: '+name)
print('Microphone driver and shim: signed, architecture and dependencies verified; not installed')
PYMIC
[[ ! -e "$MICROPHONE/roc-microphone-sources.tar.gz" ]] || { echo 'error: sources duplicated inside app' >&2; exit 1; }
python3 - "$MICROPHONE/source-archive.json" "$MOUNT_POINT" <<'PYSOURCES'
import json, pathlib, re, sys
manifest = json.loads(pathlib.Path(sys.argv[1]).read_text())
assert manifest['file'] == 'roc-microphone-sources.tar.gz'
assert manifest['bundled'] is False
assert manifest['obtain']
assert re.fullmatch('[0-9a-f]{64}', manifest['sha256'])
assert type(manifest['size']) is int and manifest['size'] > 0
mount = pathlib.Path(sys.argv[2])
visible = {path.name for path in mount.iterdir() if not path.name.startswith('.')}
assert visible == {'DeskFuse.app', 'Applications', '麦克风驱动'}, f'unexpected DMG contents: {visible}'
assert not list(mount.rglob('roc-microphone-sources.tar.gz')), 'source archive inside DMG'
print('DMG layout: existing app, Applications and driver entry retained; no source archive')
PYSOURCES

# The signed distribution marker selects the updater, independent of install path.
[[ -f "$CONTENTS_DIR/Resources/update-channel" ]] || { echo "error: update channel missing" >&2; exit 1; }
UPDATE_CHANNEL="$(cat "$CONTENTS_DIR/Resources/update-channel")"
case "$UPDATE_CHANNEL" in
  lan)
    [[ -f "$MOUNT_POINT/.updates/manifest.json" && -f "$CONTENTS_DIR/Resources/LanUpdate/manifest.json" ]] || {
      echo "error: LAN package is missing its signed update payload" >&2; exit 1;
    }
    ;;
  github|manual)
    [[ ! -e "$MOUNT_POINT/.updates" && ! -e "$CONTENTS_DIR/Resources/LanUpdate" ]] || {
      echo "error: $UPDATE_CHANNEL package contains a LAN update payload" >&2; exit 1;
    }
    ;;
  *) echo "error: invalid update channel" >&2; exit 1 ;;
esac

# Metadata is part of the signed payload. An empty placeholder does not prove
# system discovery, but must never pass the package integrity check.
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

[[ -d "$APP_BUNDLE" ]] || { echo "error: DeskFuse.app missing from DMG" >&2; exit 1; }
[[ -x "$BINARY_PATH" ]] || { echo "error: deskfuse-app missing or not executable" >&2; exit 1; }
[[ -f "$INFO_PLIST" ]] || { echo "error: Info.plist missing" >&2; exit 1; }
[[ -f "$CONTENTS_DIR/Resources/DeskFuse.icns" ]] || { echo "error: app icon missing" >&2; exit 1; }
[[ -f "$CONTENTS_DIR/Resources/deskfuse-status-icon.png" ]] || { echo "error: status icon missing" >&2; exit 1; }
# Both SVG sources are bundled; the menu bar uses its own 18pt/2x template.
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
[[ -L "$MOUNT_POINT/Applications" ]] || { echo "error: Applications shortcut missing" >&2; exit 1; }
[[ "$(readlink "$MOUNT_POINT/Applications")" == "/Applications" ]] || {
  echo "error: Applications shortcut has wrong target" >&2
  exit 1
}
[[ -f "$MOUNT_POINT/.background/installer-background.png" ]] || {
  echo "error: installer background missing" >&2
  exit 1
}
if [[ "${DESKFUSE_DMG_FINDER_LAYOUT:-1}" != "0" ]]; then
  [[ -f "$MOUNT_POINT/.DS_Store" ]] || { echo "error: Finder layout missing" >&2; exit 1; }
fi
[[ ! -e "$CONTENTS_DIR/Resources/DeskFuse.entitlements" ]] || {
  echo "error: entitlements must be used for signing, not shipped in Contents/Resources" >&2
  exit 1
}

ARCHES="$(lipo -archs "$BINARY_PATH")"
[[ "$ARCHES" == "$EXPECTED_ARCH" ]] || { echo "error: expected $EXPECTED_ARCH-only binary, got: $ARCHES" >&2; exit 1; }
python3 - "$CONTENTS_DIR/Info.plist" <<'PYINPUTSERVICE'
import plistlib, sys
with open(sys.argv[1], 'rb') as source:
    services = plistlib.load(source).get('NSBonjourServices', [])
if not {'_deskfuse._udp', '_deskfuse-input._udp'}.issubset(services):
    raise SystemExit('error: missing control/input Bonjour declaration')
print('Bonjour: separate control and input services declared')
PYINPUTSERVICE
codesign --verify --deep --strict "$APP_BUNDLE"
python3 - "$APP_BUNDLE" <<'PYAUTOMATION'
import plistlib, subprocess, sys
result = subprocess.run(["codesign", "-d", "--entitlements", ":-", sys.argv[1]], capture_output=True, check=True)
entitlements = plistlib.loads(result.stdout)
if entitlements.get("com.apple.security.device.audio-input") is not True:
    raise SystemExit("error: microphone entitlement missing from signed app")
with open(sys.argv[1]+"/Contents/Info.plist", "rb") as source:
    if not plistlib.load(source).get("NSMicrophoneUsageDescription"):
        raise SystemExit("error: microphone usage description missing")
if entitlements.get("com.apple.security.automation.apple-events") is not True:
    raise SystemExit("error: Finder automation entitlement missing from signed app")
print("Finder automation: signed Apple Events entitlement present")
PYAUTOMATION
[[ -x "$CONTENTS_DIR/MacOS/deskfuse-update-helper" ]] || { echo "error: update helper missing" >&2; exit 1; }
[[ "$(lipo -archs "$CONTENTS_DIR/MacOS/deskfuse-update-helper")" == "$EXPECTED_ARCH" ]] || { echo "error: update helper architecture" >&2; exit 1; }
codesign --verify --strict "$CONTENTS_DIR/MacOS/deskfuse-update-helper"
[[ -x "$CONTENTS_DIR/MacOS/deskfuse-input-service" ]] || { echo "error: input service missing" >&2; exit 1; }
[[ "$(lipo -archs "$CONTENTS_DIR/MacOS/deskfuse-input-service")" == "$EXPECTED_ARCH" ]] || { echo "error: input service architecture" >&2; exit 1; }
codesign --verify --strict "$CONTENTS_DIR/MacOS/deskfuse-input-service"
python3 "$ROOT_DIR/scripts/verify-input-service.py" "$CONTENTS_DIR/MacOS/deskfuse-input-service"
[[ -x "$CONTENTS_DIR/MacOS/deskfuse-ocr-helper" ]] || { echo "error: OCR helper missing" >&2; exit 1; }
[[ "$(lipo -archs "$CONTENTS_DIR/MacOS/deskfuse-ocr-helper")" == "$EXPECTED_ARCH" ]] || { echo "error: OCR helper architecture" >&2; exit 1; }
codesign --verify --strict "$CONTENTS_DIR/MacOS/deskfuse-ocr-helper"
[[ -x "$CONTENTS_DIR/MacOS/deskfuse-history-files" ]] || { echo "error: history file importer missing" >&2; exit 1; }
[[ "$(lipo -archs "$CONTENTS_DIR/MacOS/deskfuse-history-files")" == "$EXPECTED_ARCH" ]] || { echo "error: history file importer architecture" >&2; exit 1; }
codesign --verify --strict "$CONTENTS_DIR/MacOS/deskfuse-history-files"
# Inspect all shipped Mach-O files: Info.plist alone cannot establish the
# deployment target, and Rust must be statically linked into the native client.
python3 - "$BINARY_PATH" "$CONTENTS_DIR/MacOS/deskfuse-update-helper" "$CONTENTS_DIR/MacOS/deskfuse-input-service" "$CONTENTS_DIR/MacOS/deskfuse-ocr-helper" "$CONTENTS_DIR/MacOS/deskfuse-history-files" <<'PYNATIVE'
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
    if binary.endswith("/deskfuse-app") and "SwiftUI.framework" not in links:
        raise SystemExit("error: native SwiftUI framework dependency missing")
    print(f"Mach-O: {binary.rsplit('/', 1)[-1]} minimum macOS {versions[0]}, SDK {sdks[0] if sdks else 'unknown'}")
PYNATIVE
if [[ -f "$MOUNT_POINT/.updates/manifest.json" ]]; then
  if [[ -n "${DESKFUSE_RELEASE_TOOL:-}" && -x "${DESKFUSE_RELEASE_TOOL:-}" ]]; then
    "$DESKFUSE_RELEASE_TOOL" verify "$ROOT_DIR/resources/deskfuse-release-public-key.hex" "$MOUNT_POINT/.updates/manifest.json" "$MOUNT_POINT/.updates"
  else
    cargo run --locked --quiet -p deskfuse-update --bin deskfuse-release -- verify "$ROOT_DIR/resources/deskfuse-release-public-key.hex" "$MOUNT_POINT/.updates/manifest.json" "$MOUNT_POINT/.updates"
  fi
  EMBEDDED_UPDATE="$CONTENTS_DIR/Resources/LanUpdate"
  cmp "$MOUNT_POINT/.updates/manifest.json" "$EMBEDDED_UPDATE/manifest.json"
  cmp "$MOUNT_POINT/.updates/DeskFuse-$ARTIFACT_SUFFIX-update.zip" "$EMBEDDED_UPDATE/DeskFuse-$ARTIFACT_SUFFIX-update.zip"
  python3 - "$CONTENTS_DIR/Resources/release.json" "$EMBEDDED_UPDATE/manifest.json" "$EMBEDDED_UPDATE/DeskFuse-$ARTIFACT_SUFFIX-update.zip" <<'PYSEED'
import json, pathlib, sys, zipfile
release = json.loads(pathlib.Path(sys.argv[1]).read_text())
manifest = json.loads(pathlib.Path(sys.argv[2]).read_text())["manifest"]
if manifest["release"] != release:
    raise SystemExit("error: embedded update release differs from installed app")
with zipfile.ZipFile(sys.argv[3]) as payload:
    if json.loads(payload.read("DeskFuse.app/Contents/Resources/release.json")) != release:
        raise SystemExit("error: update payload release differs from installed app")
    if any(name.endswith("roc-microphone-sources.tar.gz") for name in payload.namelist()):
        raise SystemExit("error: full microphone sources duplicated inside update payload")
    if any(name.startswith("DeskFuse.app/Contents/Resources/LanUpdate/") for name in payload.namelist()):
        raise SystemExit("error: recursive embedded update payload")
print("LAN distribution: installed app carries the exact verified release payload")
PYSEED
fi
SIGNING_INFO="$(codesign -dv --verbose=4 "$APP_BUNDLE" 2>&1)"
SIGNING_AUTHORITY="$(printf '%s\n' "$SIGNING_INFO" | sed -n 's/^Authority=//p' | head -n 1)"
TEAM_ID="$(printf '%s\n' "$SIGNING_INFO" | sed -n 's/^TeamIdentifier=//p' | head -n 1)"
RUNTIME_FLAG="$(printf '%s\n' "$SIGNING_INFO" | sed -n 's/^CodeDirectory.*flags=//p' | head -n 1)"
if [[ -n "$SIGNING_AUTHORITY" && "$TEAM_ID" != "not set" ]]; then
  SIGNING_STATUS="$SIGNING_AUTHORITY (Team ID $TEAM_ID)"
else
  SIGNING_STATUS="ad-hoc"
fi
if [[ "$SIGNING_STATUS" != "ad-hoc" && "$SIGNING_AUTHORITY" == Developer\ ID\ Application:* ]]; then
  if [[ "$RUNTIME_FLAG" != *runtime* ]]; then
    echo "error: Developer ID app is missing the hardened runtime flag" >&2
    exit 1
  fi
  if ! printf '%s\n' "$SIGNING_INFO" | grep -q '^Timestamp='; then
    echo "error: Developer ID app is missing a secure timestamp" >&2
    exit 1
  fi
fi
if [[ "${DESKFUSE_REQUIRE_NOTARIZE:-0}" == "1" ]]; then
  if [[ "$SIGNING_AUTHORITY" != Developer\ ID\ Application:* ]]; then
    echo "error: notarized package must be signed with Developer ID Application" >&2
    exit 1
  fi
  xcrun stapler validate "$DMG_PATH"
fi

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

# Release metadata is part of the signed bundle, not inferred from a Git checkout.
python3 - "$APP_BUNDLE/Contents/Resources/release.json" "$INFO_PLIST" <<'PYMETA'
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

echo "verified: $DMG_PATH"
echo "architecture: $ARCHES"
echo "update channel: $UPDATE_CHANNEL"
echo "signing: $SIGNING_STATUS"
if [[ "${DESKFUSE_REQUIRE_NOTARIZE:-0}" == "1" ]]; then
  echo "notarization: stapled ticket validated"
elif [[ "$SIGNING_AUTHORITY" == Developer\ ID\ Application:* ]]; then
  echo "notarization: Developer ID signed; staple is checked when DESKFUSE_REQUIRE_NOTARIZE=1"
else
  echo "notarization: not assessed; this verifier does not establish Gatekeeper acceptance"
fi
