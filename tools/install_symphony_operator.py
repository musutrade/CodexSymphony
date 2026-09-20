#!/usr/bin/env python3
"""Install host-only recovery credentials and external-operation runner."""
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess


def install():
    os.umask(0o077)
    state = Path.home() / '.local/share/codexsymphony/symphony/operator'
    state.mkdir(parents=True, exist_ok=True, mode=0o700)
    token = state / 'token'
    if not token.exists():
        token.write_text(secrets.token_hex(32) + '\n')
    token.chmod(0o600)
    environment = state / 'operator.env'
    environment.write_text('SYMPHONY_OPERATOR_TOKEN=' + token.read_text().strip() + '\n')
    environment.chmod(0o600)
    if not (state / 'grants.json').exists():
        (state / 'grants.json').write_text('{"version": 1, "grants": []}\n')
    preflight = state / 'product_identity_preflight.py'
    shutil.copyfile(Path(__file__).parent / 'symphony/product_identity_preflight.py', preflight)
    preflight.chmod(0o700)
    registry_path = state / 'grants.json'
    registry = json.loads(registry_path.read_text())
    profile = {'operation': 'product.identity_preflight', 'repo': 'musutrade/CodexSymphony',
               'resume_condition': 'health=ok; database=ok; repositories=1360824360,1377749969; delivery_ready=true',
               'argv': [str(preflight)], 'timeout_seconds': 120,
               'executable_sha256': hashlib.sha256(preflight.read_bytes()).hexdigest()}
    registry['profiles'] = [p for p in registry.get('profiles', []) if p.get('operation') != profile['operation']] + [profile]
    registry_path.write_text(json.dumps(registry, indent=2) + '\n')
    script = state / 'operator_bridge.py'
    shutil.copyfile(Path(__file__).parent / 'symphony/operator_bridge.py', script)
    script.chmod(0o700)
    units = Path.home() / '.config/systemd/user'
    dropin = units / 'symphony-codexsymphony.service.d'
    dropin.mkdir(parents=True, exist_ok=True)
    (dropin / 'operator.conf').write_text(f'[Service]\nEnvironmentFile={environment}\n')
    (units / 'codexsymphony-operator.service').write_text(f'''[Unit]
Description=Run pinned authorized Symphony external operations
After=symphony-codexsymphony.service
[Service]
Type=oneshot
ExecStart=/usr/bin/python3 {script}
TimeoutStartSec=660
UMask=0077
''')
    (units / 'codexsymphony-operator.timer').write_text('''[Unit]
Description=Observe Symphony external operation requests
[Timer]
OnBootSec=1min
OnUnitInactiveSec=15s
[Install]
WantedBy=timers.target
''')
    subprocess.run(['systemctl', '--user', 'daemon-reload'], check=True)
    subprocess.run(['systemctl', '--user', 'enable', '--now', 'codexsymphony-operator.timer'], check=True)
    print('Installed host operator bridge; credentials remain outside agent workspace.')


if __name__ == '__main__':
    install()
