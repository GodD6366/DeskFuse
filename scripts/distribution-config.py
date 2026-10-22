#!/usr/bin/env python3
"""Read the active distribution contract; target names never switch traffic."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent.parent


def load(root: Path = ROOT) -> dict:
    directory = (root / "config").resolve()
    config = json.loads((directory / "deskfuse.json").read_text())
    if config["schema_version"] != 1:
        raise ValueError("unsupported product configuration schema")
    active_path = (directory / config["active_distribution"]).resolve()
    if not active_path.is_relative_to(directory):
        raise ValueError("active distribution must be inside config")
    active = json.loads(active_path.read_text())
    if active["schema_version"] != 1:
        raise ValueError("unsupported distribution configuration schema")
    if set(active["assets"]) != {"arm64", "x86_64", "windows_x64"} or len(set(active["assets"].values())) != 3:
        raise ValueError("distribution must define distinct arm64, x86_64, and Windows assets")
    if not active["assets"]["arm64"].endswith(".dmg") or not active["assets"]["x86_64"].endswith(".dmg"):
        raise ValueError("macOS distribution assets must be DMGs")
    if not active["assets"]["windows_x64"].endswith(".exe"):
        raise ValueError("Windows distribution asset must be an installer EXE")
    for key in ("source_repository", "release_repository", "homebrew_tap"):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", active[key]):
            raise ValueError(f"invalid {key}")
    # 过渡期：旧客户端只认旧发行仓库，过渡版本需要同时镜像过去；过渡完成后删除该字段。
    legacy = active.get("legacy_release_repository")
    if legacy is not None and (not isinstance(legacy, str)
                               or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", legacy)
                               or legacy == active["release_repository"]):
        raise ValueError("invalid legacy_release_repository")
    if not re.fullmatch(r"https://[A-Za-z0-9.-]+(:[0-9]+)?(/[A-Za-z0-9._~/-]*)?", active["release_feed_url"]):
        raise ValueError("invalid release_feed_url")
    for value in (*active["assets"].values(), active["app_bundle"], active["cask_token"]):
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", value):
            raise ValueError("invalid distribution filename or token")
    return active


def release_tag(root: Path = ROOT) -> str:
    descriptor = json.loads((root / "release.json").read_text())
    version = descriptor.get("version")
    if not isinstance(version, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ValueError("invalid release descriptor version")
    return f"v{version}"


def website_projection(active: dict, root: Path = ROOT) -> dict:
    legacy = active.get("legacy_release_repository", "")
    return {
        "schema_version": 1,
        "release_repository": active["release_repository"],
        # 过渡期发行列表要合并旧仓库；回退下载链接指向确实存有该 tag 的仓库。
        "legacy_release_repository": legacy,
        "fallback_release_repository": legacy or active["release_repository"],
        "homebrew_tap": active["homebrew_tap"],
        "cask_token": active["cask_token"],
        "assets": active["assets"],
        "fallback_release_tag": release_tag(root),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    output = parser.add_mutually_exclusive_group()
    output.add_argument("--github-output", type=Path)
    output.add_argument("--website-output", type=Path)
    output.add_argument("--value", help="print one dotted field for packaging scripts")
    parser.add_argument("--check", action="store_true", help="check the generated website config without writing")
    args = parser.parse_args()
    if args.check and not args.website_output:
        parser.error("--check requires --website-output")
    active = load()
    if args.value:
        value = active
        for key in args.value.split("."):
            value = value[key]
        if not isinstance(value, str) or "\n" in value or "\r" in value:
            raise ValueError("requested field must be a single-line string")
        print(value)
        return
    if args.website_output:
        projection = website_projection(active)
        content = json.dumps(projection, indent=2) + "\n"
        if args.check:
            if args.website_output.read_text() != content:
                raise ValueError("website distribution config is stale")
        else:
            args.website_output.parent.mkdir(parents=True, exist_ok=True)
            args.website_output.write_text(content)
        return
    if args.github_output:
        values = {
            "release_repository": active["release_repository"],
            "legacy_release_repository": active.get("legacy_release_repository", ""),
            "arm64_asset": active["assets"]["arm64"],
            "x86_64_asset": active["assets"]["x86_64"],
            "windows_x64_asset": active["assets"]["windows_x64"],
            "cask_path": f'Casks/{active["cask_token"]}.rb',
            "macos_runner": json.dumps(active["macos_runner"], separators=(",", ":")),
        }
        if any("\n" in value or "\r" in value for value in values.values()):
            raise ValueError("multiline workflow output")
        with args.github_output.open("a") as stream:
            for key, value in values.items():
                stream.write(f"{key}={value}\n")
    else:
        print(json.dumps(active, indent=2))


if __name__ == "__main__":
    main()
