#!/usr/bin/env python3
"""Install the reviewed local Elixir development workflow; scheduling starts separately."""
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT=Path(__file__).resolve().parents[1]
HOME=Path.home()
BASE=HOME/'.local/share/codexsymphony'

def routed_workflow(workflow, previous=None):
    pattern=rb'  required_labels:\n(?:    - [^\n]+\n)+'
    expected=re.search(pattern,workflow)
    if expected is None:raise ValueError('expected label filter missing')
    if previous is not None:
        existing=re.search(pattern,previous)
        if existing is None:raise ValueError('existing label filter missing; refusing to replace active routing')
        labels=existing.group()
    else:
        labels=b'  required_labels:\n    - symphony-ready\n    - symphony-environment-acceptance\n'
    return workflow[:expected.start()]+labels+workflow[expected.end():]

def environment_sources():
    directory=ROOT/'tools/symphony/environment'
    return {path.relative_to(directory):path.read_bytes() for path in directory.rglob('*') if path.is_file() and '__pycache__' not in path.parts}

def check_environment_resources(state):
    required=['client/pg/psql','client/pg/ld-musl-x86_64.so.1','arc-admin/SOURCE.json','npm-cache-seed/_cacache']
    missing=[name for name in required if not (state/'environment-template'/name).exists()]
    if missing:
        raise ValueError('Approved host environment resources are missing: '+', '.join(missing)+
                         '. Provision environment-template first; existing workflow has not been replaced.')

def main():
    os.umask(0o077)
    state=BASE/'symphony';state.mkdir(parents=True,exist_ok=True)
    workflow=(ROOT/'WORKFLOW.lifecycle.md').read_bytes()
    wrapper=(ROOT/'tools/symphony/codex_sandbox.py').read_bytes()
    provision=(ROOT/'tools/symphony/provision_issue_environment.py').read_bytes()
    sources=environment_sources()
    check_environment_resources(state)
    active=state/'WORKFLOW.lifecycle.md'
    routed=routed_workflow(workflow,active.read_bytes() if active.exists() else None)
    revision=hashlib.sha256(workflow+wrapper+provision+b''.join(sources[key] for key in sorted(sources))).hexdigest()[:16]
    release=state/'releases'/revision;release.mkdir(parents=True,exist_ok=True)
    for name,data in [('WORKFLOW.lifecycle.md',workflow),('codex_sandbox.py',wrapper)]:
        path=release/name
        if path.exists() and path.read_bytes()!=data:raise ValueError('immutable release differs')
        path.write_bytes(data)
    # Stable journal path is retained across workflow releases.
    (state/'provision_issue_environment.py').write_bytes(provision)
    for name,data in sources.items():
        path=state/'environment-template'/name;path.parent.mkdir(parents=True,exist_ok=True)
        path.write_bytes(data)
        if name==Path('client/bin/psql'):path.chmod(0o755)
    active.write_bytes(routed)
    command=state/'codex-sandbox';command.write_text('#!/bin/sh\nexec /usr/bin/python3 '+str(release/'codex_sandbox.py')+' "$@"\n');command.chmod(0o700)
    environment=HOME/'.config/symphony/codexsymphony.env'
    if not environment.exists():
        old=(HOME/'.config/symphony/harness-gate.env').read_text().splitlines()
        token=next(line for line in old if line.startswith('GITHUB_TOKEN='))
        environment.write_text(token+'\nCODEXSYMPHONY_WORKSPACE_ROOT='+str(BASE/'workspaces')+'\nPATH=/home/gem/.local/bin:/home/gem/.cargo/bin:/usr/local/bin:/usr/bin:/bin\n')
        environment.chmod(0o600)
    (BASE/'workspaces').mkdir(exist_ok=True)
    service=HOME/'.config/systemd/user/symphony-codexsymphony.service'
    service.write_text(f'''[Unit]
Description=Local Symphony - CodexSymphony serial development
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/home/gem/symphony/elixir
EnvironmentFile={environment}
Environment=HTTP_PROXY=http://127.0.0.1:7890
Environment=HTTPS_PROXY=http://127.0.0.1:7890
Environment=ALL_PROXY=http://127.0.0.1:7890
Environment=NO_PROXY=127.0.0.1,localhost,::1
Environment=NO_COLOR=1
ExecStart=/home/gem/.local/bin/mise exec -- /home/gem/symphony/elixir/bin/symphony --i-understand-that-this-will-be-running-without-the-usual-guardrails {active} --port 4011 --logs-root {state}/logs
Restart=on-failure
RestartSec=15
KillMode=control-group
TimeoutStopSec=90
UMask=0077

[Install]
WantedBy=default.target
''')
    subprocess.run(['systemctl','--user','daemon-reload'],check=True)
    print('Prepared Symphony release '+revision+'; scheduling has not been started.')

if __name__=='__main__':main()
