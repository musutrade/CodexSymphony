#!/usr/bin/env python3
"""Host-only cache collection and disk pressure protection for managed services.

Never run from an issue sandbox. Retained evidence is not garbage collected.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

ROOT = Path.home() / '.local/share/codexsymphony'
SERVICES = ('symphony-codexsymphony.service', 'codexsymphony-remote-gate.service')
GIB = 1024 ** 3
STOP_FREE = 12 * GIB
RESUME_FREE = 20 * GIB


def check_retention_files(files):
    for name, expected in files.items():
        path = Path(name)
        if path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError('retention deployment drift: ' + name)


def check_retention(root):
    receipt = json.loads((root / 'storage-maintenance/deployment.json').read_text())
    if receipt['schema'] != 'retention-deployment/v1':
        raise ValueError('unsupported retention deployment')
    check_retention_files(receipt['files'])
    for unit, expected in receipt['commands'].items():
        actual = subprocess.check_output(['systemctl', '--user', 'show', unit,
                                          '--property=ExecStart', '--value'], text=True)
        if 'argv[]=' not in actual or actual.split('argv[]=', 1)[1].split(' ;', 1)[0] != expected:
            raise ValueError('retention deployment drift: ' + unit)


def guard_retention(root, control_service=None):
    # Admission failures preserve the pause ledger before stopping dispatch.
    control_service = control_service or control
    try:
        check_retention(root)
    except (ValueError, OSError, KeyError, subprocess.SubprocessError):
        path = root / 'storage-maintenance/state.json'
        state = json.loads(path.read_text()) if path.exists() else {'paused_services': []}
        for service in SERVICES:
            if service not in state['paused_services']:
                state['paused_services'].append(service)
                save(path, state)
            control_service('stop', service)
        raise


def protected_session(arguments, command):
    # sshd pads its process title with spaces; match only the existing session forms.
    if arguments[0] == '/usr/lib/systemd/systemd' and '--user' in arguments:
        return True
    title = command.rstrip('\0').rstrip(' ')
    if title == '(sd-pam)':
        return True
    return re.fullmatch(r'sshd-session: [a-zA-Z0-9_-]+@(?:pts/[0-9]+|notty)', title) is not None


def fixed_fixture_broker(arguments):
    if len(arguments) != 2 or arguments[0] != '/usr/bin/python3':
        return False
    broker_root = ROOT / 'symphony'
    return re.fullmatch(re.escape(str(broker_root)) + r'/gh\d+-environment/broker.py', arguments[1]) is not None


def gate_launcher(arguments):
    for argument in arguments:
        if argument.endswith('/run.py') and ('/quality-host/' in argument or '/gate-host/releases/' in argument):
            return True
    return False


def process_paths_busy(process, runs):
    links = [process / 'cwd', *(process / 'fd').iterdir()]
    for link in links:
        try:
            value = os.readlink(link)
        except FileNotFoundError:
            continue
        if value == str(runs) or value.startswith(str(runs) + '/'):
            return True
    return False


def process_busy(process, runs, include_launchers):
    if process.stat().st_uid != os.getuid():
        return False
    command = (process / 'cmdline').read_bytes().decode(errors='replace')
    if str(runs) + '/' in command:
        return True
    arguments = command.rstrip('\0').split('\0')
    if fixed_fixture_broker(arguments) or protected_session(arguments, command):
        return False
    if include_launchers and gate_launcher(arguments):
        return True
    return process_paths_busy(process, runs)


def gate_busy(runs, proc=Path('/proc'), include_launchers=True):
    """Inspect workers, including orphans; unreadable worker state defers cleanup."""
    for process in proc.iterdir():
        if not process.name.isdigit() or int(process.name) == os.getpid():
            continue
        try:
            if process_busy(process, runs, include_launchers):
                return True
        except FileNotFoundError:
            continue
        except (PermissionError, OSError):
            return True
    return False


def collect(root, busy=gate_busy, now=None):
    runs = root / 'gate-host/runs'
    if not runs.exists():
        return {'removed': [], 'deferred': False}
    if runs.resolve() != runs.absolute():
        raise ValueError('runs directory must not contain symlinks')
    now = time.time() if now is None else now
    candidates = []
    for run in sorted(runs.iterdir()):
        target = run / 'target'
        if not re.fullmatch(r'run-[0-9a-f]{12}', run.name) or run.is_symlink():
            continue
        if target.is_symlink() or not target.is_dir():
            continue
        if now - target.stat().st_mtime >= 300:
            candidates.append(target)
    removed = []
    for target in candidates:
        # A finalized, old UUID cannot be reused by a new Gate run. Check its
        # own workers instead of letting another PR indefinitely pin its cache.
        result = target.parent / 'verify-result.json'
        finalized = result.is_file() and now - result.stat().st_mtime >= 60
        in_use = (gate_busy(target.parent, include_launchers=False)
                  if busy is gate_busy and finalized else busy(runs))
        if in_use:
            return {'removed': removed, 'deferred': True}
        # No active capture can still be populating these old UUID directories.
        # rmtree uses fd-based symlink protection on this Linux host.
        if target.is_symlink() or target.parent.is_symlink():
            raise ValueError('cache path changed to symlink')
        device = runs.stat().st_dev
        for directory, dirs, files in os.walk(target, followlinks=False):
            for path in [Path(directory), *(Path(directory) / name for name in dirs + files)]:
                if path.lstat().st_dev != device or path.is_mount():
                    raise ValueError('cache contains mount: ' + str(path))
        shutil.rmtree(target)
        removed.append(str(target))
    return {'removed': removed, 'deferred': False}


def collect_workspace_caches(root, busy=gate_busy, scheduler_stopped=None):
    """Reclaim rebuildable objects, retaining coverage binaries/profiles and evidence."""
    ledger = root / 'symphony/WORKFLOW.lifecycle.md.handoffs.json'
    if not ledger.exists():
        return {'removed': [], 'deferred': False}
    if scheduler_stopped is None:
        scheduler_stopped = not control('is-active', SERVICES[0])
    entries = json.loads(ledger.read_text())['entries']
    workspaces = root / 'workspaces'
    removed = []
    for entry in entries.values():
        phase = entry.get('phase')
        if phase != 'done' and not (scheduler_stopped and phase in ('waiting', 'blocked')):
            continue
        name = entry.get('identifier', '')
        if not re.fullmatch(r'GH-\d+', name):
            continue
        workspace = workspaces / name
        if workspace.resolve() != workspace.absolute() or not workspace.is_dir():
            continue
        if busy(workspace):
            continue
        for relative in ('target/debug', 'target/llvm-cov-target/debug/incremental'):
            cache = workspace / relative
            if not cache.is_dir() or cache.resolve() != cache.absolute():
                continue
            if time.time() - cache.stat().st_mtime < 300:
                continue
            device = workspace.stat().st_dev
            for directory, dirs, files in os.walk(cache, followlinks=False):
                for path in [Path(directory), *(Path(directory) / n for n in dirs + files)]:
                    if path.lstat().st_dev != device or path.is_mount():
                        raise ValueError('workspace cache contains mount: ' + str(path))
            if busy(workspace):
                continue
            shutil.rmtree(cache)
            removed.append(str(cache))
    return {'removed': removed, 'deferred': False}


def save(path, state):
    pending = path.with_suffix('.new')
    pending.write_text(json.dumps(state, indent=2) + '\n')
    pending.replace(path)


def control(action, service):
    result = subprocess.run(['systemctl', '--user', action, service],
                            capture_output=True, text=True, timeout=120)
    if action == 'is-active':
        return result.stdout.strip() in ('active', 'activating', 'reloading')
    result.check_returncode()
    if action == 'start' and not control('is-active', service):
        # systemd reports a skipped ExecCondition as a successful start job.
        # Keep the pause record so the next healthy tick retries admission.
        raise RuntimeError('service did not start: ' + service)


def maintain(root, control_service=control, free_bytes=None, collector=collect):
    free_bytes = free_bytes or (lambda: shutil.disk_usage(root).free)
    state_path = root / 'storage-maintenance/state.json'
    state_path.parent.mkdir(parents=True, exist_ok=True)
    state = json.loads(state_path.read_text()) if state_path.exists() else {'paused_services': []}
    before = free_bytes()
    if before < STOP_FREE:
        for service in SERVICES:
            if control_service('is-active', service):
                if service not in state['paused_services']:
                    state['paused_services'].append(service)
                    save(state_path, state)  # Survives an interrupted stop.
                control_service('stop', service)
    outcome = collector(root)
    if collector is collect:
        workspace_outcome = collect_workspace_caches(root, scheduler_stopped=not control_service('is-active', SERVICES[0]))
        outcome['removed'].extend(workspace_outcome['removed'])
    after = free_bytes()
    if after >= RESUME_FREE:
        for service in list(state['paused_services']):
            if service not in SERVICES:
                raise ValueError('unexpected managed service')
            control_service('start', service)
            state['paused_services'].remove(service)
            save(state_path, state)
    state.update(outcome, checked_at=time.time(), free_before=before, free_after=after,
                 disk_pressure=after < STOP_FREE)
    save(state_path, state)
    return state


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check-start', action='store_true')
    parser.add_argument('--check-retention', action='store_true')
    parser.add_argument('--guard-retention', action='store_true')
    args = parser.parse_args()
    if args.guard_retention:
        guard_retention(ROOT)
        return
    if args.check_retention:
        check_retention(ROOT)
        return
    if args.check_start:
        check_retention(ROOT)
        free = shutil.disk_usage(ROOT).free
        if free < RESUME_FREE:
            print(f'Disk guard: {free / GIB:.1f} GiB free; start requires 20 GiB', flush=True)
            raise SystemExit(1)
        return
    if not shutil.rmtree.avoids_symlink_attacks:
        raise RuntimeError('fd-safe rmtree required')
    home = ROOT / 'storage-maintenance'
    home.mkdir(parents=True, exist_ok=True)
    with (home / 'maintenance.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        guard_retention(ROOT)
        print(json.dumps(maintain(ROOT)), flush=True)


if __name__ == '__main__':
    main()
