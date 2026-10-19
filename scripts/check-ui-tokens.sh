#!/usr/bin/env bash
# 设计系统守卫：DesignSystem/ 之外的界面代码不允许出现写死的字号、圆角、颜色与按钮样式。
# 每条规则给出中文说明，命中时打印具体位置，便于直接修掉。
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCES="$ROOT_DIR/apps/deskfuse-macos/Sources/DeskFuse"

# 每行：说明<TAB>扩展正则
RULES=(
  $'手写字号，请使用 DFType\t\\.font\\(\\.system\\(size:'
  $'系统语义字号（.caption/.callout 等），请使用 DFType\t\\.font\\(\\.(caption2?|footnote|subheadline|callout|headline|body|title[23]?|largeTitle)[.)]'
  $'手写圆角数值，请使用 DFRadius\tcornerRadius:[[:space:]]*[0-9]'
  $'非语义调色（红橙绿蓝等），请使用 DFPalette 或 DFTone\t(Color)?\\.(orange|red|green|blue|yellow|purple|teal|indigo|pink|brown|gray)\\b'
  $'.borderedProminent，请使用 .dfPrimary 或 .df(.primary)\tborderedProminent'
  $'accentColor，请使用 DFPalette.accent\tColor\\.accentColor|\\.accentColor\\b'
  $'已废弃的 .foregroundColor，请使用 .foregroundStyle\t\\.foregroundColor\\('
)

status=0
for rule in "${RULES[@]}"; do
  message="${rule%%$'\t'*}"
  pattern="${rule#*$'\t'}"
  # 行内 `ui-tokens: allow <原因>` 可豁免单行（例如色板结构体的字段名本身叫 red/green/blue）。
  if matches="$(grep -rEn --include='*.swift' --exclude-dir=DesignSystem -e "$pattern" "$SOURCES" \
      | grep -v 'ui-tokens: allow' || true)" && [[ -n "$matches" ]]; then
    printf 'error: %s\n%s\n\n' "$message" "$matches" >&2
    status=1
  fi
done

if [[ "$status" -eq 0 ]]; then
  echo "ui tokens: ok"
fi
exit "$status"
