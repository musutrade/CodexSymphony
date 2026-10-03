#!/usr/bin/env python3
"""Install the complete-validation entrypoint without starting Symphony."""
import hashlib
import argparse
import importlib.util
import json
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[1]
HOME=Path.home()/'.local/share/codexsymphony/publication'
APPROVAL=Path.home()/'.local/share/codexsymphony/gate-host/approval.json'
sys.path.insert(0,str(ROOT/'tools'))
import environment_contract as contract


def approved_module(approval,name):
    """Load a host module only if its bytes are the ones the installed approval pins.

    It is registered under its own name, so the approved layout imports this exact
    fixed_workspace instead of whatever sys.path would resolve.
    """
    path=Path(approval['host_release'])/(name+'.py')
    expected=approval['runtime_files'].get(str(path))
    if expected is None or path.resolve()!=path or hashlib.sha256(path.read_bytes()).hexdigest()!=expected:
        raise ValueError('approved host module missing or changed: '+name)
    spec=importlib.util.spec_from_file_location(name,path)
    module=importlib.util.module_from_spec(spec)
    sys.modules[spec.name]=module
    spec.loader.exec_module(module)
    return module


def lease_binding(approval):
    """Execution leases the approved v3 host holds (bounded_layout.lease), bound to this approval."""
    if approval.get('execution_version')!=3:
        return {'approval':contract.digest(approval),'host_release':approval['host_release'],'leases':[]}
    saved={name:sys.modules.pop(name,None) for name in ('fixed_workspace','bounded_layout')}
    try:
        approved_module(approval,'fixed_workspace')
        layout=approved_module(approval,'bounded_layout')
    finally:
        for name,module in saved.items():
            sys.modules.pop(name,None)
            if module is not None:sys.modules[name]=module
    leases=[layout.VOLUME/'cache/coordination'/'writer.lock',layout.slot()/'writer.lock']
    for lease in leases:
        if lease.resolve()!=lease or not lease.is_file():
            raise ValueError('approved execution lease missing or aliased: '+str(lease))
    return {'approval':contract.digest(approval),'host_release':approval['host_release'],'leases':[str(p) for p in leases]}


def publication_settings(args):
    if args is None or args.validation_contract is None:
        if args is not None and args.remote_config is not None:
            raise ValueError('remote preflight requires a reviewed validation contract')
        return None
    if args.remote_config is None:
        raise ValueError('validation contract requires the installed remote config')
    sys.path.insert(0,str(ROOT/'tools/publication'))
    import validation
    path=args.validation_contract.resolve(strict=True)
    remote=args.remote_config.resolve(strict=True)
    setting={'path':str(path),'sha256':validation.file_digest(path)}
    validation.reviewed(setting,json.loads(APPROVAL.read_text()))
    config=json.loads(remote.read_text())
    if config.get('candidate_registration') is not True or config.get('mode')!='verify-only':
        raise ValueError('remote candidate registration must be deployed first')
    if config.get('verification_contract') != setting:
        raise ValueError('local and remote verification contracts must match')
    return {'contract':setting,'remote_config':str(remote),'remote_config_sha256':validation.file_digest(remote)}


def main(args=None):
    sources={'gate.py':ROOT/'tools/publication/gate.py',
             'validation.py':ROOT/'tools/publication/validation.py',
             'evidence_pins.py':ROOT/'tools/quality-host/evidence_pins.py',
             'environment_contract.py':ROOT/'tools/environment_contract.py',
             'evidence_ledger.py':ROOT/'tools/evidence_ledger.py',
             'environment.lock.json':ROOT/'environment.lock.json'}
    binding=(json.dumps(lease_binding(json.loads(APPROVAL.read_text())),indent=2,sort_keys=True)+'\n').encode()
    files={name:hashlib.sha256(path.read_bytes()).hexdigest() for name,path in sources.items()}
    files['host-leases.json']=hashlib.sha256(binding).hexdigest()
    settings=publication_settings(args)
    settings_bytes=(json.dumps(settings,indent=2,sort_keys=True)+'\n').encode() if settings else None
    if settings_bytes:
        files['publication-settings.json']=hashlib.sha256(settings_bytes).hexdigest()
    revision=hashlib.sha256(json.dumps(files,sort_keys=True).encode()).hexdigest()[:16]
    release=HOME/'releases'/revision
    release.mkdir(parents=True,exist_ok=True)
    contents={name:source.read_bytes() for name,source in sources.items()}|{'host-leases.json':binding}
    if settings_bytes:
        contents['publication-settings.json']=settings_bytes
    for name,data in contents.items():
        target=release/name
        if target.exists() and target.read_bytes()!=data:
            raise ValueError('installed publication release differs')
        target.write_bytes(data)
    (release/'installed-files.json').write_text(json.dumps({str(release/name):digest for name,digest in files.items()},indent=2)+'\n')
    guard=HOME/'guard'
    guard.write_text('#!/bin/sh\nexec /usr/bin/python3 '+str(release/'gate.py')+' "$@"\n')
    guard.chmod(0o700)
    print(json.dumps({'guard':str(guard),'release':str(release),'scheduling_started':False}))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--validation-contract',type=Path)
    parser.add_argument('--remote-config',type=Path)
    main(parser.parse_args())
