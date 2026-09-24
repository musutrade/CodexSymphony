#!/usr/bin/env python3
"""Install the complete-validation entrypoint without starting Symphony."""
import hashlib
import json
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
HOME=Path.home()/'.local/share/codexsymphony/publication'


def main():
    sources={'gate.py':ROOT/'tools/publication/gate.py',
             'environment_contract.py':ROOT/'tools/environment_contract.py',
             'environment.lock.json':ROOT/'environment.lock.json'}
    files={name:hashlib.sha256(path.read_bytes()).hexdigest() for name,path in sources.items()}
    revision=hashlib.sha256(json.dumps(files,sort_keys=True).encode()).hexdigest()[:16]
    release=HOME/'releases'/revision
    release.mkdir(parents=True,exist_ok=True)
    for name,source in sources.items():
        target=release/name
        if target.exists() and target.read_bytes()!=source.read_bytes():
            raise ValueError('installed publication release differs')
        target.write_bytes(source.read_bytes())
    (release/'installed-files.json').write_text(json.dumps({str(release/name):digest for name,digest in files.items()},indent=2)+'\n')
    guard=HOME/'guard'
    guard.write_text('#!/bin/sh\nexec /usr/bin/python3 '+str(release/'gate.py')+' "$@"\n')
    guard.chmod(0o700)
    print(json.dumps({'guard':str(guard),'release':str(release),'scheduling_started':False}))


if __name__=='__main__':main()
