#!/usr/bin/env python3
"""Check the public Git index/checkout boundary and local Markdown links (offline)."""
from pathlib import Path
import argparse
import hashlib
import re
import json
import sys
import tomllib
import urllib.parse

from repository_files import source_files

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worktree", action="store_true", help="include untracked source and pending deletions without staging")
    args = parser.parse_args()
    files = source_files(worktree=args.worktree)
    errors = []
    forbidden = ("artifacts/", "target/", "dist/", ".agents/", ".codex/", "docs/archive/", "docs/history/", "design/iterations/", "design/current/archive/", ".antigravitycli/", "design/atelier/", "design/proposals/", "resources/generated/")
    generated = ("apps/unidesk-macos/Generated/", "apps/unidesk-macos/Sources/UniDeskEngine/", "apps/unidesk-macos/Sources/UniDeskEngineFFI/")
    for name in sorted(files):
        if name.startswith(generated):
            errors.append(f"generated native binding cannot be published: {name}")
    secret_patterns = [r"-----BEGIN (?:RSA |EC |OPENSSH |ENCRYPTED )?PRIVATE KEY-----", r"\bAKIA[0-9A-Z]{16}\b", r"\bgh[pousr]_[A-Za-z0-9]{30,}\b", r"\bgithub_pat_[A-Za-z0-9_]{40,}\b"]
    # Exact public TLS bootstrap fixtures; never allow arbitrary DER keys/certificates.
    bootstrap_hashes = {
        "resources/unidesk-transport-cert.der": "e1d72b0257f1ff0f91aedbc279cd3f779b971c1a03a14c2a53b3bf473a9b437f",
        "resources/unidesk-transport-key.der": "b673c567f36d4e2acc4f61b8aa82cf1e0fd5e4f275c8f93897c6cd797eab05ac",
    }
    for name in sorted(files):
        path = ROOT / name
        if (name.startswith(forbidden) or "/target/" in name or name.endswith((".p12", ".pfx", ".keychain-db", ".DS_Store"))
                or (name.endswith("Cargo.lock") and name != "Cargo.lock") or name in {"AGENTS.md", "CLAUDE.md", "docs/INPUT_DATAGRAM_PLAN.md", "docs/NATIVE_MIGRATION.md", "skills-lock.json", "node", ".codesign-identity", "identity.key", "signing-key.hex"}
                or any(part in {".build", ".swiftpm", "__pycache__"} for part in path.parts)
                or path.name in {".env", "identity.key", "signing-key.hex"}
                or (path.name.startswith(".env.") and path.name != ".env.example")):
            errors.append(f"excluded local file is tracked: {name}")
        if path.is_symlink() or not path.resolve().is_relative_to(ROOT):
            errors.append(f"symlink or external path cannot be published: {name}")
            continue
        if not path.is_file():
            errors.append(f"tracked file missing: {name}")
            continue
        raw = path.read_bytes()
        if len(raw) > 5 * 1024 * 1024:
            errors.append(f"file exceeds 5 MiB public-source limit: {name}")
        if name in bootstrap_hashes and hashlib.sha256(raw).hexdigest() != bootstrap_hashes[name]:
            errors.append(f"public bootstrap fixture changed: {name}; explicitly review authentication and update fingerprint")
        if "\0" in raw[:8192].decode("utf-8", errors="ignore"):
            continue
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            continue
        if any(re.search(pattern, text) for pattern in secret_patterns):
            errors.append(f"possible credential found (content not printed): {name}")
        if path.suffix == ".md":
            if re.search(r"/Users/[\w.-]+/|(?:chang|godd)@10\.", text):
                errors.append(f"personal machine reference in public document: {name}")
            for link in re.findall(r"!?\[[^\]]*\]\(([^\s)]+)(?:\s+[^)]*)?\)", text):
                parsed = urllib.parse.urlsplit(link)
                if parsed.scheme or link.startswith(("#", "//")):
                    continue
                target = (path.parent / urllib.parse.unquote(parsed.path)).resolve()
                if not target.is_relative_to(ROOT) or not target.exists():
                    errors.append(f"broken local link: {name} -> {link}")
                elif target.is_file() and target.relative_to(ROOT).as_posix() not in files:
                    errors.append(f"local link target excluded from Git: {name} -> {link}")
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())
    for member in workspace["workspace"]["members"]:
        package = tomllib.loads((ROOT / member / "Cargo.toml").read_text())["package"]
        for key in ("version", "edition", "rust-version", "license", "repository", "publish"):
            if package.get(key) != {"workspace": True}:
                errors.append(f"{member}: {key} must inherit workspace")
    toolchain = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    if toolchain != workspace["workspace"]["package"]["rust-version"]:
        errors.append("workspace rust-version differs from pinned toolchain")
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    if any(p['name'].startswith('gpui') for p in lock['package']):
        errors.append("GPUI must not remain in the native client dependency graph")
    reviewed = json.loads((ROOT / "resources/licenses/manifest.json").read_text())
    locked_packages = {f"{p['name']}@{p['version']}" for p in lock['package']}
    for name, record in reviewed.items():
        if name not in locked_packages:
            errors.append(f"reviewed license no longer belongs to lockfile: {name}")
        if record.get("file"):
            license_file = ROOT / "resources/licenses" / record["file"]
            if not license_file.is_file() or hashlib.sha256(license_file.read_bytes()).hexdigest() != record["sha256"]:
                errors.append(f"reviewed license missing or changed: {name}")
    if errors:
        print("\n".join(f"error: {e}" for e in errors), file=sys.stderr)
        return 1
    print(f"Public repository checks passed ({len(files)} files); heuristic scan is not a full credential audit.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
