#!/usr/bin/env python3
"""Administrator-owned M2 launch boundary for trusted project commands.

Install outside candidate checkouts. No credentials or policy come from a Run.
This is a deployment mount boundary, not a hostile-code sandbox.
"""
import json
import os
from pathlib import Path
import stat
import sys


def protected(path):
    path = Path(path)
    if not path.is_absolute() or path.is_symlink():
        raise ValueError('absolute, non-symlink configuration required')
    info = path.stat()
    if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077:
        raise ValueError('configuration must be a private regular file (0600)')
    if info.st_uid not in (0, os.getuid()):
        raise ValueError('configuration owner mismatch')
    return json.loads(path.read_text())


def canonical(value):
    path = Path(value)
    if not path.is_absolute() or str(path.resolve(strict=True)) != str(path):
        raise ValueError('mount paths must be existing canonical absolute paths')
    return path


def overlaps(left, right):
    return left.is_relative_to(right) or right.is_relative_to(left)


def prepare(path, arguments):
    config = protected(path)
    if set(config) != {'version', 'role', 'workspace_root', 'runtime_home_root',
                       'mounts', 'private_paths', 'environment', 'program'}:
        raise ValueError('incomplete or unknown boundary configuration')
    if config['version'] != 1 or config['role'] not in ('coding', 'validation'):
        raise ValueError('unsupported boundary role')
    workspace_root = canonical(config['workspace_root'])
    cwd = Path.cwd().resolve(strict=True)
    if cwd == workspace_root or not cwd.is_relative_to(workspace_root):
        raise ValueError('cwd must be a project below the approved workspace root')
    private = [canonical(p) for p in config['private_paths']]
    if not private:
        raise ValueError('control-plane and credential paths must be declared')
    mounts = [(cwd, True)]
    for item in config['mounts']:
        if set(item) != {'path', 'writable'} or type(item['writable']) is not bool:
            raise ValueError('invalid mount')
        mounts.append((canonical(item['path']), item['writable']))
    home = None
    if config['role'] == 'coding':
        root = canonical(config['runtime_home_root'])
        home = canonical(os.environ['CODEX_HOME'])
        if home == root or not home.is_relative_to(root) or home.name != 'codex-home':
            raise ValueError('Runtime home is outside its approved root')
        mounts.append((home, True))
    elif config['runtime_home_root'] is not None:
        raise ValueError('validation must not inherit a Runtime home')
    for source, _ in mounts:
        if source in (Path('/'), Path('/home'), Path('/etc'), Path('/tmp'), Path('/proc'), Path('/dev')):
            raise ValueError('broad or special filesystem mount forbidden')
        if any(overlaps(source, secret) for secret in private):
            raise ValueError('mount overlaps a private path')
        if overlaps(source, Path(path).resolve()):
            raise ValueError('executor configuration must remain outside the child')
    program = canonical(config['program'])
    if not any(program.is_relative_to(source) for source, writable in mounts if not writable):
        raise ValueError('entry executable must be on an approved read-only mount')
    command = ['/usr/bin/bwrap', '--die-with-parent', '--new-session',
               '--unshare-user', '--unshare-pid', '--unshare-ipc', '--unshare-uts',
               '--cap-drop', 'ALL', '--clearenv', '--proc', '/proc', '--dev', '/dev',
               '--tmpfs', '/tmp', '--dir', '/home/executor']
    # Network remains the ordinary trusted project network. Only authorized test
    # credentials belong in environment; the host owns endpoint authorization.
    for source, writable in mounts:
        command += ['--bind' if writable else '--ro-bind', str(source), str(source)]
    for name in ('bin', 'sbin', 'lib', 'lib64'):
        if Path('/' + name).is_symlink():
            command += ['--symlink', os.readlink('/' + name), '/' + name]
    environment = {'HOME': '/home/executor', 'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}
    allowed = {'PATH', 'LANG', 'LC_ALL', 'TZ', 'CARGO_HOME', 'RUSTUP_HOME',
               'CARGO_TARGET_DIR', 'TEST_DATABASE_URL', 'DEV_DATABASE_URL',
               'SSL_CERT_FILE', 'SSL_CERT_DIR'}
    if not set(config['environment']).issubset(allowed):
        raise ValueError('unapproved environment variable')
    environment.update(config['environment'])
    if home:
        environment['CODEX_HOME'] = str(home)
    for key, value in environment.items():
        if not isinstance(value, str) or '\x00' in value:
            raise ValueError('invalid environment value')
        command += ['--setenv', key, value]
    return command + ['--chdir', str(cwd), '--', str(program)] + arguments


def main():
    if len(sys.argv) < 2:
        raise ValueError('administrator configuration required')
    command = prepare(sys.argv[1], sys.argv[2:])
    # Python's launch descriptors are closed by default; exec never adds a shell.
    os.closerange(3, 65536)
    os.execve(command[0], command, {})


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError):
        # Do not echo paths, environment values, candidate output or secrets.
        sys.exit('M2 execution boundary configuration rejected')
