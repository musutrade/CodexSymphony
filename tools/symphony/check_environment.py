#!/usr/bin/env python3
"""Check host and actual development namespace against the same contract."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import environment_contract as contract


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace',type=Path,required=True)
    args=parser.parse_args()
    root=args.workspace.resolve(strict=True)
    value=contract.load(root)
    host=contract.fingerprint(root,dict(os.environ,PATH=contract.tool_path(value),**contract.test_environment(value)))
    entry=Path.home()/'.local/share/codexsymphony/symphony/codex-trusted'
    # Use the installed wrapper selected by the service; no model call.
    script="import json,sys;sys.path.insert(0,'/opt/symphony-env');import environment_contract as c;from pathlib import Path;print(json.dumps(c.fingerprint(Path.cwd())))"
    import shlex
    wrapper=shlex.split(entry.read_text().splitlines()[1])[2]
    sys.path.insert(0,str(Path(wrapper).parent))
    spec=importlib.util.spec_from_file_location('installed_environment',wrapper)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    prior=Path.cwd()
    try:
        os.chdir(root)
        command=module.command(['python3','-c',script])
        actual=json.loads(subprocess.check_output(command,text=True))
    finally:
        os.chdir(prior)
    if host['fingerprint']!=actual['fingerprint']:
        raise ValueError('environment drift: host and Symphony namespace fingerprints differ')
    print(json.dumps({'status':'PASS','host':host,'symphony':actual},indent=2))


if __name__=='__main__':main()
