"""Fixed host probe through the actual pinned Agent launcher. No arbitrary commands."""
import json,time,uuid
from pathlib import Path
from preflight import execute,ROOT

COMMAND = r'''
import os,pathlib,subprocess,tempfile,errno,uuid,json
assert os.getuid()==1000
core=subprocess.check_output(['harness-gate','--version'],text=True).strip()
assert core=='harness-gate 0.4.5',core
codex=subprocess.check_output(['codex','--version'],text=True).strip()
assert codex=='codex-cli 0.154.0',codex
pathlib.Path('target').mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(dir='target') as d:
 pathlib.Path(d,'probe').write_text('writable')
canary=pathlib.Path('.git')/('readiness-'+uuid.uuid4().hex)
try:
 fd=os.open(canary,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
except OSError as e:
 assert e.errno in (errno.EROFS,errno.EACCES,errno.EPERM)
else:
 os.close(fd);canary.unlink();raise RuntimeError('Git unexpectedly writable')
for hidden in ['/home/gem/.secrets','/home/gem/.local/share/codexsymphony/gate-host/approval.json']:
 assert not pathlib.Path(hidden).exists(),hidden
print('1000\n'+core)
print(json.dumps({'codex':codex,'workspace':str(pathlib.Path.cwd()),'target_writable':True,'git_readonly':True,'host_credentials_hidden':True}))
'''


def probe():
    proof=execute(['python3','-c',COMMAND],timeout=30,readiness=True)
    result=proof['command_exec']['result']
    if result['exitCode']!=0:
        raise RuntimeError('Execution readiness failed: '+result['stderr'][-2000:])
    network=(proof['requirements'].get('requirements') or {}).get('network')
    if not network or not network.get('enabled'):
        raise RuntimeError('Managed network requirements unavailable')
    proof['requirements']={'requirements':{'network':network}}
    proof.update(ok=True,workspace=str(ROOT),sample_id=uuid.uuid4().hex,checked_at=time.time())
    destination=Path(__file__).parent/'client/execution-readiness.json'
    temporary=destination.with_suffix('.new');temporary.write_text(json.dumps(proof,indent=2)+'\n');temporary.replace(destination)
    return proof
