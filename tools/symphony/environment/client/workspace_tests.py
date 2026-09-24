"""Run the required full Rust suite against a fresh disposable test fixture."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from dbctl import request
import environment_contract as contract


def main():
    policy=contract.load(Path.cwd())
    expected='codex-cli '+policy['tools']['codex']
    actual=subprocess.check_output(['codex','--version'],text=True,timeout=10).strip()
    if actual!=expected:
        raise RuntimeError(f'Codex version mismatch: expected {expected}, got {actual}')
    fixture=request('recreate','test')
    if not fixture['running'] or fixture['memory_limit_bytes'] != policy['postgres']['test']['memory']:
        raise RuntimeError('Disposable PostgreSQL fixture is not ready with its memory policy')
    temporary=tempfile.mkdtemp(prefix='cs-',dir='/tmp')
    print(json.dumps({'test_fixture':fixture,'tmpdir':temporary}),flush=True)
    env=os.environ.copy()
    env.update(contract.test_environment(policy))
    print(json.dumps(contract.fingerprint(Path.cwd(),env)),flush=True)
    env['TMPDIR']=temporary
    result=subprocess.call([sys.executable,str(Path(__file__).with_name('run.py')),
                            'cargo','test','--workspace','--locked'],env=env)
    print(json.dumps({'exit_code':result,'test_fixture_after':request('status','test')}),
          flush=True)
    return result


if __name__=='__main__':
    sys.exit(main())
