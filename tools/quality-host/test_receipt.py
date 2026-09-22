"""Host-issued proof of this run's complete backend test capture.

The host creates the receipt only after the capture process exits successfully.
It lives outside every capture-writable mount. Verify sees it read-only; local
verification without a host receipt executes the ordinary complete test suite.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib


def digest(path):
    with Path(path).open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def backend_inputs(root):
    names = ['Cargo.toml', 'Cargo.lock']
    for directory in ('apps', 'migrations'):
        names += [str(p.relative_to(root)) for p in (root / directory).rglob('*') if p.is_file()]
    result = {}
    for name in sorted(names):
        p = root / name
        if p.resolve() != p or not p.is_file():
            raise ValueError('unsafe backend input: ' + name)
        result[name] = digest(p)
    return result


def supported(root):
    workspace = tomllib.loads((root / 'Cargo.toml').read_text()).get('workspace', {})
    # The current source collector captures exactly this package. A larger
    # workspace/configuration falls back to Cargo's complete default selection.
    return (workspace.get('members') == ['apps/server']
            and workspace.get('default-members', ['apps/server']) == ['apps/server']
            and not workspace.get('exclude')
            and not (root / '.cargo').exists())


def seal(run, repository, context):
    if not supported(repository):
        return
    capture = run / 'probes/backend'
    bundle = json.loads((capture / 'bundle.json').read_text())['request']
    receipt = bundle['parameters']['receipt']
    pipeline = receipt['pipeline']
    if pipeline['capture'] != 'cargo-llvm-cov-locked/v1' or pipeline['tests'] or pipeline['manifest'] != 'apps/server/Cargo.toml':
        raise ValueError('capture does not cover the complete backend test selection')
    inputs = backend_inputs(repository)
    if inputs != receipt['inputs'] or bundle['context']['commit'] != context['commit']:
        raise ValueError('captured test inputs differ from the verification source')
    stdout = capture / 'capture.stdout'
    if not re.search(r'(?m)^test result: ok\. [1-9][0-9]* passed;', stdout.read_text()):
        raise ValueError('capture has no passing Rust tests')
    value = {'schema': 'host-test-capture/v1', 'context': context, 'exit_code': 0,
             'inputs': inputs, 'pipeline': pipeline,
             'stdout': str(stdout), 'stdout_sha256': digest(stdout)}
    # Exclusive creation prevents a project-generated file from becoming proof.
    with (run / 'test-capture.json').open('x') as output:
        json.dump(value, output)


def verified_log(path, root):
    path = Path(path)
    if not path.is_absolute() or path.resolve() != path:
        raise ValueError('noncanonical test receipt')
    value = json.loads(path.read_text())
    expected_run = path.parent.name
    head = subprocess.check_output(['git', '-C', root, 'rev-parse', 'HEAD'], text=True).strip()
    if (value.get('schema') != 'host-test-capture/v1' or type(value.get('exit_code')) is not int
            or value['exit_code'] != 0 or value['context']['commit'] != head
            or value['context']['run'] != expected_run):
        raise ValueError('test capture identity mismatch')
    pipeline = value['pipeline']
    if (not supported(root) or value['inputs'] != backend_inputs(root)
            or pipeline['capture'] != 'cargo-llvm-cov-locked/v1'
            or pipeline['tests'] or pipeline['manifest'] != 'apps/server/Cargo.toml'):
        raise ValueError('test capture selection or inputs changed')
    log = path.parent / 'probes/backend/capture.stdout'
    if value['stdout'] != str(log) or log.resolve() != log or digest(log) != value['stdout_sha256']:
        raise ValueError('test capture log changed')
    return log.read_bytes()


def main():
    root = Path(__file__).resolve().parents[2]
    receipt = os.environ.get('HARNESS_GATE_TEST_RECEIPT')
    if receipt:
        log = verified_log(receipt, root)
        print('Reusing successful source-bound host coverage tests; running doc-tests separately.', flush=True)
        sys.stdout.buffer.write(log)
        sys.stdout.buffer.flush()
        args = ['cargo', 'test', '--manifest-path', str(root / 'Cargo.toml'), '--locked', '--doc', '--', '--nocapture']
    else:
        args = ['cargo', 'test', '--manifest-path', str(root / 'Cargo.toml'), '--locked', '--', '--nocapture']
    raise SystemExit(subprocess.run(args, cwd=root).returncode)


if __name__ == '__main__':
    main()
