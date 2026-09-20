#!/usr/bin/env python3
"""Install fixture collection independently of the running agent workflow."""
from pathlib import Path
import os
import shutil
import subprocess


def install():
    os.umask(0o077)
    state = Path.home() / '.local/share/codexsymphony/symphony'
    state.mkdir(parents=True, exist_ok=True)
    source = Path(__file__).parent / 'symphony/cleanup_issue_environments.py'
    destination = state / source.name
    temporary = destination.with_suffix('.new')
    shutil.copyfile(source, temporary)
    temporary.chmod(0o700)
    temporary.replace(destination)
    units = Path.home() / '.config/systemd/user'
    units.mkdir(parents=True, exist_ok=True)
    (units / 'codexsymphony-fixture-cleanup.service').write_text(f'''[Unit]
Description=Collect completed Symphony disposable Docker fixtures
ConditionPathExists={state}/WORKFLOW.lifecycle.md.handoffs.json
[Service]
Type=oneshot
ExecStart=/usr/bin/python3 {destination} --apply
TimeoutStartSec=300
UMask=0077
Nice=10
''')
    (units / 'codexsymphony-fixture-cleanup.timer').write_text('''[Unit]
Description=Collect completed Symphony fixtures every five minutes
[Timer]
OnBootSec=2min
OnUnitInactiveSec=5min
Unit=codexsymphony-fixture-cleanup.service
[Install]
WantedBy=timers.target
''')
    subprocess.run(['systemctl', '--user', 'daemon-reload'], check=True)
    subprocess.run(['systemctl', '--user', 'enable', '--now', 'codexsymphony-fixture-cleanup.timer'], check=True)


if __name__ == '__main__':
    install()
