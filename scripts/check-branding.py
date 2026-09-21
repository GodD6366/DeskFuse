#!/usr/bin/env python3
"""Reject legacy branding in tracked/unignored source paths and file contents."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
LEGACY_CONFIG = "config/legacy/distribution.json"
RETIRED_KEYWORD = json.loads((ROOT / LEGACY_CONFIG).read_text())["retired_keyword"]
PATTERN = re.compile(re.escape(RETIRED_KEYWORD.encode()), re.IGNORECASE)
# Keep this list explicit: a tests/ or docs/ exemption would conceal new leaks.
ALLOWED = {LEGACY_CONFIG, "web/src/config/legacy/distribution.json"}


def violations(root: Path):
    output = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=root
    )
    for raw in sorted(set(output.split(b"\0")) - {b""}):
        name = raw.decode("utf-8", errors="surrogateescape")
        if name in ALLOWED:
            continue
        path = root / name
        if not os.path.lexists(path):
            continue
        if PATTERN.search(raw):
            yield f"{name}: legacy filename"
        if path.is_symlink():
            if PATTERN.search(os.fsencode(os.readlink(path))):
                yield f"{name}: legacy symlink target"
            continue
        # Byte scanning also finds text embedded in generated/binary assets.
        for line, content in enumerate(path.read_bytes().split(b"\n"), 1):
            if PATTERN.search(content):
                yield f"{name}:{line}: legacy content"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--report", action="store_true", help="inventory only; not an acceptance check")
    args = parser.parse_args()
    found = list(violations(args.root.resolve()))
    for item in found:
        print(item)
    print(f"branding: {len(found)} violations", file=sys.stderr)
    return 0 if args.report or not found else 1


if __name__ == "__main__":
    raise SystemExit(main())
