"""Linux host isolation: expose only approved code, tools and per-run outputs."""
import os
from pathlib import Path
import subprocess

HOME = Path('/home/gem')

def command(argv, *, run, repository, plugins, writable=(), readonly=(), mounts=(), environment=None, cwd=None):
    run = Path(run).resolve()
    cargo = run / 'cargo-home'
    cargo.mkdir(exist_ok=True)
    temporary = run / 'tmp'
    temporary.mkdir(exist_ok=True)
    args = ['bwrap', '--die-with-parent', '--new-session', '--unshare-user', '--unshare-pid',
            '--ro-bind', '/usr', '/usr', '--ro-bind', '/etc', '/etc',
            '--symlink', 'usr/bin', '/bin', '--symlink', 'usr/lib', '/lib', '--symlink', 'usr/lib64', '/lib64',
            '--proc', '/proc', '--dev', '/dev', '--tmpfs', '/run', '--bind', str(temporary), '/tmp',
            '--dir', str(HOME), '--bind', str(cargo), str(HOME / '.cargo'),
            '--ro-bind', str(HOME / '.cargo/registry'), str(HOME / '.cargo/registry'),
            '--ro-bind', str(HOME / '.cargo/bin'), str(HOME / '.cargo/bin'),
            '--ro-bind', str(HOME / '.rustup'), str(HOME / '.rustup'),
            '--ro-bind', str(repository), str(repository),
            '--ro-bind', str(plugins), str(plugins)]
    for path in readonly:
        args += ['--ro-bind', str(path), str(path)]
    for path in writable:
        args += ['--bind', str(path), str(path)]
    for source, target in mounts:
        args += ['--ro-bind', str(source), str(target)]
    env = {'HOME': str(HOME), 'CARGO_HOME': str(HOME / '.cargo'), 'RUSTUP_HOME': str(HOME / '.rustup'),
           'PATH': str(HOME / '.cargo/bin') + ':/usr/local/bin:/usr/bin:/bin',
           'LANG': 'C.UTF-8', 'TZ': 'UTC', 'CARGO_NET_OFFLINE': 'true',
           'CARGO_TARGET_DIR': str(run / 'target')}
    env.update(environment or {})
    args += ['--clearenv']
    for name, value in env.items(): args += ['--setenv', name, value]
    args += ['--chdir', str(cwd or repository), '--', *map(str, argv)]
    return args
