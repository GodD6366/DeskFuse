#!/usr/bin/env python3
"""导出随 WebView 客户端一同分发的 Go 模块与 npm 生产依赖及其许可证正文。

Go 模块来自 `go list -deps ./cmd/deskfuse` 实际参与编译的模块（包含间接依赖），npm 包来自
`package-lock.json` 的生产条目；开发依赖不进入应用，因此不在此列出。
许可证正文优先取模块/包目录内的 LICENSE 类文件；缺失时保留声明的许可证
标识并列入 missing，由发布评审确认，不静默略过。
"""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
APP_DIR = ROOT / 'apps/deskfuse-go'
FRONTEND_DIR = APP_DIR / 'frontend'
LICENSE_PREFIXES = ('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'UNLICENSE', 'COPYRIGHT')

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()

missing = []
lines = [
    '# WebView 客户端第三方依赖声明',
    '',
    'Go 模块取自 `go list -deps ./cmd/deskfuse` 实际参与编译的模块；npm 包取自 `package-lock.json` 的生产条目。',
    '原始许可选择与上游版权声明仍然优先。',
    '',
]


def license_files(directory: Path):
    if not directory.is_dir():
        return []
    return sorted(
        path for path in directory.iterdir()
        if path.is_file() and path.name.upper().startswith(LICENSE_PREFIXES)
    )


def decode_json_stream(raw: str):
    decoder = json.JSONDecoder()
    index = 0
    while index < len(raw):
        while index < len(raw) and raw[index].isspace():
            index += 1
        if index >= len(raw):
            break
        module, index = decoder.raw_decode(raw, index)
        yield module


def go_modules():
    # 只列出真正参与编译的模块：`go list -m all` 还包含未下载、未链接的图节点。
    raw = subprocess.check_output(['go', 'list', '-deps', '-json', './cmd/deskfuse'], cwd=APP_DIR, text=True)
    seen = set()
    for package in decode_json_stream(raw):
        module = package.get('Module')
        if not module or module.get('Main') or module['Path'] in seen:
            continue
        seen.add(module['Path'])
        name = module['Path']
        lines.extend([
            f"## go: {name} {module['Version']}",
            '',
            f"License: {module.get('License') or 'see supplied license files'}",
            '',
            f"Source: {module.get('Dir') or 'https://' + name}",
            '',
        ])
        candidates = license_files(Path(module['Dir'])) if module.get('Dir') else []
        if not emit_license_texts(name, candidates):
            missing.append(f"go: {name}")


def npm_packages():
    lockfile = json.loads((FRONTEND_DIR / 'package-lock.json').read_text())
    packages = lockfile.get('packages', {})
    for key in sorted(packages):
        entry = packages[key]
        if not key.startswith('node_modules/') or entry.get('dev'):
            continue
        name = key[len('node_modules/'):]
        lines.extend([
            f"## npm: {name} {entry.get('version', 'unknown')}",
            '',
            f"License: {entry.get('license') or 'see supplied license files'}",
            '',
            f"Source: https://www.npmjs.com/package/{name}",
            '',
        ])
        # 构建 WebView 客户端时 node_modules 已由 npm ci 安装，正文随包提供。
        if not emit_license_texts(name, license_files(FRONTEND_DIR / key)):
            missing.append(f"npm: {name}")


def emit_license_texts(name, candidates):
    emitted = False
    for path in candidates:
        lines.extend([f'### {path.name}', '', path.read_text(errors='replace').strip(), ''])
        emitted = True
    return emitted


go_modules()
npm_packages()

args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text('\n'.join(lines))
print(f'WebView 依赖声明：{args.output}；缺少许可证正文：{", ".join(missing) or "无"}')
