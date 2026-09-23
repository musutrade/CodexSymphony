"""Validate live auth observations in memory before credential-safe retention.

Both raw and masked values must satisfy the pinned rc.5 validator.
Masks are evidence placeholders, never claims about the actual wire values.
"""
import hashlib
import json
import subprocess
from pathlib import Path

PLUGIN = Path('/home/gem/.local/share/harness-gate/http-contract/0.1.0-rc.5/node_modules/@harness-gate/http-json-contract-collector')
# Exact scalar field names only. Business containers such as authorizations,
# session metadata and token budgets do not make their descendants credentials.
CREDENTIAL_FIELDS = frozenset({
    'password', 'currentpassword', 'newpassword', 'oldpassword', 'secret',
    'clientsecret', 'token', 'accesstoken', 'refreshtoken', 'idtoken', 'apikey',
    'csrf', 'csrftoken', 'csrfproof', 'sessiontoken', 'sessioncookie', 'cookie',
    'authorization',
})

PIN_FILES = ('measure.cjs', 'schema.cjs', 'clients.cjs', 'legacy-measure.cjs',
             'protocol.cjs', 'project.cjs', 'cli.cjs', 'strict-json.cjs', 'npm-shrinkwrap.json')
SCRIPT = r'''
const fs=require('fs'),assert=require('assert/strict');
try {
 const M=require(process.argv[1]+'/measure.cjs'),S=require(process.argv[1]+'/schema.cjs');
 const q=JSON.parse(fs.readFileSync(0,'utf8')),ops=S.operations(q.spec);
 // Validate even setup observations superseded by an explicit business scenario.
 for(const o of q.all){
  const op=ops[o.method+' '+(o.operation_path||o.path)];assert(op);
  const s=op.responses[String(o.status)];assert(s);
  if(s.type==='void'){assert.equal(o.body,null);assert.equal(o.content_type,'');}
  else {assert(o.content_type.toLowerCase().startsWith('application/json'));S.validate(s,o.body);}
  if(op.request&&o.status>=200&&o.status<300){
   assert(o.request_body!==undefined||!op.request.required);
   if(o.request_body!==undefined)S.validate(op.request.schema,o.request_body);
  }
 }
 const first=Object.values(ops)[0],name=Object.keys(ops).length===1?'HealthResponse':S.name(first);
 const client="import {HttpClient} from '@angular/common/http'; import {inject} from '@angular/core'; class Probe {http=inject(HttpClient); read(){return this.http."+first.method.toLowerCase()+"<"+name+">('"+first.path+"');}}";
 const types=M.generate(q.spec,'HealthResponse','check');
 const run=rows=>M.measure(q.spec,q.spec,types,client,'HealthResponse',rows);
 const raw=run(q.raw),safe=run(q.safe);assert.deepEqual(raw,safe);
 process.stdout.write(JSON.stringify({validated:true,metrics:safe}));
} catch (_) {
 // Assertion details can contain passwords or response proofs. Never emit them.
 process.stderr.write('credential-safe contract validation failed\n');process.exitCode=1;
}
'''


def digest(data):
    return hashlib.sha256(data).hexdigest()


def strings(value):
    if isinstance(value, str):
        if value: yield value
    elif isinstance(value, dict):
        for item in value.values(): yield from strings(item)
    elif isinstance(value, list):
        for item in value: yield from strings(item)


class ContractCapture:
    def __init__(self, variables):
        self.secrets = set(strings({k: v for k, v in variables.items() if k != 'origin'}))
        self.rows = []

    def observe(self, observation, *, setup=False, record=True, cookies=(), credentials=()):
        self.secrets.update(value for value in cookies if value)
        self.secrets.update(value for value in credentials if isinstance(value, str) and value)
        def sensitive(value):
            if isinstance(value, dict):
                for key, item in value.items():
                    field = key.replace('_', '').replace('-', '').lower()
                    if field in CREDENTIAL_FIELDS and isinstance(item, str) and item:
                        self.secrets.add(item)
                    sensitive(item)
            elif isinstance(value, list):
                for item in value: sensitive(item)
        sensitive(observation)
        self.rows.append((observation, setup, record))

    def seal(self, spec):
        # Explicit business observations take precedence over setup for the same
        # variant. Duplicate explicit observations still fail the pinned measure.
        explicit = {(o['method'], o.get('operation_path', o['path']), o['status'])
                    for o, setup, record in self.rows if not setup and record}
        selected, setup_seen = [], set()
        for o, setup, record in self.rows:
            key = (o['method'], o.get('operation_path', o['path']), o['status'])
            if setup:
                if key in explicit or key in setup_seen: continue
                setup_seen.add(key)
            elif not record:
                continue
            selected.append(o)
        secrets = sorted(self.secrets, key=len, reverse=True)
        def mask(value):
            if isinstance(value, str):
                for secret in secrets:
                    value = value.replace(secret, 'x' * len(secret))
                return value
            if isinstance(value, dict): return {mask(k): mask(v) for k, v in value.items()}
            if isinstance(value, list): return [mask(v) for v in value]
            return value
        safe = mask(selected)
        # Metadata must remain an exact wire identity; never relabel a route/status.
        for raw, row in zip(selected, safe):
            if {k:v for k,v in raw.items() if k not in ('body','request_body')} != {k:v for k,v in row.items() if k not in ('body','request_body')}:
                raise ValueError('credential in observation metadata')
        payload = {'spec': spec, 'all': [o for o, _, _ in self.rows], 'raw': selected, 'safe': safe}
        result = subprocess.run(['node', '-e', SCRIPT, str(PLUGIN)], input=json.dumps(payload),
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=60)
        if result.returncode:
            raise ValueError('credential-safe contract validation failed; raw or masked schema/coverage rejected')
        validation = json.loads(result.stdout)
        encoded = json.dumps(safe, indent=2) + '\n'
        if any(secret in encoded for secret in secrets):
            raise ValueError('credential sentinel remains in retained observations')
        receipt = {'schema': 'http-auth-contract-bridge/v1', 'raw_validated_in_memory': True,
                   'retained_values': 'credential-masked-and-revalidated',
                   'observations_sha256': digest(encoded.encode()),
                   'contract_sha256': digest(json.dumps(spec, sort_keys=True).encode()),
                   'bridge_sha256': digest(Path(__file__).read_bytes()),
                   'collector_files': {name: digest((PLUGIN/name).read_bytes()) for name in PIN_FILES},
                   'variants': [f"{o['method']} {o.get('operation_path', o['path'])}:{o['status']}" for o in safe],
                   'validation': validation}
        self.rows.clear()
        return safe, receipt
