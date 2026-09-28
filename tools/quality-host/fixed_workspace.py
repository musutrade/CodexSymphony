"""One reusable validation checkout, protected for the caller's entire run.

The installed caller owns the directory and must hold lease() through capture,
verification and evidence handoff. This module does not approve or retain evidence.
"""
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess


def canonical(path):
    path = Path(path)
    if not path.is_absolute() or path.resolve() != path:
        raise ValueError('canonical, non-symlink workspace path required')
    return path


def git(repository, *args):
    return subprocess.check_output(['git', '-C', str(repository), *args], stderr=subprocess.PIPE)


def fingerprint(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError('regular source file required: ' + str(path))
    return {'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
            'mode': stat.S_IMODE(path.stat().st_mode)}


def sources(repository):
    names = git(repository, 'ls-files', '-z', '--cached', '--others', '--exclude-standard')
    result = {}
    for raw in names.split(b'\0'):
        if not raw:
            continue
        name = os.fsdecode(raw)
        path = repository / name
        if not path.resolve().is_relative_to(repository):
            raise ValueError('source escapes repository')
        if path.exists() or path.is_symlink():
            result[name] = fingerprint(path)
    return result


def write_json(path, value):
    temporary = path.with_suffix('.pending')
    with temporary.open('x') as stream:
        json.dump(value, stream, sort_keys=True)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


@contextmanager
def lease(directory):
    directory = canonical(directory)
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(directory / 'writer.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield directory
    finally:
        os.close(descriptor)


def binding(repository, root):
    info = root.stat()
    return {'repository': str(repository), 'device': info.st_dev, 'inode': info.st_ino}


def initialize(repository, directory):
    root = directory / 'workspace'
    owner = directory / 'owner.json'
    if owner.exists():
        if owner.is_symlink() or root.is_symlink():
            raise ValueError('symlink validation workspace or owner')
        if json.loads(owner.read_text()) != binding(repository, root):
            raise ValueError('validation workspace ownership changed')
        return root
    if root.exists() or root.is_symlink():
        raise ValueError('refuse to adopt an unowned checkout')
    git(directory, 'clone', '--quiet', '--no-hardlinks', '--local', str(repository), str(root))
    write_json(owner, binding(repository, root))
    return root


def prior_inputs(directory):
    path = directory / 'source-inputs.json'
    if path.is_symlink():
        raise ValueError('symlink source inventory')
    if path.exists():
        return json.loads(path.read_text())
    return {}


def verify_previous(root, previous):
    for name, expected in previous.items():
        if fingerprint(root / name) != expected:
            raise ValueError('validation source changed outside synchronization: ' + name)
    extras = set(sources(root)) - set(previous)
    if extras:
        raise ValueError('unowned validation source files: ' + ', '.join(sorted(extras)))


def copy_sources(repository, root, current):
    for name in current:
        target = root / name
        if target.is_symlink() or target.resolve() != target:
            raise ValueError('symlink validation destination')
        if target.is_file() and fingerprint(target) == current[name]:
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(repository / name, target)
        shutil.copymode(repository / name, target)


def refresh(repository, root, current, previous):
    git(root, 'fetch', '--quiet', '--no-tags', str(repository), 'HEAD')
    if git(root, 'rev-parse', 'HEAD') != git(root, 'rev-parse', 'FETCH_HEAD'):
        git(root, 'checkout', '--quiet', '--detach', '--force', 'FETCH_HEAD')
    stale = (set(sources(root)) | set(previous)) - set(current)
    for name in sorted(stale):
        path = root / name
        if path.exists():
            path.unlink()
    copy_sources(repository, root, current)


def synchronize(repository, directory):
    """Called under lease; an interrupted sync blocks until explicitly recovered."""
    repository, directory = canonical(repository), canonical(directory)
    current = sources(repository)
    previous = prior_inputs(directory)
    initialized = (directory / 'owner.json').exists()
    root = initialize(repository, directory)
    if initialized:
        verify_previous(root, previous)
    refresh(repository, root, current, previous)
    if sources(repository) != current or sources(root) != current:
        raise ValueError('source changed during synchronization')
    write_json(directory / 'source-inputs.json', current)
    return root, {name: value['sha256'] for name, value in current.items()}


def synchronize_revision(repository, directory, revision):
    """Check out a reviewed remote commit in the same leased validation slot."""
    import re
    if not re.fullmatch(r'[0-9a-f]{40}', revision):
        raise ValueError('exact Git commit required')
    repository, directory = canonical(repository), canonical(directory)
    previous = prior_inputs(directory)
    initialized = (directory / 'owner.json').exists()
    root = initialize(repository, directory)
    if initialized:
        verify_previous(root, previous)
    git(root, 'fetch', '--quiet', '--no-tags', str(repository), revision)
    git(root, 'checkout', '--quiet', '--detach', '--force', revision)
    tracked = set(os.fsdecode(name) for name in git(root, 'ls-files', '-z', '--cached').split(b'\0') if name)
    for name in set(previous) - tracked:
        path = root / name
        if path.exists():
            path.unlink()
    current = sources(root)
    if set(current) != tracked or git(root, 'rev-parse', 'HEAD').decode().strip() != revision:
        raise ValueError('remote validation checkout is not the exact commit')
    write_json(directory / 'source-inputs.json', current)
    return root, {name: value['sha256'] for name, value in current.items()}
