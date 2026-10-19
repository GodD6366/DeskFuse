#!/usr/bin/env python3
"""Content receipts for the Roc build, including its corresponding source archive."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys


def digest_files(base, paths):
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(str(path.relative_to(base)).encode() + b"\0")
        digest.update(str(path.stat().st_mode).encode() + b"\0")
        with path.open("rb") as stream:
            digest.update(hashlib.file_digest(stream, "sha256").digest())
    return digest.hexdigest()


def input_key(root, arch):
    paths = []
    for directory in ("vendor/roc-vad", "apps/deskfuse-macos/MicrophoneNative"):
        paths.extend(path for path in (root / directory).rglob("*") if path.is_file())
    for name in (
        "scripts/build-roc-microphone.sh", "scripts/roc-build-cache.py",
        "scripts/test-microphone-native.sh", "config/previous-installation.json",
        "resources/microphone/compatibility.sh", "resources/microphone/driver-identity.json",
    ):
        path = root / name
        if path.is_file():
            paths.append(path)
    settings = Path(os.environ["SDKROOT"]) / "SDKSettings.json"
    state = {
        "sources": digest_files(root, paths), "arch": arch,
        "environment": {key: os.environ.get(key, "") for key in (
            "CC", "CXX", "SDKROOT", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS",
            "CMAKE_PREFIX_PATH", "MACOSX_DEPLOYMENT_TARGET",
        )},
        "compiler": subprocess.check_output([os.environ["CXX"], "--version"]).decode(),
        "cmake": subprocess.check_output(["cmake", "--version"]).decode(),
        "sdk": hashlib.sha256(settings.read_bytes()).hexdigest(),
    }
    return hashlib.sha256(json.dumps(state, sort_keys=True).encode()).hexdigest()


def output_key(output):
    return digest_files(output, [path for path in output.rglob("*") if path.is_file()])


def main():
    action, root_arg, arch = sys.argv[1:4]
    root = Path(root_arg)
    output = root / "target/roc-microphone" / arch
    receipt = output.parent / (".build-receipt-" + arch + ".json")
    if action == "key":
        print(input_key(root, arch))
        return 0
    key = sys.argv[4]
    if action == "check":
        try:
            saved = json.loads(receipt.read_text())
            return 0 if saved["input"] == key and saved["output"] == output_key(output) else 1
        except (OSError, ValueError, KeyError):
            return 1
    if action == "record":
        temporary = receipt.with_suffix(".tmp")
        temporary.write_text(json.dumps({"input": key, "output": output_key(output)}))
        temporary.replace(receipt)
        return 0
    raise ValueError(action)


if __name__ == "__main__":
    sys.exit(main())
