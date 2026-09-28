"""Capture fresh LLVM evidence from an already frozen, reusable checkout.

The host owns the checkout lease and mounts it read-only. Evidence is new for every
attempt; compiler outputs may be reused. This entry never signs a Gate verdict.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import uuid

import fixed_workspace


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_collector(directory):
    sys.path.insert(0, str(directory))
    spec = importlib.util.spec_from_file_location('fixed_capture_collector', directory / 'plugin.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def arguments(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--target-dir', type=Path, required=True)
    parser.add_argument('--collector', type=Path, required=True)
    parser.add_argument('--manifest', default='apps/server/Cargo.toml')
    parser.add_argument('--source-root', action='append', required=True)
    parser.add_argument('--test', action='append', default=[])
    return parser.parse_args(argv)


def prepare(args):
    root = fixed_workspace.canonical(args.repository)
    output = fixed_workspace.canonical(args.output)
    target = fixed_workspace.canonical(args.target_dir)
    manifest = root / args.manifest
    if not manifest.is_file() or not manifest.resolve().is_relative_to(root):
        raise ValueError('manifest escapes frozen checkout or is missing')
    if output.is_relative_to(root) or target.is_relative_to(root):
        raise ValueError('capture outputs must be outside the frozen checkout')
    output.mkdir(mode=0o700)
    target.mkdir(mode=0o700, parents=True, exist_ok=True)
    return root, output, target


def execute(args, root, output, target):
    command = ['cargo', 'llvm-cov', '--manifest-path', str(root / args.manifest),
               '--locked', '--json', '--output-path', str(output / 'cargo-coverage.json'), '--verbose']
    for selection in args.test:
        command += ['--test', selection]
    # cargo-llvm-cov's normal invocation cleans old profiles before executing tests;
    # do not pass --no-clean, --no-run or --no-report when reusing this target.
    environment = dict(os.environ, CARGO_TARGET_DIR=str(target))
    with (output / 'capture.stdout').open('wb') as stdout, (output / 'capture.stderr').open('wb') as stderr:
        subprocess.run(command, env=environment, cwd=root, stdout=stdout, stderr=stderr, check=True)


def export_command(output):
    exports = []
    for line in (output / 'capture.stderr').read_text().splitlines():
        line = line.strip()
        if line.startswith('Running `') and '/llvm-cov export ' in line:
            exports.append(shlex.split(line[len('Running `'):-1]))
    if len(exports) != 1:
        raise ValueError('expected one native LLVM export invocation')
    return exports[0]


def artifact_groups(export, target):
    objects = [Path(export[i + 1]) for i, arg in enumerate(export) if arg == '-object']
    profiles = [Path(arg.split('=', 1)[1]) for arg in export if arg.startswith('-instr-profile=')]
    if not objects or len(profiles) != 1:
        raise ValueError('native objects or unique merged profile missing')
    if not profiles[0].resolve().is_relative_to(target):
        raise ValueError('native profile escapes compiler target')
    return {'objects': objects, 'profiles': sorted(profiles[0].parent.rglob('*.profraw'))}


def retain_artifacts(groups, output, target):
    raw = output / 'raw'
    raw.mkdir()
    retained, names = {}, {}
    for kind, paths in groups.items():
        if not paths:
            raise ValueError('native evidence group is empty: ' + kind)
        names[kind] = []
        for index, path in enumerate(paths):
            if path.is_symlink() or not path.resolve().is_relative_to(target):
                raise ValueError('native artifact escapes compiler target')
            name = f'{kind}-{index}-{path.name}'
            shutil.copyfile(path, raw / name)
            retained[name] = digest(raw / name)
            names[kind].append(name)
    return raw, retained, names


def tool_identity(export):
    tools = {}
    for name in ('llvm-cov', 'llvm-profdata'):
        path = Path(export[0]).with_name(name)
        tools[name] = {'path': str(path), 'sha256': digest(path),
                       'version': subprocess.check_output([path, '--version'], text=True).strip()}
    tools['rustc'] = subprocess.check_output(['rustc', '-vV'], text=True).strip()
    tools['cargo-llvm-cov'] = subprocess.check_output(['cargo', 'llvm-cov', '--version'], text=True).strip()
    return tools


def request_base(args, root, output, collector):
    revision = fixed_workspace.git(root, 'rev-parse', 'HEAD').decode().strip()
    return {'schema': 'harness-collector-request/v1', 'project': 'codexsymphony', 'component': 'backend',
            'collector': collector.COLLECTOR,
            'context': {'commit': revision, 'base_commit': revision, 'run': 'rust-source-' + uuid.uuid4().hex,
                        'target': 'x86_64-unknown-linux-gnu'},
            'requested_capabilities': sorted(collector.TYPES), 'workspace_root': str(root),
            'output_root': str(output / 'evidence'),
            'parameters': {'source_roots': args.source_root, 'boundary': 'production',
                           'artifact_subdir': 'backend-source'}}


def capture(args):
    root, output, target = prepare(args)
    collector = load_collector(args.collector)
    inputs = fixed_workspace.sources(root)
    request = request_base(args, root, output, collector)
    sources = collector.inventory(request)
    from measure import source_inventories
    source_inventories(root, list(sources), args.collector / 'inventory')
    execute(args, root, output, target)
    export = export_command(output)
    raw, retained, groups = retain_artifacts(artifact_groups(export, target), output, target)
    if fixed_workspace.sources(root) != inputs:
        raise ValueError('frozen source changed during native capture')
    request['parameters']['receipt'] = make_receipt(args, root, request, inputs, sources,
                                                   raw, retained, groups, tool_identity(export))
    request['parameters']['subjects'] = collector.discover(request)
    bundle = {'request': request, 'series': collector.series(request)}
    (output / 'bundle.json').write_text(collector.canonical(bundle) + '\n')
    return bundle


def make_receipt(args, root, request, inputs, sources, raw, retained, groups, tools):
    hashes = {name: value['sha256'] for name, value in inputs.items()}
    configuration = {name: value for name, value in hashes.items()
                     if name.endswith(('Cargo.toml', 'Cargo.lock')) or name.startswith('.cargo/')}
    tools = dict(tools, capture_host={'sha256': digest(Path(__file__)),
                                     'workspace_sha256': digest(Path(fixed_workspace.__file__))})
    return {'schema': 'rust-source-capture/v1', 'context': request['context'], 'coverage_root': str(root),
            'sources': sources, 'inputs': native_inputs(args, hashes), 'source_inputs': hashes,
            'pipeline': {'files': configuration, 'tools': tools, 'tests': args.test, 'manifest': args.manifest,
                         'capture': 'cargo-llvm-cov-fixed-source/v1'},
            'raw_root': str(raw), 'raw': retained, **groups}


def native_inputs(args, hashes):
    # Match the installed backend input boundary; retain the complete Git
    # snapshot separately, including documents the collector cannot name.
    roots = ['apps', 'migrations', '.cargo', *args.source_root]
    result = {}
    for name, digest_value in hashes.items():
        path = Path(name)
        if name in ('Cargo.toml', 'Cargo.lock', args.manifest):
            result[name] = digest_value
        elif any(path.is_relative_to(directory) for directory in roots):
            result[name] = digest_value
    return result


if __name__ == '__main__':
    capture(arguments())
