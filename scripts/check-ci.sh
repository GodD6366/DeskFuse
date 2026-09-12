#!/usr/bin/env bash
# Pinned, checksum-verified workflow lint and secret scanning. No global installs.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
python3 scripts/check-repository.py --worktree
TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/unidesk-ci-check.XXXXXX")"
trap 'rm -rf "$TEMP_DIR"' EXIT
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    ACTION_PLATFORM=darwin_arm64
    ACTION_SHA=aba9ced2dee8d27fecca3dc7feb1a7f9a52caefa1eb46f3271ea66b6e0e6953f
    LEAK_PLATFORM=darwin_arm64
    LEAK_SHA=b40ab0ae55c505963e365f271a8d3846efbc170aa17f2607f13df610a9aeb6a5
    ;;
  Linux-x86_64)
    ACTION_PLATFORM=linux_amd64
    ACTION_SHA=8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8
    LEAK_PLATFORM=linux_x64
    LEAK_SHA=551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb
    ;;
  *) echo "error: CI tooling supports macOS arm64 and Linux x64" >&2; exit 1 ;;
esac
fetch_tool() {
  local repo="$1" tag="$2" archive="$3" digest="$4" executable="$5"
  curl --fail --location --retry 3 --max-time 120 --silent --show-error \
    "https://github.com/$repo/releases/download/$tag/$archive" --output "$TEMP_DIR/$archive"
  python3 - "$TEMP_DIR/$archive" "$digest" <<'PY'
import hashlib, pathlib, sys
if hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest() != sys.argv[2]:
    raise SystemExit('error: CI tool archive checksum mismatch')
PY
  tar -xzf "$TEMP_DIR/$archive" -C "$TEMP_DIR" "$executable"
}
fetch_tool rhysd/actionlint v1.7.12 "actionlint_1.7.12_${ACTION_PLATFORM}.tar.gz" "$ACTION_SHA" actionlint
fetch_tool gitleaks/gitleaks v8.30.1 "gitleaks_8.30.1_${LEAK_PLATFORM}.tar.gz" "$LEAK_SHA" gitleaks
"$TEMP_DIR/actionlint" -shellcheck='' -pyflakes='' .github/workflows/*.yml
for script in scripts/*.sh; do bash -n "$script"; done
# Scan the effective public worktree, including new source not staged yet.
python3 - "$TEMP_DIR/source" <<'PY'
from pathlib import Path
import shutil, sys
sys.path.insert(0, str(Path("scripts").resolve()))
from repository_files import source_files
out = Path(sys.argv[1])
for name in sorted(source_files(worktree=True)):
    path = Path(name)
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"error: cannot scan non-regular source: {name}")
    dest = out / path
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(path, dest)
PY
"$TEMP_DIR/gitleaks" dir "$TEMP_DIR/source" --redact --no-banner
"$TEMP_DIR/gitleaks" git . --log-opts=--all --redact --no-banner
