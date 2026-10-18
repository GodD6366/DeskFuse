#!/usr/bin/env python3
"""Create redistributable corresponding sources, including pinned submodules."""
import gzip
import hashlib
import json
import pathlib
import subprocess
import sys
import tarfile

root, build, output = map(pathlib.Path, sys.argv[1:])
# Use git's tracked-file list for third parties, plus each pinned git submodule;
# this includes their source/license/build scripts but not temporary build output.
def tracked(checkout):
    files = subprocess.check_output(['git', '-C', str(checkout), 'ls-files', '-z']).split(b'\0')
    result = [checkout / p.decode() for p in files if p]
    selected = ['third_party/abseil-cpp', 'third_party/cares/cares', 'third_party/protobuf', 'third_party/re2', 'third_party/zlib'] if checkout.name == 'grpc_lib' else ['3rdparty/_distfiles'] if checkout.name == 'roc_lib' else []
    for submodule in selected:
        base = checkout / submodule
        subfiles = subprocess.check_output(['git', '-C', str(base), 'ls-files', '-z']).split(b'\0')
        result.extend(base / p.decode() for p in subfiles if p)
    return result

def clean(info):
    info.uid = info.gid = info.mtime = 0
    info.uname = info.gname = ''
    return info

with output.open('wb') as raw, gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as gz, tarfile.open(fileobj=gz, mode='w') as archive:
    def add(path, name):
        if path.is_file(): archive.add(path, arcname=str(pathlib.Path('DeskFuseMicrophone-Sources') / name), recursive=False, filter=clean)
    for directory in ['vendor/roc-vad', 'apps/deskfuse-macos/MicrophoneNative']:
        for path in sorted((root / directory).rglob('*')):
            if 'bin' in path.parts or path.name.startswith('driver_protocol.') and path.suffix in {'.h', '.cc'} or path.name == 'compile_commands.json': continue
            add(path, path.relative_to(root))
    add(root / 'scripts/build-roc-microphone.sh', pathlib.Path('scripts/build-roc-microphone.sh'))
    for name in ['config/previous-installation.json', 'resources/microphone/compatibility.sh', 'scripts/test-microphone-native.sh']:
        add(root / name, pathlib.Path(name))
    lock = json.loads((root / 'vendor/roc-vad/sources.lock.json').read_text())
    allowed_distfiles = {entry['path'] for entry in lock['roc_distfiles']}
    for name in ['roc', 'aspl', 'boringssl', 'grpc', 'fmt', 'spdlog', 'cli11']:
        checkout = build / '3rdparty' / name / 'src' / (name + '_lib')
        for path in sorted(tracked(checkout)):
            relative = str(path.relative_to(checkout))
            if '/_distfiles/tarballs/' in relative and relative not in allowed_distfiles: continue
            add(path, pathlib.Path('dependencies') / name / path.relative_to(checkout))
    for entry in lock['roc_distfiles']:
        path = build / '3rdparty/roc/src/roc_lib' / entry['path']
        add(path, pathlib.Path('dependencies/roc') / entry['path'])

# Keep attribution with every binary, including app-only LAN updates. Full
# sources stay in the build cache; the application carries their exact hash.
with tarfile.open(output, 'r:gz') as archive:
    notices = []
    for member in archive:
        name = pathlib.PurePosixPath(member.name).name.lower()
        if member.isfile() and name.startswith(('license', 'copying', 'copyright', 'notice', 'authors', 'patents')):
            notices.append(member.name + "\n" + archive.extractfile(member).read().decode('utf-8', errors='replace'))
output.with_name('THIRD_PARTY_MICROPHONE.txt').write_text("\n\n".join(notices) + "\n")
output.with_name('source-archive.json').write_text(json.dumps({
    'file': output.name,
    'bundled': False,
    'obtain': '向提供此测试版的维护者索取对应源码，提供应用版本、构建号及此 SHA-256。',
    'sha256': hashlib.sha256(output.read_bytes()).hexdigest(),
    'size': output.stat().st_size,
}, ensure_ascii=False, indent=2) + "\n")
