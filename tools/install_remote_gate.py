#!/usr/bin/env python3
"""Install reviewed remote gate bridge and service; does not start scheduling."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
sys.path.insert(0,str(Path(__file__).resolve().parent/'symphony'))
sys.path.insert(0,str(Path(__file__).resolve().parent/'remote-gate'))
from check_deployment import check_commands

ROOT=Path(__file__).resolve().parents[1]
HOME=Path.home()/'.local/share/codexsymphony/remote-gate'
SERVICE=Path.home()/'.config/systemd/user/codexsymphony-remote-gate.service'
# Beside the bounded Gate runs it protects (evidence_pins.pinnable); the storage deployment is the records budget authority.
PINS=Path('/mnt/dev-ssd/codexsymphony-bounded/data/evidence/pins')
STORAGE=Path.home()/'.local/share/codexsymphony/storage-maintenance/deployment.json'


def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def arguments():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--gate-approval',type=Path,default=Path.home()/'.local/share/codexsymphony/gate-host/approval.json')
    parser.add_argument('--previous-config',type=Path,action='append',default=[],help='retain this reviewed primary snapshot; repeat for active older branches')
    parser.add_argument('--gate-timeout-seconds',type=int,default=1500)
    parser.add_argument('--cache-max-bytes',type=int,default=40*1024**3)
    parser.add_argument('--cache-ttl-seconds',type=int,default=7*86400)
    parser.add_argument('--reuse-identical-tree',action='store_true',help='enable explicit post-merge equivalence receipts after cache warmup')
    parser.add_argument('--dependency-source',type=Path,default=ROOT/'web/angular/node_modules')
    parser.add_argument('--verify-only',action='store_true',help='admit trusted local complete PASS evidence; never run a Gate')
    parser.add_argument('--publication-ledger',type=Path,default=Path.home()/'.local/share/codexsymphony/publication/ledger')
    parser.add_argument('--equivalence-audit',type=Path,help='reviewed tree-equivalence audit for this exact approval')
    parser.add_argument('--candidate-registration',action='store_true',help='enable bounded operator registration without per-candidate installation')
    parser.add_argument('--validation-contract',type=Path,help='independently reviewed complete verification input contract')
    parser.add_argument('--audit-window-seconds',type=int,help='verify-only: retention of a main publication after merge (required)')
    parser.add_argument('--pin-ttl-seconds',type=int,help='verify-only: retention of a PR publication awaiting merge (required)')
    args=parser.parse_args()
    if args.gate_timeout_seconds <= 0:parser.error('--gate-timeout-seconds must be positive')
    if args.cache_max_bytes < 0 or args.cache_ttl_seconds <= 0:parser.error('invalid cache capacity or TTL')
    error=verification_error(args)
    if error:parser.error(error)
    return args


def verification_error(args):
    if getattr(args,'validation_contract',None) and not getattr(args,'candidate_registration',False):
        return '--validation-contract requires --candidate-registration'
    if getattr(args,'candidate_registration',False) and (not args.verify_only or args.equivalence_audit):
        return '--candidate-registration requires --verify-only and replaces --equivalence-audit'
    if args.equivalence_audit and not args.verify_only:
        return '--equivalence-audit requires --verify-only'
    error=window_error(args)
    if error or not args.verify_only:
        return error
    ledger=args.publication_ledger
    if not ledger.is_absolute() or ledger.resolve()!=ledger or not (ledger/'lock').is_file():
        return '--publication-ledger must be the canonical installed ledger'
    return None


def window_error(args):
    """Retention windows are explicit for verify-only and absent otherwise; there are no implicit defaults."""
    windows=(args.audit_window_seconds,args.pin_ttl_seconds)
    if not args.verify_only:
        return '--audit-window-seconds and --pin-ttl-seconds require --verify-only' if windows!=(None,None) else None
    if None in windows:
        return '--verify-only requires explicit --audit-window-seconds and --pin-ttl-seconds'
    return window_bound(args.audit_window_seconds,args.pin_ttl_seconds,retention_seconds())


def retention_seconds():
    """The installed records retention (compact_gate_evidence.maintain hours), read without executing it."""
    import evidence_admission as admission
    return admission.pins.retention_hours(STORAGE)*3600


def window_bound(window,ttl,limit):
    """A run stays pinned at most ttl (PR awaiting merge), then the merge hand-off (window), then main's window."""
    if not 0<window<=ttl or ttl+2*window>limit:
        return f'retention windows must satisfy 0 < audit window <= pin TTL and pin TTL + 2 * audit window <= {limit} installed retention seconds'
    return None


def equivalence(path,approval,ledger):
    """Install a tree-equivalence audit only if admission accepts it for this approval and its one ledger PASS."""
    import evidence_admission as admission
    path=path.resolve(strict=True)
    rule={'rule':admission.TREE_EQUIVALENCE,'audit':str(path),'audit_sha256':sha(path)}
    # The same checks the host runs at admission; installation cannot accept a weaker audit.
    audit=admission.audit_document(rule,approval)
    record=admission.ledger.load_record(ledger,audit['validation_id'])|{'validation_id':audit['validation_id']}
    admission.check_binding(audit,record,record['inputs']['tree'])
    return rule


def verification(args,approval):
    """The deployment mode is always explicit; a config without it never runs."""
    if not args.verify_only:return {'mode':'execute'}
    value={'mode':'verify-only','publication_ledger':str(args.publication_ledger),'pins':str(PINS),
           'storage_deployment':str(STORAGE),'audit_window_seconds':args.audit_window_seconds,
           'pin_ttl_seconds':args.pin_ttl_seconds}
    if args.equivalence_audit:value['equivalence']=equivalence(args.equivalence_audit,approval,args.publication_ledger)
    if getattr(args,'candidate_registration',False):value['candidate_registration']=True
    if getattr(args,'validation_contract',None):
        import evidence_admission as admission
        verifier=admission.load_verification()
        path=args.validation_contract.resolve(strict=True)
        setting={'path':str(path),'sha256':sha(path)}
        verifier.reviewed(setting,approval)
        value['verification_contract']=setting
    return value


def previous_deployments(paths):
    previous=[]
    for previous_config in paths:
        prior=json.loads(previous_config.read_text())
        if prior['repository']!='musutrade/CodexSymphony':raise ValueError('previous repository differs')
        candidate={key:prior[key] for key in ('protected_files','gate_approval')}
        json.loads(Path(candidate['gate_approval']).read_text())
        if candidate not in previous:previous.append(candidate)
    return previous


def protected_files(approval):
    for name,digest in approval['trusted_files'].items():
        if sha(ROOT/name)!=digest:raise ValueError('gate host input changed: '+name)
    names=['.github/workflows/quality.yml','WORKFLOW.lifecycle.md','web/angular/package.json','web/angular/package-lock.json']
    names += ['tools/install_remote_gate.py','tools/install_symphony_development.py','tools/install_sccache.py','tools/symphony/trusted_environment.py','tools/symphony/reviewed_gate.py','tools/symphony/check_deployment.py']
    names += [str(p.relative_to(ROOT)) for p in sorted((ROOT/'tools/remote-gate').glob('*.py'))]+['tools/evidence_ledger.py','tools/quality-host/evidence_pins.py','tools/publication/validation.py']
    return {name:sha(ROOT/name) for name in names}


def bridge_sources():
    return [*(ROOT/'tools/remote-gate').glob('*.py'),ROOT/'tools/evidence_ledger.py',ROOT/'tools/quality-host/evidence_pins.py',ROOT/'tools/publication/validation.py']


def install_release(version):
    release=HOME/'releases'/version
    if not release.exists():
        shutil.copytree(ROOT/'tools/remote-gate',release,ignore=shutil.ignore_patterns('__pycache__'))
        shutil.copyfile(ROOT/'tools/evidence_ledger.py',release/'evidence_ledger.py')
        shutil.copyfile(ROOT/'tools/quality-host/evidence_pins.py',release/'evidence_pins.py')
        shutil.copyfile(ROOT/'tools/publication/validation.py',release/'validation.py')
    for source in bridge_sources():
        if sha(source)!=sha(release/source.name):raise ValueError('installed bridge changed')
    return release


def install_pins(value):
    """The pin root and its lock are installed once; pins never create them at runtime."""
    if value.get('mode')!='verify-only':return
    root=Path(value['pins'])
    if root.parent.resolve()!=root.parent or not (root.parent/'gate').is_dir():
        raise ValueError('pin root must lie beside the bounded Gate runs')
    root.mkdir(mode=0o700,exist_ok=True)
    if root.is_symlink() or root.resolve()!=root:raise ValueError('pin root must be canonical')
    lock=root/'lock'
    if lock.is_symlink():raise ValueError('pin lock must be a regular file')
    lock.touch(mode=0o600)
    if value.get('candidate_registration') is True:
        registration=root/'registration.lock'
        if registration.is_symlink() or (registration.exists() and not registration.is_file()):
            raise ValueError('candidate registration lock must be a regular file')
        registration.touch(mode=0o600)


def install_service(release,path):
    service=SERVICE
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
    check_commands({'codexsymphony-remote-gate.service':f'/usr/bin/python3 {release}/host.py --config {path}'})
    return service


def main():
    os.umask(0o077);HOME.mkdir(parents=True,exist_ok=True)
    args=arguments()
    gate=args.gate_approval
    previous=previous_deployments(args.previous_config)
    approval=json.loads(gate.read_text())
    protected=protected_files(approval)
    settings={'dependency_source':str(args.dependency_source.resolve()),'previous_deployments':previous,'gate_timeout_seconds':args.gate_timeout_seconds,
              'cache_max_bytes':args.cache_max_bytes,'cache_ttl_seconds':args.cache_ttl_seconds,'reuse_identical_tree':args.reuse_identical_tree}|verification(args,approval)
    identity={'protected_files':protected,'gate_approval':str(gate.resolve())}|settings
    version=hashlib.sha256(json.dumps(identity,sort_keys=True).encode()).hexdigest()[:16]
    release=install_release(version)
    config={'schema':'codexsymphony-remote-gate/v1','repository':'musutrade/CodexSymphony','app_id':4867361,
            'app_key':str(Path.home()/'.secrets/my-disposable-bot.2026-09-08.private-key.pem'),
            'state_root':str(HOME),'gate_approval':str(gate.resolve()),'protected_files':protected}|settings
    path=HOME/('config-'+version+'.json');body=json.dumps(config,indent=2)+'\n'
    if path.exists() and path.read_text()!=body:raise ValueError('immutable config collision')
    path.write_text(body)
    install_pins(settings)
    service=install_service(release,path)
    print(json.dumps({'release':str(release),'config':str(path),'service':str(service),'started':False}))

if __name__=='__main__':main()
