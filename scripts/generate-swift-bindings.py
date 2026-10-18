#!/usr/bin/env python3
"""Cache Swift generation by compiled library, generator and binding config.

The receipt also hashes all outputs: deletion or corruption regenerates them.
The shared Swift source directory has one receipt, not one per architecture.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
OUTPUTS = {
    "DeskFuseEngine.swift": "Sources/DeskFuseEngine/DeskFuseEngine.swift",
    "DeskFuseEngineFFI.h": "Sources/DeskFuseEngineFFI/DeskFuseEngineFFI.h",
    "DeskFuseEngineFFI.modulemap": "Sources/DeskFuseEngineFFI/module.modulemap",
}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def generate(library, bindgen, package, config, force=False):
    inputs = {str(path): digest(path) for path in (library, bindgen, config, Path(__file__))}
    receipt = package / "SDK/.swift-bindings-fingerprint.json"
    outputs = [package / "Generated" / name for name in OUTPUTS]
    outputs += [package / name for name in OUTPUTS.values()]
    try:
        previous = json.loads(receipt.read_text())
        current = {str(path): digest(path) for path in outputs}
        if not force and previous == {"inputs": inputs, "outputs": current}:
            print("Swift bindings cache hit")
            return False
    except (OSError, ValueError):
        pass

    # Don't let a failed generator overwrite the last successful bindings.
    with tempfile.TemporaryDirectory(prefix="deskfuse-bindings-") as temporary:
        # uniffi 0.32 rejects flat --config overrides; the generator discovers
        # crates/deskfuse-ffi/uniffi.toml through cargo metadata instead, so the
        # configuration file is hashed above but not passed on the command line.
        subprocess.run([
            str(bindgen), "generate", "--library", str(library),
            "--language", "swift", "--out-dir", temporary,
        ], check=True, cwd=ROOT)
        generated = Path(temporary)
        for name in OUTPUTS:
            if not (generated / name).is_file():
                raise RuntimeError(f"bindgen did not produce {name}")
        for name, destination in OUTPUTS.items():
            source = generated / name
            for target in (package / "Generated" / name, package / destination):
                target.parent.mkdir(parents=True, exist_ok=True)
                if not target.is_file() or digest(source) != digest(target):
                    shutil.copyfile(source, target)
    receipt.parent.mkdir(parents=True, exist_ok=True)
    state = {"inputs": inputs, "outputs": {str(path): digest(path) for path in outputs}}
    with tempfile.NamedTemporaryFile(mode="w", dir=receipt.parent, delete=False) as stream:
        pending = Path(stream.name)
        json.dump(state, stream, sort_keys=True)
    try:
        pending.replace(receipt)
    finally:
        pending.unlink(missing_ok=True)
    print("Swift bindings generated")
    return True


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--library", type=Path, required=True)
    parser.add_argument("--bindgen", type=Path, required=True)
    args = parser.parse_args()
    generate(args.library.resolve(), args.bindgen.resolve(), ROOT / "apps/deskfuse-macos",
             ROOT / "crates/deskfuse-ffi/uniffi.toml",
             force=os.environ.get("DESKFUSE_FORCE_RUST") == "1")
