#!/usr/bin/env python3
"""Install reviewed remote gate bridge and service; does not start scheduling."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT=Path(__file__).resolve().parents[1]
HOME=Path.home()/'.local/share/codexsymphony/remote-gate'


def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    os.umask(0o077);HOME.mkdir(parents=True,exist_ok=True)
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--gate-approval',type=Path,default=Path.home()/'.local/share/codexsymphony/gate-host/approval.json')
    parser.add_argument('--previous-config',type=Path,action='append',default=[],help='retain this reviewed primary snapshot; repeat for active older branches')
    parser.add_argument('--gate-timeout-seconds',type=int,default=1500)
    parser.add_argument('--cache-max-bytes',type=int,default=40*1024**3)
    parser.add_argument('--cache-ttl-seconds',type=int,default=7*86400)
    parser.add_argument('--reuse-identical-tree',action='store_true',help='enable explicit post-merge equivalence receipts after cache warmup')
    parser.add_argument('--dependency-source',type=Path,default=ROOT/'web/angular/node_modules')
    args=parser.parse_args()
    if args.gate_timeout_seconds <= 0:parser.error('--gate-timeout-seconds must be positive')
    if args.cache_max_bytes < 0 or args.cache_ttl_seconds <= 0:parser.error('invalid cache capacity or TTL')
    gate=args.gate_approval
    previous=[]
    for previous_config in args.previous_config:
        prior=json.loads(previous_config.read_text())
        if prior['repository']!='musutrade/CodexSymphony':raise ValueError('previous repository differs')
        candidate={key:prior[key] for key in ('protected_files','gate_approval')}
        json.loads(Path(candidate['gate_approval']).read_text())
        if candidate not in previous:previous.append(candidate)
    approval=json.loads(gate.read_text())
    for name,digest in approval['trusted_files'].items():
        if sha(ROOT/name)!=digest:raise ValueError('gate host input changed: '+name)
    names=['.github/workflows/quality.yml','WORKFLOW.lifecycle.md','web/angular/package.json','web/angular/package-lock.json']
    names += ['tools/install_remote_gate.py','tools/install_symphony_development.py','tools/symphony/trusted_environment.py','tools/symphony/reviewed_gate.py']
    names += [str(p.relative_to(ROOT)) for p in sorted((ROOT/'tools/remote-gate').glob('*.py'))]
    protected={name:sha(ROOT/name) for name in names}
    identity={'protected_files':protected,'gate_approval':str(gate.resolve()),
              'dependency_source':str(args.dependency_source.resolve()),'previous_deployments':previous,'gate_timeout_seconds':args.gate_timeout_seconds,
              'cache_max_bytes':args.cache_max_bytes,'cache_ttl_seconds':args.cache_ttl_seconds,'reuse_identical_tree':args.reuse_identical_tree}
    version=hashlib.sha256(json.dumps(identity,sort_keys=True).encode()).hexdigest()[:16]
    release=HOME/'releases'/version
    if not release.exists():shutil.copytree(ROOT/'tools/remote-gate',release,ignore=shutil.ignore_patterns('__pycache__'))
    for source in (ROOT/'tools/remote-gate').glob('*.py'):
        if sha(source)!=sha(release/source.name):raise ValueError('installed bridge changed')
    config={'schema':'codexsymphony-remote-gate/v1','repository':'musutrade/CodexSymphony','app_id':4867361,
            'app_key':str(Path.home()/'.secrets/my-disposable-bot.2026-09-08.private-key.pem'),
            'state_root':str(HOME),'gate_approval':str(gate.resolve()),'protected_files':protected,
            'dependency_source':str(args.dependency_source.resolve()),'previous_deployments':previous,'gate_timeout_seconds':args.gate_timeout_seconds,
            'cache_max_bytes':args.cache_max_bytes,'cache_ttl_seconds':args.cache_ttl_seconds,'reuse_identical_tree':args.reuse_identical_tree}
    path=HOME/('config-'+version+'.json');body=json.dumps(config,indent=2)+'\n'
    if path.exists() and path.read_text()!=body:raise ValueError('immutable config collision')
    path.write_text(body)
    service=Path.home()/'.config/systemd/user/codexsymphony-remote-gate.service'
    service.write_text(f'''[Unit]
Description=CodexSymphony trusted remote quality gate
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory={HOME}
Environment=PATH=/home/gem/.local/bin:/home/gem/.cargo/bin:/usr/local/bin:/usr/bin:/bin
ExecStart=/usr/bin/python3 {release}/host.py --config {path}
Restart=on-failure
RestartSec=15
KillMode=control-group
TimeoutStopSec=30
UMask=0077

[Install]
WantedBy=default.target
''')
    subprocess.run(['systemctl','--user','daemon-reload'],check=True)
    print(json.dumps({'release':str(release),'config':str(path),'service':str(service),'started':False}))

if __name__=='__main__':main()
