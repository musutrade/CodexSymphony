#!/usr/bin/env python3
"""Collect explicitly registered capture caches after verifying retained raw evidence."""
import argparse
import fcntl
import json
from pathlib import Path
import sys
import time

from cache_retention import collect, in_use
from compact_gate_evidence import save, sha
from storage_maintenance import ROOT

SCHEMA = 'capture-cache-registry/v1'


def capture_receipt(root):
    if not root.is_absolute() or root.resolve() != root or not root.is_dir():
        raise ValueError('canonical capture directory required')
    bundle = root / 'probes/backend/bundle.json'
    if bundle.resolve() != bundle or not bundle.is_file():
        raise ValueError('capture bundle missing or symlinked')
    receipt = json.loads(bundle.read_text())['request']['parameters']['receipt']
    raw = root / 'probes/backend/raw'
    validate_receipt(receipt, raw)
    return bundle, raw, receipt


def validate_receipt(receipt, raw):
    if receipt['schema'] != 'rust-source-capture/v1' or receipt['raw_root'] != str(raw):
        raise ValueError('capture raw identity differs')
    if not receipt['raw'] or not receipt['context']['commit'] or not receipt['inputs']:
        raise ValueError('capture source identity or raw inventory missing')


def root_identity(root):
    info = root.stat()
    return {'device': info.st_dev, 'inode': info.st_ino}


def read_registry(path):
    if not path.exists():
        return {'schema': SCHEMA, 'captures': []}
    if path.is_symlink():
        raise ValueError('symlink registry')
    value = json.loads(path.read_text())
    if value['schema'] != SCHEMA or not isinstance(value['captures'], list):
        raise ValueError('invalid capture registry')
    return value


def register(path, root):
    bundle, _, _ = capture_receipt(root)
    value = read_registry(path)
    entry = {'root': str(root), 'identity': root_identity(root),
             'bundle_sha256': sha(bundle), 'registered_at': time.time()}
    for prior in value['captures']:
        if prior['root'] == str(root):
            if prior['identity'] != entry['identity'] or prior['bundle_sha256'] != entry['bundle_sha256']:
                raise ValueError('registered capture replaced or changed')
            return prior
    value['captures'].append(entry)
    save(path, value)
    return entry


def retained_raw(root, entry):
    bundle, raw, receipt = capture_receipt(root)
    if root_identity(root) != entry['identity'] or sha(bundle) != entry['bundle_sha256']:
        raise ValueError('registered capture replaced or changed')
    if raw.resolve() != raw or not raw.is_dir():
        raise ValueError('retained raw directory missing or symlinked')
    for name, digest in receipt['raw'].items():
        verify_raw_file(raw, name, digest)


def verify_raw_file(raw, name, digest):
    path = raw / name
    if Path(name).name != name or path.resolve() != path or not path.is_file():
        raise ValueError('unsafe or missing raw evidence')
    if sha(path) != digest:
        raise ValueError('retained raw checksum mismatch')


def capture_busy(target):
    # Protect the whole capture, including workers writing profiles outside target.
    return in_use(target.parent)


def collect_capture(entry, apply=False):
    root = Path(entry['root'])
    if not root.exists():
        return {'path': str(root), 'status': 'ABSENT'}
    if root.resolve() != root or root_identity(root) != entry['identity']:
        raise ValueError('registered capture replaced or symlinked')
    target = root / 'target'
    result = collect(target, 0, busy=capture_busy, min_idle_seconds=0)
    if result['status'] != 'WOULD_CLEAN':
        return result
    retained_raw(root, entry)
    if apply:
        result = collect(target, 0, apply=True, busy=capture_busy, min_idle_seconds=0)
    return result


def maintain(state, apply=False):
    results = []
    for entry in read_registry(state / 'capture-caches.json')['captures']:
        try:
            result = collect_capture(entry, apply=apply)
        except (OSError, ValueError, KeyError) as error:
            result = {'path': entry.get('root'), 'status': 'ERROR', 'error': str(error)}
        results.append(result)
    record = {'schema': 'capture-cache-retention/v1', 'checked_at': time.time(),
              'apply': apply, 'results': results}
    save(state / 'capture-cache-retention.json', record)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--register', type=Path)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    state = ROOT / 'storage-maintenance'
    with (state / 'maintenance.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.register:
            print(json.dumps(register(state / 'capture-caches.json', args.register)))
            return
        record = maintain(state, apply=args.apply)
        print(json.dumps(record), flush=True)
        if any(item['status'] == 'ERROR' for item in record['results']):
            sys.exit(1)


if __name__ == '__main__':
    main()
