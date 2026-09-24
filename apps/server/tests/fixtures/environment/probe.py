#!/usr/bin/env python3
"""Controlled test adapter: observes real tools, files and a live process.

The fixture's settings file represents a generated service configuration. This
is not a general host installer, container probe or validation implementation.
"""
import hashlib
import json
from pathlib import Path
import shutil
import socket
import subprocess
import sys

request = json.load(sys.stdin)
root = Path(request['resource_root'])
settings_bytes = (root / 'settings.json').read_bytes()
settings = json.loads(settings_bytes)
tool = shutil.which(settings['tool'])
actual = {
    'tool.version': subprocess.check_output([tool, '--version'], text=True).strip(),
    'tool.digest': hashlib.sha256(Path(tool).resolve().read_bytes()).hexdigest(),
    'schedule.threads': settings['threads'],
    'image': settings['image'],
    'memory': settings['memory'],
    'cache': settings['cache'],
    'runtime.state': (root / 'state').read_text(),
}
if (root / 'service.pid').exists():
    pid = int((root / 'service.pid').read_text())
    with socket.socket(socket.AF_UNIX) as connection:
        connection.connect(str(root / 'service.sock'))
        effective = connection.recv(1024).decode()
    actual.update({
        'service.fixture.installed': hashlib.sha256((root / 'installed').read_bytes()).hexdigest(),
        'service.fixture.process': hashlib.sha256(Path(f'/proc/{pid}/exe').read_bytes()).hexdigest(),
        'service.fixture.config': hashlib.sha256(settings_bytes).hexdigest(),
        'service.fixture.effective_config': effective,
        'service.fixture.syntax_valid': True,
        'service.fixture.semantic_valid': settings['threads'] > 0,
    })
digest = hashlib.sha256(json.dumps(actual, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
checks = [{'id': 'environment', 'verdict': 'pass', 'evidence': [{'artifact_id': 'actual', 'sha256': digest}]}]
evaluation = {'call': request['call'], 'verdict': 'pass', 'checks': checks} if request['call'] else None
print(json.dumps({'request': request, 'actual': actual, 'checks': checks, 'evaluation': evaluation}))
