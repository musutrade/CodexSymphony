"""Reviewed verification inputs, distinct from a publication's exact commit.

The contract is installed by the operator, never discovered in a candidate.
Absent or unclassified dependencies disable reuse. Original reports are immutable.
"""
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import time

SCHEMA = 'codexsymphony-validation-contract/v1'
INPUT_SCHEMA = 'codexsymphony-verification-inputs/v1'
BINDING_SCHEMA = 'codexsymphony-publication-binding/v1'
KEYS = {'schema', 'approval', 'runtime_files', 'config_files', 'trusted_files', 'baseline',
        'git_reads', 'external_inputs', 'required_evidence', 'max_age_seconds', 'complete_dependency_review'}
READ_KEYS = {'reader', 'sha256', 'command', 'classification'}
COMMANDS = {
    'head': ['rev-parse', '--verify', 'HEAD^{commit}'],
    'branch': ['symbolic-ref', '--short', 'HEAD'],
    'parents': ['rev-list', '--parents', '--max-count=1', 'HEAD'],
    'tracked': ['ls-files', '-z', '--cached'],
    # Working bytes are compared to HEAD by the source tree guard. Inspect only
    # the index here: porcelain status may run candidate-configured clean filters.
    'status': ['diff-index', '--cached', '--name-only', '-z', '--no-ext-diff', '--no-textconv', 'HEAD', '--'],
}
CLASSIFICATIONS = {'label-only', 'source-selection', 'input'}
MINIMUM_EVIDENCE = {'report', 'environment.json', 'requests.json', 'source-inputs.json',
                    'source-archive.json', 'source.tar.gz', 'measurement-summary.json',
                    'backend-measurements', 'capture-registration.json', 'verification-inputs.json'}


def load_pins():
    path = Path(__file__).with_name('evidence_pins.py')
    if not path.is_file():
        path = Path(__file__).resolve().parent.parent / 'quality-host/evidence_pins.py'
    spec = importlib.util.spec_from_file_location('publication_evidence_pins', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


pins = load_pins()


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def file_digest(path):
    path = Path(path)
    if path.resolve() != path or not path.is_file():
        raise ValueError('canonical regular verification input required: ' + str(path))
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def reviewed(setting, approval):
    if not setting:
        return None
    if set(setting) != {'path', 'sha256'} or file_digest(setting['path']) != setting['sha256']:
        raise ValueError('installed verification contract changed')
    value = json.loads(Path(setting['path']).read_text())
    if set(value) != KEYS or value['schema'] != SCHEMA or value['complete_dependency_review'] is not True:
        raise ValueError('complete verification dependency review required')
    if value['approval'] != digest(approval):
        raise ValueError('verification contract names another approval')
    for name in ('runtime_files', 'config_files', 'trusted_files', 'baseline'):
        if value[name] != approval[name]:
            raise ValueError('verification contract differs from approved ' + name)
    if type(value['max_age_seconds']) is not int or value['max_age_seconds'] <= 0:
        raise ValueError('verification expiry must be positive')
    required = value['required_evidence']
    if not isinstance(required, list) or not all(isinstance(name, str) for name in required) or \
            not MINIMUM_EVIDENCE <= set(required) or len(required) != len(set(required)):
        raise ValueError('verification contract lacks required payloads')
    reads = value['git_reads']
    external = value['external_inputs']
    if not isinstance(external, list) or not external or not all(isinstance(name, str) for name in external) or \
            len(external) != len(set(external)):
        raise ValueError('actual dependency inputs must be reviewed')
    for name in external:
        path = Path(name)
        if not path.is_absolute() or path.resolve() != path or not path.exists():
            raise ValueError('external verification input unavailable: ' + name)
    if not isinstance(reads, list) or not reads:
        raise ValueError('Git dependency review required')
    seen = set()
    for entry in reads:
        if not isinstance(entry, dict) or set(entry) != READ_KEYS:
            raise ValueError('unclassified Git dependency')
        if entry['classification'] not in CLASSIFICATIONS or entry['command'] not in COMMANDS:
            raise ValueError('unsupported Git dependency')
        reader = entry['reader']
        pinned = approval['runtime_files'].get(reader, approval['trusted_files'].get(reader))
        if pinned != entry['sha256'] or (reader, entry['command']) in seen:
            raise ValueError('Git reader missing, changed or duplicated')
        seen.add((reader, entry['command']))
        if entry['classification'] == 'source-selection' and entry['command'] != 'tracked':
            raise ValueError('unsupported source selection dependency')
        if entry['classification'] == 'input' and entry['command'] == 'branch':
            raise ValueError('validation checkout branch dependency is unsupported')
    return value


def git(root, arguments):
    # These are installed, enumerated read commands, never candidate shell text.
    return subprocess.check_output(['/usr/bin/git', '-c', 'core.fsmonitor=false',
                                    '-c', 'core.hooksPath=/dev/null', '-C', str(root), *arguments],
                                   env={'PATH': '/usr/bin:/bin', 'HOME': '/nonexistent', 'LANG': 'C.UTF-8',
                                        'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': '/dev/null'},
                                   timeout=60)


def freeze(root, current, value):
    if value is None:
        return None
    status = git(root, COMMANDS['status'])
    if status:
        raise ValueError('reviewed reuse requires a clean committed candidate; dirty/untracked inputs are unsupported')
    if git(root, ['rev-parse', '--verify', 'HEAD^{tree}']).decode().strip() != current['tree']:
        raise ValueError('verification source differs from the committed tree')
    # Bind the snapshot's selection and dirty/untracked state as well as bytes/modes.
    # A label-only HEAD may differ; every input-classified read must stay identical.
    metadata = {}
    for entry in value['git_reads']:
        if entry['classification'] == 'input':
            metadata[entry['reader'] + ':' + entry['command']] = git(root, COMMANDS[entry['command']]).hex()
    return {'schema': INPUT_SCHEMA, 'contract': digest(value), **current,
            'baseline': value['baseline'],
            'source_inventory': source_inventory(root),
            'tracked': hashlib.sha256(git(root, COMMANDS['tracked'])).hexdigest(),
            'workspace_status': hashlib.sha256(status).hexdigest(),
            'external_inputs': {name: external_input(Path(name)) for name in value['external_inputs']},
            'git_inputs': metadata}


def source_inventory(root):
    rows = {}
    names = git(root, ['ls-files', '-z', '--cached', '--others', '--exclude-standard'])
    for raw in names.split(b'\0'):
        if not raw:
            continue
        name = raw.decode('utf-8')
        path = root / name
        if not path.resolve().is_relative_to(root) or path.is_symlink():
            raise ValueError('unsafe verification source: ' + name)
        if not path.exists():
            continue
        rows[name] = {'sha256': file_digest(path), 'mode': path.stat().st_mode & 0o777}
    return digest(rows)


def external_input(path):
    """Bind actual dependency bytes and modes, including ignored installed inputs.

    Escaping links, sockets, absent files and other unsupported inputs fail closed.
    The reviewer must enumerate every externally consumed root in the contract.
    """
    if path.resolve() != path:
        raise ValueError('external dependency root is aliased')
    if path.is_file():
        return {'sha256': file_digest(path), 'mode': path.stat().st_mode & 0o777}
    if not path.is_dir():
        raise ValueError('external dependency input unavailable')
    rows = {}
    for item in sorted(path.rglob('*')):
        name = str(item.relative_to(path))
        if item.is_symlink():
            target = item.resolve(strict=True)
            if not target.is_relative_to(path) or not target.is_file():
                raise ValueError('unsupported external dependency link: ' + name)
            rows[name] = {'link': str(target.relative_to(path)), 'sha256': file_digest(target)}
        elif item.is_file():
            rows[name] = {'sha256': file_digest(item), 'mode': item.stat().st_mode & 0o777}
        elif not item.is_dir():
            raise ValueError('unsupported external dependency entry: ' + name)
    return {'inventory_sha256': digest(rows), 'files': len(rows)}


def reusable(record, frozen, value, now=None):
    if value is None or frozen is None:
        raise ValueError('no reviewed verification contract; complete Gate required')
    if record.get('outcome') != 'pass':
        raise ValueError('verification result is not a PASS')
    expected = {'schema', 'contract', 'tree', 'environment', 'approval', 'baseline', 'source_inventory',
                'tracked', 'workspace_status', 'external_inputs', 'git_inputs'}
    if set(frozen) != expected or frozen['schema'] != INPUT_SCHEMA or frozen['contract'] != digest(value) or \
            frozen['baseline'] != value['baseline'] or frozen['approval'] != value['approval']:
        raise ValueError('verification result names an old or incompatible contract')
    if any(frozen[name] != record['inputs'][name] for name in ('tree', 'environment', 'approval')):
        raise ValueError('verification inputs differ from original ledger provenance')
    declared = {entry['reader'] + ':' + entry['command'] for entry in value['git_reads']
                if entry['classification'] == 'input'}
    if not isinstance(frozen['git_inputs'], dict) or set(frozen['git_inputs']) != declared:
        raise ValueError('verification result has missing or unclassified Git inputs')
    details = record['details']
    if details.get('scope') != 'complete-local-isolated-gate' or details.get('verification_inputs') != frozen:
        raise ValueError('complete verification inputs differ; complete Gate required')
    elapsed = (time.time() if now is None else now) * 1000 - record['created_at_ms']
    if not 0 <= elapsed < value['max_age_seconds'] * 1000:
        raise ValueError('verification result expired or future-dated')
    evidence = record['evidence']
    for name in value['required_evidence']:
        if name not in evidence or file_digest(evidence[name]['path']) != evidence[name]['sha256']:
            raise ValueError('verification payload missing or changed: ' + name)
    measurement = measurement_evidence(evidence, Path(record['details']['run']))
    if str(measurement) != evidence['backend-measurements']['path']:
        raise ValueError('retained source-bound measurement differs from its ledger binding')
    saved = json.loads(Path(evidence['verification-inputs.json']['path']).read_text())
    if saved != frozen:
        raise ValueError('retained verification inputs differ')
    report = json.loads(Path(evidence['report']['path']).read_text())
    if report.get('passed') is not True or report.get('evidence_complete') is not True or \
            report.get('source_identity') != 'working-tree:' + record['inputs']['commit']:
        raise ValueError('original complete report is invalid')


def measurement_evidence(evidence, run):
    summary = evidence['measurement-summary.json']
    path = summary['path'] if isinstance(summary, dict) else summary
    value = json.loads(Path(path).read_text())
    measurement = Path(value['measurement'])
    if value.get('coverage_and_crap') != 'PASS' or not measurement.is_relative_to(run) or \
            pins.cleanable(run, measurement) or file_digest(measurement) != value['measurement_sha256']:
        raise ValueError('source-bound measurements missing, changed or failed')
    return measurement


def binding(record, commit, frozen, base):
    return {'schema': BINDING_SCHEMA, 'commit': commit, 'validation_id': record['validation_id'],
            'validated_commit': record['inputs']['commit'], 'verification_inputs': digest(frozen),
            'base': base, 'checks': ['current inputs', 'standing ledger PASS', 'retained payloads',
                                    'current main ancestry', 'installed admission compatibility'],
            'expensive_capture_count': 0, 'native_added_bytes': 0}


@contextmanager
def phase(state, name):
    started = time.monotonic()
    row = {'phase': name, 'started_at': datetime.now(timezone.utc).isoformat(), 'status': 'FAIL'}
    try:
        yield
        row['status'] = 'PASS'
    finally:
        row['duration_ms'] = round((time.monotonic() - started) * 1000)
        row['ended_at'] = datetime.now(timezone.utc).isoformat()
        with (state / 'timings.jsonl').open('a') as output:
            output.write(json.dumps(row) + '\n')
