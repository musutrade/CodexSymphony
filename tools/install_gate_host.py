#!/usr/bin/env python3
"""Install the locally reviewed host outside the repository and pin its inputs.

This is an administrator operation, never a step executed by an untrusted PR.
A prior complete local gate acceptance is required. Old approvals are retained.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import sys

ROOT=Path(__file__).resolve().parents[1]
HOME=Path.home()/'.local/share/codexsymphony/gate-host'

def main():
    previous=json.loads((HOME/'approval.json').read_text())
    source=ROOT/'tools/quality-host'
    files={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(source.glob('*.py'))}
    version=hashlib.sha256(json.dumps(files,sort_keys=True).encode()).hexdigest()[:16]
    target=HOME/'releases'/version
    if not target.exists(): shutil.copytree(source,target,ignore=shutil.ignore_patterns('__pycache__'))
    for name,digest in files.items():
        if hashlib.sha256((target/name).read_bytes()).hexdigest()!=digest: raise ValueError('installed host version differs')
    sys.path.insert(0,str(target))
    spec=importlib.util.spec_from_file_location('installed_gate_host',target/'run.py')
    host=importlib.util.module_from_spec(spec);spec.loader.exec_module(host)
    if host.configuration_files(ROOT)!=previous['config_files']: raise ValueError('policy differs from last complete acceptance')
    approval=previous|{'runtime_files':host.runtime_pins(),'trusted_files':host.trusted_files(ROOT),'host_release':str(target)}
    approvals=HOME/'approvals';approvals.mkdir(exist_ok=True)
    path=approvals/(version+'.json')
    content=json.dumps(approval,indent=2)+'\n'
    if path.exists() and path.read_text()!=content: raise ValueError('approval version collision')
    path.write_text(content)
    active=HOME/'approval.json'
    if not active.is_symlink(): active.rename(HOME/'approval-bootstrap.json')
    pending=HOME/'approval.new';pending.symlink_to(path);pending.replace(active)
    executable=HOME/'run'
    executable.write_text('#!/bin/sh\nexec /usr/bin/python3 '+str(target/'run.py')+' --approval '+str(path)+' "$@"\n')
    executable.chmod(0o700)
    print(json.dumps({'host':str(executable),'approval':str(path),'release':str(target)}))

if __name__=='__main__':main()
