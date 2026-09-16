#!/bin/bash
# 包装 package-macos.sh，按已准备的公开发行 descriptor 组织输出目录。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DESCRIPTOR="$ROOT_DIR/release.json"

if [[ ! -f "$DESCRIPTOR" ]]; then
  echo "错误: release.json 不存在" >&2
  exit 1
fi

read -r VERSION BUILD_NUMBER < <(
  python3 - "$DESCRIPTOR" <<'PY'
import json, sys
value = json.load(open(sys.argv[1]))
version = value.get("version")
build = value.get("build_number")
if not isinstance(version, str) or not isinstance(build, int):
    raise SystemExit("错误: release.json 缺少 version/build_number")
print(version, build)
PY
)

VERSIONED_DIST_DIR="$ROOT_DIR/dist/$VERSION-build$BUILD_NUMBER"

echo "版本: $VERSION build $BUILD_NUMBER"
echo "输出目录: $VERSIONED_DIST_DIR"

export DIST_DIR="$VERSIONED_DIST_DIR"
export RELEASE_METADATA="$DESCRIPTOR"
exec "$SCRIPT_DIR/package-macos.sh" "$@"
