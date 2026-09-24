#!/usr/bin/env python3
"""Install the reviewed local Elixir development workflow; scheduling starts separately."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

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
    sources={path.relative_to(directory):path.read_bytes() for path in directory.rglob('*') if path.is_file() and '__pycache__' not in path.parts}
    return sources

def check_environment_resources(state):
    required=['client/pg/psql','client/pg/ld-musl-x86_64.so.1','arc-admin/SOURCE.json','npm-cache-seed/_cacache']
    missing=[name for name in required if not (state/'environment-template'/name).exists()]
    if missing:
        raise ValueError('Approved host environment resources are missing: '+', '.join(missing)+
                         '. Provision environment-template first; existing workflow has not been replaced.')

def check_preservation_controller(state):
    marker=state/'preservation-controller.json'
    binary=HOME/'symphony/elixir/bin/symphony'
    patch=ROOT/'tools/symphony/controller-preservation.patch'
    if not marker.is_file():
        raise ValueError('Required preservation controller is not installed; existing workflow has not been replaced')
    receipt=json.loads(marker.read_text())
    if (receipt.get('binary_sha256')!=hashlib.sha256(binary.read_bytes()).hexdigest()
        or receipt.get('patch_sha256')!=hashlib.sha256(patch.read_bytes()).hexdigest()):
        raise ValueError('Preservation controller receipt mismatch; existing workflow has not been replaced')

def check_publication_controller(state):
    marker=state/'publication-controller.json'
    binary=HOME/'symphony/elixir/bin/symphony'
    patch=ROOT/'tools/symphony/controller-publication.patch'
    if not marker.is_file():
        raise ValueError('Publication guard controller must be installed before enabling this workflow')
    receipt=json.loads(marker.read_text())
    if receipt.get('binary_sha256')!=hashlib.sha256(binary.read_bytes()).hexdigest() or receipt.get('patch_sha256')!=hashlib.sha256(patch.read_bytes()).hexdigest():
        raise ValueError('Publication guard controller receipt mismatch')

def main():
    os.umask(0o077)
    state=BASE/'symphony';state.mkdir(parents=True,exist_ok=True)
    workflow=(ROOT/'WORKFLOW.lifecycle.md').read_bytes()
    wrapper=(ROOT/'tools/symphony/trusted_environment.py').read_bytes()
    reviewed_gate=(ROOT/'tools/symphony/reviewed_gate.py').read_bytes()
    provision=(ROOT/'tools/symphony/provision_issue_environment.py').read_bytes()
    sources=environment_sources()
    shared={'environment_contract.py':(ROOT/'tools/environment_contract.py').read_bytes(),
            'environment.lock.json':(ROOT/'environment.lock.json').read_bytes()}
    sources.update({Path('client')/name:data for name,data in shared.items()})
    check_environment_resources(state)
    check_preservation_controller(state)
    check_publication_controller(state)
    active=state/'WORKFLOW.lifecycle.md'
    routed=routed_workflow(workflow,active.read_bytes() if active.exists() else None)
    revision=hashlib.sha256(workflow+wrapper+reviewed_gate+provision+b''.join(sources[key] for key in sorted(sources))).hexdigest()[:16]
    release=state/'releases'/revision;release.mkdir(parents=True,exist_ok=True)
    for name,data in [('WORKFLOW.lifecycle.md',workflow),('trusted_environment.py',wrapper),('reviewed_gate.py',reviewed_gate)]:
        path=release/name
        if path.exists() and path.read_bytes()!=data:raise ValueError('immutable release differs')
        path.write_bytes(data)
    for name,data in shared.items():
        (state/name).write_bytes(data)
        (release/name).write_bytes(data)
    (state/'reviewed_gate.py').write_bytes(reviewed_gate)
    # Stable journal path is retained across workflow releases.
    (state/'provision_issue_environment.py').write_bytes(provision)
    (state/'preserve_workspace.py').write_bytes((ROOT/'tools/symphony/preserve_workspace.py').read_bytes())
    # Retire superseded executable entrypoints while retaining their artifacts.
    retired=state/'retired-execution-entries'/revision
    obsolete=['execution_readiness.py','product_preparation_acceptance.py',
              'runtime_command_readiness.py','runtime_product_acceptance.py',
              'backend_test_acceptance.py','reviewed-runtime',
              'client/execution_readiness.py','client/product_preparation_acceptance.py',
              'client/runtime_command_readiness.py','client/runtime_product_acceptance.py',
              'client/runtime_smoke.py','client/backend_tests.py','client/reviewed-preparation']
    for directory in [state/'environment-template', *state.glob('gh*-environment')]:
        for name in obsolete:
            source=directory/name
            if source.exists():
                target=retired/directory.name/name
                target.parent.mkdir(parents=True,exist_ok=True)
                source.rename(target)
    legacy=state/'codex-sandbox'
    if legacy.exists():
        retired.mkdir(parents=True,exist_ok=True)
        legacy.rename(retired/'codex-sandbox')
    sources[Path('managed-files.json')]=json.dumps([str(name) for name in sources if name.suffix=='.py' or name.parent==Path('client/bin')]).encode()
    for name,data in sources.items():
        path=state/'environment-template'/name;path.parent.mkdir(parents=True,exist_ok=True)
        path.write_bytes(data)
        if name==Path('client/bin/psql'):path.chmod(0o755)
    if active.exists():
        previous = active.read_bytes()
        match = re.search(rb'\n## (?:GH-[0-9]+ operator recovery:|Operator recovery:|Operator deployment completed|Supported release completed)', previous)
        if match and previous[match.start():] not in routed:
            routed += previous[match.start():]
    managed={str(state/name):hashlib.sha256((state/name).read_bytes()).hexdigest() for name in ['reviewed_gate.py','provision_issue_environment.py','environment_contract.py','environment.lock.json']}
    managed.update({str(release/name):hashlib.sha256((release/name).read_bytes()).hexdigest() for name in ['trusted_environment.py','reviewed_gate.py','environment_contract.py','environment.lock.json']})
    managed.update({str(state/'environment-template'/name):hashlib.sha256((state/'environment-template'/name).read_bytes()).hexdigest() for name in sources})
    for directory in [state,release]: (directory/'installed-files.json').write_text(json.dumps(managed,indent=2)+'\n')
    active.write_bytes(routed)
    command=state/'codex-trusted';command.write_text('#!/bin/sh\nexec /usr/bin/python3 '+str(release/'trusted_environment.py')+' "$@"\n');command.chmod(0o700)
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
Environment=HTTP_PROXY=http://192.168.0.26:10809
Environment=HTTPS_PROXY=http://192.168.0.26:10809
Environment=ALL_PROXY=http://192.168.0.26:10809
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
    subprocess.run([sys.executable,str(ROOT/'tools/install_publication_gate.py')],check=True)
    subprocess.run(['systemctl','--user','daemon-reload'],check=True)
    subprocess.run([sys.executable, str(ROOT/'tools/install_symphony_cleanup.py')], check=True)
    subprocess.run([sys.executable, str(ROOT/'tools/install_symphony_operator.py')], check=True)
    print('Prepared Symphony release '+revision+'; scheduling has not been started.')

if __name__=='__main__':main()
