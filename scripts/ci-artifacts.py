#!/usr/bin/env python3
"""Transfer CI packages through a run-scoped draft in the private source repo."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import zipfile


def gh(*args: str) -> str:
    return subprocess.check_output(['gh', *args], text=True)


def context() -> tuple[str, str, str]:
    repo = os.environ['GITHUB_REPOSITORY']
    run = os.environ['GITHUB_RUN_ID']
    commit = os.environ['GITHUB_SHA']
    if not re.fullmatch(r'[\w.-]+/[\w.-]+', repo) or not run.isdigit() or not re.fullmatch(r'[0-9a-f]{40}', commit):
        raise ValueError('invalid CI artifact context')
    if gh('api', f'repos/{repo}', '--jq', '.private').strip() != 'true':
        raise ValueError('CI packages require a private source repository')
    return repo, f'ci-artifacts-{run}', commit


def verify_draft(repo: str, tag: str, commit: str) -> None:
    release = json.loads(gh('release', 'view', tag, '--repo', repo, '--json', 'isDraft,targetCommitish'))
    if release != {'isDraft': True, 'targetCommitish': commit}:
        raise ValueError('CI draft does not match the current source commit')


def pack(path: Path, files: list[Path], commit: str) -> None:
    names = [p.name for p in files]
    if not files or len(names) != len(set(names)) or '.ci-artifact.json' in names:
        raise ValueError('CI artifact requires distinct input filenames')
    manifest = {'commit': commit, 'files': {}}
    with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_STORED) as archive:
        for source in files:
            if source.is_symlink() or not source.is_file() or source.stat().st_size == 0:
                raise ValueError(f'invalid artifact input: {source}')
            data = source.read_bytes()
            manifest['files'][source.name] = hashlib.sha256(data).hexdigest()
            archive.writestr(source.name, data)
        archive.writestr('.ci-artifact.json', json.dumps(manifest))


def unpack(path: Path, directory: Path, commit: str) -> None:
    with zipfile.ZipFile(path) as archive:
        manifest = json.loads(archive.read('.ci-artifact.json'))
        files = manifest['files']
        expected = set(files) | {'.ci-artifact.json'}
        names = archive.namelist()
        if manifest['commit'] != commit or not files or set(names) != expected or len(names) != len(expected):
            raise ValueError('artifact inventory or commit mismatch')
        payloads = {}
        for name, digest in files.items():
            if name in ('.', '..') or '/' in name or '\\' in name or ':' in name:
                raise ValueError('unsafe artifact filename')
            data = archive.read(name)
            if hashlib.sha256(data).hexdigest() != digest:
                raise ValueError('artifact checksum mismatch')
            payloads[name] = data
    directory.mkdir(parents=True, exist_ok=True)
    for name, data in payloads.items():
        target = directory / name
        if target.is_symlink():
            raise ValueError('artifact destination is a symlink')
        target.write_bytes(data)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['upload', 'download', 'cleanup'])
    parser.add_argument('--name')
    parser.add_argument('--directory', type=Path)
    parser.add_argument('files', nargs='*', type=Path)
    args = parser.parse_args()
    repo, tag, commit = context()
    if args.command != 'cleanup' and (not args.name or not re.fullmatch(r'[A-Za-z0-9_-]+', args.name)):
        parser.error('a safe artifact name is required')
    if args.command == 'upload':
        # Concurrent architecture jobs may race to create the same draft.
        probe = subprocess.run(['gh', 'release', 'view', tag, '--repo', repo], capture_output=True)
        if probe.returncode:
            subprocess.run(['gh', 'release', 'create', tag, '--repo', repo, '--target', commit,
                            '--draft', '--title', tag, '--notes', 'Temporary private CI package handoff.'],
                           capture_output=True)
        verify_draft(repo, tag, commit)
        with tempfile.TemporaryDirectory(prefix='deskfuse-ci-artifact-') as temporary:
            archive = Path(temporary) / f'{args.name}.zip'
            pack(archive, args.files, commit)
            gh('release', 'upload', tag, '--repo', repo, '--clobber', str(archive))
    elif args.command == 'download':
        if args.directory is None:
            parser.error('--directory is required')
        verify_draft(repo, tag, commit)
        with tempfile.TemporaryDirectory(prefix='deskfuse-ci-artifact-') as temporary:
            gh('release', 'download', tag, '--repo', repo, '--pattern', f'{args.name}.zip', '--dir', temporary)
            unpack(Path(temporary) / f'{args.name}.zip', args.directory, commit)
    else:
        verify_draft(repo, tag, commit)
        gh('release', 'delete', tag, '--repo', repo, '--yes')
    print(f'CI artifact {args.command} complete: {tag}')


if __name__ == '__main__':
    main()
