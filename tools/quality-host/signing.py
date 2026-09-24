"""Host-only request signing; private material never enters collector mounts."""
import base64
import json
import os
from pathlib import Path
import subprocess
import time
from capture import RUST, TS, HTTP, sha, write, run_logged, contract

CORE=Path.home()/'.local/share/harness-gate/versions'/('v'+contract.load()['tools']['gate'])/'bin/harness-gate'

def canonical(value): return json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=False)

def configuration_files(root):
    names=['.harness-gate/flow.toml','.harness-gate/quality.toml']
    names += [f'.harness-gate/packs/{c}/{name}.json' for c in ('backend','frontend','frontend-api') for name in ('policy',)]
    return {name:sha((root/name).read_bytes()) for name in names}

def runtime_launcher(host, collector):
    import sys,shutil
    plugin={'backend':RUST,'frontend':TS,'frontend-api':HTTP}[collector]
    interpreter=Path(sys.executable if collector=='backend' else shutil.which('node')).resolve()
    roots=[plugin]
    if collector!='backend': roots.append(plugin.parent.parent/'typescript')
    pins={str(p):sha(p.read_bytes()) for base in roots for p in sorted(base.rglob('*')) if p.is_file() and '__pycache__' not in p.parts}
    pins[str(interpreter)]=sha(interpreter.read_bytes())
    entry=plugin/('plugin.py' if collector=='backend' else 'cli.cjs')
    launcher=host/(collector+'-collector')
    launcher.write_text('#!/usr/bin/python3\nimport os,sys,hashlib,json,socket\nfrom pathlib import Path\n'
                        +'pins='+repr(pins)+'\n'
                        +'for name, expected in pins.items():\n    if hashlib.sha256(Path(name).read_bytes()).hexdigest()!=expected: raise SystemExit("collector runtime changed")\n'
                        +'data=sys.stdin.buffer.read()\nrequest=json.loads(data)\nclaim={k:request[k] for k in ("nonce","invocation_id","step_id","config_digest")}\n'
                        +f'connection=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)\nconnection.settimeout(5)\nconnection.connect({str(host.parent / "nonce.sock")!r})\n'
                        +'connection.sendall((json.dumps(claim)+"\\n").encode())\nanswer=json.loads(connection.makefile().readline())\nconnection.close()\n'
                        +'if not answer["accepted"]: raise SystemExit(answer["reason"])\n'
                        +'fd=os.memfd_create("collector-request")\nos.write(fd,data)\nos.lseek(fd,0,0)\nos.dup2(fd,0)\nos.close(fd)\n'
                        +f'os.execv({str(interpreter)!r},[{str(interpreter)!r},{str(entry)!r},*sys.argv[1:]])\n')
    if collector in ('backend', 'frontend'):
        source=(Path(__file__).parent/'artifact_packaging.py').read_text()
        text=launcher.read_text()
        execute=f'os.execv({str(interpreter)!r},[{str(interpreter)!r},{str(entry)!r},*sys.argv[1:]])\n'
        replacement=(source+'\nimport subprocess\n'
                     +f'child=subprocess.run([{str(interpreter)!r},{str(entry)!r},*sys.argv[1:]],stdout=subprocess.PIPE)\n'
                     +'if child.returncode: raise SystemExit(child.returncode)\n'
                     +'response=json.loads(child.stdout)\n'
                     +'print(json.dumps(compact_artifacts(response,Path(request["artifact_root"]))))\n')
        if execute not in text: raise ValueError('missing collector execution boundary')
        launcher.write_text(text.replace(execute,replacement))
    launcher.chmod(0o700);return launcher

def provision(run, root, state, requests, expected_config, key_root):
    if configuration_files(root)!=expected_config: raise ValueError('configuration differs from host-approved policy')
    host=run/'signed';host.mkdir(exist_ok=True)
    runtime=root/'.harness-gate/runtime'
    for c in requests: write(runtime/(c+'-request.json'),{})
    state_path=runtime/(state['profile']+'-state.json')
    def pin():
        state['config_files']=expected_config|{f'.harness-gate/runtime/{c}-request.json':sha((runtime/(c+'-request.json')).read_bytes()) for c in requests}
        write(state_path,state)
    pin();compiled_path=host/'compiled.json'
    run_logged(run,'compile',[CORE,'quality','compile','--repository-root',root,'--state',state_path,'--output',compiled_path])
    compiled=json.loads(compiled_path.read_text())
    payload={'schema':'quality-collector-binding/v1','config_files':expected_config,'series':state['series'],'profile':state['profile']}
    payload.update({name:compiled[name] for name in ('project','policy','expected','selection','mappings','exceptions')})
    digest=sha(canonical(payload).encode())
    key_root.mkdir(mode=0o700,parents=True,exist_ok=True)
    key=key_root/(run.name+'.pem')
    prior=os.umask(0o077)
    try: subprocess.run(['openssl','genpkey','-algorithm','ED25519','-out',key],check=True,capture_output=True)
    finally: os.umask(prior)
    try:
        public=subprocess.check_output(['openssl','pkey','-in',key,'-pubout','-outform','DER'])
        write(runtime/'trusted-keys.json',[{'key_id':'codexsymphony-local-host','public_key':base64.b64encode(public[12:]).decode()}])
        for collector,measurement in requests.items():
            claims=sorted([{'subject':s['id'],'capability':m,'series':state['series'][collector]['id']} for s in measurement['parameters']['subjects'] for m in measurement['requested_capabilities']],key=lambda v:(v['subject'],v['capability'],v['series']))
            inner={'schema':'harness-project-collector-request/v1','project':measurement['project'],'collector':measurement['collector'],'context':measurement['context'],'workspace_root':str(root),'output_root':measurement['output_root'],'selection':compiled['selection'],'bindings':claims}
            schema={'backend':'rust-source-project-binding/v1','frontend':'typescript-project-collector-binding/v1','frontend-api':'http-json-project-collector-binding/v1'}[collector]
            binding=host/(collector+'-binding.json');write(binding,{'schema':schema,'input':inner,'config_digest':digest,'request':measurement})
            launcher=runtime_launcher(host,collector);now=int(time.time()*1000)
            request={'protocol_version':2,'result_schema_version':'1','adapter':measurement['collector']|{'executable':str(launcher),'source_digest':sha(launcher.read_bytes()),'signature':{'algorithm':'ed25519','key_id':'codexsymphony-local-host','value':''}},'invocation_id':state['expected']['run'],'step_id':collector,'timeout_ms':120000,'config_digest':digest,'artifact_root':measurement['output_root'],'nonce':os.urandom(16).hex(),'issued_at_ms':now,'expires_at_ms':now+900000,'args':['project','--binding',str(binding),'--binding-sha256',sha(binding.read_bytes())],'environment':{},'capabilities':{'network':[],'resources':[],'environment':[]},'input':inner}
            signed={'domain':'harness-gate/adapter-request/v2','protocol_version':2,'result_schema_version':'1','adapter':{k:request['adapter'][k] for k in ('name','version','executable','source_digest')}}
            signed['adapter']['signature']={'algorithm':'ed25519','key_id':'codexsymphony-local-host'}
            for name in ('invocation_id','step_id','timeout_ms','config_digest','artifact_root','nonce','issued_at_ms','expires_at_ms','args','environment','capabilities','input'):
                signed[name]=json.loads(canonical(request[name])) if name in ('environment','input') else request[name]
            sign_input=host/(collector+'-sign-input.json');sign_input.write_text(json.dumps(signed,separators=(',',':'),ensure_ascii=False))
            signature=subprocess.check_output(['openssl','pkeyutl','-sign','-rawin','-inkey',key,'-in',sign_input])
            request['adapter']['signature']['value']=base64.b64encode(signature).decode()
            write(runtime/(collector+'-request.json'),request)
    finally:
        key.unlink()
    pin();return state_path
