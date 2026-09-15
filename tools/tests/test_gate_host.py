import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HOST=Path(__file__).parents[1]/'quality-host'
sys.path.insert(0,str(HOST))
from isolation import command
from signing import configuration_files,provision

class HostTrust(unittest.TestCase):
    def test_policy_change_rejected_before_key_creation(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)/'repository';root.mkdir()
            names=['.harness-gate/flow.toml','.harness-gate/quality.toml']+[f'.harness-gate/packs/{c}/policy.json' for c in ('backend','frontend','frontend-api')]
            for name in names:
                p=root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text('approved')
            approved=configuration_files(root)
            (root/names[-1]).write_text('weakened policy')
            keys=Path(tmp)/'keys'
            with self.assertRaisesRegex(ValueError,'host-approved policy'):
                provision(Path(tmp)/'run',root,{}, {},approved,keys)
            self.assertFalse(keys.exists())

    def test_test_namespace_cannot_read_host_key_or_write_policy(self):
        with tempfile.TemporaryDirectory() as tmp:
            base=Path(tmp);repo=base/'repository';repo.mkdir();run=base/'run';run.mkdir()
            policy=repo/'policy.json';policy.write_text('approved')
            secret=base/'host-key';secret.write_text('canary-not-a-real-key')
            script=f'''import json\nfrom pathlib import Path\nr={{"host_key_visible":Path({str(secret)!r}).exists()}}\ntry:\n Path({str(policy)!r}).write_text("weakened")\n r["policy_writable"]=True\nexcept OSError:\n r["policy_writable"]=False\nprint(json.dumps(r))\n'''
            args=command(['python3','-c',script],run=run,repository=repo,plugins=Path('/home/gem/.local/share/harness-gate'))
            result=subprocess.run(args,capture_output=True,text=True,check=True)
            self.assertEqual(json.loads(result.stdout),{'host_key_visible':False,'policy_writable':False})
            self.assertEqual(policy.read_text(),'approved')

class DurableReplay(unittest.TestCase):
    def test_claim_survives_broker_restart(self):
        import socket
        from replay import broker
        with tempfile.TemporaryDirectory() as tmp:
            base=Path(tmp);run=base/'run';runtime=run/'workspace/.harness-gate/runtime';runtime.mkdir(parents=True)
            request={'nonce':'test-nonce','invocation_id':'run','step_id':'backend','config_digest':'digest'}
            (runtime/'backend-request.json').write_text(json.dumps(request))
            ledger=base/'host-ledger'
            def claim(value):
                with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as connection:
                    connection.connect(str(run/'nonce.sock'));connection.sendall((json.dumps(value)+'\n').encode())
                    return json.loads(connection.makefile().readline())
            with broker(run,ledger):
                self.assertFalse(claim(request|{'nonce':'unknown'})['accepted'])
                self.assertTrue(claim(request)['accepted'])
            with broker(run,ledger):
                answer=claim(request)
                self.assertFalse(answer['accepted']);self.assertEqual(answer['reason'],'nonce has already been used')

if __name__=='__main__': unittest.main()
