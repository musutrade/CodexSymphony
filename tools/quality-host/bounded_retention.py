"""Apply the installed retention policy to completed bounded captures only."""
import argparse
import importlib.util
import inspect
import json
import os
from pathlib import Path
import re
import shutil
import sys
import time

import bounded_layout as layout
import bounded_records
import fixed_workspace
import manual_measure
import rust_capture


def installed_policy():
    deployment = json.loads(manual_measure.DEPLOYMENT.read_text())
    release = fixed_workspace.canonical(deployment['release'])
    for name, digest in deployment['files'].items():
        path = Path(name)
        if path.is_relative_to(release) and rust_capture.digest(path) != digest:
            raise ValueError('installed retention policy changed')
    entry = release / 'compact_gate_evidence.py'
    if str(entry) not in deployment['files']:
        raise ValueError('retention policy is not approved')
    sys.path.insert(0, str(release))
    spec = importlib.util.spec_from_file_location('bounded_installed_retention', entry)
    policy = importlib.util.module_from_spec(spec); spec.loader.exec_module(policy)
    return policy


def pending_runs():
    pending = set()
    for name in ('pending-capture.json', 'pending-gate.json'):
        path = layout.slot() / name
        if path.is_symlink():
            raise ValueError('symlink pending capture')
        if path.exists():
            pending.add(json.loads(path.read_text())['capture'])
    return pending


def completed(run):
    archive = run / 'source-archive.json'
    registration = run / 'capture-registration.json'
    if not archive.exists() or not registration.exists():
        return False
    record = json.loads(archive.read_text())
    if rust_capture.digest(run / 'source.tar.gz') != record['sha256']:
        raise ValueError('retained source archive changed')
    binding = json.loads(registration.read_text())
    if binding['root'] != str(run) or binding['bundle_sha256'] != rust_capture.digest(run / 'probes/backend/bundle.json'):
        raise ValueError('retained capture registration changed')
    for name in ('measurement-summary.json', 'recovery-measurement.json'):
        path = run / name
        if path.exists():
            result = json.loads(path.read_text())
            measurement = fixed_workspace.canonical(result['measurement'])
            if not measurement.is_relative_to(run) or rust_capture.digest(measurement) != result['measurement_sha256']:
                raise ValueError('retained measurement changed')
            return result['coverage_and_crap'] == 'PASS'
    return False


def maintain(apply=False):
    with layout.lease(layout.REPOSITORY):
        return maintain_locked(apply)


def maintain_locked(apply=False):
    """Caller holds the common workspace lease through this operation."""
    policy = installed_policy()
    defaults = inspect.signature(policy.maintain).parameters
    keep, hours, budget = [defaults[key].default for key in ('keep', 'hours', 'budget')]
    pending = pending_runs()
    parent = layout.VOLUME / 'evidence/gate'
    candidates = []
    for run in parent.glob('run-*'):
        if not re.fullmatch(r'run-[0-9a-f]{12}', run.name) or run.resolve() != run:
            raise ValueError('unsafe bounded evidence directory')
        if str(run) not in pending and completed(run):
            candidates.append(run)
    results = expire(policy, candidates, keep, hours, budget, apply)
    return results + bounded_records.maintain(parent, candidates, policy, keep, hours, budget, apply)


def expire(policy, candidates, keep, hours, budget, apply):
    candidates.sort(key=lambda run: (run / 'source-archive.json').stat().st_mtime, reverse=True)
    used, results = 0, []
    for index, run in enumerate(candidates):
        if apply:
            cleanup_scratch(run)
        size = policy.bytes_used(policy.payloads(run))
        age = time.time() - (run / 'source-archive.json').stat().st_mtime
        if index < keep and age < hours * 3600 and used + size <= budget:
            used += size
            continue
        if apply:
            results.append(policy.compact(run))
        else:
            results.append({'run': str(run), 'rebuildable_bytes': size})
    return results


def cleanup_scratch(run):
    """Completed captures need neither isolated HOME nor temporary app copies."""
    if not shutil.rmtree.avoids_symlink_attacks:
        raise ValueError('fd-safe scratch deletion required')
    for name in ('tmp', 'cargo-home', 'target'):
        path = fixed_workspace.canonical(run / name)
        if not path.exists():
            continue
        for directory, dirs, files in os.walk(path, followlinks=False):
            for child in [Path(directory), *(Path(directory) / part for part in dirs)]:
                if child.is_mount() or child.lstat().st_dev != run.stat().st_dev:
                    raise ValueError('scratch contains a foreign mount')
        shutil.rmtree(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    print(json.dumps(maintain(parser.parse_args().apply), indent=2))


if __name__ == '__main__': main()
