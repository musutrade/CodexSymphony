"""Execute the complete declared gate with read-only code and trust inputs."""
import os
from pathlib import Path
import shutil
import subprocess
from capture import PLUGIN_ROOT, write, load
import bounded_layout as layout
import database_pool
from isolation import command
from replay import broker
from signing import CORE

def verify(run, repository, root, profile="ci"):
    layout.ensure(repository)
    target = layout.target('normal')
    reports=root/'.harness-gate/reports'
    writable=[reports,target]
    for name in ('web/angular/.angular','web/angular/dist'):
        p=root/name;p.mkdir(parents=True,exist_ok=True);writable.append(p)
    modules=root/'web/angular/node_modules';modules.mkdir(exist_ok=True)
    paths=[str(CORE.parent),str(Path(shutil.which('harness-gate-rust-collector')).resolve().parent)]
    environment={'PATH':':'.join(paths+['/opt/codex','/home/gem/.cargo/bin','/usr/local/bin','/usr/bin','/bin'])}
    if (run/'test-capture.json').exists():
        environment['HARNESS_GATE_TEST_RECEIPT']=str(run/'test-capture.json')
    baseline=Path(load(run/'requests.json')['frontend-api']['parameters']['receipt']['baseline']['path'])
    container,url=database_pool.acquire(run,repository=repository)
    try:
        environment['TEST_DATABASE_URL']=url
        args=command(['python3',root/'tools/gate.py','verify','--profile',profile,'--all'],run=run,repository=root,plugins=PLUGIN_ROOT,readonly=[run,baseline],writable=writable,
                     mounts=[(repository/'web/angular/node_modules',modules)],environment=environment,cwd=root,compiler_target=target)
        ledger=Path('/home/gem/.local/share/codexsymphony/gate-host/nonces')
        with broker(run,ledger,workspace=root), (run/'verify.stdout').open('wb') as out,(run/'verify.stderr').open('wb') as err:
            result=subprocess.run(args,stdout=out,stderr=err)
        write(run/'verify-result.json',{'exit':result.returncode,'source_root':str(root),'scope':'complete-local-isolated-gate'})
        return result.returncode
    finally:
        database_pool.release(container)
