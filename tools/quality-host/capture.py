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
RUST = PLUGIN_ROOT / 'rust-source/0.1.0-rc.3'
TS = PLUGIN_ROOT / 'typescript/0.1.0-rc.4/node_modules/@harness-gate/typescript-collector'
HTTP = PLUGIN_ROOT / 'http-contract/0.1.0-rc.4/node_modules/@harness-gate/http-json-contract-collector'

def sha(data): return hashlib.sha256(data).hexdigest()
def write(path, data): path.write_text(json.dumps(data, indent=2) + '\n')
def load(path): return json.loads(path.read_text())

def run_logged(run, label, args, **kwargs):
    with (run / (label + '.stdout')).open('wb') as out, (run / (label + '.stderr')).open('wb') as err:
        subprocess.run(args, stdout=out, stderr=err, check=True, **kwargs)

def node(plugin, operation, request):
    script = "const p=require(process.argv[1]);const q=JSON.parse(require('fs').readFileSync(0,'utf8'));console.log(JSON.stringify(" + operation + "));"
    return json.loads(subprocess.check_output(['node','-e',script,str(plugin / 'protocol.cjs')],input=json.dumps(request),text=True))

def database(run, purpose=""):
    name = 'codexsymphony-gate-' + run.name[-16:] + purpose
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

def wait_http_address(server, output, timeout=20):
    """Drain OS pipe chunks; buffered readline can hide subsequent ready lines."""
    import selectors
    pending=b''
    deadline=time.monotonic()+timeout
    with selectors.DefaultSelector() as selector:
        selector.register(server.stdout,selectors.EVENT_READ)
        while time.monotonic()<deadline:
            if selector.select(min(.5,max(0,deadline-time.monotonic()))):
                chunk=os.read(server.stdout.fileno(),65536)
                if not chunk:
                    raise RuntimeError('server exited before readiness; inspect http-server.stderr')
                output.write(chunk);output.flush();pending+=chunk
                while b'\n' in pending:
                    line,pending=pending.split(b'\n',1)
                    if b'API listening at http://' in line:
                        return line.split(b'API listening at http://',1)[1].decode().strip()
                if len(pending)>1024*1024:
                    raise RuntimeError('server startup line too large')
            if server.poll() is not None:
                raise RuntimeError('server exited before readiness; inspect http-server.stderr')
    raise RuntimeError('server readiness timeout; inspect http-server.stdout and http-server.stderr')

def prepare_http_fixture(run, repository, container):
    """Project-owned SQL runs only inside this capture's disposable database."""
    fixture=repository/'api/capture-fixture.sql'
    if not fixture.exists() and not fixture.is_symlink(): return
    if fixture.is_symlink() or not fixture.resolve().is_relative_to(repository.resolve()):
        raise ValueError('HTTP fixture must be a repository-owned regular file')
    if not fixture.is_file() or fixture.stat().st_size>1024*1024:
        raise ValueError('HTTP fixture must be a SQL file of at most 1 MiB')
    # No host shell or developer-selected database/container. Even psql meta
    # commands execute inside the disposable container, never on the signer host.
    args=['docker','exec','--interactive','--user','postgres',container,
          'psql','--no-psqlrc','--single-transaction','--set','ON_ERROR_STOP=1',
          '--username=gate_test','--dbname=gate_test']
    run_logged(run,'http-fixture',args,input=fixture.read_bytes(),timeout=30)

def capture_http(run, repository, container, url):
    binary=run/'target/debug/codexsymphony-server'
    args=command(['cargo','build','--locked','--bin','codexsymphony-server'],run=run,repository=repository,plugins=PLUGIN_ROOT,writable=[run/'probes',run/'target'],environment={'TEST_DATABASE_URL':url})
    run_logged(run,'http-build',args)
    # Runtime state belongs in the isolated, retained per-run temporary mount.
    # The source checkout must stay read-only even when the service needs storage.
    args=command([binary],run=run,repository=repository,plugins=PLUGIN_ROOT,readonly=[binary],environment={'DATABASE_URL':url,'BIND_ADDRESS':'127.0.0.1:0','RUST_LOG':'info','EXECUTION_DIRECTORY':'/tmp/codexsymphony-execution'})
    # Server stdout contains the actual listener address, never a guessed port.
    stderr=(run/'http-server.stderr').open('wb')
    try:
        server=subprocess.Popen(args,stdout=subprocess.PIPE,stderr=stderr,start_new_session=True)
    except BaseException:
        stderr.close();raise
    try:
        with (run/'http-server.stdout').open('wb') as output:
            address=wait_http_address(server,output)
        prepare_http_fixture(run,repository,container)
        from http_scenarios import capture
        scenario_path=repository/'api/capture-scenarios.json'
        scenarios=load(scenario_path) if scenario_path.exists() else []
        observations=capture(address,load(repository/'api/openapi.json'),scenarios)

        for status in (200,503):
            if status==503: subprocess.run(['docker','stop','--time','1',container],check=True,capture_output=True)
            request=urllib.request.Request('http://'+address+'/api/health')
            try: response=urllib.request.urlopen(request,timeout=8)
            except urllib.error.HTTPError as error: response=error
            with response:
                if response.status!=status: raise RuntimeError(f'expected HTTP {status}, got {response.status}')
                observations.append({'method':'GET','path':'/api/health','status':response.status,'content_type':response.headers['Content-Type'],'body':json.load(response)})
        write(run/'http-observations.json',observations)
        shutil.copyfile(binary,run/'http-server')
        return observations,sha(binary.read_bytes())
    finally:
        stderr.close()
        try: os.killpg(server.pid,signal.SIGTERM)
        except ProcessLookupError: pass
        try: server.wait(timeout=5)
        except subprocess.TimeoutExpired: os.killpg(server.pid,signal.SIGKILL);server.wait()

def captures(run, repository, root, context, baseline):
    for directory in ('probes','target'): (run/directory).mkdir()
    container,url=database(run)
    try:
        args=command(['python3',RUST/'capture.py','--repository',repository,'--output',run/'probes/backend','--target-dir',run/'target','--source-root','apps/server/src','--input','Cargo.toml','--input','Cargo.lock','--input','apps','--input','migrations','--manifest','apps/server/Cargo.toml'],run=run,repository=repository,plugins=PLUGIN_ROOT,writable=[run/'probes',run/'target'],environment={'TEST_DATABASE_URL':url})
        run_logged(run,'backend-capture',args)
        args=command(['node',repository/'web/angular/tools/probe-typescript-risk.cjs'],run=run,repository=repository,plugins=PLUGIN_ROOT,writable=[run/'probes'],environment={'HARNESS_GATE_TYPESCRIPT_PLUGIN':str(TS)})
        run_logged(run,'frontend-capture',args)
        # Contract fixtures must not inherit rows left by backend tests.
        http_container,http_url=database(run,purpose='-http')
        try:
            observations,binary_hash=capture_http(run,repository,http_container,http_url)
        finally:
            subprocess.run(['docker','rm','--force',http_container],check=True,capture_output=True)
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
    contract={'schema':'harness-collector-request/v1','project':'codexsymphony','component':'backend','collector':{'name':'http-json-contract','version':'0.1.0-rc.4'},'context':context,'workspace_root':str(root),'output_root':str(output),'requested_capabilities':['contract.breaking_changes','contract.client_drift','contract.compatible'],
              'parameters':{'boundary':'contract','consumer_boundary':'production','contract':'api/openapi.json','client':'web/angular/src/app/health.ts','type_file':'web/angular/src/app/health-response.ts','type_name':'HealthResponse','observations':'.harness-gate/runtime/http-observations.json','artifact_subdir':'frontend-api','relationship':'frontend-api','consumer':'frontend','consumer_source_root':'web/angular/src','exclude':p['exclude']}}
    files=['api/openapi.json','web/angular/src/app/health.ts','web/angular/src/app/health-response.ts','apps/server/src/lib.rs','apps/server/src/main.rs','Cargo.toml','Cargo.lock','apps/server/Cargo.toml']
    for name in ('api/capture-scenarios.json','api/capture-fixture.sql'):
        if (root/name).exists(): files.append(name)
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
