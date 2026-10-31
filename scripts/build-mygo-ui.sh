#!/usr/bin/env bash
# 仅构建 WebView 客户端；不构建 Rust、媒体库，不接管正式发行。
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_DIR="$ROOT_DIR/apps/deskfuse-go"
OUT_DIR="$ROOT_DIR/target/mygo-webview"
# 可选参数：直接输出到指定路径（构建 app bundle 时使用）。
OUTPUT="${1:-$OUT_DIR/deskfuse-mygo}"
# 打包 x86_64 DMG 时在 arm64 主机交叉编译；生成器与 Vite 仍按宿主架构运行。
GO_ARCH="${DESKFUSE_GOARCH:-$(go env GOARCH)}"
mkdir -p "$OUT_DIR" "$APP_DIR/frontend/dist"
# 首次构建只提供 embed 输入；正式产物必须由后续 Vite build 覆盖。
if [[ ! -f "$APP_DIR/frontend/dist/index.html" ]]; then
  cp "$APP_DIR/frontend/index.html" "$APP_DIR/frontend/dist/index.html"
fi
if [[ ! -d "$APP_DIR/frontend/node_modules" ]]; then
  npm --prefix "$APP_DIR/frontend" ci
fi
# 侧栏品牌位使用与原生应用相同的图标（浅色/深色两版）。
if [[ -f "$ROOT_DIR/resources/generated/DeskFuse-icon-1024.png" ]]; then
  mkdir -p "$APP_DIR/frontend/public"
  cp "$ROOT_DIR/resources/generated/DeskFuse-icon-light-1024.png" "$APP_DIR/frontend/public/deskfuse-icon-light.png"
  cp "$ROOT_DIR/resources/generated/DeskFuse-icon-1024.png" "$APP_DIR/frontend/public/deskfuse-icon.png"
fi
(cd "$APP_DIR" && MYGO_GENERATE=frontend/src/mygo.ts go run ./cmd/deskfuse)
npm --prefix "$APP_DIR/frontend" run build
mkdir -p "$(dirname "$OUTPUT")"
CGO_ENABLED=0 GOARCH="$GO_ARCH" go -C "$APP_DIR" build -trimpath -buildvcs=false -ldflags='-s -w' -o "$OUTPUT" ./cmd/deskfuse
printf 'WebView 客户端：%s (%s)\n' "$OUTPUT" "$GO_ARCH"
