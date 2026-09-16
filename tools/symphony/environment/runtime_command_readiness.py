"""Fixed host-owned Runtime command environment check, never product acceptance."""
import hashlib
import json
from pathlib import Path
import socket
import time
import tomllib
import uuid
from preflight import execute, ROOT

BASE = Path(__file__).parent


def probe():
    # Resolve outside the command namespace, as in the product preparation probe.
    addresses = socket.getaddrinfo('index.crates.io', 443, type=socket.SOCK_STREAM)
    config = dict(allowed_url='https://index.crates.io/config.json',
                  denied_url='https://example.com', direct_addresses=addresses)
    command = '''import json,sys,subprocess
from pathlib import Path
sys.path.insert(0,'/opt/symphony-env')
from runtime_smoke import isolation
sys.path.insert(0,'/opt/symphony-env/reviewed-preparation')
from sandbox_probe import network
proof=isolation(Path.cwd())
proof['codex']=subprocess.check_output(['codex','--version'],text=True).strip()
assert proof['codex']=='codex-cli 0.154.0'
home=Path('/home/gem/.local/share/codexsymphony/codex-home')
assert not (home/'auth.json').exists()
proof['shared_auth_absent']=True
result=subprocess.run(['/bin/true'],capture_output=True,text=True,timeout=5)
assert result.returncode==0
proof['command_exit']=result.returncode
proof['network']=network(json.loads(sys.argv[1]))
print(json.dumps(proof))
'''
    result = execute(['python3', '-c', command, json.dumps(config)],
                     timeout=60, readiness=True, runtime=True)
    execution = result['command_exec']['result']
    if execution['exitCode'] != 0:
        raise RuntimeError('Runtime command environment failed: '+execution['stderr'][-2000:])
    sample = json.loads(execution['stdout'])
    network = result['requirements']['requirements']['network']
    configured = tomllib.loads((BASE/'requirements.toml').read_text())['experimental_network']
    expected = sorted(k for k, v in configured['domains'].items() if v == 'allow')
    if network['enabled'] is not True or sorted(network['allowedDomains']) != expected:
        raise ValueError('Runtime effective network differs from deployed policy')
    if not all(sample['network'][key] for key in
               ('allowed_probe', 'denied_probe', 'direct_connection_rejected')):
        raise ValueError('Runtime network boundary not established: '+json.dumps(sample['network']))
    proof = dict(ok=True, status='PASS', checked_at=time.time(), sample_id=uuid.uuid4().hex,
                 workspace=str(ROOT), mode='host-launched-isolated-runtime', model_calls=0,
                 product_runtime_acceptance=False, nested_runtime_supported=False,
                 command_exec=execution, sample=sample, network=network,
                 source_sha=__import__('subprocess').check_output(
                     ['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip(),
                 source_hashes={name: hashlib.sha256((BASE/name).read_bytes()).hexdigest()
                                for name in ('preflight.py', 'requirements.toml',
                                             'client/runtime_smoke.py',
                                             'client/reviewed-preparation/sandbox_probe.py')},
                 probe_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest())
    destination = BASE/'client/runtime-command-readiness.json'
    pending = destination.with_suffix('.new')
    pending.write_text(json.dumps(proof, indent=2)+'\n')
    pending.replace(destination)
    return proof


if __name__ == '__main__':
    print(json.dumps(probe(), indent=2))
