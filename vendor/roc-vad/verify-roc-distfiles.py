#!/usr/bin/env python3
"""Pin Roc's compiled/build-tool distribution archives before its build starts."""
import hashlib
import json
import pathlib
import sys
import urllib.request
lock=json.loads((pathlib.Path(__file__).parent/'sources.lock.json').read_text())
checkout=pathlib.Path(sys.argv[1])
for entry in lock['roc_distfiles']:
    path=checkout/entry['path']
    if not path.exists() and entry.get('url'):
        with urllib.request.urlopen(entry['url'],timeout=120) as response:
            path.write_bytes(response.read())
    if hashlib.sha256(path.read_bytes()).hexdigest()!=entry['sha256']:
        raise SystemExit('Roc distribution checksum mismatch: '+entry['path'])
