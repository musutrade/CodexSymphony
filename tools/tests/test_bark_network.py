"""Real Linux network namespace proof, without real keys or public traffic."""
import json
from pathlib import Path
import socket
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]


class NotifierNetworkTests(unittest.TestCase):
    def test_private_notifier_loopback_is_unreachable_from_agent_network(self):
        with socket.socket() as probe:
            probe.bind(('127.0.0.1',0))
            port = probe.getsockname()[1]
        code = '''import http.server, json, runpy, sys, threading
bark=runpy.run_path(sys.argv[1])
received=[]
class Receiver(http.server.BaseHTTPRequestHandler):
 def log_message(self,*args): pass
 def do_POST(self):
  received.append((self.path,json.loads(self.rfile.read(int(self.headers['Content-Length'])))))
  self.send_response(200); self.end_headers(); self.wfile.write(b'{"code":200}')
server=http.server.HTTPServer(('127.0.0.1',int(sys.argv[2])),Receiver)
threading.Thread(target=server.serve_forever,daemon=True).start()
print('ready',flush=True)
sys.stdin.readline()
cfg={'endpoint':'http://127.0.0.1:'+sys.argv[2]+'/push','device_key':'synthetic-namespace-only','application_origin':'https://platform.example.invalid'}
assert bark['send'](cfg,{'kind':'question','requirement_id':74})=='accepted'
assert received[0][0]=='/push'
assert received[0][1]['device_key']=='synthetic-namespace-only'
print('isolated JSON POST accepted',flush=True)
server.shutdown()
'''
        command = ['/usr/bin/bwrap','--unshare-user','--unshare-net','--unshare-pid',
                   '--ro-bind','/','/','--proc','/proc','--', '/usr/bin/python3','-I','-c',code,
                   str(ROOT/'apps/notifier/bark.py'),str(port)]
        child = subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        try:
            self.assertEqual(child.stdout.readline().strip(),'ready')
            with self.assertRaises(OSError):
                socket.create_connection(('127.0.0.1',port),timeout=.5)
            stdout,stderr = child.communicate('\n',timeout=10)
            self.assertEqual(child.returncode,0,stderr)
            self.assertIn('isolated JSON POST accepted',stdout)
            self.assertNotIn('synthetic-namespace-only',stdout+stderr)
        finally:
            if child.poll() is None:
                child.kill(); child.communicate()


if __name__ == '__main__':
    unittest.main()
