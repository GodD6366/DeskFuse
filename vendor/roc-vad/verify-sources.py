#!/usr/bin/env python3
"""Verify dependency checkouts and git-pinned transitive submodules, never run tests."""
import hashlib
import json
import pathlib
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parent
build = pathlib.Path(sys.argv[1]).resolve()
partial = '--if-present' in sys.argv
lock = json.loads((root / 'sources.lock.json').read_text())
paths = ['roc', 'aspl', 'boringssl', 'grpc', 'fmt', 'spdlog', 'cli11']
for name, source in zip(paths, lock['dependencies'], strict=True):
    checkout = build / '3rdparty' / name / 'src' / (name + '_lib')
    if partial and not (checkout / '.git').exists():
        continue
    head = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
    if head != source['commit']:
        raise SystemExit('Dependency commit mismatch: ' + name)
    excluded = [':!'+p['path'] for p in lock.get('dependency_patches', []) if p['parent'] == name]
    subprocess.run(['git', '-C', str(checkout), 'diff', '--exit-code', '--quiet', '--ignore-submodules', 'HEAD', '--', '.']+excluded, check=True)
    selected = ['third_party/abseil-cpp', 'third_party/cares/cares', 'third_party/protobuf', 'third_party/re2', 'third_party/zlib'] if name == 'grpc' else []
    command = ['git', '-C', str(checkout), 'submodule', 'status']
    command += ['--'] + selected if selected else ['--recursive']
    modules = subprocess.check_output(command, text=True)
    if any(line and line[0] != ' ' for line in modules.splitlines()):
        raise SystemExit('Transitive dependency commit mismatch: ' + name)
for entry in lock.get('submodules', []):
    checkout = build / '3rdparty' / entry['parent'] / 'src' / (entry['parent'] + '_lib') / entry['path']
    if partial and not (checkout / '.git').exists(): continue
    head = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
    tree = subprocess.check_output(['git', '-C', str(checkout), 'ls-tree', '-rz', '--full-tree', 'HEAD'])
    if head != entry['commit'] or hashlib.sha256(tree).hexdigest() != entry['git_tree_sha256']:
        raise SystemExit('Submodule identity mismatch: ' + entry['path'])
for entry in lock.get('dependency_patches', []):
    path = build / '3rdparty' / entry['parent'] / 'src' / (entry['parent'] + '_lib') / entry['path']
    if partial: continue  # Patch command runs as part of bootstrap.
    if hashlib.sha256(path.read_bytes()).hexdigest() != entry['sha256']:
        raise SystemExit('Dependency build patch mismatch: ' + entry['path'])
# Roc's embedded tar archives and fetched OpenFEC have their own immutable hashes.
roc = build / '3rdparty/roc/src/roc_lib'
for entry in lock.get('roc_distfiles', []):
    path = roc / entry['path']
    if partial and not path.exists():
        continue
    if hashlib.sha256(path.read_bytes()).hexdigest() != entry['sha256']:
        raise SystemExit('Roc dependency archive mismatch: ' + entry['path'])
print('Verified pinned Roc microphone dependencies')
