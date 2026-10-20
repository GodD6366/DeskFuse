#!/usr/bin/env python3
"""Remove build-machine LC_RPATH entries before signing, or reject them on verification."""
import argparse
from pathlib import Path
import re
import subprocess


def rpaths(binary: Path) -> list[str]:
    load = subprocess.check_output(["otool", "-l", str(binary)], text=True)
    return re.findall(r"\bcmd LC_RPATH\s+cmdsize \d+\s+path (.*?) \(offset \d+\)", load)


def portable(path: str) -> bool:
    roots = ("@loader_path", "@executable_path", "/usr/lib", "/System/Library")
    return any(path == root or path.startswith(root + "/") for root in roots)


def check(binary: Path, fix: bool = False) -> None:
    bad = list(dict.fromkeys(path for path in rpaths(binary) if not portable(path)))
    if bad and not fix:
        raise ValueError(f"{binary}: non-portable LC_RPATH: {', '.join(bad)}")
    for path in bad:
        subprocess.run(["install_name_tool", "-delete_rpath", path, str(binary)], check=True)
        print(f"removed build rpath: {path}")
    if fix:
        check(binary)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fix", action="store_true", help="Modify a staging binary before signing")
    parser.add_argument("binaries", nargs="+", type=Path)
    arguments = parser.parse_args()
    for binary in arguments.binaries:
        check(binary, fix=arguments.fix)


if __name__ == "__main__":
    main()
