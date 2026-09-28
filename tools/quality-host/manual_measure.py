"""Measure the pending bounded capture, then register independently verified raw data."""
import argparse
from contextlib import contextmanager
import json
from pathlib import Path
import subprocess
import tempfile

import bounded_layout as layout
import fixed_workspace
import rust_capture
from timing import phase

DEPLOYMENT = Path('/home/gem/.local/share/codexsymphony/storage-maintenance/deployment.json')


@contextmanager
def bounded_temporary():
    previous = tempfile.tempdir
    tempfile.tempdir = str(layout.VOLUME / 'tmp')
    try:
        yield
    finally:
        tempfile.tempdir = previous


def fails(value, rule):
    if not rule['required'] or rule['on_violation'] != 'fail' or rule['operator'] not in ('ge', 'le'):
        raise ValueError('unsupported or weakened required policy')
    if value is None or value['denominator'] <= 0:
        return True
    limit = rule['limit']
    numerator = limit.get('covered', limit.get('numerator'))
    denominator = limit.get('total', limit.get('denominator'))
    left, right = value['numerator'] * denominator, value['denominator'] * numerator
    return left < right if rule['operator'] == 'ge' else left > right


def violations(functions, rules):
    if not functions or not rules:
        raise ValueError('empty required measurement or policy')
    result = []
    for function in functions:
        for rule in rules:
            if fails(function.get(rule['metric']), rule):
                result.append({'source': function['source'], 'function': function['name'],
                               'metric': rule['metric'], 'value': function.get(rule['metric'])})
    return result


def raw_inventory(run, request):
    receipt = request['parameters']['receipt']
    raw = fixed_workspace.canonical(run / 'probes/backend/raw')
    if receipt['raw_root'] != str(raw):
        raise ValueError('raw capture identity differs')
    actual = {}
    for path in sorted(raw.iterdir()):
        if path.is_symlink() or not path.is_file():
            raise ValueError('non-regular raw capture entry')
        actual[path.name] = rust_capture.digest(path)
    if not actual or actual != receipt['raw']:
        raise ValueError('independent raw inventory differs')
    (run / 'independent-raw-inventory.json').write_text(json.dumps(actual, indent=2) + '\n')


def register(run):
    deployment = json.loads(DEPLOYMENT.read_text())
    release = fixed_workspace.canonical(Path(deployment['release']))
    for name, expected in deployment['files'].items():
        path = Path(name)
        if path.is_relative_to(release) and rust_capture.digest(path) != expected:
            raise ValueError('installed retention tool drift')
    program = release / 'capture_cache_retention.py'
    if str(program) not in deployment['files']:
        raise ValueError('retention entrypoint lacks installed approval')
    result = subprocess.check_output(['python3', str(program), '--register', str(run)], text=True)
    registration = json.loads(result)
    if registration['root'] != str(run):
        raise ValueError('capture registration identity differs')
    (run / 'capture-registration.json').write_text(json.dumps(registration, indent=2) + '\n')


def measure(run, root):
    bundle = run / 'probes/backend/bundle.json'
    request = json.loads(bundle.read_text())['request']
    if request['workspace_root'] != str(root):
        raise ValueError('capture source identity differs')
    raw_inventory(run, request)
    from capture import RUST
    collector = rust_capture.load_collector(RUST)
    # Each re-export has fresh output; native evidence and source are never replaced.
    output = Path(tempfile.mkdtemp(prefix='measurement-', dir=run / 'probes/backend'))
    with bounded_temporary(), phase(run, 'backend-source-measurement'):
        result = collector.reexport(request, output / 'llvm-reexport.json')
        rules = json.loads((root / '.harness-gate/packs/backend/policy.json').read_text())['rules']
        bad = violations(result['functions'], rules)
    (output / 'measurements.json').write_text(json.dumps(result, indent=2) + '\n')
    return {'coverage_and_crap': 'FAIL' if bad else 'PASS', 'violations': bad,
            'functions': len(result['functions']), 'measurement': str(output / 'measurements.json'),
            'measurement_sha256': rust_capture.digest(output / 'measurements.json')}


def finish(repository):
    with layout.lease(repository):
        pending = layout.slot() / 'pending-capture.json'
        if pending.is_symlink():
            raise ValueError('symlink pending capture')
        record = json.loads(pending.read_text())
        run = fixed_workspace.canonical(Path(record['capture']))
        if run.parent != layout.VOLUME / 'evidence/gate':
            raise ValueError('pending capture is outside bounded evidence')
        root = layout.slot() / 'workspace'
        current = fixed_workspace.sources(repository)
        if current != fixed_workspace.sources(root):
            raise ValueError('development and capture source differ')
        if {name: row['sha256'] for name, row in current.items()} != record['source_inputs']:
            raise ValueError('pending source inputs changed')
        result = measure(run, root)
        register(run)
        (run / 'measurement-summary.json').write_text(json.dumps(result, indent=2) + '\n')
        if result['coverage_and_crap'] == 'PASS':
            from capture_handoff import complete
            complete(run, root, pending, result)
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', type=Path, default=layout.REPOSITORY)
    result = finish(parser.parse_args().repository)
    print(json.dumps(result, indent=2))
    raise SystemExit(result['coverage_and_crap'] != 'PASS')


if __name__ == '__main__': main()
