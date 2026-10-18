#!/usr/bin/env python3
"""Check the shipped macOS input helper's embedded background identity."""
from pathlib import Path
import plistlib
import re
import subprocess
import sys


def validate(binary: Path) -> None:
    load = subprocess.check_output(["otool", "-l", str(binary)], text=True)
    section = re.search(
        r"sectname __info_plist\s+segname __TEXT\s+addr \S+\s+"
        r"size (0x[0-9a-fA-F]+)\s+offset (\d+)",
        load,
    )
    if section is None:
        raise ValueError("input service is missing its embedded background Info.plist")
    size, offset = int(section[1], 16), int(section[2])
    info = plistlib.loads(binary.read_bytes()[offset:offset + size])
    if not isinstance(info, dict) or info.get("LSBackgroundOnly") is not True:
        raise ValueError("input service must declare LSBackgroundOnly=true")
    if info.get("CFBundleIdentifier") != "com.deskfuse.app.input-service":
        raise ValueError("input service must have its own bundle identifier")
    if info.get("CFBundleExecutable") != "deskfuse-input-service":
        raise ValueError("input service executable identity is incorrect")


if __name__ == "__main__":
    try:
        validate(Path(sys.argv[1]))
    except (IndexError, OSError, ValueError, plistlib.InvalidFileException,
            subprocess.CalledProcessError) as error:
        raise SystemExit(f"error: {error}") from error
    print("input service: embedded background identity; no Dock tile")
