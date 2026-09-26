#!/usr/bin/env python3
"""Bound explicitly listed rebuildable debug caches; retain all evidence."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

from storage_maintenance import ROOT, gate_busy
from retire_pr_attempts import disposable
from compact_gate_evidence import save

GIB = 1024 ** 3
MIN_IDLE_SECONDS = 24 * 3600


def policies():
    return [
        (Path.home() / 'cargo-target/debug', 20 * GIB),
        (ROOT / 'gh88-product-acceptance/build/target/debug', 2 * GIB),
    ]


def aliases(path):
    """Account for both names of the existing SSD bind mounts."""
    if path.is_relative_to(ROOT):
        return [path, Path('/mnt/dev-ssd/codexsymphony-state') / path.relative_to(ROOT)]
    cargo = Path.home() / 'cargo-target'
    if path.is_relative_to(cargo):
        return [path, Path('/mnt/dev-ssd/cargo-target') / path.relative_to(cargo)]
    return [path]


def in_use(path):
    for candidate in aliases(path):
        if gate_busy(candidate, include_launchers=False):
            return True
    return False


def cache_info(path):
    size = int(subprocess.check_output(
        ['du', '-s', '-x', '-B1', str(path)], text=True, timeout=300).split()[0])
    newest = path.stat().st_mtime
    for directory, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            newest = max(newest, (Path(directory) / name).lstat().st_mtime)
    return size, newest


def collect(path, budget, apply=False, now=None, busy=in_use):
    now = time.time() if now is None else now
    result = {'path': str(path), 'budget_bytes': budget}
    if path.is_symlink():
        raise ValueError('symlink cache root')
    if not path.exists():
        return dict(result, status='ABSENT')
    disposable(path)
    size, newest = cache_info(path)
    result.update(bytes_before=size, newest_mtime=newest)
    if size <= budget:
        return dict(result, status='WITHIN_BUDGET')
    if now - newest < MIN_IDLE_SECONDS:
        return dict(result, status='DEFERRED_RECENT')
    if busy(path):
        return dict(result, status='DEFERRED_BUSY')
    if not apply:
        return dict(result, status='WOULD_CLEAN')
    return remove_cache(path, result, busy)


def remove_cache(path, result, busy):
    # Repeat containment and liveness checks immediately before deletion.
    # Never trim a running build merely to meet a capacity target.
    disposable(path)
    if busy(path):
        return dict(result, status='DEFERRED_BUSY')
    shutil.rmtree(path)
    return dict(result, status='CLEANED', finished_at=time.time())


def maintain(state, apply=False):
    results = []
    for path, budget in policies():
        try:
            result = collect(path, budget, apply=apply)
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            result = {'path': str(path), 'status': 'ERROR', 'error': str(error)}
        results.append(result)
    record = {'schema': 'debug-cache-retention/v1', 'checked_at': time.time(),
              'apply': apply, 'results': results}
    save(state / 'cache-retention.json', record)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    if not os.path.ismount(ROOT):
        raise ValueError('runtime SSD bind mount missing')
    if not shutil.rmtree.avoids_symlink_attacks:
        raise ValueError('fd-safe removal required')
    state = ROOT / 'storage-maintenance'
    with (state / 'maintenance.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        result = maintain(state, apply=args.apply)
        print(json.dumps(result), flush=True)
        if any(item['status'] == 'ERROR' for item in result['results']):
            sys.exit(1)


if __name__ == '__main__':
    main()
