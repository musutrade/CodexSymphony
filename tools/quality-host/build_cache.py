"""Bounded host cache: trusted main publishes; PRs receive private copies only."""
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import uuid

DEFAULT_BYTES = 40 * 1024**3
DEFAULT_TTL = 7 * 86400


def cache_key(repository, approval):
    names = subprocess.check_output(['git', '-C', repository, 'ls-files', '-z'], text=True).split('\0')
    inputs = {}
    for name in filter(None, names):
        p = Path(name)
        if p.name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain', 'rust-toolchain.toml') or '.cargo' in p.parts:
            inputs[name] = hashlib.sha256((repository / p).read_bytes()).hexdigest()
    value = {'schema': 1, 'inputs': inputs, 'runtime': approval['runtime_files'],
             'policy': approval['config_files'], 'rustc': subprocess.check_output(['rustc', '-vV'], text=True),
             'cargo': subprocess.check_output(['cargo', '-V'], text=True)}
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def inventory(root):
    size = 0
    for base, directories, files in os.walk(root, followlinks=False):
        for name in directories + files:
            p = Path(base) / name
            if p.is_symlink() or (not p.is_dir() and not p.is_file()):
                raise ValueError('unsafe compiler cache entry')
            if p.is_file():
                size += p.stat().st_size
    return size


def copy_tree(source, destination):
    """No hard links: PR writes cannot alter the host seed or another run."""
    if source.is_symlink() or destination.exists() or destination.is_symlink():
        raise ValueError('unsafe compiler cache copy')
    inventory(source)
    subprocess.run(['cp', '--archive', '--reflink=auto', '--', str(source), str(destination)], check=True)


@contextmanager
def locked(root):
    if root.resolve() != root:
        raise ValueError('noncanonical compiler cache root')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    if (root / '.lock').is_symlink():
        raise ValueError('symlink compiler cache lock')
    with (root / '.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def prune(root, max_bytes, ttl, now=None):
    if type(max_bytes) is not int or max_bytes < 0 or type(ttl) is not int or ttl <= 0:
        raise ValueError('invalid compiler cache limits')
    now = time.time() if now is None else now
    entries = []
    for p in root.iterdir():
        if p.name.startswith('.pending-'):
            if p.is_symlink() or not p.is_dir():
                raise ValueError('unsafe incomplete compiler cache')
            # Called while holding the cache lock, so no writer is active.
            shutil.rmtree(p)
            continue
        if len(p.name) != 64 or any(c not in '0123456789abcdef' for c in p.name):
            continue
        if p.is_symlink() or not p.is_dir():
            raise ValueError('unsafe compiler cache directory')
        entries.append((p.stat().st_mtime, p, inventory(p)))
    remaining = sum(size for _, _, size in entries)
    for modified, p, size in sorted(entries):
        if now - modified > ttl or remaining > max_bytes:
            shutil.rmtree(p)
            remaining -= size


def restore(root, key, target, max_bytes=DEFAULT_BYTES, ttl=DEFAULT_TTL):
    with locked(root):
        prune(root, max_bytes, ttl)
        source = root / key
        if not source.exists():
            return {'hit': False}
        receipt = json.loads((source / 'cache.json').read_text())
        if receipt['key'] != key or receipt['producer'] != 'main-full-pass':
            raise ValueError('invalid compiler cache producer')
        copy_tree(source / 'target', target)
        os.utime(source, None)
        return {'hit': True, 'source_sha': receipt['source_sha'], 'bytes': receipt['bytes']}


def publish(root, key, target, source_sha, max_bytes=DEFAULT_BYTES, ttl=DEFAULT_TTL):
    # The caller permits this only for main after complete verification. The
    # cache is never mounted into any project process, including main's tests.
    with locked(root):
        stage = root / ('.pending-' + uuid.uuid4().hex)
        stage.mkdir(mode=0o700)
        try:
            copy_tree(target, stage / 'target')
            # Counters must always describe this execution. Incremental objects
            # have poor copy/space economics; Cargo's dependency cache remains.
            for base, directories, files in os.walk(stage / 'target', topdown=True):
                if 'incremental' in directories:
                    shutil.rmtree(Path(base) / 'incremental')
                    directories.remove('incremental')
                for name in files:
                    if name.endswith(('.profraw', '.profdata')):
                        (Path(base) / name).unlink()
            size = inventory(stage / 'target')
            if not max_bytes or size > max_bytes:
                return {'published': False, 'reason': 'entry exceeds cache budget', 'bytes': size}
            receipt = {'schema': 1, 'key': key, 'producer': 'main-full-pass',
                       'source_sha': source_sha, 'bytes': size}
            (stage / 'cache.json').write_text(json.dumps(receipt) + '\n')
            destination = root / key
            if destination.is_symlink():
                raise ValueError('unsafe compiler cache destination')
            if destination.exists():
                shutil.rmtree(destination)
            stage.rename(destination)
            prune(root, max_bytes, ttl)
            return {'published': True, 'bytes': size}
        finally:
            if stage.exists():
                shutil.rmtree(stage)
