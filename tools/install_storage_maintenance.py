#!/usr/bin/env python3
"""Install host disk protection without changing signed gate code or policies."""
import hashlib
from pathlib import Path
import subprocess


def main():
    source = Path(__file__).with_name('storage_maintenance.py').read_bytes()
    version = hashlib.sha256(source).hexdigest()[:16]
    release = Path.home() / '.local/share/codexsymphony/storage-maintenance/releases' / version
    release.mkdir(parents=True, exist_ok=True)
    script = release / 'storage_maintenance.py'
    if script.exists() and script.read_bytes() != source:
        raise ValueError('immutable maintenance release changed')
    script.write_bytes(source)
    units = Path.home() / '.config/systemd/user'
    units.mkdir(parents=True, exist_ok=True)
    (units / 'codexsymphony-storage.service').write_text(f'''[Unit]
Description=CodexSymphony cache cleanup and disk pressure guard

[Service]
Type=oneshot
ExecStart=/usr/bin/python3 {script}
TimeoutStartSec=300
UMask=0077
''')
    (units / 'codexsymphony-storage.timer').write_text('''[Unit]
Description=Check CodexSymphony disk pressure every 15 seconds

[Timer]
OnBootSec=30
OnUnitInactiveSec=15
AccuracySec=1
Unit=codexsymphony-storage.service

[Install]
WantedBy=timers.target
''')
    for name in ('codexsymphony-remote-gate', 'symphony-codexsymphony'):
        dropin = units / (name + '.service.d')
        dropin.mkdir(exist_ok=True)
        (dropin / 'disk-guard.conf').write_text(f'''[Service]
ExecCondition=/usr/bin/python3 {script} --check-start
''')
    subprocess.run(['systemctl', '--user', 'daemon-reload'], check=True)
    subprocess.run(['systemctl', '--user', 'enable', '--now', 'codexsymphony-storage.timer'], check=True)
    subprocess.run(['systemctl', '--user', 'start', 'codexsymphony-storage.service'], check=True)
    print(script)


if __name__ == '__main__':
    main()
