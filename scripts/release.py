#!/usr/bin/env python3
"""Release coordinator glue. No signing keys, mutable CI counters, or auto-updater uploads."""
from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request
import uuid
import shutil
import runpy

ROOT = Path(__file__).resolve().parent.parent
DISTRIBUTION = runpy.run_path(str(ROOT / "scripts/distribution-config.py"))["load"]()


def version(root: Path = ROOT) -> str:
    return tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]


def app_zip_name(arch: str) -> str:
    return f"DeskFuse-{version()}-{arch}.zip"


# Current clients install the app zip. DMGs stay so already-shipped clients,
# which only accept those file names, can still update.
# ASSETS[0] must remain the arm64 DMG: build-info dmg_sha256 is that file.
ARM64_APP_ZIP = app_zip_name("arm64")
X86_64_APP_ZIP = app_zip_name("x86_64")
ASSETS = (
    DISTRIBUTION["assets"]["arm64"],
    DISTRIBUTION["assets"]["x86_64"],
    ARM64_APP_ZIP,
    X86_64_APP_ZIP,
    DISTRIBUTION["assets"]["windows_x64"],
    "release.json",
    "build-info.json",
    "SHA256SUMS",
)
MACOS_ASSETS = (
    DISTRIBUTION["assets"]["arm64"],
    DISTRIBUTION["assets"]["x86_64"],
    ARM64_APP_ZIP,
    X86_64_APP_ZIP,
    "release.json",
    "build-info.json",
    "SHA256SUMS",
)
TARGET_ASSETS = {
    DISTRIBUTION["assets"]["arm64"]: "aarch64-apple-darwin",
    DISTRIBUTION["assets"]["x86_64"]: "x86_64-apple-darwin",
    ARM64_APP_ZIP: "aarch64-apple-darwin",
    X86_64_APP_ZIP: "x86_64-apple-darwin",
    DISTRIBUTION["assets"]["windows_x64"]: "win-x64",
}
MACOS_TARGET_ASSETS = {name: target for name, target in TARGET_ASSETS.items()
                       if target.endswith("-apple-darwin")}
# v0.4.5 was published before the Windows asset became part of the public
# distribution contract. Keep it probeable so a retry does not try to replace
# an already-published two-DMG release unless --macos-only is explicit.
LEGACY_RELEASE_VERSIONS = {"0.4.5"}


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


def inventory(directory: Path, commit: str, include_windows: bool = True,
              rustc_version: str | None = None) -> None:
    if not re.fullmatch(r"[a-f0-9]{40}", commit):
        raise ValueError("expected full source commit SHA")
    release = validate(directory / "release.json", version())
    asset_names = ASSETS if include_windows else MACOS_ASSETS
    target_assets = TARGET_ASSETS if include_windows else MACOS_TARGET_ASSETS
    missing = [name for name in asset_names[:-2] if not (directory / name).is_file()]
    if missing:
        raise ValueError(f"release assets missing: {', '.join(missing)}")
    targets = []
    for name, target in target_assets.items():
        sidecar = None
        if name.endswith("arm64.dmg") or name == ARM64_APP_ZIP:
            sidecar = directory / ".signing-arm64"
        elif name.endswith("x86_64.dmg") or name == X86_64_APP_ZIP:
            sidecar = directory / ".signing-x86_64"
        if sidecar is not None and not sidecar.is_file():
            sidecar = directory / ".signing"
        if name.endswith(".dmg") or name in {ARM64_APP_ZIP, X86_64_APP_ZIP}:
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
    # The publish job runs where the compilers are not; it passes the version the
    # build jobs recorded so the published metadata keeps describing the build.
    rustc = rustc_version
    if rustc is None:
        rustc = "unavailable"
        if shutil.which("rustc"):
            rustc = subprocess.check_output(["rustc", "--version"], text=True).strip()
    elif not rustc.startswith("rustc "):
        raise ValueError("rustc version must be the `rustc --version` line")
    write_json(directory / "build-info.json", {
        "commit": commit, "release": release, "targets": targets,
        "minimum_macos": "14.0", "rustc": rustc,
        "dmg_sha256": checksum(directory / asset_names[0]),
    })
    (directory / "SHA256SUMS").write_text("".join(f"{checksum(directory / name)}  {name}\n" for name in asset_names if name != "SHA256SUMS"))
    verify_inventory(directory, release, commit, include_windows=include_windows)


def verify_inventory(directory: Path, release: dict, commit: str, include_windows: bool | None = None,
                     digests: dict[str, str] | None = None) -> None:
    """Validate an inventory from local files, or from GitHub's stored digests.

    `digests` lets the same checks run without downloading the large assets: the
    caller passes sha256 values GitHub reports for the uploaded bytes and only the
    small metadata files are read from `directory`.
    """
    def asset_digest(name: str) -> str:
        if digests is not None:
            if name not in digests:
                raise ValueError("release inventory is missing an uploaded asset")
            return digests[name]
        return checksum(directory / name)

    all_expected = {name for name in ASSETS if name != "SHA256SUMS"}
    legacy_expected = all_expected - {DISTRIBUTION["assets"]["windows_x64"]}
    actual = {}
    for line in (directory / "SHA256SUMS").read_text().splitlines():
        match = re.fullmatch(r"([a-f0-9]{64})  ([A-Za-z0-9._-]+)", line)
        if not match or match[2] not in all_expected or match[2] in actual:
            raise ValueError("invalid checksum manifest")
        actual[match[2]] = match[1]
    if include_windows is None:
        include_windows = DISTRIBUTION["assets"]["windows_x64"] in actual
        if not include_windows and release["version"] not in LEGACY_RELEASE_VERSIONS:
            raise ValueError("Windows installer required for this release")
    expected = all_expected if include_windows else legacy_expected
    target_map = TARGET_ASSETS if include_windows else MACOS_TARGET_ASSETS
    if set(actual) != expected or any(asset_digest(name) != digest for name, digest in actual.items()):
        raise ValueError("release assets failed checksum verification")
    if json.loads((directory / "release.json").read_text()) != release:
        raise ValueError("existing release descriptor conflicts with source")
    info = json.loads((directory / "build-info.json").read_text())
    targets = info.get("targets")
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
            or info.get("dmg_sha256") != actual[MACOS_ASSETS[0]]):
        raise ValueError("existing build information conflicts with source")


def token() -> str:
    value = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if not value:
        raise ValueError("GH_TOKEN required")
    return value


def api(path: str, *, missing_ok: bool = False) -> dict | None:
    token_value = token()
    request = urllib.request.Request("https://api.github.com/" + path, headers={
        "Authorization": f"Bearer {token_value}", "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "deskfuse-release",
    })
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if error.code == 404 and missing_ok:
            return None
        raise ValueError(f"GitHub API request failed: HTTP {error.code}") from None


def remote_asset_digests(repo: str, tag: str, release: dict,
                         include_windows: bool = True,
                         remote: dict | None = None) -> dict[str, str] | None:
    """Return the sha256 GitHub stores for each published asset.

    Returns None when this GitHub deployment does not report digests; callers then
    verify by downloading the assets instead.
    """
    remote = remote if remote is not None else release_record(repo, tag)
    assets = {item["name"]: item for item in remote["assets"]}
    expected = set(ASSETS if include_windows else MACOS_ASSETS)
    legacy = expected - {DISTRIBUTION["assets"]["windows_x64"]}
    if assets.keys() != expected and not (
            include_windows and assets.keys() == legacy
            and release["version"] in LEGACY_RELEASE_VERSIONS):
        raise ValueError("existing release type/assets conflict; refusing to overwrite")
    digests = {}
    for name, item in assets.items():
        digest = item.get("digest") or ""
        if not digest.startswith("sha256:"):
            return None
        digests[name] = digest.split(":", 1)[1]
    return digests


def remote_release(repo: str, tag: str, *, missing_ok: bool = False) -> dict | None:
    return api(f"repos/{repo}/releases/tags/{urllib.parse.quote(tag, safe='')}", missing_ok=missing_ok)


def release_record(repo: str, tag: str) -> dict:
    """Return the release for `tag`, including a draft that is not tag-addressable yet."""
    remote = remote_release(repo, tag, missing_ok=True)
    if remote is not None:
        return remote
    for item in api(f"repos/{repo}/releases?per_page=100") or []:
        if item.get("tag_name") == tag and item.get("draft"):
            return item
    raise ValueError(f"release {tag} is missing")


def verify_ci(repo: str, commit: str, job: str = "macOS arm64 checks") -> None:
    """Require the macOS check job to have passed for the tagged commit.

    Re-running the whole CI suite inside the release only repeated work the tag's
    commit had already validated; reading the recorded result keeps the guarantee
    and costs seconds. A failing Windows job must not block a macOS release, so
    the decision is made per job instead of per workflow run.
    """
    if not re.fullmatch(r"[a-f0-9]{40}", commit):
        raise ValueError("expected full source commit SHA")
    checks = api(f"repos/{repo}/commits/{commit}/check-runs?per_page=100")["check_runs"]
    names = {job, f"checks / {job}"}
    matching = [check for check in checks if check.get("name") in names]
    if not matching:
        raise ValueError(f"no `{job}` check run for {commit}; push the release commit and wait for CI before tagging")
    if any(check.get("conclusion") == "success" for check in matching):
        return
    raise ValueError(f"`{job}` did not succeed for {commit}; wait for a green CI run before tagging")


def download_assets(repo: str, tag: str, directory: Path, names: set[str] | None = None) -> None:
    """Download release assets directly.

    `gh release download` moves the same bytes at a fraction of the speed on this
    route, and the metadata-only verification path should not pay for it at all.
    """
    remote = remote_release(repo, tag)
    if remote is None:
        raise ValueError("release is missing")
    for item in remote["assets"]:
        if names is not None and item["name"] not in names:
            continue
        target = directory / item["name"]
        run_curl(["--location", "--output", str(target), item["browser_download_url"]])


def run_curl(args: list[str]) -> None:
    """Move release bytes with curl; `gh` and urllib both crawl on this link."""
    subprocess.run(["curl", "--fail", "--silent", "--show-error", "--retry", "3",
                    "--retry-delay", "2", "--connect-timeout", "30", *args], check=True)


def download_metadata(repo: str, tag: str, directory: Path) -> None:
    download_assets(repo, tag, directory, {"release.json", "build-info.json", "SHA256SUMS"})


UPLOAD_CONCURRENCY = 4
UPLOAD_ATTEMPTS = 3


def upload_asset(url: str, path: Path) -> None:
    """POST the file body to a release upload URL, retrying transient failures."""
    token_value = token()
    for attempt in range(1, UPLOAD_ATTEMPTS + 1):
        try:
            run_curl(["--max-time", "900", "--request", "POST",
                      "--output", os.devnull,
                      "--header", f"Authorization: Bearer {token_value}",
                      "--header", "Accept: application/vnd.github+json",
                      "--header", "Content-Type: application/octet-stream",
                      "--header", "X-GitHub-Api-Version: 2022-11-28",
                      "--data-binary", f"@{path}", url])
            return
        except subprocess.CalledProcessError as error:
            if attempt == UPLOAD_ATTEMPTS:
                raise ValueError(f"failed to upload {path.name}: curl exit {error.returncode}") from None
        print(f"upload retry {attempt} for {path.name}", flush=True)
        time.sleep(2 * attempt)


def upload_assets(directory: Path, repo: str, tag: str, names: tuple[str, ...]) -> None:
    """Upload every asset on its own connection and report how long each took.

    A single stalled stream must not serialize the whole release, and per-asset
    timing makes the slow one visible in the job log.
    """
    remote = release_record(repo, tag)
    base = remote["upload_url"].split("{", 1)[0]

    def upload(name: str) -> None:
        started = time.monotonic()
        upload_asset(f"{base}?name={urllib.parse.quote(name)}", directory / name)
        print(f"uploaded {name} in {time.monotonic() - started:.1f}s", flush=True)

    failures: list[Exception] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=min(UPLOAD_CONCURRENCY, len(names))) as pool:
        futures = [pool.submit(upload, name) for name in names]
        for future in concurrent.futures.as_completed(futures):
            try:
                future.result()
            except Exception as error:  # surfaced again after every upload settles
                failures.append(error)
    if failures:
        raise failures[0]


def probe(repo: str, tag: str, commit: str, include_windows: bool = True) -> bool:
    # A repository lookup disambiguates a missing tag from missing repo access.
    api(f"repos/{repo}")
    release = validate(ROOT / "release.json", version(), tag)
    remote = remote_release(repo, tag, missing_ok=True)
    if remote is None:
        if f"## [{version()}]" not in (ROOT / "CHANGELOG.md").read_text():
            raise ValueError("CHANGELOG entry required for release version")
        return False
    if remote["draft"]:
        raise ValueError("unfinished release draft exists; inspect/delete that draft before retrying (do not recreate the tag)")
    if not remote["prerelease"]:
        raise ValueError("existing release is not a prerelease; refusing to overwrite")
    digests = remote_asset_digests(repo, tag, release, include_windows, remote=remote)
    remote_assets = {item["name"] for item in remote["assets"]}
    with tempfile.TemporaryDirectory(prefix="deskfuse-release-verify-") as temporary:
        target = Path(temporary)
        if digests is None:
            download_assets(repo, tag, target)
            verify_inventory(target, release, commit,
                             include_windows=DISTRIBUTION["assets"]["windows_x64"] in remote_assets)
        else:
            download_metadata(repo, tag, target)
            verify_inventory(target, release, commit,
                             include_windows=DISTRIBUTION["assets"]["windows_x64"] in remote_assets,
                             digests=digests)
    return True


def publish(directory: Path, repo: str, tag: str, commit: str, include_windows: bool = True) -> None:
    release = validate(ROOT / "release.json", version(), tag)
    verify_inventory(directory, release, commit, include_windows=include_windows)
    if probe(repo, tag, commit, include_windows=include_windows):
        print("Existing published release verified; unchanged.")
        return
    info = json.loads((directory / "build-info.json").read_text())
    arm64_info = next(item for item in info["targets"] if item["asset"] == DISTRIBUTION["assets"]["arm64"])
    if arm64_info.get("signing") == "developer-id" and arm64_info.get("notarized") is True:
        signing_note = "**Developer ID signed and notarized.** Gatekeeper should accept the stapled app and DMG; do not disable SIP."
    else:
        signing_note = "**Experimental prerelease: ad-hoc signed, not notarized.** macOS may block launch; see README installation limitations."
    notes = (f"macOS 14+ · Apple Silicon (arm64) and Intel (x86_64)\n\n"
             "Website download: open the app zip and follow the install window. "
             "Homebrew also installs the app zip. An installed Developer ID build replaces itself from "
             f"`{ARM64_APP_ZIP}` and does not show that window. "
             "Already shipped clients only recognize the DMG, so this release still includes it.\n\n"
             f"{signing_note}\n\n"
             f"Source: `{commit}` · Build: `{release['build_number']}` · Release ID: `{release['release_id']}`\n\n"
             "Build/package checks do not establish Finder paste, drag-out, native input permissions or physical two-device acceptance. "
             "The app zip is not a LAN-signed manifest.\n\n")
    changelog = (ROOT / "CHANGELOG.md").read_text()
    heading = f"## [{version()}]"
    if heading not in changelog:
        raise ValueError("CHANGELOG entry required for release version")
    notes += changelog.split(heading, 1)[1].split("\n## ", 1)[0].strip()
    with tempfile.TemporaryDirectory(prefix="deskfuse-release-notes-") as temporary:
        path = Path(temporary) / "notes.md"
        path.write_text(notes)
        subprocess.run(["gh", "release", "create", tag, "--repo", repo, "--verify-tag", "--draft", "--prerelease", "--title", f"DeskFuse {tag} (prerelease)", "--notes-file", str(path)], check=True)
        asset_names = ASSETS if include_windows else MACOS_ASSETS
        upload_assets(directory, repo, tag, asset_names)
        # Verify the uploaded bytes while still a draft; a mismatch leaves an
        # unpublished draft. GitHub reports the sha256 of what it stored, so the
        # large assets only have to be downloaded when digests are unavailable.
        digests = remote_asset_digests(repo, tag, release, include_windows)
        if digests is None:
            # The digest can lag a moment behind the asset record after upload.
            time.sleep(5)
            digests = remote_asset_digests(repo, tag, release, include_windows)
        if digests is None:
            download = Path(temporary) / "download"
            download.mkdir()
            download_assets(repo, tag, download)
            verify_inventory(download, release, commit, include_windows=include_windows)
        else:
            for name, digest in digests.items():
                if checksum(directory / name) != digest:
                    raise ValueError(f"uploaded asset digest mismatch: {name}")
        subprocess.run(["gh", "release", "edit", tag, "--repo", repo, "--draft=false", "--prerelease", "--latest=false"], check=True)
    if not probe(repo, tag, commit, include_windows=include_windows):
        raise ValueError("release publication was not confirmed")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["version", "prepare", "validate", "inventory", "probe", "publish", "verify-ci"])
    parser.add_argument("--descriptor", type=Path, default=ROOT / "release.json")
    parser.add_argument("--directory", type=Path, default=ROOT / "dist")
    parser.add_argument("--tag")
    parser.add_argument("--repo", default=os.environ.get("DESKFUSE_RELEASE_REPOSITORY", DISTRIBUTION["release_repository"]))
    parser.add_argument("--commit", default=os.environ.get("GITHUB_SHA"))
    parser.add_argument("--rustc", default=None, help="`rustc --version` line recorded by the build jobs")
    parser.add_argument("--macos-only", action="store_true", help="publish and verify only the macOS DMGs and app zips")
    args = parser.parse_args()
    if args.command == "version":
        print(version())
    elif args.command == "prepare":
        prepare(args.descriptor)
    elif args.command == "validate":
        validate(args.descriptor, version(), args.tag)
        print("release descriptor verified")
    elif args.command == "inventory":
        inventory(args.directory, args.commit or "", include_windows=not args.macos_only, rustc_version=args.rustc)
    elif args.command == "verify-ci":
        if not args.repo or not args.commit:
            parser.error("--repo and --commit required")
        verify_ci(args.repo, args.commit)
        print("release commit checks passed")
    else:
        if not args.tag or not args.repo or not args.commit:
            parser.error("--repo, --tag and --commit required")
        if args.command == "publish":
            publish(args.directory, args.repo, args.tag, args.commit, include_windows=not args.macos_only)
        else:
            existing = probe(args.repo, args.tag, args.commit, include_windows=not args.macos_only)
            if os.environ.get("GITHUB_OUTPUT"):
                with open(os.environ["GITHUB_OUTPUT"], "a") as out:
                    out.write(f"exists={str(existing).lower()}\n")
            print("existing release verified" if existing else "new release")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"error: {error}")
