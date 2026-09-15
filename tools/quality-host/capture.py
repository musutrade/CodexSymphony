"""Capture three real producers before any signing key exists."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time
import urllib.error
import urllib.request
from isolation import command

PLUGIN_ROOT = Path('/home/gem/.local/share/harness-gate')
RUST = PLUGIN_ROOT / 'rust-source/0.1.0-rc.1'
TS = PLUGIN_ROOT / 'typescript/0.1.0-rc.4/node_modules/@harness-gate/typescript-collector'
HTTP = PLUGIN_ROOT / 'http-contract/0.1.0-rc.3/node_modules/@harness-gate/http-json-contract-collector'

def sha(data): return hashlib.sha256(data).hexdigest()
def write(path, data): path.write_text(json.dumps(data, indent=2) + '\n')
def load(path): return json.loads(path.read_text())

def run_logged(run, label, args, **kwargs):
    with (run / (label + '.stdout')).open('wb') as out, (run / (label + '.stderr')).open('wb') as err:
        subprocess.run(args, stdout=out, stderr=err, check=True, **kwargs)

def node(plugin, operation, request):
    script = "const p=require(process.argv[1]);const q=JSON.parse(require('fs').readFileSync(0,'utf8'));console.log(JSON.stringify(" + operation + "));"
    return json.loads(subprocess.check_output(['node','-e',script,str(plugin / 'protocol.cjs')],input=json.dumps(request),text=True))

def database(run):
    name = 'codexsymphony-gate-' + run.name[-16:]
    subprocess.run(['docker','run','--detach','--name',name,'--publish','127.0.0.1::5432',
                    '--env','POSTGRES_DB=gate_test','--env','POSTGRES_USER=gate_test','--env','POSTGRES_PASSWORD=gate_test',
                    '--tmpfs','/var/lib/postgresql/data','postgres:16-alpine'],check=True,capture_output=True)
    for _ in range(60):
        probe=subprocess.run(['docker','exec',name,'pg_isready','-U','gate_test','-d','gate_test'],capture_output=True)
        if probe.returncode==0: break
        time.sleep(.5)
    else: raise RuntimeError('test database did not become ready')
    port=subprocess.check_output(['docker','port',name,'5432/tcp'],text=True).strip().split(':')[-1]
    return name, f'postgres://gate_test:gate_test@127.0.0.1:{port}/gate_test'

def capture_http(run, repository, container, url):
    binary=run/'target/debug/codexsymphony-server'
    args=command(['cargo','build','--locked','--bin','codexsymphony-server'],run=run,repository=repository,plugins=PLUGIN_ROOT,writable=[run/'probes',run/'target'],environment={'TEST_DATABASE_URL':url})
    run_logged(run,'http-build',args)
    args=command([binary],run=run,repository=repository,plugins=PLUGIN_ROOT,readonly=[binary],environment={'DATABASE_URL':url,'BIND_ADDRESS':'127.0.0.1:0','RUST_LOG':'info'})
    # Server stdout contains the actual listener address, never a guessed port.
    server=subprocess.Popen(args,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
    try:
        import selectors
        selector=selectors.DefaultSelector();selector.register(server.stdout,selectors.EVENT_READ)
        address=None; deadline=time.monotonic()+20
        while time.monotonic()<deadline:
            if selector.select(.5):
                line=server.stdout.readline()
                if 'API listening at http://' in line:
                    address=line.split('API listening at http://',1)[1].strip();break
            if server.poll() is not None: raise RuntimeError('server exited before readiness: '+server.stderr.read())
        if address is None: raise RuntimeError('server readiness timeout')
        observations=[]
        for status in (200,503):
            if status==503: subprocess.run(['docker','stop','--time','1',container],check=True,capture_output=True)
            request=urllib.request.Request('http://'+address+'/api/health')
            try: response=urllib.request.urlopen(request,timeout=8)
            except urllib.error.HTTPError as error: response=error
            with response:
                if response.status!=status: raise RuntimeError(f'expected HTTP {status}, got {response.status}')
                observations.append({'method':'GET','path':'/api/health','status':response.status,'content_type':response.headers['Content-Type'],'body':json.load(response)})
        write(run/'http-observations.json',observations)
        return observations,sha(binary.read_bytes())
    finally:
        os.killpg(server.pid,signal.SIGTERM)
        try: server.wait(timeout=5)
        except subprocess.TimeoutExpired: os.killpg(server.pid,signal.SIGKILL);server.wait()

def captures(run, repository, root, context, baseline):
    for directory in ('probes','target'): (run/directory).mkdir()
    container,url=database(run)
    try:
        args=command(['python3',RUST/'capture.py','--repository',repository,'--output',run/'probes/backend','--target-dir',run/'target','--source-root','apps/server/src','--input','Cargo.toml','--input','Cargo.lock','--input','apps','--input','migrations','--manifest','apps/server/Cargo.toml','--test','health','--test','startup'],run=run,repository=repository,plugins=PLUGIN_ROOT,writable=[run/'probes',run/'target'],environment={'TEST_DATABASE_URL':url})
        run_logged(run,'backend-capture',args)
        args=command(['node',repository/'web/angular/tools/probe-typescript-risk.cjs'],run=run,repository=repository,plugins=PLUGIN_ROOT,writable=[run/'probes'],environment={'HARNESS_GATE_TYPESCRIPT_PLUGIN':str(TS)})
        run_logged(run,'frontend-capture',args)
        observations,binary_hash=capture_http(run,repository,container,url)
    finally:
        subprocess.run(['docker','rm','--force',container],check=True,capture_output=True)
    runtime=root/'.harness-gate/runtime'; runtime.mkdir(parents=True,exist_ok=True)
    output=root/'.harness-gate/reports/evidence';output.mkdir(parents=True,exist_ok=True)
    backend=load(run/'probes/backend/bundle.json')['request']
    backend.update(workspace_root=str(root),output_root=str(output),context=context)
    backend['parameters']['receipt']['context']=context
    # Source hashes are checked against the combined immutable checkout by the plugin.
    frontend_dir=next((run/'tmp').glob('codexsymphony-ts-risk-*'))
    frontend=load(frontend_dir/'collector-bundle.json')['request']
    receipt=frontend['parameters']['receipt']
    raw=load(frontend_dir/'coverage.json'); prefix=receipt['coverage_root']+'/'
    rebased={}
    for name,value in raw.items():
        if not name.startswith(prefix): raise ValueError('unexpected captured TypeScript path')
        key='/harness-capture/web/angular/'+name[len(prefix):]
        value['path']=key;rebased[key]=value
    coverage=runtime/'frontend-coverage.json';write(coverage,rebased)
    frontend.update(workspace_root=str(root),output_root=str(output),context=context)
    p=frontend['parameters'];p.update(source_root='web/angular/src',coverage='.harness-gate/runtime/frontend-coverage.json',artifact_subdir='frontend')
    p['exclude']=['web/angular/'+name for name in p['exclude']]
    receipt['coverage_root']='/harness-capture';receipt['coverage_sha256']=sha(coverage.read_bytes())
    receipt['inputs']={'web/angular/'+name:digest for name,digest in receipt['inputs'].items()}
    receipt['pipeline']['files']={'web/angular/'+name:digest for name,digest in receipt['pipeline']['files'].items()}
    receipt['pipeline']['files']['tools/quality-host/capture.py']=sha((root/'tools/quality-host/capture.py').read_bytes())
    receipt['pipeline']['tools']['path-rebase']='original-app-to-repository-prefix/v1'
    discovery=node(TS,'p.discover(q)',frontend);p['subjects']=discovery['subjects'];receipt['sources']=[{k:f[k] for k in ('path','sha256')} for f in discovery['sources']]
    receipt['request']=node(TS,'p.binding(q)',frontend)
    observation_path=runtime/'http-observations.json';write(observation_path,observations)
    contract={'schema':'harness-collector-request/v1','project':'codexsymphony','component':'backend','collector':{'name':'http-json-contract','version':'0.1.0-rc.3'},'context':context,'workspace_root':str(root),'output_root':str(output),'requested_capabilities':['contract.breaking_changes','contract.client_drift','contract.compatible'],
              'parameters':{'boundary':'contract','consumer_boundary':'production','contract':'api/openapi.json','client':'web/angular/src/app/health.ts','type_file':'web/angular/src/app/health-response.ts','type_name':'HealthResponse','observations':'.harness-gate/runtime/http-observations.json','artifact_subdir':'frontend-api','relationship':'frontend-api','consumer':'frontend','consumer_source_root':'web/angular/src','exclude':p['exclude']}}
    files=['api/openapi.json','web/angular/src/app/health.ts','web/angular/src/app/health-response.ts','apps/server/src/lib.rs','apps/server/src/main.rs','Cargo.toml','Cargo.lock','apps/server/Cargo.toml']
    contract['parameters']['receipt']={'schema':'http-json-capture/v1','context':context,'inputs':{name:sha((root/name).read_bytes()) for name in files},'baseline':baseline,'observations_sha256':sha(observation_path.read_bytes()),'binary_sha256':binary_hash,'consumer_sources':{f['path']:f['sha256'] for f in discovery['sources']}}
    contract['parameters']['subjects']=node(HTTP,'p.discover(q)',contract)['subjects']
    requests={'backend':backend,'frontend':frontend,'frontend-api':contract}
    # Re-discover Rust subjects against the combined source tree, not old paths.
    import sys
    sys.path.insert(0,str(RUST));import plugin as rust
    backend['parameters']['subjects']=rust.discover(backend)
    identities={'backend':rust.series(backend),'frontend':node(TS,'p.series(q,q.parameters.receipt)',frontend),'frontend-api':node(HTTP,'p.series(q)',contract)}
    write(run/'requests.json',requests);write(run/'series.json',identities)
    return requests,identities
