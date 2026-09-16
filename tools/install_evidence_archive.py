#!/usr/bin/env python3
"""Install cold-evidence retention; refuses to operate before the SSD bind mount."""
import hashlib,subprocess
from pathlib import Path

root=Path(__file__).resolve().parents[1]
base=Path.home()/'.local/share/codexsymphony'
names=('archive_gate_evidence.py','storage_maintenance.py','compact_gate_evidence.py','retire_pr_attempts.py')
content={name:(root/'tools'/name).read_bytes() for name in names}
version=hashlib.sha256(b''.join(content.values())).hexdigest()[:16]
release=base/'evidence-archive/releases'/version;release.mkdir(parents=True,exist_ok=True)
for name,data in content.items():
    path=release/name
    if path.exists() and path.read_bytes()!=data:raise ValueError('immutable archive release changed')
    path.write_bytes(data)
units=Path.home()/'.config/systemd/user'
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
subprocess.run(['systemctl','--user','daemon-reload'],check=True)
subprocess.run(['systemctl','--user','enable','codexsymphony-archive.timer'],check=True)
print('Installed; start archive timer after migration validation:',release)
