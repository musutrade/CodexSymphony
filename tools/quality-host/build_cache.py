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
import tomllib
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
    value = {'schema': 3, 'scope': 'dependencies-and-sccache', 'inputs': inputs, 'runtime': approval['runtime_files'],
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


def workspace_outputs(repository):
    """Names belonging to repository packages and automatic/explicit targets.

    A name shared with a dependency only causes a harmless cache miss. Cargo
    still fingerprints each retained dependency when restoring the seed.
    """
    packages, targets = set(), set()
    names = subprocess.check_output(['git', '-C', repository, 'ls-files', '-z'], text=True).split('\0')
    for name in names:
        if Path(name).name != 'Cargo.toml':
            continue
        manifest = repository / name
        value = tomllib.loads(manifest.read_text())
        if 'package' not in value:
            continue
        package = value['package']['name']
        packages.add(package)
        targets.update((package, package.replace('-', '_')))
        if 'lib' in value and 'name' in value['lib']:
            targets.add(value['lib']['name'])
        for kind in ('bin', 'test', 'bench', 'example'):
            for target in value.get(kind, []):
                if 'name' in target:
                    targets.update((target['name'], target['name'].replace('-', '_')))
        for directory in ('src/bin', 'tests', 'benches', 'examples'):
            for source in (manifest.parent / directory).glob('*'):
                if source.suffix == '.rs':
                    targets.update((source.stem, source.stem.replace('-', '_')))
                elif source.is_dir() and (source / 'main.rs').is_file():
                    targets.update((source.name, source.name.replace('-', '_')))
    return packages, targets


def copy_dependencies(source, destination, packages, targets):
    if source.is_symlink() or destination.exists() or destination.is_symlink():
        raise ValueError('unsafe dependency cache copy')
    inventory(source)

    def ignore(directory, names):
        parent = Path(directory).name
        excluded = []
        for name in names:
            if name == 'incremental' or name.endswith(('.profraw', '.profdata')):
                excluded.append(name)
            elif parent in ('.fingerprint', 'build') and any(name.startswith(p + '-') for p in packages):
                excluded.append(name)
            elif parent in ('deps', 'examples') and any(
                    name.startswith(prefix + '-') or name == prefix
                    for target in targets for prefix in (target, 'lib' + target)):
                excluded.append(name)
            elif parent in ('debug', 'release') and any(
                    name == target or name.startswith(target + '.') or name.startswith('lib' + target + '.')
                    for target in targets):
                excluded.append(name)
        return excluded

    # Filter before copying; full test executables dwarf most dependencies.
    # Independent files preserve timestamps without sharing writable inodes.
    shutil.copytree(source, destination, ignore=ignore, copy_function=shutil.copy2)


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
        if (receipt['key'] != key or receipt['producer'] != 'main-full-pass'
                or receipt.get('schema') != 3 or receipt.get('scope') != 'dependencies-and-sccache'):
            raise ValueError('invalid compiler cache producer')
        copy_tree(source / 'target', target)
        os.utime(source, None)
        return {'hit': True, 'source_sha': receipt['source_sha'], 'bytes': receipt['bytes']}


def publish(root, key, target, source_sha, max_bytes=DEFAULT_BYTES, ttl=DEFAULT_TTL, repository=None):
    # The caller permits this only for main after complete verification. The
    # cache is never mounted into any project process, including main's tests.
    with locked(root):
        stage = root / ('.pending-' + uuid.uuid4().hex)
        stage.mkdir(mode=0o700)
        try:
            packages, targets = workspace_outputs(repository) if repository is not None else (set(), set())
            copy_dependencies(target, stage / 'target', packages, targets)
            size = inventory(stage / 'target')
            if not max_bytes or size > max_bytes:
                return {'published': False, 'reason': 'entry exceeds cache budget', 'bytes': size}
            receipt = {'schema': 3, 'scope': 'dependencies-and-sccache', 'key': key, 'producer': 'main-full-pass',
                       'source_sha': source_sha, 'bytes': size}
            (stage / 'cache.json').write_text(json.dumps(receipt) + '\n')
            if inventory(stage) > max_bytes:
                return {'published': False, 'reason': 'entry metadata exceeds cache budget', 'bytes': size}
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
