#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
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
RELEASE_FLAG=""
if [[ "$CONFIGURATION" == release ]]; then RELEASE_FLAG="--release"; fi
RUST_OUTPUT="${CARGO_TARGET_DIR:-$ROOT_DIR/target}/$TARGET_TRIPLE/$CONFIGURATION"
SDK_DIR="$ROOT_DIR/apps/deskfuse-macos/SDK"
mkdir -p "$SDK_DIR" apps/deskfuse-macos/Generated apps/deskfuse-macos/Sources/DeskFuseEngine apps/deskfuse-macos/Sources/DeskFuseEngineFFI
RUST_FINGERPRINT="$(python3 - "$ROOT_DIR" <<'PY'
import hashlib, os, pathlib, sys
root = pathlib.Path(sys.argv[1])
hasher = hashlib.sha256()
hasher.update(str(root.resolve()).encode())
# Both the parent and input helper embed this ID. A new package must not reuse
# binaries carrying a different bootstrap/diagnostic build identity.
hasher.update(os.environ["DESKFUSE_INPUT_BUILD_ID"].encode())
for relative in ("Cargo.lock", "Cargo.toml", "rust-toolchain.toml",
                 "resources/deskfuse-transport-cert.der",
                 "resources/deskfuse-transport-key.der",
                 "resources/deskfuse-release-public-key.hex"):
    hasher.update(relative.encode())
    hasher.update((root / relative).read_bytes())
for path in sorted((root / "config").rglob("*.json")):
    hasher.update(str(path.relative_to(root)).encode())
    hasher.update(path.read_bytes())
for folder in (
    "crates", "extensions", "tools",
    "apps/deskfuse-service", "apps/deskfuse-probe",
    "apps/deskfuse-input-macos", "apps/deskfuse-input-windows",
):
    base = root / folder
    if not base.exists():
        continue
    for path in sorted(base.rglob("*")):
        if path.suffix in {".rs", ".toml", ".sql"} and path.is_file():
            hasher.update(str(path.relative_to(root)).encode())
            hasher.update(path.read_bytes())
print(hasher.hexdigest())
PY
)"
# Each target/profile owns its cache evidence. A successful arm64 build must
# never validate stale Intel libraries left by an earlier source revision.
FINGERPRINT_FILE="$SDK_DIR/.rust-fingerprint-$TARGET_TRIPLE-$CONFIGURATION"
SKIP_RUST=0
if [[ "${DESKFUSE_FORCE_RUST:-0}" != 1 \
  && -f "$FINGERPRINT_FILE" \
  && "$(cat "$FINGERPRINT_FILE")" == "$RUST_FINGERPRINT" \
  && -f "$RUST_OUTPUT/libdeskfuse_ffi.dylib" \
  && -f "$RUST_OUTPUT/libdeskfuse_ffi.a" \
  && -f "$RUST_OUTPUT/deskfuse-input-service" \
  && -f "$ROOT_DIR/apps/deskfuse-macos/Sources/DeskFuseEngine/DeskFuseEngine.swift" ]]; then
  SKIP_RUST=1
  echo "Rust SDK cache hit ($CONFIGURATION $TARGET_TRIPLE)"
fi
if [[ "$SKIP_RUST" -eq 0 ]]; then
  "$ROOT_DIR/scripts/cargo-macos.sh" build --locked -p deskfuse-ffi --lib --target "$TARGET_TRIPLE" $RELEASE_FLAG
  "$ROOT_DIR/scripts/cargo-macos.sh" build --locked -p deskfuse-engine --bin deskfuse-input-service --target "$TARGET_TRIPLE" $RELEASE_FLAG
  cargo run --locked -p deskfuse-ffi --bin deskfuse-bindgen -- generate --library "$RUST_OUTPUT/libdeskfuse_ffi.dylib" --language swift --out-dir "$ROOT_DIR/apps/deskfuse-macos/Generated"
  cp apps/deskfuse-macos/Generated/DeskFuseEngine.swift apps/deskfuse-macos/Sources/DeskFuseEngine/
  cp apps/deskfuse-macos/Generated/DeskFuseEngineFFI.h apps/deskfuse-macos/Sources/DeskFuseEngineFFI/
  cp apps/deskfuse-macos/Generated/DeskFuseEngineFFI.modulemap apps/deskfuse-macos/Sources/DeskFuseEngineFFI/module.modulemap
  printf '%s\n' "$RUST_FINGERPRINT" > "$FINGERPRINT_FILE"
fi
DESKFUSE_RUST_LIBRARY_DIR="$RUST_OUTPUT" swift build "${SWIFT_FLAGS[@]}"

SWIFT_OUTPUT="$(DESKFUSE_RUST_LIBRARY_DIR="$RUST_OUTPUT" swift build "${SWIFT_FLAGS[@]}" --show-bin-path)"
DESTINATION="${CARGO_TARGET_DIR:-$ROOT_DIR/target}/native-macos/$CONFIGURATION"
mkdir -p "$DESTINATION"
cp "$SWIFT_OUTPUT/deskfuse-app" "$DESTINATION/deskfuse-app"
cp "$RUST_OUTPUT/deskfuse-input-service" "$DESTINATION/deskfuse-input-service"
LINKED_SDK="$(xcrun vtool -show-build "$DESTINATION/deskfuse-app" | awk '$1 == "sdk" { print $2; exit }')"
[[ "${LINKED_SDK%%.*}" -ge 26 ]] || { echo "error: native client linked with unexpected SDK $LINKED_SDK" >&2; exit 1; }
echo "native executable: $DESTINATION/deskfuse-app (minimum macOS 14.0; linked SDK $LINKED_SDK)"

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
