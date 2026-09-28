"""Recover packaging of a completed native capture without repeating its tests.

Requires the original frozen source and unchanged compiler outputs. This emits a
new, explicitly recovered bundle; it never overwrites a bundle or claims Gate PASS.
"""
import json
import os
from pathlib import Path
from types import SimpleNamespace

import bounded_layout as layout
import fixed_workspace as workspace
import manual_capture
import manual_measure
import rust_capture as capture
from timing import phase


def pending_capture():
    path = layout.slot() / 'pending-capture.json'
    if path.is_symlink():
        raise ValueError('symlink pending capture')
    pending = json.loads(path.read_text())
    run = workspace.canonical(pending['capture'])
    if run.parent != layout.VOLUME / 'evidence/gate':
        raise ValueError('capture outside bounded evidence')
    root = layout.slot() / 'workspace'
    inputs = workspace.sources(root)
    if {name: item['sha256'] for name, item in inputs.items()} != pending['source_inputs']:
        raise ValueError('frozen source changed since failed packaging')
    output = run / 'probes/backend'
    if (output / 'bundle.json').exists() or (output / 'bundle.json').is_symlink():
        raise ValueError('capture bundle already exists; use measurement entry')
    return run, root, inputs


def retained_artifacts(output, target):
    export = capture.export_command(output)
    groups = capture.artifact_groups(export, target)
    raw = workspace.canonical(output / 'raw')
    names, retained = {}, {}
    for kind, paths in groups.items():
        if not paths:
            raise ValueError('empty native evidence group')
        names[kind] = []
        for index, path in enumerate(paths):
            if path.is_symlink() or not path.resolve().is_relative_to(target):
                raise ValueError('native artifact outside fixed target')
            name = f'{kind}-{index}-{path.name}'
            original = workspace.fingerprint(path)['sha256']
            if workspace.fingerprint(raw / name)['sha256'] != original:
                raise ValueError('retained native artifact changed')
            retained[name] = original
            names[kind].append(name)
    if set(item.name for item in raw.iterdir()) != set(retained):
        raise ValueError('raw inventory differs from native export')
    return export, raw, retained, names


def environment(run, root):
    contract = manual_capture.contract
    policy = contract.load(root)
    variables = dict(os.environ, **contract.test_environment(policy), PATH=contract.tool_path(policy))
    current = contract.fingerprint(root, variables)
    if current != json.loads((run / 'environment.json').read_text()):
        raise ValueError('capture environment changed')
    return policy


def package(run, root, inputs, policy):
    output = run / 'probes/backend'
    export, raw, retained, names = retained_artifacts(output, layout.target('instrumented'))
    collector = capture.load_collector(manual_capture.host_capture.PLUGIN_ROOT / 'rust-source' / policy['collectors']['rust_source'])
    args = SimpleNamespace(source_root=['apps/server/src'], manifest='apps/server/Cargo.toml', test=[])
    request = capture.request_base(args, root, output, collector)
    request['context']['run'] = run.name + '-packaging-recovery'
    tools = capture.tool_identity(export)
    receipt = capture.make_receipt(args, root, request, inputs, collector.inventory(request), raw, retained, names, tools)
    receipt['pipeline']['tools']['capture_host'] = {
        'sha256': capture.digest(root / 'tools/quality-host/rust_capture.py'),
        'workspace_sha256': capture.digest(root / 'tools/quality-host/fixed_workspace.py')}
    receipt['pipeline']['tools']['packaging_recovery'] = {
        'sha256': capture.digest(Path(__file__)), 'receipt_builder_sha256': capture.digest(Path(capture.__file__))}
    receipt['recovery'] = {'original_run': str(run), 'stage': 'packaging',
                           'native_tests_repeated': False,
                           'native_log_sha256': capture.digest(output / 'capture.stderr')}
    request['parameters']['receipt'] = receipt
    with manual_measure.bounded_temporary():
        request['parameters']['subjects'] = collector.discover(request)
    bundle = {'request': request, 'series': collector.series(request)}
    with (output / 'bundle.json').open('x') as stream:
        stream.write(collector.canonical(bundle) + '\n')
    return bundle


def recover():
    with layout.lease(layout.REPOSITORY):
        run, root, inputs = pending_capture()
        policy = environment(run, root)
        with phase(run, 'recover-native-packaging'):
            package(run, root, inputs, policy)
        result = manual_measure.measure(run, root)
        manual_measure.register(run)
        result['scope'] = 'original frozen capture only; not current development tree or Gate PASS'
        (run / 'recovery-measurement.json').write_text(json.dumps(result, indent=2) + '\n')
        if result['coverage_and_crap'] == 'PASS':
            from capture_handoff import complete
            complete(run, root, layout.slot() / 'pending-capture.json', result)
        return result


if __name__ == '__main__':
    print(json.dumps(recover(), indent=2))
