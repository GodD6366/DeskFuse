#!/usr/bin/env python3
"""Export locked target dependency license declarations and available license texts."""
import argparse
import json
import hashlib
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--target', default='aarch64-apple-darwin')
args = parser.parse_args()
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version', '1', '--filter-platform', args.target], cwd=ROOT))
resolved = {n['id'] for n in metadata['resolve']['nodes']}
packages = sorted((p for p in metadata['packages'] if p['id'] in resolved and p.get('source')), key=lambda p: (p['name'],p['version']))
lines = ['# Locked third-party dependency notices', '', f'Target: `{args.target}`. Includes resolved build/dev dependencies; not a claim that every package is linked into the app.', '', 'Generated from Cargo.lock; original license choices and notices remain authoritative.', '']
missing = []
reviewed = json.loads((ROOT / 'resources/licenses/manifest.json').read_text())
for package in packages:
    base = Path(package['manifest_path']).parent
    lines += [f"## {package['name']} {package['version']}", '', f"License: {package.get('license') or 'see supplied license file'}", '', f"Source: {package['source']}", '']
    candidates = []
    if package.get('license_file'):
        candidates.append(base / package['license_file'])
    candidates += [p for p in base.iterdir() if p.is_file() and p.name.upper().startswith(('LICENSE','LICENCE','COPYING','NOTICE','UNLICENSE','COPYRIGHT'))]
    # Git workspace members sometimes reference a shared license at workspace root.
    if not candidates and package['source'].startswith('git+'):
        for parent in list(base.parents)[:3]:
            candidates += [p for p in parent.glob('LICENSE*') if p.is_file()]
            if candidates:
                break
    if not candidates:
        record = reviewed.get(f"{package['name']}@{package['version']}")
        if record and record.get('declaration_only'):
            raw = (base / record['manifest_file']).read_bytes()
            if hashlib.sha256(raw).hexdigest() != record['manifest_sha256'] or package['license'] != record['license']:
                raise SystemExit(f"Changed license declaration: {package['name']}; review required")
            lines += [record['reason'], '', f"Declaration: {record['source']}", '', "Upstream package authors: " + ', '.join(package.get('authors', [])), '']
            candidates.append(ROOT / 'resources/licenses/standard' / (record['standard'] + '.txt'))
        elif record:
            path = ROOT / 'resources/licenses' / record['file']
            if hashlib.sha256(path.read_bytes()).hexdigest() != record['sha256']:
                raise SystemExit(f"Changed cached license text: {path.name}")
            lines += [f"License text retrieved from: {record['source']}", '', f"Provenance: {record['provenance']}", '']
            candidates.append(path)
        else:
            missing.append(package['name'])
    # Reviewed vendored native libraries can carry terms separate from the binding crate.
    record = reviewed.get(f"{package['name']}@{package['version']}", {})
    for notice in record.get('archive_notices', []):
        path = base / notice['file']
        if hashlib.sha256(path.read_bytes()).hexdigest() != notice['sha256']:
            raise SystemExit(f"Changed vendored notice: {package['name']}/{notice['file']}")
        lines += [f"Bundled notice from locked crate archive: {notice['file']}", '']
        candidates.append(path)
    for p in sorted(set(candidates)):
        lines += [f'### {p.name}', '', p.read_text(errors='replace').strip(), '']
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text('\n'.join(lines))
print(f'Exported {len(packages)} dependency records to {args.output}; license text missing for: {", ".join(missing) or "none"}')
if missing:
    raise SystemExit('Review missing license texts before shipping this package')
