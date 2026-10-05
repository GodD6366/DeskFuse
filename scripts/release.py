#!/usr/bin/env python3
"""Release coordinator glue. No signing keys, mutable CI counters, or auto-updater uploads."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import tomllib
import urllib.error
import urllib.parse
import urllib.request
import uuid
import shutil
import runpy

ROOT = Path(__file__).resolve().parent.parent
DISTRIBUTION = runpy.run_path(str(ROOT / "scripts/distribution-config.py"))["load"]()
ASSETS = (
    DISTRIBUTION["assets"]["arm64"],
    DISTRIBUTION["assets"]["x86_64"],
    DISTRIBUTION["assets"]["windows_x64"],
    "release.json",
    "build-info.json",
    "SHA256SUMS",
)
TARGET_ASSETS = {
    DISTRIBUTION["assets"]["arm64"]: "aarch64-apple-darwin",
    DISTRIBUTION["assets"]["x86_64"]: "x86_64-apple-darwin",
    DISTRIBUTION["assets"]["windows_x64"]: "win-x64",
}
LEGACY_TARGET_ASSETS = {name: target for name, target in TARGET_ASSETS.items() if name != DISTRIBUTION["assets"]["windows_x64"]}
# v0.4.5 was published before the Windows asset became part of the public
# distribution contract. Keep it probeable so a retry does not try to replace
# an already-published two-DMG release; every later release must include it.
LEGACY_RELEASE_VERSIONS = {"0.4.5"}


def version(root: Path = ROOT) -> str:
    return tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]


def validate(path: Path, expected: str, tag: str | None = None) -> dict:
    data = json.loads(path.read_text())
    if set(data) != {"version", "build_number", "release_id"}:
        raise ValueError("unexpected release descriptor fields")
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", expected):
        raise ValueError("first-release pipeline requires X.Y.Z (GitHub marks it prerelease)")
    if data["version"] != expected or (tag is not None and tag != f"v{expected}"):
        raise ValueError("tag, workspace version and release descriptor must match")
    if type(data["build_number"]) is not int or not 1 <= data["build_number"] <= 9999:
        raise ValueError("CFBundleVersion build must be in 1..9999")
    if not isinstance(data["release_id"], str) or not uuid.UUID(data["release_id"]).int:
        raise ValueError("release_id must be a nonzero UUID")
    return data


def checksum(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def write_json(path: Path, data: dict) -> None:
    path.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")


def prepare(out: Path) -> None:
    if out.exists():
        raise ValueError("descriptor already exists; remove it explicitly only when preparing a NEW release")
    ledger = Path(os.environ.get("DESKFUSE_RELEASE_LEDGER", str(Path.home() / "Library/Application Support/DeskFuse-release/build-ledger"))).resolve()
    if ledger.is_relative_to(ROOT.resolve()):
        raise ValueError("release ledger must stay outside the checkout")
    ledger.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["cargo", "run", "--locked", "--quiet", "-p", "deskfuse-update", "--bin", "deskfuse-release", "--", "next", version(), str(ledger), str(out)], cwd=ROOT, check=True)
    validate(out, version())


def signing_status(directory: Path, sidecar: Path | None = None) -> tuple[str, bool]:
    allowed = {"ad-hoc", "apple-development", "developer-id"}
    sidecar = sidecar or directory / ".signing"
    if sidecar.is_file():
        lines = [line.strip() for line in sidecar.read_text().splitlines() if line.strip()]
        if len(lines) != 2 or lines[0] not in allowed or lines[1] not in {"true", "false"}:
            raise ValueError("invalid packaging signing sidecar")
        kind, notarized = lines[0], lines[1] == "true"
    else:
        kind = os.environ.get("DESKFUSE_SIGNING", "ad-hoc")
        if kind not in allowed:
            raise ValueError("DESKFUSE_SIGNING must be ad-hoc, apple-development, or developer-id")
        notarized = os.environ.get("DESKFUSE_NOTARIZED", "").lower() in {"1", "true", "yes"}
    if notarized and kind != "developer-id":
        raise ValueError("only Developer ID Application builds can be notarized")
    return kind, notarized


def inventory(directory: Path, commit: str) -> None:
    if not re.fullmatch(r"[a-f0-9]{40}", commit):
        raise ValueError("expected full source commit SHA")
    release = validate(directory / "release.json", version())
    missing = [name for name in ASSETS[:-2] if not (directory / name).is_file()]
    if missing:
        raise ValueError(f"release assets missing: {', '.join(missing)}")
    targets = []
    for name, target in TARGET_ASSETS.items():
        sidecar = None
        if name.endswith("arm64.dmg"):
            sidecar = directory / ".signing-arm64"
        elif name.endswith("x86_64.dmg"):
            sidecar = directory / ".signing-x86_64"
        if sidecar is not None and not sidecar.is_file():
            sidecar = directory / ".signing"
        if name.endswith(".dmg"):
            kind, notarized = signing_status(directory, sidecar)
            target_info = {
                "asset": name,
                "target": target,
                "sha256": checksum(directory / name),
                "signing": kind,
                "notarized": notarized,
            }
        else:
            manifest_path = directory / f"{name}.manifest.json"
            authenticode = "unknown"
            if manifest_path.is_file():
                manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
                if (manifest.get("target") != target
                        or manifest.get("installer_sha256") != checksum(directory / name)
                        or any(manifest.get(key) != value for key, value in release.items())):
                    raise ValueError("Windows installer manifest conflicts with asset")
                authenticode = manifest.get("authenticode", "unknown")
                if manifest.get("release_signed") is not True:
                    raise ValueError("Windows installer payload is not release signed")
            target_info = {
                "asset": name,
                "target": target,
                "sha256": checksum(directory / name),
                "signing": "authenticode" if authenticode == "Valid" else "not-signed",
                "notarized": False,
                "authenticode": authenticode,
            }
        targets.append(target_info)
    rustc = "unavailable"
    if shutil.which("rustc"):
        rustc = subprocess.check_output(["rustc", "--version"], text=True).strip()
    write_json(directory / "build-info.json", {
        "commit": commit, "release": release, "targets": targets,
        "minimum_macos": "14.0", "rustc": rustc,
        "dmg_sha256": checksum(directory / ASSETS[0]),
    })
    (directory / "SHA256SUMS").write_text("".join(f"{checksum(directory / name)}  {name}\n" for name in ASSETS if name != "SHA256SUMS"))
    verify_inventory(directory, release, commit)


def verify_inventory(directory: Path, release: dict, commit: str) -> None:
    all_expected = {name for name in ASSETS if name != "SHA256SUMS"}
    legacy_expected = all_expected - {DISTRIBUTION["assets"]["windows_x64"]}
    actual = {}
    for line in (directory / "SHA256SUMS").read_text().splitlines():
        match = re.fullmatch(r"([a-f0-9]{64})  ([A-Za-z0-9._-]+)", line)
        if not match or match[2] not in all_expected or match[2] in actual:
            raise ValueError("invalid checksum manifest")
        actual[match[2]] = match[1]
    expected = all_expected if DISTRIBUTION["assets"]["windows_x64"] in actual else legacy_expected
    if expected == legacy_expected and release["version"] not in LEGACY_RELEASE_VERSIONS:
        raise ValueError("Windows installer required for this release")
    if set(actual) != expected or any(checksum(directory / name) != digest for name, digest in actual.items()):
        raise ValueError("release assets failed checksum verification")
    if json.loads((directory / "release.json").read_text()) != release:
        raise ValueError("existing release descriptor conflicts with source")
    info = json.loads((directory / "build-info.json").read_text())
    targets = info.get("targets")
    target_map = TARGET_ASSETS if expected == all_expected else LEGACY_TARGET_ASSETS
    if not isinstance(targets, list) or len(targets) != len(target_map):
        raise ValueError("existing build information conflicts with source")
    by_asset = {}
    for target_info in targets:
        if not isinstance(target_info, dict):
            raise ValueError("existing build information conflicts with source")
        asset = target_info.get("asset")
        if asset in by_asset or asset not in target_map:
            raise ValueError("existing build information conflicts with source")
        if target_info.get("target") != target_map[asset] or target_info.get("sha256") != actual[asset]:
            raise ValueError("existing build information conflicts with source")
        if target_info.get("signing") not in {"ad-hoc", "apple-development", "developer-id", "authenticode", "not-signed"}:
            raise ValueError("existing build information conflicts with source")
        if not isinstance(target_info.get("notarized"), bool):
            raise ValueError("existing build information conflicts with source")
        if target_info["notarized"] and target_info["signing"] != "developer-id":
            raise ValueError("existing build information conflicts with source")
        if asset == DISTRIBUTION["assets"]["windows_x64"]:
            if target_info.get("signing") != "authenticode" or target_info.get("authenticode") != "Valid":
                raise ValueError("Windows installer signing metadata is invalid")
        by_asset[asset] = target_info
    if set(by_asset) != set(target_map):
        raise ValueError("existing build information conflicts with source")
    if (info.get("commit") != commit or info.get("release") != release
            or info.get("dmg_sha256") != actual[ASSETS[0]]):
        raise ValueError("existing build information conflicts with source")


def api(path: str, *, missing_ok: bool = False) -> dict | None:
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if not token:
        raise ValueError("GH_TOKEN required")
    request = urllib.request.Request("https://api.github.com/" + path, headers={
        "Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "deskfuse-release",
    })
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if error.code == 404 and missing_ok:
            return None
        raise ValueError(f"GitHub API request failed: HTTP {error.code}") from None


def probe(repo: str, tag: str, commit: str) -> bool:
    # A repository lookup disambiguates a missing tag from missing repo access.
    api(f"repos/{repo}")
    release = validate(ROOT / "release.json", version(), tag)
    remote = api(f"repos/{repo}/releases/tags/{urllib.parse.quote(tag, safe='')}", missing_ok=True)
    if remote is None:
        if f"## [{version()}]" not in (ROOT / "CHANGELOG.md").read_text():
            raise ValueError("CHANGELOG entry required for release version")
        return False
    if remote["draft"]:
        raise ValueError("unfinished release draft exists; inspect/delete that draft before retrying (do not recreate the tag)")
    if not remote["prerelease"]:
        raise ValueError("existing release is not a prerelease; refusing to overwrite")
    remote_assets = {item["name"] for item in remote["assets"]}
    expected_assets = set(ASSETS)
    legacy_assets = expected_assets - {DISTRIBUTION["assets"]["windows_x64"]}
    if remote_assets != expected_assets and not (
            remote_assets == legacy_assets and release["version"] in LEGACY_RELEASE_VERSIONS):
        raise ValueError("existing release type/assets conflict; refusing to overwrite")
    with tempfile.TemporaryDirectory(prefix="deskfuse-release-verify-") as temporary:
        subprocess.run(["gh", "release", "download", tag, "--repo", repo, "--dir", temporary], check=True)
        verify_inventory(Path(temporary), release, commit)
    return True


def publish(directory: Path, repo: str, tag: str, commit: str) -> None:
    release = validate(ROOT / "release.json", version(), tag)
    verify_inventory(directory, release, commit)
    if probe(repo, tag, commit):
        print("Existing published release verified; unchanged.")
        return
    info = json.loads((directory / "build-info.json").read_text())
    arm64_info = next(item for item in info["targets"] if item["asset"] == DISTRIBUTION["assets"]["arm64"])
    if arm64_info.get("signing") == "developer-id" and arm64_info.get("notarized") is True:
        signing_note = "**Developer ID signed and notarized.** Gatekeeper should accept the stapled DMG; do not disable SIP."
    else:
        signing_note = "**Experimental prerelease: ad-hoc signed, not notarized.** macOS may block launch; see README installation limitations."
    notes = (f"macOS 14+ · Apple Silicon (arm64) and Intel (x86_64) DMG\n\n"
             f"{signing_note}\n\n"
             f"Source: `{commit}` · Build: `{release['build_number']}` · Release ID: `{release['release_id']}`\n\n"
             "Build/package checks do not establish Finder paste, drag-out, native input permissions or physical two-device acceptance. "
             "No signed LAN auto-update payload is published.\n\n")
    changelog = (ROOT / "CHANGELOG.md").read_text()
    heading = f"## [{version()}]"
    if heading not in changelog:
        raise ValueError("CHANGELOG entry required for release version")
    notes += changelog.split(heading, 1)[1].split("\n## ", 1)[0].strip()
    with tempfile.TemporaryDirectory(prefix="deskfuse-release-notes-") as temporary:
        path = Path(temporary) / "notes.md"
        path.write_text(notes)
        subprocess.run(["gh", "release", "create", tag, "--repo", repo, "--verify-tag", "--draft", "--prerelease", "--title", f"DeskFuse {tag} (prerelease)", "--notes-file", str(path)], check=True)
        subprocess.run(["gh", "release", "upload", tag, "--repo", repo, *[str(directory / name) for name in ASSETS]], check=True)
        # Verify uploaded bytes while still a draft. Failure leaves an unpublished draft.
        download = Path(temporary) / "download"
        subprocess.run(["gh", "release", "download", tag, "--repo", repo, "--dir", str(download)], check=True)
        verify_inventory(download, release, commit)
        subprocess.run(["gh", "release", "edit", tag, "--repo", repo, "--draft=false", "--prerelease", "--latest=false"], check=True)
    if not probe(repo, tag, commit):
        raise ValueError("release publication was not confirmed")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["version", "prepare", "validate", "inventory", "probe", "publish"])
    parser.add_argument("--descriptor", type=Path, default=ROOT / "release.json")
    parser.add_argument("--directory", type=Path, default=ROOT / "dist")
    parser.add_argument("--tag")
    parser.add_argument("--repo", default=os.environ.get("DESKFUSE_RELEASE_REPOSITORY", DISTRIBUTION["release_repository"]))
    parser.add_argument("--commit", default=os.environ.get("GITHUB_SHA"))
    args = parser.parse_args()
    if args.command == "version":
        print(version())
    elif args.command == "prepare":
        prepare(args.descriptor)
    elif args.command == "validate":
        validate(args.descriptor, version(), args.tag)
        print("release descriptor verified")
    elif args.command == "inventory":
        inventory(args.directory, args.commit or "")
    else:
        if not args.tag or not args.repo or not args.commit:
            parser.error("--repo, --tag and --commit required")
        if args.command == "publish":
            publish(args.directory, args.repo, args.tag, args.commit)
        else:
            existing = probe(args.repo, args.tag, args.commit)
            if os.environ.get("GITHUB_OUTPUT"):
                with open(os.environ["GITHUB_OUTPUT"], "a") as out:
                    out.write(f"exists={str(existing).lower()}\n")
            print("existing release verified" if existing else "new release")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"error: {error}")
