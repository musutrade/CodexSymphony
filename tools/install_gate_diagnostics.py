#!/usr/bin/env python3
"""Install the operator-owned diagnostic exporter without changing Gate approval."""
from pathlib import Path
import hashlib
import subprocess

source=Path(__file__).with_name('export_gate_diagnostics.py').read_bytes()
root=Path.home()/'.local/share/codexsymphony/gate-diagnostics'/hashlib.sha256(source).hexdigest()[:16]
root.mkdir(parents=True,exist_ok=True)
script=root/'export_gate_diagnostics.py'
if script.exists() and script.read_bytes()!=source:raise ValueError('immutable release differs')
script.write_bytes(source)
units=Path.home()/'.config/systemd/user'
(units/'codexsymphony-diagnostics.service').write_text(f'''[Unit]
Description=Export redacted exact-SHA Gate diagnostics to read-only issue mounts
[Service]
Type=oneshot
ExecStart=/usr/bin/python3 {script}
UMask=0077
TimeoutStartSec=60
''')
(units/'codexsymphony-diagnostics.timer').write_text('''[Unit]
Description=Refresh Gate diagnostics every five seconds
[Timer]
OnBootSec=10
OnUnitInactiveSec=5
AccuracySec=1
[Install]
WantedBy=timers.target
''')
for args in [('daemon-reload',),('enable','--now','codexsymphony-diagnostics.timer'),('start','codexsymphony-diagnostics.service')]:
 subprocess.run(['systemctl','--user',*args],check=True)
print(script)
