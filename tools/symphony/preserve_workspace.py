#!/usr/bin/env python3
"""Retain declared evidence before an opt-in, fail-closed workspace removal."""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import stat
import tempfile

MANIFEST = '.symphony-evidence.json'
LIMIT = 1 << 30


def digest(data):
    return hashlib.sha256(data).hexdigest()


@contextlib.contextmanager
def source(root, name):
    parts = PurePosixPath(name).parts
    if not parts or name != '/'.join(parts) or any(p in ('.', '..') for p in parts) or name.startswith('/'):
        raise ValueError('evidence paths must be canonical and workspace-relative')
    with contextlib.ExitStack() as stack:
        fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        stack.callback(os.close, fd)
        for part in parts[:-1]:
            fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            stack.callback(os.close, fd)
        final = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
        stack.callback(os.close, final)
        if not stat.S_ISREG(os.fstat(final).st_mode):
            raise ValueError('evidence must be a regular file')
        yield final


def read_small(root, name):
    with source(root, name) as fd:
        data = os.read(fd, (1 << 20) + 1)
    if len(data) > 1 << 20:
        raise ValueError('manifest exceeds 1 MiB')
    return data


def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def write(path, data):
    with path.open('xb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def entries(data):
    value = json.loads(data)
    if value.get('schema') != 'symphony-evidence/v1' or not isinstance(value.get('files'), list):
        raise ValueError('symphony-evidence/v1 files manifest required')
    files = value['files']
    if len(files) > 4096 or (not files and not value.get('empty_reason')):
        raise ValueError('bounded evidence list or explicit empty_reason required')
    names = set()
    for item in files:
        name, sha = item['path'], item['sha256']
        if name in names or not isinstance(sha, str) or len(sha) != 64 or any(c not in '0123456789abcdef' for c in sha):
            raise ValueError('duplicate path or invalid SHA-256')
        names.add(name)
    return files


def copy_evidence(workspace, target, files):
    total = 0
    result = []
    for index, item in enumerate(files):
        sha = hashlib.sha256()
        size = 0
        # Flat archive names cannot collide with control files or one another.
        destination = target / f'{index:04d}.evidence'
        with source(workspace, item['path']) as fd, destination.open('xb') as output:
            before = os.fstat(fd)
            while chunk := os.read(fd, 1 << 20):
                size += len(chunk)
                total += len(chunk)
                if total > LIMIT:
                    raise ValueError('declared evidence exceeds 1 GiB')
                sha.update(chunk)
                output.write(chunk)
            output.flush()
            os.fsync(output.fileno())
            after = os.fstat(fd)
            if (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_size, after.st_mtime_ns, after.st_ctime_ns):
                raise ValueError('evidence changed during preservation')
        if sha.hexdigest() != item['sha256']:
            raise ValueError('evidence SHA-256 mismatch: ' + item['path'])
        result.append({**item, 'bytes': size, 'retained': destination.name})
    return result


def verify(target, receipt):
    for item in receipt['files']:
        sha = hashlib.sha256()
        with source(target, item['retained']) as fd:
            while chunk := os.read(fd, 1 << 20):
                sha.update(chunk)
        if sha.hexdigest() != item['sha256']:
            raise ValueError('retained evidence checksum mismatch')


def preserve(workspace, archive):
    workspace, archive = Path(workspace).absolute(), Path(archive).absolute()
    if workspace.resolve() != workspace or archive.resolve() != archive:
        raise ValueError('canonical non-symlink roots required')
    if archive.is_relative_to(workspace) or workspace.is_relative_to(archive):
        raise ValueError('archive and workspace must be disjoint')
    data = read_small(workspace, MANIFEST)
    files = entries(data)
    archive.mkdir(parents=True, exist_ok=True, mode=0o700)
    key = digest(str(workspace).encode())[:24] + '-' + digest(data)
    target = archive / key
    lock_fd = os.open(archive / '.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(lock_fd, 'w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        # A fresh source check is required even when retrying a completed copy.
        with tempfile.TemporaryDirectory(prefix='.partial-', dir=archive) as temporary:
            staging = Path(temporary) / 'archive'
            staging.mkdir()
            copied = copy_evidence(workspace, staging, files)
            if read_small(workspace, MANIFEST) != data:
                raise ValueError('manifest changed during preservation')
            receipt = {'schema': 'symphony-preservation/v1', 'workspace': str(workspace),
                       'manifest_sha256': digest(data), 'files': copied}
            write(staging / MANIFEST, data)
            write(staging / 'receipt.json', json.dumps(receipt, sort_keys=True, indent=2).encode())
            verify(staging, receipt)
            sync_dir(staging)
            if target.exists():
                if read_small(target, 'receipt.json') != (staging / 'receipt.json').read_bytes() or read_small(target, MANIFEST) != data:
                    raise ValueError('existing archive identity mismatch')
                verify(target, receipt)
            else:
                os.rename(staging, target)
            sync_dir(archive)
    return target / 'receipt.json'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('workspace', type=Path)
    parser.add_argument('--archive', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    try:
        print(preserve(args.workspace, args.archive))
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, 'Workspace retained; evidence preservation failed: ' + str(error) + '\n')


if __name__ == '__main__':
    main()
