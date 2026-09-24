#!/usr/bin/env python3
"""Apply the repository's controller patch, run required gates, and build.

The service remains stopped. Existing work is preserved; incompatible patches
fail before build. A receipt binds this patch to the resulting binary.
"""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time

ROOT=Path(__file__).resolve().parents[1]
SOURCE=Path.home()/'symphony'
STATE=Path.home()/'.local/share/codexsymphony/symphony'


def main():
    active=subprocess.run(['systemctl','--user','is-active','--quiet','symphony-codexsymphony.service'])
    if active.returncode==0:
        raise ValueError('Stop Symphony before installing the controller')
    patch=ROOT/'tools/symphony/controller-publication.patch'
    applied=subprocess.run(['git','-C',str(SOURCE),'apply','--reverse','--check',str(patch)],capture_output=True)
    if applied.returncode:
        subprocess.run(['git','-C',str(SOURCE),'apply','--check',str(patch)],check=True)
        subprocess.run(['git','-C',str(SOURCE),'apply',str(patch)],check=True)
    mise=Path.home()/'.local/bin/mise'
    subprocess.run([str(mise),'exec','--','make','all'],cwd=SOURCE/'elixir',check=True)
    backup=STATE/'publication-deployment'/str(time.time_ns())
    backup.mkdir(parents=True)
    binary=SOURCE/'elixir/bin/symphony'
    shutil.copy2(binary,backup/'symphony')
    subprocess.run([str(mise),'exec','--','mix','escript.build'],cwd=SOURCE/'elixir',check=True)
    digest=hashlib.sha256(binary.read_bytes()).hexdigest()
    receipt={'binary_sha256':digest,'patch_sha256':hashlib.sha256(patch.read_bytes()).hexdigest(),
             'validation':'make all PASS','backup':str(backup)}
    (STATE/'publication-controller.json').write_text(json.dumps(receipt,indent=2)+'\n')
    previous=STATE/'preservation-controller.json'
    shutil.copy2(previous,backup/previous.name)
    prior=json.loads(previous.read_text());prior['binary_sha256']=digest
    previous.write_text(json.dumps(prior,indent=2)+'\n')
    print(json.dumps(receipt))


if __name__=='__main__':main()
