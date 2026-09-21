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
    if set(active["assets"]) != {"arm64", "x86_64"} or any(
        not name.endswith(".dmg") for name in active["assets"].values()
    ) or len(set(active["assets"].values())) != 2:
        raise ValueError("distribution must define distinct arm64 and x86_64 DMG assets")
    for key in ("source_repository", "release_repository", "homebrew_tap"):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", active[key]):
            raise ValueError(f"invalid {key}")
    for value in (*active["assets"].values(), active["app_bundle"], active["cask_token"]):
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", value):
            raise ValueError("invalid distribution filename or token")
    return active


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
        product = json.loads((ROOT / "config/deskfuse.json").read_text())
        legacy = json.loads((ROOT / "config/legacy/distribution.json").read_text())
        projection = {
            "schema_version": 1,
            "release_repository": active["release_repository"],
            "homebrew_tap": active["homebrew_tap"],
            "cask_token": active["cask_token"],
            "assets": active["assets"],
            "preferred_assets": product["target"]["assets"],
            "legacy_assets": legacy["assets"],
            "fallback_assets": legacy["assets"],
            "fallback_release_tag": legacy["fallback_release_tag"],
        }
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
            "arm64_asset": active["assets"]["arm64"],
            "x86_64_asset": active["assets"]["x86_64"],
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
