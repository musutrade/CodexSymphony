"""Operator installation of the reviewed fixed Rust Runtime integration test.

Compilation runs through the assigned command sandbox. Only pinned copies of
the resulting executables run in the independent test namespace.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

BASE = Path('/home/gem/.local/share/codexsymphony')


def sources(root):
    paths = [root/name for name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'codex-version.lock')]
    for directory in ('apps', 'migrations', '.cargo'):
        paths += [p for p in (root/directory).rglob('*') if p.is_file() and p.suffix in ('.rs', '.toml', '.sql')]
    result = {}
    for path in paths:
        if path.resolve() != path.absolute():
            raise ValueError('source symlinks are not reviewable')
        result[str(path.relative_to(root))] = hashlib.sha256(path.read_bytes()).hexdigest()
    return result


def install(root):
    root = Path(root).absolute()
    if root.resolve() != root or root.parent != BASE/'workspaces' or not re.fullmatch(r'GH-\d+', root.name):
        raise ValueError('assigned workspace required')
    provision = BASE/'symphony'/(root.name.lower().replace('-', '')+'-environment')
    before = sources(root)
    spec = importlib.util.spec_from_file_location('installed_preflight', provision/'preflight.py')
    preflight = importlib.util.module_from_spec(spec);spec.loader.exec_module(preflight)
    result = preflight.execute(['cargo', 'test', '--workspace', '--locked', '--test', 'runtime_real',
                                '--no-run', '--message-format=json'], timeout=120)
    (provision/'runtime-build.json').write_text(json.dumps(result)+'\n')
    if result['exitCode'] != 0:
        raise RuntimeError('reviewed Runtime build failed: '+result['stderr'][-4000:])
    if sources(root) != before:
        raise ValueError('source changed during compilation')
    executable = None
    for line in result['stdout'].splitlines():
        record = json.loads(line)
        if record.get('target', {}).get('name') == 'runtime_real' and record.get('executable'):
            executable = Path(record['executable'])
    if executable is None or not executable.resolve().is_relative_to(root/'target'):
        raise ValueError('missing compiled Runtime test')
    directory = provision/'reviewed-runtime'
    directory.mkdir(mode=0o700, exist_ok=True)
    binaries = {}
    for name, source in [('test', executable), ('supervisor', root/'target/debug/codexsymphony-server')]:
        if source.resolve() != source.absolute():
            raise ValueError('binary symlink')
        temporary = directory/(name+'.new')
        shutil.copyfile(source, temporary);temporary.chmod(0o500)
        binaries[name] = hashlib.sha256(temporary.read_bytes()).hexdigest()
        temporary.replace(directory/name)
    manifest = dict(workspace=str(root), sources=before, binaries=binaries, installed_at=time.time(),
                    source_sha=subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'],text=True).strip())
    temporary = directory/'manifest.new'
    temporary.write_text(json.dumps(manifest, indent=2)+'\n');temporary.replace(directory/'manifest.json')
    print(json.dumps({'installed': True, 'workspace': str(root), 'sources':len(before), 'binaries': binaries}))


if __name__ == '__main__':
    install(sys.argv[1])
