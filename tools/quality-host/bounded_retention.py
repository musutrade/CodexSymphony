"""Apply the installed retention policy to completed bounded captures only.

Completed captures, pending captures and their ranking are evidence_pins'
definitions, the ones pin admission charges the current record by.

Lock order: workspace lease -> pin lock. The whole pass (compaction of rebuildable
payloads, scratch cleanup, record retirement and resumed deletions) runs under the
exclusive pin lock, so a pin is never created or checked between a cleanup decision
and its rename or delete. Pinned runs are still compacted: only CLEANABLE paths are
removed, which never hold pinned evidence.
"""
import argparse
import importlib.util
import inspect
import json
import os
from pathlib import Path
import shutil
import sys
import time

import bounded_layout as layout
import bounded_records
import evidence_pins
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
    return evidence_pins.pending(layout.slot())


def completed(run):
    """Shared with pin admission (evidence_pins.completed), so both rank the same current record."""
    return evidence_pins.completed(run)


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
    with evidence_pins.cleanup(layout.VOLUME / 'evidence/pins') as pins:
        return retain(policy, parent, pending, pins, keep, hours, budget, apply)


def retain(policy, parent, pending, pins, keep, hours, budget, apply):
    candidates = evidence_pins.candidates(parent, pending)
    results = expire(policy, candidates, keep, hours, budget, apply)
    return results + bounded_records.maintain(parent, candidates, keep, hours, budget, apply, pins)


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
