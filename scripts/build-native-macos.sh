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
# Swift Build can incorrectly stamp the deployment target as the linked SDK.
# Pass the *actual selected SDK* to both compilation and the real linker, so
# AppKit opts into modern system chrome while the minimum OS remains 14.0.
SWIFT_FLAGS=(--build-system swiftbuild --package-path "$ROOT_DIR/apps/unidesk-macos" --configuration "$CONFIGURATION"
  --triple arm64-apple-macosx14.0 --sdk "$SDK_PATH"
  -Xlinker -platform_version -Xlinker macos -Xlinker "$MACOSX_DEPLOYMENT_TARGET" -Xlinker "$SDK_VERSION")
TARGET_TRIPLE=aarch64-apple-darwin
export UNIDESK_INPUT_BUILD_ID="${UNIDESK_INPUT_BUILD_ID:-dev-$(git rev-parse --short=12 HEAD)}"
RELEASE_FLAG=""
if [[ "$CONFIGURATION" == release ]]; then RELEASE_FLAG="--release"; fi
RUST_OUTPUT="${CARGO_TARGET_DIR:-$ROOT_DIR/target}/$TARGET_TRIPLE/$CONFIGURATION"
SDK_DIR="$ROOT_DIR/apps/unidesk-macos/SDK"
mkdir -p "$SDK_DIR" apps/unidesk-macos/Generated apps/unidesk-macos/Sources/UniDeskEngine apps/unidesk-macos/Sources/UniDeskEngineFFI
RUST_FINGERPRINT="$(python3 - "$ROOT_DIR" <<'PY'
import hashlib, pathlib, sys
root = pathlib.Path(sys.argv[1])
hasher = hashlib.sha256()
for relative in ("Cargo.lock", "Cargo.toml", "rust-toolchain.toml"):
    hasher.update(relative.encode())
    hasher.update((root / relative).read_bytes())
for folder in (
    "crates", "extensions", "tools",
    "apps/unidesk-service", "apps/unidesk-probe",
    "apps/unidesk-input-macos", "apps/unidesk-input-windows",
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
FINGERPRINT_FILE="$SDK_DIR/.rust-fingerprint"
SKIP_RUST=0
if [[ "${UNIDESK_FORCE_RUST:-0}" != 1 \
  && -f "$FINGERPRINT_FILE" \
  && "$(cat "$FINGERPRINT_FILE")" == "$RUST_FINGERPRINT" \
  && -f "$RUST_OUTPUT/libunidesk_ffi.dylib" \
  && -f "$RUST_OUTPUT/libunidesk_ffi.a" \
  && -f "$RUST_OUTPUT/unidesk-input-service" \
  && -f "$ROOT_DIR/apps/unidesk-macos/Sources/UniDeskEngine/UniDeskEngine.swift" ]]; then
  SKIP_RUST=1
  echo "Rust SDK cache hit ($CONFIGURATION $TARGET_TRIPLE)"
fi
if [[ "$SKIP_RUST" -eq 0 ]]; then
  "$ROOT_DIR/scripts/cargo-macos.sh" build --locked -p unidesk-ffi --lib --target "$TARGET_TRIPLE" $RELEASE_FLAG
  "$ROOT_DIR/scripts/cargo-macos.sh" build --locked -p unidesk-engine --bin unidesk-input-service --target "$TARGET_TRIPLE" $RELEASE_FLAG
  cargo run --locked -p unidesk-ffi --bin unidesk-bindgen -- generate --library "$RUST_OUTPUT/libunidesk_ffi.dylib" --language swift --out-dir "$ROOT_DIR/apps/unidesk-macos/Generated"
  cp apps/unidesk-macos/Generated/UniDeskEngine.swift apps/unidesk-macos/Sources/UniDeskEngine/
  cp apps/unidesk-macos/Generated/UniDeskEngineFFI.h apps/unidesk-macos/Sources/UniDeskEngineFFI/
  cp apps/unidesk-macos/Generated/UniDeskEngineFFI.modulemap apps/unidesk-macos/Sources/UniDeskEngineFFI/module.modulemap
  printf '%s\n' "$RUST_FINGERPRINT" > "$FINGERPRINT_FILE"
fi
UNIDESK_RUST_LIBRARY_DIR="$RUST_OUTPUT" swift build "${SWIFT_FLAGS[@]}"

SWIFT_OUTPUT="$(UNIDESK_RUST_LIBRARY_DIR="$RUST_OUTPUT" swift build "${SWIFT_FLAGS[@]}" --show-bin-path)"
DESTINATION="${CARGO_TARGET_DIR:-$ROOT_DIR/target}/native-macos/$CONFIGURATION"
mkdir -p "$DESTINATION"
cp "$SWIFT_OUTPUT/unidesk-app" "$DESTINATION/unidesk-app"
cp "$RUST_OUTPUT/unidesk-input-service" "$DESTINATION/unidesk-input-service"
LINKED_SDK="$(xcrun vtool -show-build "$DESTINATION/unidesk-app" | awk '$1 == "sdk" { print $2; exit }')"
[[ "${LINKED_SDK%%.*}" -ge 26 ]] || { echo "error: native client linked with unexpected SDK $LINKED_SDK" >&2; exit 1; }
echo "native executable: $DESTINATION/unidesk-app (minimum macOS 14.0; linked SDK $LINKED_SDK)"

# SwiftPM compiles AppIntent constant values but does not assemble our hand-built
# .app bundle. Extract real metadata from the same compilation, without forcing
# output or suppressing processor errors.
PROCESSOR="$(xcrun --find appintentsmetadataprocessor)"
CONFIGURATION_TITLE=Debug
[[ "$CONFIGURATION" != release ]] || CONFIGURATION_TITLE=Release
APP_OBJECTS="$ROOT_DIR/apps/unidesk-macos/.build/out/Intermediates.noindex/uniDesk.build/$CONFIGURATION_TITLE/unidesk-app-p.build/Objects-normal/arm64"
SOURCE_LIST="$APP_OBJECTS/unidesk-app.SwiftFileList"
OUTPUT_MAP="$APP_OBJECTS/unidesk-app-OutputFileMap.json"
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
  --module-name UniDesk --sdk-root "$SDK_PATH" \
  --xcode-version "$(xcodebuild -version | awk '/Build version/ {print $3}')" \
  --platform-family macOS --deployment-target "$MACOSX_DEPLOYMENT_TARGET" \
  --target-triple arm64-apple-macosx14.0 \
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
