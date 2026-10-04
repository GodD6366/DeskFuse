#!/usr/bin/env bash
# Build FreeRDP (headless client, static) and the DeskFuse RDP bridge into a
# single static library consumed by the macOS app's Swift Package.
#
# Output: apps/deskfuse-macos/RemoteApplication/Vendor/FreeRDP/
#   RdpBridge.h            bridge header (imported by Swift via CDeskFuseRdp)
#   module.modulemap       Swift Package system-library descriptor
#   lib/libdeskfuse-rdp.a   FreeRDP + WinPR + client + RdpBridge combined
#
# The combined archive still needs these system libraries at final link time:
#   z (macOS SDK), and frameworks: Foundation, CoreServices, Security.
# OpenSSL and json-c are built from pinned source and included statically.
set -euo pipefail
[[ "$#" -le 1 && ( "$#" == 0 || "$1" == --test ) ]] || { echo "usage: $0 [--test]" >&2; exit 2; }
if [[ -z "${DEVELOPER_DIR:-}" && "$(xcode-select -p)" == */CommandLineTools ]]; then
  for candidate in /Applications/Xcode.app/Contents/Developer /Applications/Xcode-beta.app/Contents/Developer; do
    if [[ -d "$candidate" ]]; then export DEVELOPER_DIR="$candidate"; break; fi
  done
fi

FREERDP_VERSION="3.14.1"
CACHE_DIR="${FREERDP_CACHE_DIR:-$HOME/.cache/deskfuse-freerdp}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REMOTE_APP="$ROOT/apps/deskfuse-macos/RemoteApplication"
BRIDGE_SRC="$REMOTE_APP/Sources/DeskFuseRemoteApplication/RdpBridge"
VENDOR="$REMOTE_APP/Vendor/FreeRDP"

CLANG="${CLANG:-$(xcrun --find clang)}"
ARCH="${DESKFUSE_MACOS_ARCH:-$(uname -m)}"
case "$ARCH" in arm64|x86_64) ;; *) echo "error: unsupported architecture: $ARCH" >&2; exit 2 ;; esac
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
SDK="$(xcrun --sdk macosx --show-sdk-path)"
JOBS="${JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || echo 4)}"

# Build pinned dependencies for the requested architecture and deployment target.
# Homebrew bottles can require a newer macOS and cannot cross-link Intel builds.
for tool in cmake git perl make libtool; do
  command -v "$tool" >/dev/null || { echo "error: required tool not found: $tool" >&2; exit 1; }
done
checkout() {
  local url="$1" tag="$2" revision="$3" destination="$4"
  if [[ ! -d "$destination/.git" ]]; then
    git clone --depth 1 --branch "$tag" "$url" "$destination"
  fi
  [[ "$(git -C "$destination" rev-parse HEAD)" == "$revision" ]] || {
    echo "error: unexpected source revision in $destination" >&2; exit 1;
  }
}
mkdir -p "$CACHE_DIR"
OPENSSL_VERSION=3.6.5
JSON_VERSION=0.18
DEPS_KEY="$( { printf '%s\n' "$ARCH" "$SDK" "$MACOSX_DEPLOYMENT_TARGET" "$OPENSSL_VERSION" "$JSON_VERSION"; "$CLANG" --version; } | shasum -a 256 | awk '{print $1}')"
DEPS="$CACHE_DIR/deps-$ARCH-$DEPS_KEY"
openssl_prefix="$DEPS/openssl"
jsonc_prefix="$DEPS/json-c"
if [[ ! -f "$DEPS/.complete" ]]; then
  OPENSSL_SRC="$CACHE_DIR/openssl-$OPENSSL_VERSION"
  JSON_SRC="$CACHE_DIR/json-c-$JSON_VERSION"
  checkout https://github.com/openssl/openssl.git "openssl-$OPENSSL_VERSION" c8bd5a57108599ac650bbae77fcabe3109dab2e8 "$OPENSSL_SRC"
  checkout https://github.com/json-c/json-c.git json-c-0.18-20240915 41a55cfcedb54d9c1874f2f0eb07b504091d7e37 "$JSON_SRC"
  mkdir -p "$DEPS/openssl-build"
  (
    cd "$DEPS/openssl-build"
    CC="$CLANG" "$OPENSSL_SRC/Configure" "darwin64-$ARCH-cc" no-shared no-tests no-module \
      --prefix="$openssl_prefix" --libdir=lib \
      -isysroot "$SDK" -mmacosx-version-min="$MACOSX_DEPLOYMENT_TARGET"
    make -j"$JOBS"
    make install_sw
  )
  cmake -S "$JSON_SRC" -B "$DEPS/json-build" -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5 -DCMAKE_INSTALL_PREFIX="$jsonc_prefix" -DBUILD_SHARED_LIBS=OFF -DBUILD_TESTING=OFF \
    -DCMAKE_OSX_ARCHITECTURES="$ARCH" -DCMAKE_OSX_SYSROOT="$SDK" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOSX_DEPLOYMENT_TARGET"
  cmake --build "$DEPS/json-build" -j"$JOBS"
  cmake --install "$DEPS/json-build"
  touch "$DEPS/.complete"
fi
DEPENDENCIES=("$openssl_prefix/lib/libssl.a" "$openssl_prefix/lib/libcrypto.a" "$jsonc_prefix/lib/libjson-c.a")
for library in "${DEPENDENCIES[@]}"; do lipo -verify_arch "$ARCH" "$library"; done
# Include native dependencies and toolchain so upgrades cannot reuse stale outputs.
FINGERPRINT="$( { shasum -a 256 "$0" "$BRIDGE_SRC/RdpBridge.c" "$BRIDGE_SRC/RdpBridge.h" "${DEPENDENCIES[@]}" | awk '{print $1}'; printf '%s\n' "$ARCH" "$SDK" "$MACOSX_DEPLOYMENT_TARGET"; "$CLANG" --version; } | shasum -a 256 | awk '{print $1}')"
if [[ "${1:-}" != --test && -f "$VENDOR/NOTICES.txt" && -f "$VENDOR/.fingerprint" && "$(cat "$VENDOR/.fingerprint")" == "$FINGERPRINT" && -s "$VENDOR/lib/libdeskfuse-rdp.a" && -s "$VENDOR/module.modulemap" && -s "$VENDOR/RdpBridge.h" ]]; then
  echo "FreeRDP cache hit ($ARCH)"
  exit 0
fi

echo "==> Preparing FreeRDP $FREERDP_VERSION"
SRC="$CACHE_DIR/FreeRDP-$FREERDP_VERSION"
checkout https://github.com/FreeRDP/FreeRDP.git "$FREERDP_VERSION" 4019dcd4097d550d02dd6b70ee88b6a1189be26c "$SRC"

BUILD="$CACHE_DIR/build-$FREERDP_VERSION-$ARCH-$FINGERPRINT"
echo "==> Configuring FreeRDP (headless, static, RemoteApp-only channels)"
cmake -B "$BUILD" -S "$SRC" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_OSX_ARCHITECTURES="$ARCH" \
  -DCMAKE_OSX_SYSROOT="$SDK" \
  -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOSX_DEPLOYMENT_TARGET" \
  -DOPENSSL_ROOT_DIR="$openssl_prefix" \
  -DOPENSSL_INCLUDE_DIR="$openssl_prefix/include" \
  -DOPENSSL_SSL_LIBRARY="$openssl_prefix/lib/libssl.a" \
  -DOPENSSL_CRYPTO_LIBRARY="$openssl_prefix/lib/libcrypto.a" \
  -DCMAKE_PREFIX_PATH="$openssl_prefix;$jsonc_prefix" \
  -DJSONC_INCLUDE_DIR="$jsonc_prefix/include/json-c" \
  -DJSONC_LIBRARY="$jsonc_prefix/lib/libjson-c.a" \
  -DWITH_JSONC_REQUIRED=ON \
  -DWITH_PKCS11=OFF \
  -DCMAKE_DISABLE_FIND_PACKAGE_libusb-1.0=TRUE \
  -DBUILD_SHARED_LIBS=OFF \
  -DBUILD_TESTING=OFF \
  -DWITH_SAMPLE=OFF \
  -DWITH_CLIENT_COMMON=ON \
  -DWITH_CLIENT=OFF \
  -DWITH_SERVER=OFF \
  -DWITH_CHANNELS=ON \
  -DWITH_CLIENT_CHANNELS=ON \
  -DWITH_OPUS=OFF \
  -DWITH_MANPAGES=OFF -DWITH_DOCS=OFF -DWITH_DEBUG_ALL=OFF \
  -DWITH_OPENH264=OFF -DWITH_SWSCALE=OFF -DWITH_CAIRO=OFF -DWITH_FFMPEG=OFF \
  -DWITH_GSM=OFF -DWITH_LAME=OFF -DWITH_OPENSLES=OFF -DWITH_OSS=OFF \
  -DWITH_PULSE=OFF -DWITH_ALSA=OFF -DWITH_CUPS=OFF -DWITH_PCSC=OFF \
  -DWITH_KRB5=OFF -DWITH_JPEG=OFF -DWITH_GSTREAMER_1_0=OFF \
  -DCHANNEL_RDPSND=OFF -DCHANNEL_AUDIN=OFF -DCHANNEL_RDPDR=OFF \
  -DCHANNEL_URBDRC=OFF -DCHANNEL_SMARTCARD=OFF -DCHANNEL_PRINTER=OFF \
  -DCHANNEL_SERIAL=OFF -DCHANNEL_PARALLEL=OFF -DCHANNEL_TSMF=OFF \
  -DCHANNEL_VIDEO=OFF -DCHANNEL_ENCOMSP=OFF -DCHANNEL_REMDESK=OFF \
  -DCHANNEL_LOCATION=OFF -DCHANNEL_ECHO=OFF -DCHANNEL_SSHAGENT=OFF \
  -DCHANNEL_RDP2TCP=OFF -DCHANNEL_GFXREDIR=OFF -DCHANNEL_RDPEAR=OFF \
  -DCHANNEL_RDPECAM=OFF -DCHANNEL_AINPUT=OFF -DCHANNEL_GEOMETRY=OFF \
  -DCHANNEL_TELEMETRY=OFF -DCHANNEL_DISP=ON -DCHANNEL_RDPGFX=ON \
  -DCHANNEL_CLIPRDR=ON -DCHANNEL_RAIL=ON -DCHANNEL_RDPEI=ON

echo "==> Building FreeRDP"
cmake --build "$BUILD" --target freerdp winpr freerdp-client -j"$JOBS"

echo "==> Compiling RdpBridge.c"
COMMON_FLAGS=(
  -std=gnu11 -O2 -fPIC -arch "$ARCH" -isysroot "$SDK" -mmacosx-version-min="$MACOSX_DEPLOYMENT_TARGET"
  -DNONAMELESSUNION -DWITH_OPENSSL -DWITH_SMARTCARD_EMULATE
  -D_DARWIN_C_SOURCE -D_POSIX_C_SOURCE=200809L
  -I"$SRC/winpr/include" -I"$BUILD/winpr/include"
  -I"$BUILD" -I"$BUILD/include" -I"$SRC/include"
  -I"$BUILD/channels/client" -I"$SRC/channels/client"
  -isystem "$openssl_prefix/include"
)
"$CLANG" -c "$BRIDGE_SRC/RdpBridge.c" -o "$BUILD/RdpBridge.o" "${COMMON_FLAGS[@]}"

echo "==> Combining into libdeskfuse-rdp.a"
mkdir -p "$VENDOR/lib"
libtool -static -o "$VENDOR/lib/libdeskfuse-rdp.a" \
  "$BUILD/RdpBridge.o" \
  "$BUILD/libfreerdp/libfreerdp3.a" \
  "$BUILD/winpr/libwinpr/libwinpr3.a" \
  "$BUILD/client/common/libfreerdp-client3.a" \
  "${DEPENDENCIES[@]}"

echo "==> Installing header and module map"
cp "$BRIDGE_SRC/RdpBridge.h" "$VENDOR/RdpBridge.h"
cat > "$VENDOR/module.modulemap" <<'EOF'
module CDeskFuseRdp {
    header "RdpBridge.h"
    link "deskfuse-rdp"
    export *
}
EOF

{
  printf 'FreeRDP %s (Apache-2.0)\n' "$FREERDP_VERSION"
  cat "$SRC/LICENSE"
  printf '\nOpenSSL %s (Apache-2.0)\n' "$OPENSSL_VERSION"
  cat "$CACHE_DIR/openssl-$OPENSSL_VERSION/LICENSE.txt"
  printf '\njson-c %s (MIT)\n' "$JSON_VERSION"
  cat "$CACHE_DIR/json-c-$JSON_VERSION/COPYING"
} > "$VENDOR/NOTICES.txt"
if [[ "${1:-}" == --test ]]; then
  "$CLANG" "$ROOT/scripts/test-rdp-bridge.c" "${COMMON_FLAGS[@]}" \
    -I"$SRC/channels/rail/client" -I"$SRC/channels/rail" \
    "$VENDOR/lib/libdeskfuse-rdp.a" -lz \
    -framework Foundation -framework CoreServices -framework Security -framework IOKit \
    -o "$BUILD/test-rdp-bridge"
  "$BUILD/test-rdp-bridge"
fi
printf '%s\n' "$FINGERPRINT" > "$VENDOR/.fingerprint"
echo "==> Done"
echo "Vendor: $VENDOR"
echo "Third-party libraries statically linked ($ARCH)"
