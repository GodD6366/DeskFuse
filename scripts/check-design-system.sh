#!/usr/bin/env bash
# 设计系统守卫：样式令牌之外不允许出现写死的字号、圆角、颜色与按钮样式。
# 例外：DeviceProductIcon 按传入尺寸等比缩放图标，参数不是常量。
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_DIR="$ROOT_DIR/apps/deskfuse-macos/Sources/DeskFuse"

cd "$SOURCE_DIR"
violations="$(
  {
    grep -rn --include='*.swift' -E '\.font\(\.system\(size:[[:space:]]*[0-9]' . || true
    grep -rn --include='*.swift' -E 'cornerRadius:[[:space:]]*[0-9]' . || true
    grep -rn --include='*.swift' -E 'Color\.(orange|teal|purple|indigo)\b' . || true
    grep -rn --include='*.swift' -E '\.borderedProminent\b' . || true
    grep -rn --include='*.swift' -E 'Color\.accentColor\b' . || true
  } | grep -v '^\./DesignSystem/' | sort -u || true
)"

if [[ -n "$violations" ]]; then
  echo "error: 以下位置绕过了设计系统令牌，请改用 DesignSystem/ 中的 DFType、DFRadius、DFPalette：" >&2
  printf '%s\n' "$violations" >&2
  exit 1
fi

echo "design system guard: ok"
