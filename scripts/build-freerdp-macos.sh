#!/usr/bin/env bash
# Build FreeRDP (headless client, static) and the uniDesk RDP bridge into a
# single static library consumed by the macOS app's Swift Package.
#
# Output: apps/unidesk-macos/RemoteApplication/Vendor/FreeRDP/
#   RdpBridge.h            bridge header (imported by Swift via CUniDeskRdp)
#   module.modulemap       Swift Package system-library descriptor
#   lib/libunidesk-rdp.a   FreeRDP + WinPR + client + RdpBridge combined
#
# The combined archive still needs these system libraries at final link time:
#   ssl, crypto (openssl@3), usb-1.0 (libusb), json-c
# and frameworks: Foundation, CoreServices, Security.
set -euo pipefail

FREERDP_VERSION="3.14.1"
CACHE_DIR="${FREERDP_CACHE_DIR:-$HOME/.cache/unidesk-freerdp}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REMOTE_APP="$ROOT/apps/unidesk-macos/RemoteApplication"
BRIDGE_SRC="$REMOTE_APP/Sources/UniDeskRemoteApplication/RdpBridge"
VENDOR="$REMOTE_APP/Vendor/FreeRDP"

CLANG="${CLANG:-clang}"
JOBS="${JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || echo 4)}"

openssl_prefix="$(brew --prefix openssl@3 2>/dev/null || echo /opt/homebrew/opt/openssl@3)"
jsonc_prefix="$(brew --prefix json-c 2>/dev/null || echo /opt/homebrew/opt/json-c)"
libusb_prefix="$(brew --prefix libusb 2>/dev/null || echo /opt/homebrew/opt/libusb)"

echo "==> Preparing FreeRDP $FREERDP_VERSION"
SRC="$CACHE_DIR/FreeRDP-$FREERDP_VERSION"
if [[ ! -d "$SRC/.git" ]]; then
  mkdir -p "$CACHE_DIR"
  git clone --depth 1 --branch "$FREERDP_VERSION" https://github.com/FreeRDP/FreeRDP.git "$SRC"
fi

BUILD="$SRC/build-min"
echo "==> Configuring FreeRDP (headless, static, RemoteApp-only channels)"
cmake -B "$BUILD" -S "$SRC" \
  -DCMAKE_BUILD_TYPE=Release \
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
  -std=gnu11 -O2 -fPIC -arch "$(uname -m)"
  -DNONAMELESSUNION -DWITH_OPENSSL -DWITH_SMARTCARD_EMULATE
  -D_DARWIN_C_SOURCE -D_POSIX_C_SOURCE=200809L
  -I"$SRC/winpr/include" -I"$BUILD/winpr/include"
  -I"$BUILD" -I"$BUILD/include" -I"$SRC/include"
  -I"$BUILD/channels/client" -I"$SRC/channels/client"
  -isystem "$openssl_prefix/include"
)
"$CLANG" -c "$BRIDGE_SRC/RdpBridge.c" -o "$BUILD/RdpBridge.o" "${COMMON_FLAGS[@]}"

echo "==> Combining into libunidesk-rdp.a"
mkdir -p "$VENDOR/lib"
libtool -static -o "$VENDOR/lib/libunidesk-rdp.a" \
  "$BUILD/RdpBridge.o" \
  "$BUILD/libfreerdp/libfreerdp3.a" \
  "$BUILD/winpr/libwinpr/libwinpr3.a" \
  "$BUILD/client/common/libfreerdp-client3.a"

echo "==> Installing header and module map"
cp "$BRIDGE_SRC/RdpBridge.h" "$VENDOR/RdpBridge.h"
cat > "$VENDOR/module.modulemap" <<'EOF'
module CUniDeskRdp {
    header "RdpBridge.h"
    link "unidesk-rdp"
    export *
}
EOF

echo "==> Done"
echo "Vendor: $VENDOR"
echo "System link deps: -lssl -lcrypto -lusb-1.0 -ljson-c -framework Foundation -framework CoreServices -framework Security"
