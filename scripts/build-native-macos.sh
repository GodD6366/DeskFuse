#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/rust-toolchain-env.sh"
# New SwiftUI SDKs require the platform macro plugins bundled with full Xcode.
# Select a toolchain for this process only; never change xcode-select globally.
if [[ -z "${DEVELOPER_DIR:-}" && "$(xcode-select -p)" == */CommandLineTools ]]; then
  for candidate in /Applications/Xcode.app/Contents/Developer /Applications/Xcode-beta.app/Contents/Developer; do
    if [[ -d "$candidate" ]]; then export DEVELOPER_DIR="$candidate"; break; fi
  done
fi
CONFIGURATION="${1:-debug}"
[[ "$CONFIGURATION" == debug || "$CONFIGURATION" == release ]] || { echo "expected debug or release" >&2; exit 2; }
export MACOSX_DEPLOYMENT_TARGET=14.0
SDK_PATH="$(xcrun --sdk macosx --show-sdk-path)"
SDK_VERSION="$(xcrun --sdk macosx --show-sdk-version)"
[[ "${SDK_VERSION%%.*}" -ge 27 ]] || { echo "error: full Xcode 27+ with macOS 27+ SDK is required by the current native source (runtime minimum remains macOS 14)" >&2; exit 1; }
MACOS_ARCH="${DESKFUSE_MACOS_ARCH:-arm64}"
case "$MACOS_ARCH" in
  arm64)
    SWIFT_TRIPLE="arm64-apple-macosx14.0"
    TARGET_TRIPLE="aarch64-apple-darwin"
    APP_OBJECTS_ARCH="arm64"
    ;;
  x86_64)
    SWIFT_TRIPLE="x86_64-apple-macosx14.0"
    TARGET_TRIPLE="x86_64-apple-darwin"
    APP_OBJECTS_ARCH="x86_64"
    ;;
  *)
    echo "error: DESKFUSE_MACOS_ARCH must be arm64 or x86_64" >&2
    exit 2
    ;;
esac
# Swift Build can incorrectly stamp the deployment target as the linked SDK.
# Pass the *actual selected SDK* to both compilation and the real linker, so
# AppKit opts into modern system chrome while the minimum OS remains 14.0.
SWIFT_FLAGS=(--build-system swiftbuild --package-path "$ROOT_DIR/apps/deskfuse-macos" --configuration "$CONFIGURATION"
  --triple "$SWIFT_TRIPLE" --sdk "$SDK_PATH"
  -Xlinker -platform_version -Xlinker macos -Xlinker "$MACOSX_DEPLOYMENT_TARGET" -Xlinker "$SDK_VERSION")
export DESKFUSE_INPUT_BUILD_ID="${DESKFUSE_INPUT_BUILD_ID:-dev-$(git rev-parse --short=12 HEAD)}"
DESKFUSE_MACOS_ARCH="$MACOS_ARCH" "$ROOT_DIR/scripts/build-freerdp-macos.sh"
# Rust builds and binding generation can also run independently of Swift.
"$ROOT_DIR/scripts/build-rust-macos.sh" "$CONFIGURATION"
TARGET_DIR="$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
RUST_OUTPUT="$TARGET_DIR/$TARGET_TRIPLE/$CONFIGURATION"
SDK_DIR="$ROOT_DIR/apps/deskfuse-macos/SDK"
write_build_identity() {
  printf '%s\n' "$DESKFUSE_INPUT_BUILD_ID" > "$1"
}

# Swift 指纹覆盖源码、绑定和静态库内容。构建号变化不在其中。
SWIFT_VERSION="$(swift --version | awk 'NR==1 { print; exit }')"
XCODE_VERSION="$(xcodebuild -version | tr '\n' ' ')"
export DESKFUSE_SWIFT_CONFIGURATION="$CONFIGURATION"
export DESKFUSE_SWIFT_TRIPLE="$SWIFT_TRIPLE"
export DESKFUSE_SWIFT_SDK_VERSION="$SDK_VERSION"
export DESKFUSE_SWIFT_SDK_PATH="$SDK_PATH"
export DESKFUSE_SWIFT_VERSION="$SWIFT_VERSION"
export DESKFUSE_XCODE_VERSION="$XCODE_VERSION"
export DESKFUSE_STATIC_LIBRARY="$RUST_OUTPUT/libdeskfuse_ffi.a"
SWIFT_FINGERPRINT="$(python3 - "$ROOT_DIR" <<'PY'
import hashlib, os, pathlib, sys
root = pathlib.Path(sys.argv[1])
package = root / "apps/deskfuse-macos"
hasher = hashlib.sha256()
for key in (
    "DESKFUSE_SWIFT_CONFIGURATION",
    "DESKFUSE_SWIFT_TRIPLE",
    "MACOSX_DEPLOYMENT_TARGET",
    "DESKFUSE_SWIFT_SDK_VERSION",
    "DESKFUSE_SWIFT_SDK_PATH",
    "DESKFUSE_SWIFT_VERSION",
    "DESKFUSE_XCODE_VERSION",
):
    hasher.update(key.encode())
    hasher.update(os.environ[key].encode())
for relative in ("Package.swift", "Package.resolved", "RemoteApplication/Vendor/FreeRDP/.fingerprint"):
    path = package / relative
    if path.is_file():
        hasher.update(relative.encode())
        hasher.update(path.read_bytes())
suffixes = {".swift", ".h", ".modulemap"}
for folder in ("Host", "Sources", "RemoteApplication/Sources/DeskFuseRemoteApplication"):
    base = package / folder
    if not base.exists():
        continue
    for path in sorted(base.rglob("*")):
        if path.is_file() and path.suffix in suffixes and ".build" not in path.parts:
            hasher.update(str(path.relative_to(package)).encode())
            hasher.update(path.read_bytes())
library = pathlib.Path(os.environ["DESKFUSE_STATIC_LIBRARY"])
hasher.update(b"staticlib")
with library.open("rb") as handle:
    for chunk in iter(lambda: handle.read(1024 * 1024), b""):
        hasher.update(chunk)
print(hasher.hexdigest())
PY
)"
SWIFT_FINGERPRINT_FILE="$SDK_DIR/.swift-fingerprint-$TARGET_TRIPLE-$CONFIGURATION"
SWIFT_BIN_PATH_FILE="$SDK_DIR/.swift-bin-path-$TARGET_TRIPLE-$CONFIGURATION"
DESTINATION="$TARGET_DIR/native-macos/$CONFIGURATION"
mkdir -p "$DESTINATION"
METADATA_ACTIONS="$DESTINATION/Metadata.appintents/extract.actionsdata"
SKIP_SWIFT=0
if [[ "${DESKFUSE_FORCE_SWIFT:-0}" != 1 \
  && -f "$SWIFT_FINGERPRINT_FILE" \
  && "$(cat "$SWIFT_FINGERPRINT_FILE")" == "$SWIFT_FINGERPRINT" \
  && -f "$SWIFT_BIN_PATH_FILE" \
  && -x "$(cat "$SWIFT_BIN_PATH_FILE")/deskfuse-app" \
  && -s "$METADATA_ACTIONS" ]]; then
  SKIP_SWIFT=1
  SWIFT_OUTPUT="$(cat "$SWIFT_BIN_PATH_FILE")"
  echo "Swift cache hit ($CONFIGURATION $TARGET_TRIPLE)"
fi
if [[ "$SKIP_SWIFT" -eq 0 ]]; then
DESKFUSE_RUST_LIBRARY_DIR="$RUST_OUTPUT" swift build "${SWIFT_FLAGS[@]}"
SWIFT_OUTPUT="$(DESKFUSE_RUST_LIBRARY_DIR="$RUST_OUTPUT" swift build "${SWIFT_FLAGS[@]}" --show-bin-path)"
fi
cp "$SWIFT_OUTPUT/deskfuse-app" "$DESTINATION/deskfuse-app"
cp "$RUST_OUTPUT/deskfuse-input-service" "$DESTINATION/deskfuse-input-service"
write_build_identity "$DESTINATION/build-id"
LINKED_SDK="$(xcrun vtool -show-build "$DESTINATION/deskfuse-app" | awk '$1 == "sdk" { print $2; exit }')"
[[ "${LINKED_SDK%%.*}" -ge 26 ]] || { echo "error: native client linked with unexpected SDK $LINKED_SDK" >&2; exit 1; }
echo "native executable: $DESTINATION/deskfuse-app (minimum macOS 14.0; linked SDK $LINKED_SDK)"
if [[ "$SKIP_SWIFT" -eq 0 ]]; then

# SwiftPM compiles AppIntent constant values but does not assemble our hand-built
# .app bundle. Extract real metadata from the same compilation, without forcing
# output or suppressing processor errors.
PROCESSOR="$(xcrun --find appintentsmetadataprocessor)"
CONFIGURATION_TITLE=Debug
[[ "$CONFIGURATION" != release ]] || CONFIGURATION_TITLE=Release
APP_OBJECTS="$ROOT_DIR/apps/deskfuse-macos/.build/out/Intermediates.noindex/DeskFuse.build/$CONFIGURATION_TITLE/deskfuse-app-p.build/Objects-normal/$APP_OBJECTS_ARCH"
SOURCE_LIST="$APP_OBJECTS/deskfuse-app.SwiftFileList"
OUTPUT_MAP="$APP_OBJECTS/deskfuse-app-OutputFileMap.json"
CONST_LIST="$DESTINATION/app-intents-const-values.list"
python3 - "$OUTPUT_MAP" "$CONST_LIST" <<'PYCONST'
import json, pathlib, sys
mapping = json.loads(pathlib.Path(sys.argv[1]).read_text())
paths = sorted({value["const-values"] for value in mapping.values()
                if "const-values" in value and pathlib.Path(value["const-values"]).is_file()})
if not paths:
    raise SystemExit("error: Swift did not emit App Intents constant-value metadata")
pathlib.Path(sys.argv[2]).write_text("\n".join(paths) + "\n")
PYCONST
[[ -s "$SOURCE_LIST" ]] || { echo "error: App Intents source list missing" >&2; exit 1; }
# Remove only the prior generated metadata, so stale output cannot mask failure.
rm -rf "$DESTINATION/Metadata.appintents"
"$PROCESSOR" --output "$DESTINATION" \
  --toolchain-dir "$(dirname "$(dirname "$PROCESSOR")")" \
  --module-name DeskFuse --sdk-root "$SDK_PATH" \
  --xcode-version "$(xcodebuild -version | awk '/Build version/ {print $3}')" \
  --platform-family macOS --deployment-target "$MACOSX_DEPLOYMENT_TARGET" \
  --target-triple "$SWIFT_TRIPLE" \
  --source-file-list "$SOURCE_LIST" --swift-const-vals-list "$CONST_LIST" \
  --deployment-aware-processing
python3 - "$DESTINATION/Metadata.appintents" <<'PYINTENTS'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
actions = root / "extract.actionsdata"
if not actions.is_file() or not actions.stat().st_size:
    raise SystemExit("error: App Intents actions metadata missing")
payload = json.loads(actions.read_text())
text = json.dumps(payload, ensure_ascii=False)
if "RunWorkflowIntent" not in text or "WorkflowEntity" not in text:
    raise SystemExit("error: expected workflow intent/entity absent from metadata")
print("App Intents metadata: " + str(root))
PYINTENTS
printf '%s\n' "$SWIFT_FINGERPRINT" > "$SWIFT_FINGERPRINT_FILE"
printf '%s\n' "$SWIFT_OUTPUT" > "$SWIFT_BIN_PATH_FILE"
fi
