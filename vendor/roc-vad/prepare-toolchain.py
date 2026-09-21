#!/usr/bin/env python3
"""Invalidate only owned compiler output when Xcode/SDK changes; preserve sources."""
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys

build = pathlib.Path(sys.argv[1]).resolve()
if build.parent.name != 'roc-microphone' or not build.name.startswith('build-'):
    raise SystemExit('Refusing an unexpected build cache path')
fingerprint = {key: os.environ[key] for key in ['CC', 'CXX', 'SDKROOT']}
fingerprint['compiler_version'] = subprocess.check_output([os.environ['CXX'], '--version'], text=True)
settings = pathlib.Path(os.environ['SDKROOT']) / 'SDKSettings.json'
fingerprint['sdk_settings_sha256'] = hashlib.sha256(settings.read_bytes()).hexdigest()
marker = build / 'toolchain.json'
previous = json.loads(marker.read_text()) if marker.exists() else None

def remove(path):
    if path.is_dir() and not path.is_symlink(): shutil.rmtree(path)
    elif path.exists() or path.is_symlink(): path.unlink()

if previous != fingerprint:
    # All paths are under this product's private target/ cache. Git source
    # checkouts, pinned archives and local patches are deliberately retained.
    for item in build.iterdir():
        if item.name not in {'3rdparty', 'toolchain.json'}: remove(item)
    thirdparty = build / '3rdparty'
    if thirdparty.exists():
        for prefix in thirdparty.iterdir():
            if not prefix.is_dir():
                remove(prefix)
                continue
            for name in ['bin', 'include', 'lib', 'share']:
                remove(prefix / name)
            source_root = prefix / 'src'
            if not source_root.exists(): continue
            for item in source_root.glob('*_lib-build'): remove(item)
            for stamp in source_root.glob('*_lib-stamp'):
                for item in stamp.iterdir():
                    # Keep completed downloads so SDK changes never reclone.
                    if any(part in item.name for part in ['download', 'gitclone-lastrun', 'gitinfo']): continue
                    remove(item)
        roc = thirdparty / 'roc/src/roc_lib'
        for name in ['build', 'bin', '.sconf_temp', 'config.log', 'compile_commands.json']:
            remove(roc / name)
        for item in roc.glob('.sconsign*.dblite'): remove(item)
    print('Roc microphone: invalidated compiler caches for selected Xcode/SDK')
marker.write_text(json.dumps(fingerprint, indent=2)+'\n')
