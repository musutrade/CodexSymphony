#!/usr/bin/env python3
"""Install reviewed retention releases without starting cleanup before validation."""
import hashlib
import subprocess
from pathlib import Path

NAMES = ('archive_gate_evidence.py', 'storage_maintenance.py', 'compact_gate_evidence.py',
         'retire_pr_attempts.py', 'cache_retention.py')


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


def main():
    root = Path(__file__).resolve().parents[1]
    base = Path.home() / '.local/share/codexsymphony'
    release = install_release(root, base)
    install_units(base, release, Path.home() / '.config/systemd/user')
    subprocess.run(['systemctl', '--user', 'daemon-reload'], check=True)
    for timer in ('codexsymphony-archive.timer', 'codexsymphony-cache-retention.timer'):
        subprocess.run(['systemctl', '--user', 'enable', timer], check=True)
    print('Installed; start archive and cache-retention timers after validation:', release)


if __name__ == '__main__':
    main()
