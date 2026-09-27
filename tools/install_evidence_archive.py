#!/usr/bin/env python3
"""Install reviewed retention releases without starting cleanup before validation."""
import hashlib
import json
import os
import subprocess
from pathlib import Path

NAMES = ('archive_gate_evidence.py', 'storage_maintenance.py', 'compact_gate_evidence.py',
         'retire_pr_attempts.py', 'cache_retention.py', 'capture_cache_retention.py')
SERVICES = ('codexsymphony-archive.service', 'codexsymphony-cache-retention.service',
            'codexsymphony-capture-cache.service', 'codexsymphony-storage.service')
GUARDED = (*SERVICES, 'symphony-codexsymphony.service', 'codexsymphony-remote-gate.service')


def install_release(root, base):
    content = {name: (root / 'tools' / name).read_bytes() for name in NAMES}
    version = hashlib.sha256(b''.join(content.values())).hexdigest()[:16]
    release = base / 'evidence-archive/releases' / version
    release.mkdir(parents=True, exist_ok=True)
    for name, data in content.items():
        path = release / name
        if path.exists() and path.read_bytes() != data:
            raise ValueError('immutable archive release changed')
        path.write_bytes(data)
    return release


def install_units(base, release, units):
    units.mkdir(parents=True, exist_ok=True)
    (units/'codexsymphony-archive.service').write_text(f'''[Unit]
Description=Bounded retention of rebuildable Gate evidence
ConditionPathIsMountPoint={base}
ConditionPathIsMountPoint=/data

[Service]
Type=oneshot
ExecStart=/usr/bin/python3 {release}/compact_gate_evidence.py --apply
TimeoutStartSec=3600
Nice=10
IOSchedulingClass=idle
UMask=0077
''')
    (units/'codexsymphony-archive.timer').write_text('''[Unit]
Description=Bound hot Gate evidence retention
[Timer]
OnBootSec=5min
OnUnitInactiveSec=1h
Unit=codexsymphony-archive.service
[Install]
WantedBy=timers.target
''')
    (units/'codexsymphony-capture-cache.service').write_text(f'''[Unit]
Description=Reclaim registered completed capture compiler caches
ConditionPathIsMountPoint={base}

[Service]
Type=oneshot
ExecStart=/usr/bin/python3 {release}/capture_cache_retention.py --apply
TimeoutStartSec=1800
Nice=10
IOSchedulingClass=idle
UMask=0077
''')
    (units/'codexsymphony-capture-cache.timer').write_text('''[Unit]
Description=Collect registered capture compiler caches hourly
[Timer]
OnBootSec=10min
OnUnitInactiveSec=1h
Unit=codexsymphony-capture-cache.service
[Install]
WantedBy=timers.target
''')
    (units/'codexsymphony-cache-retention.service').write_text(f'''[Unit]
Description=Bound inactive rebuildable debug caches
ConditionPathIsMountPoint={base}

[Service]
Type=oneshot
ExecStart=/usr/bin/python3 {release}/cache_retention.py --apply
TimeoutStartSec=900
Nice=10
IOSchedulingClass=idle
UMask=0077
''')
    (units/'codexsymphony-cache-retention.timer').write_text('''[Unit]
Description=Check rebuildable debug cache budgets hourly
[Timer]
OnBootSec=10min
OnUnitInactiveSec=1h
Unit=codexsymphony-cache-retention.service
[Install]
WantedBy=timers.target
''')


def install_guard(base, release, units):
    script = release / 'storage_maintenance.py'
    (units/'codexsymphony-storage.service').write_text(f'''[Unit]
Description=CodexSymphony disk and retention deployment guard
[Service]
Type=oneshot
ExecStart=/usr/bin/python3 {script}
TimeoutStartSec=300
UMask=0077
''')
    (units/'codexsymphony-storage.timer').write_text('''[Unit]
Description=Check disk pressure and retention deployment every 15 seconds
[Timer]
OnBootSec=30
OnUnitInactiveSec=15
AccuracySec=1
Unit=codexsymphony-storage.service
[Install]
WantedBy=timers.target
''')
    for name in GUARDED:
        dropin = units / (name + '.d')
        dropin.mkdir(exist_ok=True)
        (dropin/'retention-guard.conf').write_text(f'[Service]\nExecStartPre=/usr/bin/python3 {script} --check-retention\n')
    for name in GUARDED[-2:]:
        (units/(name+'.d')/'disk-guard.conf').write_text(f'[Service]\nExecCondition=/usr/bin/python3 {script} --check-start\n')
    paths = [release/name for name in NAMES]
    paths += [units/name for service in SERVICES for name in (service, service.replace('.service', '.timer'))]
    paths += [units/(name+'.d')/'retention-guard.conf' for name in GUARDED]
    paths += [units/(name+'.d')/'disk-guard.conf' for name in GUARDED[-2:]]
    scripts = ('compact_gate_evidence.py --apply', 'cache_retention.py --apply',
               'capture_cache_retention.py --apply', 'storage_maintenance.py')
    receipt = {'schema': 'retention-deployment/v1', 'release': str(release),
               'commands': {unit: f'/usr/bin/python3 {release}/{script}' for unit, script in zip(SERVICES, scripts)},
               'files': {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}}
    state = base/'storage-maintenance'
    state.mkdir(parents=True, exist_ok=True)
    pending = state/'deployment.new'
    pending.write_text(json.dumps(receipt, indent=2)+'\n')
    pending.replace(state/'deployment.json')


def main():
    os.umask(0o077)
    root = Path(__file__).resolve().parents[1]
    base = Path.home() / '.local/share/codexsymphony'
    release = install_release(root, base)
    install_units(base, release, Path.home() / '.config/systemd/user')
    install_guard(base, release, Path.home() / '.config/systemd/user')
    subprocess.run(['systemctl', '--user', 'daemon-reload'], check=True)
    for timer in ('codexsymphony-archive.timer', 'codexsymphony-cache-retention.timer',
                  'codexsymphony-capture-cache.timer', 'codexsymphony-storage.timer'):
        subprocess.run(['systemctl', '--user', 'enable', timer], check=True)
    print('Installed; start retention timers after validation:', release)


if __name__ == '__main__':
    main()
