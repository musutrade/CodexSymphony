#!/usr/bin/env python3
"""Reject effective service overrides and mismatched Gate deployments before work."""
import hashlib
import json
from pathlib import Path
import subprocess
import shlex

BASE = Path.home()/'.local/share/codexsymphony'


def effective_command(unit):
    value = subprocess.check_output(['systemctl', '--user', 'show', unit,
                                     '--property=ExecStart', '--value'], text=True).strip()
    marker = 'argv[]='
    if marker not in value:
        raise ValueError('deployment drift: missing service command: '+unit)
    return value.split(marker, 1)[1].split(' ;', 1)[0]


def check_commands(expected):
    for unit, command in expected.items():
        actual = effective_command(unit)
        if actual != command:
            raise ValueError('deployment drift: '+unit+' expected '+command+', actual '+actual)


def main():
    receipt = json.loads((BASE/'symphony/deployment.json').read_text())
    check_commands(receipt['commands'])
    remote_unit='codexsymphony-remote-gate.service'
    pid=int(subprocess.check_output(['systemctl','--user','show',remote_unit,'--property=MainPID','--value'],text=True))
    if not pid:
        raise ValueError('deployment unavailable: remote Gate service is not running')
    actual=Path('/proc')/str(pid)/'cmdline'
    command=actual.read_bytes().rstrip(b'\0').decode().split('\0')
    if command != shlex.split(receipt['commands'][remote_unit]):
        raise ValueError('deployment drift: running remote Gate process still uses another release; restart required')
    for name, expected in receipt['files'].items():
        if hashlib.sha256(Path(name).read_bytes()).hexdigest() != expected:
            raise ValueError('deployment drift: '+name)
    config = json.loads(Path(receipt['remote_config']).read_text())
    if Path(config['gate_approval']).read_bytes() != (BASE/'gate-host/approval.json').read_bytes():
        raise ValueError('deployment drift: remote CI and local publication use different Gate approvals')
    print(json.dumps({'status': 'PASS', 'commands': receipt['commands'],
                      'gate_approval': config['gate_approval']}))


if __name__ == '__main__':
    main()
