#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: macOS packaging must run on Darwin" >&2
  exit 1
fi

for command_name in cargo swift codesign hdiutil lipo security python3 ditto shasum; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "error: required command not found: $command_name" >&2
    exit 127
  fi
done

# Public/homebrew packages never read the maintainer's LAN signing key, allocate
# a coordinator build number or seed the running app's updater.
PACKAGE_MODE=lan
AUTO_INSTALL=0
usage() { echo "usage: scripts/package-macos.sh [--lan|--homebrew] [--auto]" >&2; exit 2; }
while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --lan) ;;
    --homebrew) PACKAGE_MODE=homebrew ;;
    --auto) AUTO_INSTALL=1 ;;
    *) usage ;;
  esac
  shift
done
# --auto replaces the running local install; a public homebrew build must not
# touch /Applications on the packaging machine.
if [[ "$AUTO_INSTALL" == 1 && "$PACKAGE_MODE" == homebrew ]]; then
  echo "error: --auto is only valid with the local LAN package mode" >&2
  exit 2
fi

TARGET_TRIPLE="aarch64-apple-darwin"
export MACOSX_DEPLOYMENT_TARGET=14.0
PACKAGE_NAME="unidesk-ffi"
BINARY_NAME="${BINARY_NAME:-unidesk-app}"
WORKSPACE_VERSION="$(python3 "$ROOT_DIR/scripts/release.py" version)"
APP_VERSION="${APP_VERSION:-$WORKSPACE_VERSION}"
[[ "$APP_VERSION" == "$WORKSPACE_VERSION" ]] || { echo "error: APP_VERSION differs from workspace" >&2; exit 1; }
# A shared release descriptor replaces the old Git-commit-count build number.
# The same descriptor is supplied to the Windows build for a paired release.
RELEASE_METADATA="${RELEASE_METADATA:-}"
if [[ -n "${BUILD_VERSION:-}" && -z "$RELEASE_METADATA" ]]; then
  echo "error: supply RELEASE_METADATA rather than an independent BUILD_VERSION" >&2
  exit 1
fi
[[ -z "${BUILD_VERSION:-}" ]] || { echo "error: use only RELEASE_METADATA, not BUILD_VERSION" >&2; exit 1; }
source "$ROOT_DIR/scripts/codesign-identity.sh"
source "$ROOT_DIR/scripts/macos-codesign.sh"
unidesk_resolve_signing_kind
# DIST_DIR is resolved after the build number exists so local packages land in
# dist/{version}-build{number}/. Explicit DIST_DIR, CI artifact layout, and local
# homebrew isolation stay unchanged.
# Linked worktrees share the primary checkout's local delivery directory.
# CI and an explicit DIST_DIR retain their existing output contracts.
LOCAL_PACKAGE_ROOT="$ROOT_DIR"
if COMMON_GIT_DIR="$(git -C "$ROOT_DIR" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" \
  && [[ "$COMMON_GIT_DIR" == */.git ]]; then
  LOCAL_PACKAGE_ROOT="$(dirname "$COMMON_GIT_DIR")"
fi
EXPLICIT_DIST_DIR="${DIST_DIR:-}"
EXPLICIT_DMG_PATH="${DMG_PATH:-}"
ASSET_DIR="$ROOT_DIR/resources/generated"

if [[ "$CODESIGN_IDENTITY" == "-" ]]; then
  echo "signing: ad-hoc (CODESIGN_IDENTITY=-)"
else
  echo "signing: $SIGNING_KIND ($CODESIGN_IDENTITY)"
fi
if [[ "$SIGNING_KIND" == "developer-id" ]]; then
  echo "codesign: hardened runtime and secure timestamp"
else
  echo "notarization: skipped (requires Developer ID Application)"
fi
if [[ "${UNIDESK_REQUIRE_NOTARIZE:-0}" == "1" ]]; then
  if [[ "$SIGNING_KIND" != "developer-id" ]]; then
    echo "error: UNIDESK_REQUIRE_NOTARIZE=1 requires a Developer ID Application identity" >&2
    exit 1
  fi
  if ! unidesk_notarize_credentials_ready; then
    echo "error: UNIDESK_REQUIRE_NOTARIZE=1 needs APPLE_API_KEY + APPLE_API_KEY_ID + APPLE_API_ISSUER, or APPLE_ID + APPLE_APP_SPECIFIC_PASSWORD + APPLE_TEAM_ID. See docs/NOTARIZE.md" >&2
    exit 1
  fi
fi

STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/unidesk-package.XXXXXX")"
DMG_SOURCE_DIR="$STAGING_DIR/dmg-root"
APP_BUNDLE="$DMG_SOURCE_DIR/uniDesk.app"
CONTENTS_DIR="$APP_BUNDLE/Contents"
RW_DMG="$STAGING_DIR/uniDesk-rw.dmg"
MOUNT_POINT="$(mktemp -d "${TMPDIR:-/tmp}/unidesk-dmg-layout.XXXXXX")"
ATTACHED=0
detach_dmg() {
  if [[ "$ATTACHED" -eq 1 ]]; then
    hdiutil detach "$MOUNT_POINT" -quiet || true
  fi
  rmdir "$MOUNT_POINT" 2>/dev/null || true
}
trap 'detach_dmg; rm -rf "$STAGING_DIR"' EXIT

# Allocate one build number before building either platform. Failed allocations
# are not reused, and the coordinator ledger is never sent to a build peer.
if [[ "$PACKAGE_MODE" == homebrew && -z "$RELEASE_METADATA" ]]; then
  RELEASE_METADATA="$ROOT_DIR/release.json"
fi
if [[ "$PACKAGE_MODE" == homebrew ]]; then
  echo "distribution: Homebrew/GitHub (update-channel=github)"
else
  echo "distribution: local LAN (update-channel=lan)"
fi
if [[ -z "$RELEASE_METADATA" ]]; then
  RELEASE_METADATA="$STAGING_DIR/release.json"
  "$ROOT_DIR/scripts/prepare-release.sh" --descriptor "$RELEASE_METADATA"
fi
python3 "$ROOT_DIR/scripts/release.py" validate --descriptor "$RELEASE_METADATA"
BUILD_VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["build_number"])' "$RELEASE_METADATA")"

if [[ -n "$EXPLICIT_DIST_DIR" ]]; then
  DIST_DIR="$EXPLICIT_DIST_DIR"
elif [[ "$PACKAGE_MODE" == homebrew && "${CI:-false}" != "true" ]]; then
  DIST_DIR="$LOCAL_PACKAGE_ROOT/dist/homebrew"
elif [[ "$PACKAGE_MODE" == homebrew && "${CI:-false}" == "true" ]]; then
  DIST_DIR="$ROOT_DIR/dist"
else
  DIST_DIR="$LOCAL_PACKAGE_ROOT/dist/${APP_VERSION}-build${BUILD_VERSION}"
fi
DMG_PATH="${EXPLICIT_DMG_PATH:-$DIST_DIR/uniDesk-arm64.dmg}"
mkdir -p "$DIST_DIR"
DMG_PATH="$(python3 -c 'import os,sys; print(os.path.abspath(sys.argv[1]))' "$DMG_PATH")"
DIST_DIR="$(cd "$DIST_DIR" && pwd)"
printf 'output directory: %s\n' "$DIST_DIR"

if [[ "$PACKAGE_MODE" == lan ]]; then
  if [[ -e "$DMG_PATH" ]]; then
    echo "error: package already exists: $DMG_PATH; unset RELEASE_METADATA to allocate a new build" >&2
    exit 1
  fi
  # Preserve the allocated identity even if compilation or signing fails.
  # The repository descriptor is reserved for prepared public releases.
  if [[ ! "$RELEASE_METADATA" -ef "$DIST_DIR/release.json" ]]; then
    cp "$RELEASE_METADATA" "$DIST_DIR/release.json"
  fi
  RELEASE_METADATA="$DIST_DIR/release.json"
  echo "release descriptor: $RELEASE_METADATA"
  echo "retry: RELEASE_METADATA=\"$RELEASE_METADATA\" ./scripts/package-macos.sh"
fi

"$ROOT_DIR/scripts/generate-assets.sh"

# The package script intentionally does not install a target or toolchain. A
# missing target should fail visibly rather than downloading toolchains during
# a release operation.
echo "building $PACKAGE_NAME for $TARGET_TRIPLE"
export UNIDESK_INPUT_BUILD_ID="${APP_VERSION}-${BUILD_VERSION}"
"$ROOT_DIR/scripts/build-native-macos.sh" release
"$ROOT_DIR/scripts/cargo-macos.sh" build --release --locked --package unidesk-update --bin unidesk-update-helper --target "$TARGET_TRIPLE"

CARGO_OUTPUT_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
BINARY_PATH="$CARGO_OUTPUT_DIR/native-macos/release/$BINARY_NAME"
if [[ ! -x "$BINARY_PATH" ]]; then
  echo "error: expected binary not found or not executable: $BINARY_PATH" >&2
  exit 1
fi

ARCHES="$(lipo -archs "$BINARY_PATH")"
if [[ "$ARCHES" != "arm64" ]]; then
  echo "error: expected an arm64-only binary, got: $ARCHES" >&2
  exit 1
fi

mkdir -p "$CONTENTS_DIR/MacOS" "$CONTENTS_DIR/Resources" "$DMG_SOURCE_DIR/.background"
cp "$BINARY_PATH" "$CONTENTS_DIR/MacOS/$BINARY_NAME"
ditto "$(dirname "$BINARY_PATH")/Metadata.appintents" "$CONTENTS_DIR/Resources/Metadata.appintents"
cp "$CARGO_OUTPUT_DIR/native-macos/release/unidesk-input-service" "$CONTENTS_DIR/MacOS/unidesk-input-service"
unidesk_codesign --identifier com.unidesk.app.input-service "$CONTENTS_DIR/MacOS/unidesk-input-service"
cp "$CARGO_OUTPUT_DIR/$TARGET_TRIPLE/release/unidesk-update-helper" "$CONTENTS_DIR/MacOS/unidesk-update-helper"
unidesk_codesign "$CONTENTS_DIR/MacOS/unidesk-update-helper"
cp "$RELEASE_METADATA" "$CONTENTS_DIR/Resources/release.json"
if [[ "$PACKAGE_MODE" == homebrew ]]; then
  printf 'github\n' > "$CONTENTS_DIR/Resources/update-channel"
else
  printf 'lan\n' > "$CONTENTS_DIR/Resources/update-channel"
fi
chmod 0755 "$CONTENTS_DIR/MacOS/$BINARY_NAME"
cp "$ROOT_DIR/LICENSE" "$ROOT_DIR/LICENSE-MIT" "$ROOT_DIR/LICENSE-APACHE" "$ROOT_DIR/THIRD_PARTY_NOTICES.md" "$CONTENTS_DIR/Resources/"
cp "$ROOT_DIR/resources/icons/LICENSE" "$CONTENTS_DIR/Resources/LICENSE-LUCIDE"
python3 "$ROOT_DIR/scripts/third-party-notices.py" --output "$CONTENTS_DIR/Resources/THIRD_PARTY_DEPENDENCIES.txt"
# Ship the editable vector sources alongside the native macOS representations.
cp "$ROOT_DIR/resources/uniDesk-icon.svg" "$CONTENTS_DIR/Resources/uniDesk-icon.svg"
cp "$ROOT_DIR/resources/unidesk-status-icon.svg" "$CONTENTS_DIR/Resources/unidesk-status-icon.svg"
cp "$ASSET_DIR/uniDesk.icns" "$CONTENTS_DIR/Resources/uniDesk.icns"
cp "$ASSET_DIR/uniDesk-icon-1024.png" "$CONTENTS_DIR/Resources/uniDesk-icon-1024.png"
cp "$ASSET_DIR/unidesk-status-icon.png" "$CONTENTS_DIR/Resources/unidesk-status-icon.png"
cp "$ASSET_DIR/dmg-background.png" "$DMG_SOURCE_DIR/.background/installer-background.png"
ln -s /Applications "$DMG_SOURCE_DIR/Applications"

sed \
  -e "s/@VERSION@/$APP_VERSION/g" \
  -e "s/@BUILD_VERSION@/$BUILD_VERSION/g" \
  "$ROOT_DIR/resources/Info.plist.in" > "$CONTENTS_DIR/Info.plist"
unidesk_codesign \
  --deep \
  --entitlements "$ROOT_DIR/resources/uniDesk.entitlements" \
  "$APP_BUNDLE"

codesign --verify --deep --strict "$APP_BUNDLE"

# Preserve the exact signed bundle for later release signing and LAN forwarding.
UPDATE_ZIP="$STAGING_DIR/uniDesk-arm64-update.zip"
ditto -c -k --sequesterRsrc --keepParent "$APP_BUNDLE" "$UPDATE_ZIP"

# Release authorization is independent of pairing and platform code signing.
# CI can ship a homebrew DMG without the private LAN release key; it cannot
# publish an automatically installable release until the coordinator signs it.
SIGNED_UPDATE=0
RELEASE_KEY="${UNIDESK_RELEASE_KEY:-$HOME/Library/Application Support/uniDesk-release/signing-key.hex}"
if [[ "$PACKAGE_MODE" == lan && -f "$RELEASE_KEY" ]]; then
  python3 - "$RELEASE_METADATA" "$UPDATE_ZIP" "$STAGING_DIR/unsigned-manifest.json" <<'PYMANIFEST'
import hashlib,json,pathlib,sys
release=json.loads(pathlib.Path(sys.argv[1]).read_text())
payload=pathlib.Path(sys.argv[2])
manifest={"schema_version":1,"application_id":"com.unidesk.app","release":release,"protocol_major":1,"minimum_update_protocol":1,"artifacts":[{"target":"macos_arm64","kind":"macos_app_zip","file_name":payload.name,"size":payload.stat().st_size,"sha256":hashlib.file_digest(payload.open("rb"),"sha256").hexdigest(),"minimum_os":"14.0.0"}]}
pathlib.Path(sys.argv[3]).write_text(json.dumps(manifest))
PYMANIFEST
  cargo run --locked --quiet -p unidesk-update --bin unidesk-release -- sign "$RELEASE_KEY" "$STAGING_DIR/unsigned-manifest.json" "$STAGING_DIR" "$STAGING_DIR/manifest.json"
  cargo run --locked --quiet -p unidesk-update --bin unidesk-release -- verify "$ROOT_DIR/resources/unidesk-release-public-key.hex" "$STAGING_DIR/manifest.json" "$STAGING_DIR"
  mkdir -p "$DMG_SOURCE_DIR/.updates"
  cp "$UPDATE_ZIP" "$STAGING_DIR/manifest.json" "$DMG_SOURCE_DIR/.updates/"
  # Finder copies only the .app. Carry the exact signed payload with that copy
  # so a manually installed peer can serve updates after ejecting its DMG.
  # The payload is the already-signed base app, not this distribution wrapper:
  # embedding the wrapper in its own ZIP would create a recursive artifact.
  mkdir -p "$CONTENTS_DIR/Resources/LanUpdate"
  cp "$UPDATE_ZIP" "$STAGING_DIR/manifest.json" "$CONTENTS_DIR/Resources/LanUpdate/"
  unidesk_codesign \
    --deep \
    --entitlements "$ROOT_DIR/resources/uniDesk.entitlements" \
    "$APP_BUNDLE"
  codesign --verify --deep --strict "$APP_BUNDLE"
  SIGNED_UPDATE=1
elif [[ "$PACKAGE_MODE" == lan ]]; then
  echo "error: LAN release signing key unavailable: $RELEASE_KEY" >&2
  exit 1
fi


# Build a writable HFS+ image first: Finder persists icon positions and the
# custom background in .DS_Store, which is then compressed for distribution.
# Versioned volume names avoid Finder alias collisions with an older mounted installer.
hdiutil create \
  -volname "uniDesk $APP_VERSION ($BUILD_VERSION)" \
  -srcfolder "$DMG_SOURCE_DIR" \
  -ov \
  -format UDRW \
  "$RW_DMG" >/dev/null

# Finder must see this temporary writable volume to persist .DS_Store layout.
# The final compressed image is not auto-opened by this script.
hdiutil attach "$RW_DMG" -noverify -readwrite -mountpoint "$MOUNT_POINT" >/dev/null
ATTACHED=1

# Headless/agent packaging can avoid controlling Finder. The app and
# Applications link still work; only the custom Finder layout is omitted.
if [[ "${UNIDESK_DMG_FINDER_LAYOUT:-1}" != "0" ]]; then
osascript - "$MOUNT_POINT" "$MOUNT_POINT/.background/installer-background.png" <<'APPLESCRIPT'
on run argv
  set mounted_folder to POSIX file (item 1 of argv) as alias
  set background_file to POSIX file (item 2 of argv) as alias
  tell application "Finder"
    -- Keep the created window's identity; Finder preferences or another
    -- user window may be frontmost during packaging.
    set installer_window to make new Finder window
    set target of installer_window to mounted_folder
    activate
    delay 1
    set current view of installer_window to icon view
    set toolbar visible of installer_window to false
    set statusbar visible of installer_window to false
    set bounds of installer_window to {120, 120, 1040, 680}
    set viewOptions to the icon view options of installer_window
    set arrangement of viewOptions to not arranged
    set icon size of viewOptions to 116
    set text size of viewOptions to 14
    set background picture of viewOptions to background_file
    set position of item "uniDesk.app" of installer_window to {250, 280}
    set position of item "Applications" of installer_window to {670, 280}
    update mounted_folder without registering applications
    delay 3
    close installer_window
  end tell
end run
APPLESCRIPT
# Finder writes layout asynchronously. Do not unmount or publish an image
# before its layout exists, and never replace a deliverable with a partial one.
for ((layout_wait = 0; layout_wait < 20; layout_wait++)); do
  [[ -s "$MOUNT_POINT/.DS_Store" ]] && break
  sleep 1
done
if [[ ! -s "$MOUNT_POINT/.DS_Store" ]]; then
  echo "error: Finder did not persist DMG layout; use UNIDESK_DMG_FINDER_LAYOUT=0 for an explicitly headless package" >&2
  exit 1
fi
fi

sync
hdiutil detach "$MOUNT_POINT" -quiet
ATTACHED=0

# Validate before replacing an existing deliverable.
FINAL_DMG="$STAGING_DIR/uniDesk-arm64.dmg"
hdiutil convert "$RW_DMG" -ov -format UDZO -o "$FINAL_DMG" >/dev/null
if [[ "$SIGNING_KIND" == "developer-id" ]]; then
  codesign --force --timestamp --sign "$CODESIGN_IDENTITY" "$FINAL_DMG"
fi
# Staple is attached after Apple returns a ticket; the first pass only checks
# the signed app. A required notarization re-runs the verifier afterwards.
UNIDESK_REQUIRE_NOTARIZE=0 "$ROOT_DIR/scripts/verify-dmg.sh" "$FINAL_DMG"
NOTARIZED=false
if [[ "$SIGNING_KIND" == "developer-id" ]] && unidesk_notarize_credentials_ready; then
  "$ROOT_DIR/scripts/notarize-macos.sh" "$FINAL_DMG"
  NOTARIZED=true
  "$ROOT_DIR/scripts/verify-dmg.sh" "$FINAL_DMG"
else
  echo "notarization: not performed"
fi
unidesk_write_signing_status "$DIST_DIR" "$NOTARIZED"
mv "$FINAL_DMG" "$DMG_PATH"
mv "$UPDATE_ZIP" "$DIST_DIR/uniDesk-arm64-update.zip"
if [[ "$SIGNED_UPDATE" == "1" ]]; then
  cp "$STAGING_DIR/manifest.json" "$DIST_DIR/manifest.json"
else
  # Never leave a prior signed manifest next to an unrelated homebrew payload.
  rm -f "$DIST_DIR/manifest.json"
fi
if [[ ! "$RELEASE_METADATA" -ef "$DIST_DIR/release.json" ]]; then
  cp "$RELEASE_METADATA" "$DIST_DIR/release.json"
fi
# Refresh checksums with every successful package, including custom DMG paths.
(
  cd "$DIST_DIR"
  shasum -a 256 "${DMG_PATH#"$DIST_DIR/"}" uniDesk-arm64-update.zip release.json
  if [[ "$SIGNED_UPDATE" == "1" ]]; then shasum -a 256 manifest.json; fi
) > "$STAGING_DIR/SHA256SUMS"
mv "$STAGING_DIR/SHA256SUMS" "$DIST_DIR/SHA256SUMS"
printf 'release: %s build %s\n' "$APP_VERSION" "$BUILD_VERSION"
printf 'update payload: %s\n' "$DIST_DIR/uniDesk-arm64-update.zip"
printf 'created: %s\n' "$DMG_PATH"
shasum -a 256 "$DMG_PATH"

# Seed the running coordinator only after the full package passes verification.
# Move a complete sidecar directory; readers never observe a partial manifest.
if [[ "$SIGNED_UPDATE" == "1" && "${CI:-false}" != "true" ]]; then
  UPDATE_ROOT="$HOME/Library/Application Support/uniDesk/updates"
  mkdir -p "$UPDATE_ROOT"
  chmod 700 "$UPDATE_ROOT"
  SEED_STAGE="$(mktemp -d "$UPDATE_ROOT/.seed-XXXXXX")"
  cp "$DIST_DIR/uniDesk-arm64-update.zip" "$DIST_DIR/manifest.json" "$SEED_STAGE/"
  python3 - "$SEED_STAGE" "$UPDATE_ROOT/inbox" <<'PYSEED'
import pathlib,sys,shutil
stage,destination=map(pathlib.Path,sys.argv[1:])
previous=destination.with_name(".inbox-previous")
if previous.exists(): shutil.rmtree(previous)
if destination.exists(): destination.rename(previous)
stage.rename(destination)
if previous.exists(): shutil.rmtree(previous)
PYSEED
  echo "LAN update seed: $UPDATE_ROOT/inbox"
fi

# --auto: replace the running instance with the freshly packaged app. Only the
# LAN local-delivery flow reaches this point; homebrew mode rejects --auto.
if [[ "$AUTO_INSTALL" == 1 ]]; then
  echo "auto install: stopping running uniDesk processes"
  pkill -x "$BINARY_NAME" 2>/dev/null || true
  pkill -x "unidesk-input-service" 2>/dev/null || true
  # Give the app a moment to release the input-service child and file handles.
  for ((wait_round = 0; wait_round < 20; wait_round++)); do
    pgrep -x "$BINARY_NAME" >/dev/null 2>&1 || break
    sleep 0.5
  done
  INSTALL_PATH="/Applications/uniDesk.app"
  rm -rf "$INSTALL_PATH"
  ditto "$APP_BUNDLE" "$INSTALL_PATH"
  codesign --verify --deep --strict "$INSTALL_PATH"
  echo "auto install: $INSTALL_PATH"
  open "$INSTALL_PATH"
fi
