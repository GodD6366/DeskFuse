#!/usr/bin/env bash
# Build only. Never installs a HAL plugin or restarts coreaudiod.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -z "${DEVELOPER_DIR:-}" && "$(xcode-select -p)" == */CommandLineTools ]]; then
  for candidate in /Applications/Xcode.app/Contents/Developer /Applications/Xcode-beta.app/Contents/Developer; do
    if [[ -d "$candidate" ]]; then export DEVELOPER_DIR="$candidate"; break; fi
  done
fi
source "$ROOT/resources/microphone/compatibility.sh" "$ROOT/config/previous-installation.json"
ARCH="${1:-${DESKFUSE_MACOS_ARCH:-arm64}}"
case "$ARCH" in arm64|x86_64) ;; *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;; esac
BUILD="$ROOT/target/roc-microphone/build-$ARCH"
OUTPUT="$ROOT/target/roc-microphone/$ARCH"
PYTHON="$(command -v python3)"
# Resolve every compiler and SDK from the caller's selected DEVELOPER_DIR.
# /usr/bin/clang is an xcrun shim and must not hide SDK changes from CMake.
export CC="$(xcrun --sdk macosx --find clang)"
export CXX="$(xcrun --sdk macosx --find clang++)"
export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
export PATH="$(dirname "$CC"):/opt/homebrew/opt/libtool/libexec/gnubin:/usr/local/opt/libtool/libexec/gnubin:$PATH"
mkdir -p "$BUILD" "$OUTPUT/include"
CACHE_KEY="$("$PYTHON" "$ROOT/scripts/roc-build-cache.py" key "$ROOT" "$ARCH")"
if "$PYTHON" "$ROOT/scripts/roc-build-cache.py" check "$ROOT" "$ARCH" "$CACHE_KEY"; then
  echo "Roc microphone cache hit ($ARCH; binaries and source archive verified)"
  exit 0
fi
"$PYTHON" "$ROOT/vendor/roc-vad/prepare-toolchain.py" "$BUILD"
# Downloads are pinned both by git commit and by SHA256 of the upstream source
# archive. Retain the verified archives in build cache, outside tracked sources.
"$PYTHON" - "$ROOT" <<'PY'
import concurrent.futures, hashlib, http.client, json, os, pathlib, sys, time, threading, urllib.error, urllib.request
root = pathlib.Path(sys.argv[1])
lock = json.loads((root / 'vendor/roc-vad/sources.lock.json').read_text())
cache = root / 'target/roc-microphone/source-archives'
cache.mkdir(parents=True, exist_ok=True)
def sha256_file(p):
    h = hashlib.sha256()
    with open(p, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()
def verify(source):
    dest = cache / (source['commit'] + '.tar.gz')
    expected = source['sha256']
    repo = source['repository']
    if dest.exists():
        if sha256_file(dest) == expected:
            return
        dest.unlink()
    tmp = dest.with_name(dest.name + f'.{os.getpid()}.{threading.get_ident()}.tmp')
    last_err = None
    for attempt in range(4):
        if attempt:
            time.sleep(2 ** (attempt - 1))
        try:
            if tmp.exists():
                tmp.unlink()
            req = urllib.request.Request(source['archive'], headers={'User-Agent': 'DeskFuse-build'})
            with urllib.request.urlopen(req, timeout=120) as resp:
                tmp.write_bytes(resp.read())
            if sha256_file(tmp) != expected:
                last_err = 'SHA256 mismatch after download'
                tmp.unlink()
                continue
            os.replace(tmp, dest)
            return
        except (urllib.error.URLError, http.client.IncompleteRead, TimeoutError, OSError) as e:
            last_err = str(e)
            try:
                if tmp.exists(): tmp.unlink()
            except OSError:
                pass
    raise SystemExit(f'Download failed for {repo}: {last_err}')
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
    list(pool.map(verify, [lock['upstream']] + lock['dependencies']))
PY
cmake -S "$ROOT/vendor/roc-vad" -B "$BUILD" \
  -DBOOTSTRAP=ON -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_C_COMPILER="$CC" -DCMAKE_CXX_COMPILER="$CXX" -DCMAKE_OSX_SYSROOT="$SDKROOT" \
  -DCMAKE_OSX_ARCHITECTURES="$ARCH" -DCMAKE_OSX_DEPLOYMENT_TARGET=14.0 \
  -DGIT_RELEASE=0.0.4 -DGIT_COMMIT=19d621a8364b0d6f44969c340bdc59fe891981c8 \
  -DDRIVER_BUNDLE_NAME="$MICROPHONE_BUNDLE_NAME" -DDRIVER_BUNDLE_ID="$MICROPHONE_BUNDLE_ID" -DDESKFUSE_MICROPHONE_UID="$MICROPHONE_DEVICE_UID" \
  -DDRIVER_UUID=E88452E3-1C2E-42C1-A1A4-7B496BC3D527 \
  -DDRIVER_SOCKET=127.0.0.1:19712 -DDRIVER_MANUFACTURER=DeskFuse
"$PYTHON" "$ROOT/vendor/roc-vad/verify-sources.py" "$BUILD" --if-present
cmake --build "$BUILD" --parallel "${DESKFUSE_ROC_BUILD_JOBS:-8}"
"$PYTHON" "$ROOT/vendor/roc-vad/verify-sources.py" "$BUILD"
cmake -S "$ROOT/vendor/roc-vad" -B "$BUILD" -DBOOTSTRAP=OFF
cmake --build "$BUILD" --target "$MICROPHONE_BUNDLE_NAME" roc-microphone DeskFuse-roc --parallel "${DESKFUSE_ROC_BUILD_JOBS:-8}"
rm -rf "$OUTPUT/${MICROPHONE_BUNDLE_NAME}.driver"
cp -R "$BUILD/driver/${MICROPHONE_BUNDLE_NAME}.driver" "$OUTPUT/"
cp "$BUILD/microphone/libroc-microphone.dylib" "$OUTPUT/"
cp "$BUILD/tool/DeskFuse-roc" "$OUTPUT/DeskFuse-roc"
cp "$ROOT/apps/deskfuse-macos/MicrophoneNative/include/deskfuse_microphone.h" "$OUTPUT/include/"
lipo "$OUTPUT/${MICROPHONE_BUNDLE_NAME}.driver/Contents/MacOS/${MICROPHONE_BUNDLE_NAME}" -verify_arch "$ARCH"
lipo "$OUTPUT/libroc-microphone.dylib" -verify_arch "$ARCH"
# Complete corresponding covered source, local modifications, dependent source
# and notices. No .git directories, audio recordings or compiled output.
"$PYTHON" "$ROOT/vendor/roc-vad/archive-sources.py" "$ROOT" "$BUILD" "$OUTPUT/roc-microphone-sources.tar.gz"
shasum -a 256 "$OUTPUT/libroc-microphone.dylib" "$OUTPUT/roc-microphone-sources.tar.gz" > "$OUTPUT/SHA256SUMS"
"$PYTHON" "$ROOT/scripts/roc-build-cache.py" record "$ROOT" "$ARCH" "$CACHE_KEY"
echo "Roc microphone build: $OUTPUT"
