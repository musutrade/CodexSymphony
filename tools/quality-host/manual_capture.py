"""Bounded manual backend capture using the fixed source, target and database slots.

This creates raw source-bound evidence, never a quality PASS or publication receipt.
"""
import argparse
import json
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import environment_contract as contract
import bounded_layout as layout
import database_pool
import fixed_workspace
from isolation import command
from timing import phase
import capture as host_capture


def backend(run, root):
    target = layout.target('instrumented')
    probes = run / 'probes'; probes.mkdir()
    collector = host_capture.PLUGIN_ROOT / 'rust-source' / contract.load(root)['collectors']['rust_source']
    producer = root / 'tools/quality-host/rust_capture.py'
    argv = ['python3', producer, '--repository', root, '--output', probes / 'backend',
            '--target-dir', target, '--collector', collector, '--source-root', 'apps/server/src']
    name, url = database_pool.acquire(run, repository=root)
    try:
        args = command(argv, run=run, repository=root, plugins=host_capture.PLUGIN_ROOT,
                       writable=[probes, target], compiler_target=target,
                       environment={'TEST_DATABASE_URL': url, 'PYTHONDONTWRITEBYTECODE': '1'})
        host_capture.run_logged(run, 'backend-capture', args)
    finally:
        database_pool.release(name)


def run(repository):
    with layout.lease(repository):
        pending = layout.slot() / 'pending-capture.json'
        if pending.exists() or pending.is_symlink() or (layout.slot() / 'pending-gate.json').exists():
            raise ValueError('previous capture requires measurement, retention or explicit recovery')
        output = layout.new_run()
        policy = contract.load(repository)
        environment = dict(os.environ, **contract.test_environment(policy), PATH=contract.tool_path(policy))
        proof = contract.fingerprint(repository, environment)
        (output / 'environment.json').write_text(json.dumps(proof, indent=2) + '\n')
        with phase(output, 'fixed-source-synchronization'):
            root, inputs = fixed_workspace.synchronize(repository, layout.slot())
        (output / 'source-inputs.json').write_text(json.dumps(inputs, indent=2) + '\n')
        print(str(output), flush=True)
        with pending.open('x') as stream:
            json.dump({'capture': str(output), 'source_inputs': inputs}, stream)
        backend(output, root)
        if fixed_workspace.sources(repository) != fixed_workspace.sources(root):
            raise ValueError('development source changed during capture')
        print('CAPTURE COMPLETED; coverage and CRAP measurement still required', flush=True)
        return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', type=Path, default=layout.REPOSITORY)
    args = parser.parse_args()
    run(args.repository)


if __name__ == '__main__':
    main()
