import json
from pathlib import Path
import socket
import tempfile
import time
import unittest
from unittest.mock import MagicMock,patch

import replay


class ReplayTests(unittest.TestCase):
    def test_fixed_workspace_nonce_is_single_use_and_bad_claims_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory).resolve();run=base/'r';run.mkdir();root=base/'w'
            runtime=root/'.harness-gate/runtime';runtime.mkdir(parents=True)
            value={'nonce':'one','invocation_id':'run','step_id':'backend','config_digest':'digest'}
            (runtime/'backend-request.json').write_text(json.dumps(value))
            (run/'nonce.sock').touch()
            with replay.broker(run,base/'ledger',workspace=root):
                for content,expected in [(json.dumps(value).encode()+b'\n',True),(json.dumps(value).encode()+b'\n',False),(b'{"nonce":"wrong"}\n',False),(b'not-json\n',False),(b'',False)]:
                    with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as client:
                        client.settimeout(10);client.connect(str(run/'nonce.sock'))
                        if content:client.sendall(content)
                        else:client.shutdown(socket.SHUT_WR)
                        self.assertEqual(json.loads(client.makefile().readline())['accepted'],expected)
                time.sleep(.25)
            self.assertFalse((run/'nonce.sock').exists())
            self.assertEqual(len(list((base/'ledger').glob('nonce-*.json'))),1)

    def test_closed_listener_and_disconnected_client_release_broker(self):
        native_socket=socket.socket
        for mode in ('listener','client'):
            with tempfile.TemporaryDirectory() as directory:
                base=Path(directory).resolve();run=base/'r';run.mkdir()
                (run/'workspace/.harness-gate/runtime').mkdir(parents=True)
                real=native_socket(socket.AF_UNIX,socket.SOCK_STREAM)
                wrapper=MagicMock(wraps=real)
                if mode=='listener':wrapper.accept.side_effect=OSError('listener gone')
                else:
                    connection=MagicMock()
                    connection.__enter__.return_value=connection
                    connection.recv.return_value=b'{}\n'
                    connection.sendall.side_effect=OSError('client gone')
                    wrapper.accept.side_effect=[(connection,None),OSError('listener gone')]
                with patch.object(replay.socket,'socket',return_value=wrapper):
                    with replay.broker(run,base/'ledger'):time.sleep(.05)
                self.assertFalse((run/'nonce.sock').exists())


if __name__=='__main__':unittest.main()
