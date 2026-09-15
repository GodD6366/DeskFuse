#!/usr/bin/env python3
"""Archive a verified local DMG on the mounted NAS, without replacing releases."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tempfile


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", type=Path, default=Path(__file__).resolve().parent.parent / "dist")
    parser.add_argument("--dmg", type=Path)
    parser.add_argument("--destination-root", type=Path,
                        default=Path(os.environ.get("UNIDESK_NAS_DIR", "/Volumes/Share/MAC软件/uniDesk")))
    args = parser.parse_args()
    root = args.destination_root.absolute()
    if root.parts[:2] == ("/", "Volumes"):
        volume = Path(*root.parts[:3])
        if not volume.is_mount():
            raise RuntimeError(f"NAS volume is not mounted: {volume}; local package is retained")

    descriptor = args.dist / "release.json"
    release = json.loads(descriptor.read_text())
    version, build = release["version"], release["build_number"]
    if not isinstance(version, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ValueError("invalid release version")
    if type(build) is not int or not 1 <= build <= 9999:
        raise ValueError("invalid build number")
    sources = {"uniDesk-arm64.dmg": args.dmg or args.dist / "uniDesk-arm64.dmg",
               "release.json": descriptor}
    if (args.dist / "manifest.json").is_file():
        sources.update({"manifest.json": args.dist / "manifest.json",
                        "uniDesk-arm64-update.zip": args.dist / "uniDesk-arm64-update.zip"})
    checksums = {name: sha256(path) for name, path in sources.items()}
    manifest = "".join(f"{checksum}  {name}\n" for name, checksum in checksums.items())
    destination = root / f"uniDesk-{version}-build{build}"
    if destination.exists():
        for name, checksum in checksums.items():
            if sha256(destination / name) != checksum:
                raise RuntimeError(f"archive conflict, existing release preserved: {destination}")
        if (destination / "SHA256SUMS").read_text() != manifest:
            raise RuntimeError(f"archive checksum manifest differs: {destination}")
        print(f"NAS archive already verified: {destination}")
        return

    root.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=".unidesk-upload-", dir=root))
    try:
        for name, source in sources.items():
            shutil.copyfile(source, staging / name)
            if sha256(staging / name) != checksums[name]:
                raise RuntimeError(f"NAS copy checksum mismatch: {name}")
        (staging / "SHA256SUMS").write_text(manifest)
        staging.rename(destination)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    print(f"NAS archive verified: {destination}")
    print(manifest, end="")


if __name__ == "__main__":
    main()
