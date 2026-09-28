"""Capture three real producers before any signing key exists."""
import hashlib
import contextlib
import json
import os
from pathlib import Path

import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import environment_contract as contract
import shutil
import signal
import subprocess
import time
import urllib.error
import urllib.request
from isolation import command
from timing import phase

PLUGIN_ROOT = Path('/home/gem/.local/share/harness-gate')
COLLECTORS=contract.load()['collectors']
RUST = PLUGIN_ROOT / 'rust-source' / COLLECTORS['rust_source']
TS = PLUGIN_ROOT / 'typescript' / COLLECTORS['typescript'] / 'node_modules/@harness-gate/typescript-collector'
HTTP = PLUGIN_ROOT / 'http-contract' / COLLECTORS['http_contract'] / 'node_modules/@harness-gate/http-json-contract-collector'

def sha(data): return hashlib.sha256(data).hexdigest()
def write(path, data): path.write_text(json.dumps(data, indent=2) + '\n')
def load(path): return json.loads(path.read_text())

def run_logged(run, label, args, **kwargs):
    with phase(run, label), (run / (label + '.stdout')).open('wb') as out, (run / (label + '.stderr')).open('wb') as err:
        subprocess.run(args, stdout=out, stderr=err, check=True, **kwargs)

def node(plugin, operation, request):
    script = "const p=require(process.argv[1]);const q=JSON.parse(require('fs').readFileSync(0,'utf8'));console.log(JSON.stringify(" + operation + "));"
    return json.loads(subprocess.check_output(['node','-e',script,str(plugin / 'protocol.cjs')],input=json.dumps(request),text=True))

def database(run, purpose="", repository=None):
    import database_pool
    return database_pool.acquire(run, purpose, repository)


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
    from http_auth import load_adapter, prepare, bootstrap
    from http_tls import TLSCapture
    adapter = load_adapter(repository)
    with contextlib.ExitStack() as stack:
        tls = stack.enter_context(TLSCapture()) if adapter else None
        variables, auth_env = prepare(adapter, run, tls.origin) if adapter else ({}, {})
        return capture_http_session(run, repository, container, url, adapter, tls, variables, auth_env)


def capture_http_session(run, repository, container, url, adapter, tls, variables, auth_env):
    import bounded_layout
    target = bounded_layout.target('normal')
    binary = target / 'debug/codexsymphony-server'
    args=command(['cargo','build','--locked','--bin','codexsymphony-server'],run=run,repository=repository,plugins=PLUGIN_ROOT,writable=[run/'probes',target],compiler_target=target,environment={'TEST_DATABASE_URL':url})
    run_logged(run,'http-build',args)
    environment={'DATABASE_URL':url,'BIND_ADDRESS':'127.0.0.1:0','RUST_LOG':'info','EXECUTION_DIRECTORY':'/tmp/codexsymphony-execution', **auth_env}
    args=command([binary],run=run,repository=repository,plugins=PLUGIN_ROOT,readonly=[binary],environment=environment)
    stderr=open(os.devnull, 'wb') if adapter else (run/'http-server.stderr').open('wb')
    try:
        server=subprocess.Popen(args,stdout=subprocess.PIPE,stderr=stderr,start_new_session=True)
    except BaseException:
        stderr.close();raise
    try:
        with (open(os.devnull, 'wb') if adapter else (run/'http-server.stdout').open('wb')) as output:
            address=wait_http_address(server,output)
        observations=capture_http_observations(run, repository, container, binary, environment, address, adapter, tls, variables)
        write(run/'http-observations.json',observations)
        shutil.copyfile(binary,run/'http-server')
        return observations,sha(binary.read_bytes())
    finally:
        stderr.close()
        try: os.killpg(server.pid,signal.SIGTERM)
        except ProcessLookupError: pass
        try: server.wait(timeout=5)
        except subprocess.TimeoutExpired: os.killpg(server.pid,signal.SIGKILL);server.wait()

def capture_http_observations(run, repository, container, binary, environment, address, adapter, tls, variables):
    prepare_http_fixture(run,repository,container)
    from http_scenarios import capture
    from http_auth import bootstrap
    if adapter:
        bootstrap(adapter, variables, binary=binary, run=run, repository=repository,
                  plugins=PLUGIN_ROOT, environment=environment)
        tls.start(address)
    scenario_path=repository/'api/capture-scenarios.json'
    scenarios=load(scenario_path) if scenario_path.exists() else []
    setup=adapter['login'] if adapter else []
    from http_contract import ContractCapture
    bridge=ContractCapture(variables) if adapter else None
    observations=capture(address,load(repository/'api/openapi.json'),setup+scenarios,
                         tls=tls,variables=variables,setup_count=len(setup),
                         default_headers=adapter['headers'] if adapter else None, observer=bridge)
    capture_health(address, container, tls, observations)
    if bridge:
        for observation in observations: bridge.observe(observation)
        observations, validation=bridge.seal(load(repository/'api/openapi.json'))
        write(run/'http-auth-validation.json',validation)
    return observations


def capture_health(address, container, tls, observations):
    for status in (200,503):
        if status==503: subprocess.run(['docker','stop','--time','1',container],check=True,capture_output=True)
        request=urllib.request.Request((tls.origin if tls else 'http://'+address)+'/api/health')
        handlers=[urllib.request.ProxyHandler({})]
        from http_scenarios import NoRedirect
        handlers.append(NoRedirect())
        if tls: handlers.append(urllib.request.HTTPSHandler(context=tls.context))
        try: response=urllib.request.build_opener(*handlers).open(request,timeout=8)
        except urllib.error.HTTPError as error: response=error
        with response:
            if response.status!=status: raise RuntimeError(f'expected HTTP {status}, got {response.status}')
            observations.append({'method':'GET','path':'/api/health','status':response.status,'content_type':response.headers['Content-Type'],'body':json.load(response)})

def capture_producers(run, repository, root):
    import manual_capture
    import database_pool
    manual_capture.backend(run, root)
    import manual_measure
    result = manual_measure.measure(run, root)
    manual_measure.register(run)
    write(run / 'measurement-summary.json', result)
    if result['coverage_and_crap'] != 'PASS':
        raise ValueError('backend coverage or CRAP failed before checks')
    modules = root / 'web/angular/node_modules'
    modules.mkdir(exist_ok=True)
    args=command(['node',root/'web/angular/tools/probe-typescript-risk.cjs'],run=run,repository=root,plugins=PLUGIN_ROOT,writable=[run/'probes'],mounts=[(repository/'web/angular/node_modules',modules)],environment={'HARNESS_GATE_TYPESCRIPT_PLUGIN':str(TS)})
    run_logged(run,'frontend-capture',args)
    container,url=database(run,purpose='-http',repository=root)
    try:
        return capture_http(run,root,container,url)
    finally:
        database_pool.release(container)


def bind_frontend(run, root, output, runtime, context):
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
    return frontend, discovery


def bind_contract(run, root, output, runtime, context, baseline, observations, binary_hash, frontend, discovery):
    observation_path=runtime/'http-observations.json';write(observation_path,observations)
    contract={'schema':'harness-collector-request/v1','project':'codexsymphony','component':'backend','collector':{'name':'http-json-contract','version':'0.1.0-rc.5'},'context':context,'workspace_root':str(root),'output_root':str(output),'requested_capabilities':['contract.breaking_changes','contract.client_drift','contract.compatible'],
              'parameters':{'boundary':'contract','consumer_boundary':'production','contract':'api/openapi.json','client':'web/angular/src/app/health.ts','type_file':'web/angular/src/app/health-response.ts','type_name':'HealthResponse','observations':'.harness-gate/runtime/http-observations.json','artifact_subdir':'frontend-api','relationship':'frontend-api','consumer':'frontend','consumer_source_root':'web/angular/src','exclude':frontend['parameters']['exclude']}}
    files=['api/openapi.json','web/angular/src/app/health.ts','web/angular/src/app/health-response.ts','apps/server/src/lib.rs','apps/server/src/main.rs','Cargo.toml','Cargo.lock','apps/server/Cargo.toml']
    for name in ('api/capture-scenarios.json','api/capture-fixture.sql','api/capture-auth.json'):
        if (root/name).exists(): files.append(name)
    contract['parameters']['receipt']={'schema':'http-json-capture/v1','context':context,'inputs':{name:sha((root/name).read_bytes()) for name in files},'baseline':baseline,'observations_sha256':sha(observation_path.read_bytes()),'binary_sha256':binary_hash,'consumer_sources':{f['path']:f['sha256'] for f in discovery['sources']}}
    if (run/'http-auth-validation.json').exists():
        contract['parameters']['receipt']['auth_validation']=load(run/'http-auth-validation.json')
    contract['parameters']['subjects']=node(HTTP,'p.discover(q)',contract)['subjects']
    return contract


def captures(run, repository, root, context, baseline):
    observations,binary_hash=capture_producers(run,repository,root)
    runtime=root/'.harness-gate/runtime'; runtime.mkdir(parents=True,exist_ok=True)
    output=root/'.harness-gate/reports/evidence';output.mkdir(parents=True,exist_ok=True)
    backend=load(run/'probes/backend/bundle.json')['request']
    backend.update(workspace_root=str(root),output_root=str(output),context=context)
    backend['parameters']['receipt']['context']=context
    frontend,discovery=bind_frontend(run,root,output,runtime,context)
    contract=bind_contract(run,root,output,runtime,context,baseline,observations,binary_hash,frontend,discovery)
    requests={'backend':backend,'frontend':frontend,'frontend-api':contract}
    # Subject IDs use relative source paths and source hashes, not snapshot
    # directories or commit IDs. Validate the relocated inventory; collect()
    # still replays all native counters and checks the exact subject list.
    import sys
    sys.path.insert(0,str(RUST));import plugin as rust
    if rust.inventory(backend) != backend['parameters']['receipt']['sources']:
        raise ValueError('relocated Rust source inventory differs from capture')
    identities={'backend':rust.series(backend),'frontend':node(TS,'p.series(q,q.parameters.receipt)',frontend),'frontend-api':node(HTTP,'p.series(q)',contract)}
    write(run/'requests.json',requests);write(run/'series.json',identities)
    return requests,identities
