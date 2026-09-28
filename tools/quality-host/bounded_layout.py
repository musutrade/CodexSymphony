"""Fixed host paths on the administrator-mounted, capacity-bounded filesystem."""
from contextlib import contextmanager
from pathlib import Path
import uuid

import fixed_workspace

VOLUME = Path('/mnt/dev-ssd/codexsymphony-bounded/data')
REPOSITORY = Path('/home/gem/CodexSymphony')
CACHE_DOMAIN = 'local'


def ensure(repository):
    if repository != REPOSITORY or not repository.is_mount() or not VOLUME.is_mount():
        raise ValueError('fixed repository and bounded storage mounts are required')
    if repository.resolve() != repository or repository.stat().st_dev != VOLUME.stat().st_dev:
        raise ValueError('repository is outside bounded storage')
    for name in ('validation', 'cache', 'tmp', 'evidence'):
        path = VOLUME / name
        if path.resolve() != path or path.stat().st_dev != VOLUME.stat().st_dev:
            raise ValueError('bounded storage path changed: ' + name)


def target(kind='normal'):
    if kind not in ('normal', 'instrumented'):
        raise ValueError('unknown compiler cache kind')
    if CACHE_DOMAIN not in ('local', 'remote'):
        raise ValueError('unknown cache trust domain')
    prefix = 'cargo-' if CACHE_DOMAIN == 'local' else 'cargo-remote-'
    path = VOLUME / 'cache' / (prefix + kind) / 'target'
    fixed_workspace.canonical(path)
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    return path


def slot():
    return VOLUME / 'validation/gate'


def new_run():
    parent = VOLUME / 'evidence/gate'
    fixed_workspace.canonical(parent)
    parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    run = parent / ('run-' + uuid.uuid4().hex[:12])
    run.mkdir(mode=0o700)
    return run


@contextmanager
def lease(repository):
    ensure(repository)
    with fixed_workspace.lease(VOLUME / 'cache/coordination'):
        ensure(repository)
        with fixed_workspace.lease(slot()):
            yield
